//----------------------------------------------------------------------------
//
//  Paper-doll (纸人 / Spirit Emblem) cap
//
//  Strict rule:
//      i = number of learned "形代所持上限" skills (A..E, SkillParam 75/76/175/176/603)
//      cap = 20 + 2 * i
//
//  The learned state is read from the shared, one-pass inventory snapshot:
//  learning one of these skills immediately adds its dummy weapon
//  (650000..650400) to the runtime inventory. There is no state file, no cfg
//  override and no inference from the old cap value.
//
//  The scanner itself lives in `inventory.rs`; this module only consumes the
//  cached cap-skill flags. That keeps the per-frame `process_input` path free
//  of inventory scans and mirrors dec32/sekiro-battle-instinct's "resolve once,
//  then use the value" pattern.
//
//----------------------------------------------------------------------------

#[cfg(test)]
use std::{fs, path::Path};

use crate::{game, inventory, memory};

/// Fixed acceptance rule: initial cap 20, +2 for each of the five skills.
const BASE_CAP: u32 = 20;
const PER_SKILL: u32 = 2;
const SKILL_COUNT: usize = 5;

/// PlayerData field used by the game for the live emblem cap.
const CAP_OFFSET: usize = 0x146;

pub struct Emblem {
    enabled: bool,
    last_i: Option<u32>,
    last_scan_id: Option<u64>,
    warned_bad_cap: bool,
}

impl Emblem {
    #[cfg(test)]
    pub fn new(cfg: impl AsRef<Path>) -> Emblem {
        let text = fs::read_to_string(cfg.as_ref()).unwrap_or_default();
        Self::from_text(&text)
    }

    pub fn from_text(text: &str) -> Emblem {
        let mut emblem = Emblem {
            enabled: true,
            last_i: None,
            last_scan_id: None,
            warned_bad_cap: false,
        };
        emblem.load(text);
        log::warn!(
            "emblem settings: enabled={} rule=cap {} + {} * i (i from cached inventory snapshot)",
            emblem.enabled,
            BASE_CAP,
            PER_SKILL
        );
        emblem
    }

    fn load(&mut self, text: &str) {
        for line in text.lines() {
            let Some((key, value)) = split_kv(line) else {
                continue;
            };
            if matches!(
                key.trim().to_ascii_lowercase().as_str(),
                "纸人上限功能" | "paper cap fix"
            ) {
                self.enabled = parse_bool(value);
            }
        }
    }

    pub fn tick(&mut self) {
        if !self.enabled {
            return;
        }

        // Event-driven scan: only when the inventory object or pause-menu
        // generation changed. The common path uses the lock-free atomic image
        // and never takes the inventory mutex.
        inventory::refresh_if_needed();
        let Some((scan_id, cap_skills)) = inventory::emblem_image() else {
            return;
        };
        // Apply the cap once per fresh inventory scan, not on every frame.
        if self.last_scan_id == Some(scan_id) {
            return;
        }
        self.last_scan_id = Some(scan_id);

        let Some(player) = game::player_data() else {
            return;
        };

        self.apply(player as *mut u8, &cap_skills);
    }

    fn apply(&mut self, player: *mut u8, cap_skills: &[bool; 5]) {
        let i = count_found(cap_skills);
        let target = BASE_CAP + PER_SKILL * i;
        let cur = unsafe { memory::read_u16(player, CAP_OFFSET) } as u32;

        // Reading something that is not a cap value usually means a bad offset;
        // never overwrite it.
        if !(5..=200).contains(&cur) {
            if !self.warned_bad_cap {
                self.warned_bad_cap = true;
                log::warn!("emblem: cap offset 0x{CAP_OFFSET:x} reads {cur} - not cap-like, skip writing");
            }
            return;
        }

        if self.last_i != Some(i) {
            log::warn!(
                "emblem: skills A={} B={} C={} D={} E={} -> i={} cap={}",
                cap_skills[0] as u8,
                cap_skills[1] as u8,
                cap_skills[2] as u8,
                cap_skills[3] as u8,
                cap_skills[4] as u8,
                i,
                target
            );
            self.last_i = Some(i);
        }

        if cur != target {
            unsafe { memory::write_u16(player, CAP_OFFSET, target as u16) };
            log::warn!("emblem: cap {} -> {} (i={})", cur, target, i);
        }
    }
}

fn count_found(found: &[bool; SKILL_COUNT]) -> u32 {
    found.iter().filter(|&&v| v).count() as u32
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    // Keys may be commented (`# paper cap fix: on`) or written bare.
    let body = line.trim().trim_start_matches('#').trim();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::CAP_SKILL_ITEM_IDS;

    fn test_emblem() -> Emblem {
        Emblem::new("res/__no_such_cfg__.cfg")
    }

    #[test]
    fn formula_is_20_plus_2i() {
        for i in 0..=5u32 {
            assert_eq!(BASE_CAP + PER_SKILL * i, 20 + 2 * i);
        }
    }

    #[test]
    fn only_the_five_real_skill_items_are_counted() {
        assert_eq!(CAP_SKILL_ITEM_IDS, [650000, 650100, 650200, 650300, 650400]);
        assert_eq!(count_found(&[true, true, false, false, false]), 2);
        assert_eq!(count_found(&[true, true, false, true, false]), 3);
        assert_eq!(count_found(&[false; 5]), 0);
        assert_eq!(count_found(&[true; 5]), 5);
    }

    #[test]
    fn shipped_zh_cfg_enables_the_feature() {
        let mut emblem = test_emblem();
        emblem.load(&fs::read_to_string("res/battle_instinct_zh.cfg").expect("read cfg"));
        assert!(emblem.enabled);
    }

    #[test]
    fn commented_switch_still_parses() {
        let mut emblem = test_emblem();
        emblem.load("# paper cap fix: off\n");
        assert!(!emblem.enabled);
    }
}
