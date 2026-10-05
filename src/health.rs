//----------------------------------------------------------------------------
//
//  Player max-HP old-save runtime fix
//
//  实测 PlayerData：
//    0x18 u32 = 当前 HP
//    0x1c u32 = 实际最大 HP
//    0x20 u32 = 基础/缓存最大 HP（旧存档这里会停留在原版值）
//    0x44 u8  = HP 成长等级（1..11；11 = 10 条念珠串满级）
//
//  旧存档如果 0x20 还是原版值，就按等级算出目标最大 HP 并写回；
//  新存档/已修正存档 0x20 已经是目标值，则不二次乘算。
//----------------------------------------------------------------------------

use std::{fs, path::Path};

use crate::game;

const DEFAULT_MULTIPLIER: f32 = 2.0;
const BASE_HP: f32 = 320.0;
const HP_PER_LEVEL: f32 = 80.0;
const LEVEL_MIN: u8 = 1;
const LEVEL_MAX: u8 = 11;

const DEFAULT_LEVEL_OFFSET: usize = 0x44;
const DEFAULT_BASE_MAX_HP_OFFSET: usize = 0x20;
const DEFAULT_ACTUAL_MAX_HP_OFFSET: usize = 0x1c;
const DEFAULT_CURRENT_HP_OFFSET: usize = 0x18;

pub struct Health {
    enabled: bool,
    multiplier: f32,
    level_offset: usize,
    base_max_hp_offset: usize,
    actual_max_hp_offset: usize,
    current_hp_offset: usize,
    written_logs: u32,
}

impl Health {
    pub fn new(cfg: impl AsRef<Path>) -> Health {
        let mut health = Health {
            enabled: true,
            multiplier: DEFAULT_MULTIPLIER,
            level_offset: DEFAULT_LEVEL_OFFSET,
            base_max_hp_offset: DEFAULT_BASE_MAX_HP_OFFSET,
            actual_max_hp_offset: DEFAULT_ACTUAL_MAX_HP_OFFSET,
            current_hp_offset: DEFAULT_CURRENT_HP_OFFSET,
            written_logs: 0,
        };
        if let Ok(text) = fs::read_to_string(cfg) {
            health.load(&text);
        }
        log::warn!(
            "health settings: enabled={} multiplier={} offsets lv=0x{:x} base=0x{:x} actual=0x{:x} cur=0x{:x}",
            health.enabled,
            health.multiplier,
            health.level_offset,
            health.base_max_hp_offset,
            health.actual_max_hp_offset,
            health.current_hp_offset
        );
        health
    }

    fn load(&mut self, text: &str) {
        for line in text.lines() {
            let Some((key, value)) = split_kv(line) else {
                continue;
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "旧存档血量修正" | "health old save fix" => self.enabled = parse_bool(value),
                "血量倍率" | "血量修正倍率" | "hp multiplier" | "health multiplier" => {
                    if let Some(v) = parse_f32(value) {
                        self.multiplier = v.max(1.0);
                    }
                }
                "最大血量偏移" | "base max hp offset" => {
                    if let Some(v) = parse_usize(value) {
                        self.base_max_hp_offset = v;
                    }
                }
                "实际最大血量偏移" | "actual max hp offset" => {
                    if let Some(v) = parse_usize(value) {
                        self.actual_max_hp_offset = v;
                    }
                }
                "当前血量偏移" | "current hp offset" => {
                    if let Some(v) = parse_usize(value) {
                        self.current_hp_offset = v;
                    }
                }
                "血量等级偏移" | "hp level offset" => {
                    if let Some(v) = parse_usize(value) {
                        self.level_offset = v;
                    }
                }
                _ => (),
            }
        }
    }

    pub fn tick(&mut self) {
        if !self.enabled {
            return;
        }
        let Some(player) = player_data() else {
            return;
        };

        if self.level_offset < 0x10
            || self.base_max_hp_offset < 0x10
            || self.actual_max_hp_offset < 0x10
            || self.current_hp_offset < 0x10
            || self.base_max_hp_offset == self.actual_max_hp_offset
            || self.base_max_hp_offset == self.current_hp_offset
            || self.actual_max_hp_offset == self.current_hp_offset
        {
            if self.written_logs < 5 {
                self.written_logs += 1;
                log::error!(
                    "health: invalid offsets lv=0x{:x} base=0x{:x} actual=0x{:x} cur=0x{:x}; skip",
                    self.level_offset,
                    self.base_max_hp_offset,
                    self.actual_max_hp_offset,
                    self.current_hp_offset
                );
            }
            return;
        }

        let level = read_u8(player, self.level_offset);
        if !(LEVEL_MIN..=LEVEL_MAX).contains(&level) {
            return;
        }

        let vanilla_max = BASE_HP + (level as f32 - 1.0) * HP_PER_LEVEL;
        let target = (vanilla_max * self.multiplier).round() as u32;

        let base = read_u32(player, self.base_max_hp_offset);
        let actual = read_u32(player, self.actual_max_hp_offset);
        let current = read_u32(player, self.current_hp_offset);

        let base_is_target = (base as f32 - target as f32).abs() <= 4.0;
        let actual_is_target = (actual as f32 - target as f32).abs() <= 4.0;
        let base_is_vanilla = (base as f32 - vanilla_max).abs() <= 4.0;

        if base_is_target && actual_is_target {
            return;
        }

        if base_is_vanilla {
            let ratio = if actual > 0 && current > 0 {
                current as f32 / actual as f32
            } else {
                1.0
            };
            let new_current =
                ((ratio * target as f32).round() as u32).clamp(1, target.max(1));

            write_u32(player, self.base_max_hp_offset, target);
            write_u32(player, self.actual_max_hp_offset, target);
            write_u32(player, self.current_hp_offset, new_current);

            if self.written_logs < 20 {
                self.written_logs += 1;
                log::warn!(
                    "health: old save lv={} baseMax {} -> {} actualMax {} -> {} current {} -> {}",
                    level,
                    base,
                    target,
                    actual,
                    target,
                    current,
                    new_current
                );
            }
        } else if base_is_target {
            write_u32(player, self.actual_max_hp_offset, target);
            if actual > 0 && current > 0 && (actual as f32 - target as f32).abs() > 4.0 {
                let ratio = current as f32 / actual as f32;
                let new_current =
                    ((ratio * target as f32).round() as u32).clamp(1, target.max(1));
                write_u32(player, self.current_hp_offset, new_current);
            }
        }
    }
}

fn player_data() -> Option<*mut u8> {
    unsafe {
        let game_data = game::game_data();
        if game_data.is_null() {
            return None;
        }
        let player = (*game_data).player_data;
        if player.is_null() {
            return None;
        }
        Some(player as *mut u8)
    }
}

fn read_u8(base: *const u8, offset: usize) -> u8 {
    unsafe { (base.add(offset) as *const u8).read_unaligned() }
}

fn read_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { u32::from_le((base.add(offset) as *const u32).read_unaligned()) }
}

fn write_u32(base: *mut u8, offset: usize, value: u32) {
    unsafe {
        (base.add(offset) as *mut u32).write_unaligned(value.to_le());
    }
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let body = line.trim().strip_prefix('#')?.trim();
    for sep in [':', '：'] {
        if let Some((k, v)) = body.split_once(sep) {
            return Some((k, v));
        }
    }
    None
}

fn parse_bool(raw: &str) -> bool {
    let v = raw.trim().to_ascii_lowercase();
    matches!(v.as_str(), "1" | "开" | "on" | "true" | "yes" | "是") || v.starts_with("开")
}

fn parse_usize(raw: &str) -> Option<usize> {
    let text = raw.trim().split_whitespace().next().unwrap_or("");
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        return usize::from_str_radix(&hex, 16).ok();
    }

    let text = text.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if text.is_empty() {
        return None;
    }
    text.parse::<usize>().ok()
}

fn parse_f32(raw: &str) -> Option<f32> {
    raw.trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .parse::<f32>()
        .ok()
}