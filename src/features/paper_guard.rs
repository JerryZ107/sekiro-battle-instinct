//----------------------------------------------------------------------------
//
// Paper-doll damage reduction (纸人减伤)
//
// Damage tier is based on raw HP damage / player max HP:
//   <= 33%  -> tier 1, cost 1, reduce 60%
//   <= 66%  -> tier 2, cost 2, reduce 60%
//   >  66%  -> tier 3, cost 3, reduce 60%
//
// When there are not enough paper dolls:
//   tier 3, 2 dolls -> cost 2, reduce 30%
//   tier 3, 1 doll  -> cost 1, reduce 20%
//   tier 2, 1 doll  -> cost 1, reduce 30%
//   0 dolls         -> no reduction
//
// Temporary paper dolls (goods id 1001) are spent before normal paper dolls
// (goods id 1000).
//
// The inventory is scanned by `inventory.rs`, one pass and cached. This module
// only reads the cached stock and spends from the cached rows. In particular,
// zero paper is a normal, persistent state: it must not be treated as a cache
// miss that triggers a full rescan on every damage event.
//
//----------------------------------------------------------------------------

use std::{
    ffi::c_void,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{game, inventory, memory};

const PLAYER_CURRENT_HP_OFFSET: usize = 0x18;
const PLAYER_MAX_HP_OFFSET: usize = 0x1c;
const CHR_CURRENT_HP_OFFSET: usize = 0x130;

const DAMAGE_INFO_SCAN_LEN: usize = 0x400;
const DAMAGE_INFO_HP_DAMAGE_OFFSET: usize = 0x1e0;
const DAMAGE_INFO_TARGET_OFFSET_A: usize = 0x198;
const DAMAGE_INFO_TARGET_OFFSET_B: usize = 0x280;

static INSTANCE: OnceLock<PaperGuard> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    One,
    Two,
    Three,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MitigationPlan {
    cost: u16,
    percent: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct PendingHit {
    raw_damage: i32,
    reduced_damage: i32,
    cost: u16,
    percent: u16,
    tier: Tier,
    total_emblems: u16,
    temp_before: u16,
    normal_before: u16,
    max_hp: i32,
    hp_before: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EmblemStock {
    temp: u16,
    normal: u16,
}

impl EmblemStock {
    fn total(self) -> u16 {
        self.temp.saturating_add(self.normal)
    }
}

struct PaperGuard {
    enabled: bool,
    debug: bool,
    /// Last damage-info pointer whose page range was already validated.
    last_damage_info: AtomicUsize,
}

impl PaperGuard {
    fn from_text(text: &str) -> PaperGuard {
        let mut guard = PaperGuard {
            enabled: true,
            debug: false,
            last_damage_info: AtomicUsize::new(0),
        };
        {
            for line in text.lines() {
                let Some((key, value)) = split_kv(line) else {
                    continue;
                };
                match key.trim().to_ascii_lowercase().as_str() {
                    "纸人减伤功能" | "paper guard" | "paper guard enabled" => {
                        guard.enabled = parse_bool(value);
                    }
                    "纸人减伤调试日志" | "paper guard debug" => {
                        guard.debug = parse_bool(value);
                    }
                    _ => {}
                }
            }
        }
        log::warn!("paper guard settings: enabled={} debug={}", guard.enabled, guard.debug);
        guard
    }

    fn read_stock(&self) -> EmblemStock {
        // This takes the inventory mutex only when the cache is invalidated or
        // the one-second pickup fallback is due.
        let _ = inventory::ensure_fresh_for_damage();
        let Some((temp, normal)) = inventory::stock_fast() else {
            return EmblemStock::default();
        };
        EmblemStock { temp, normal }
    }
}

pub fn init_from_text(text: &str) {
    let guard = PaperGuard::from_text(text);
    if INSTANCE.set(guard).is_err() {
        log::warn!("paper guard: init called more than once");
    }
}

/// Called before the game's damage reaction function. Scales the HP-damage
/// field in place and returns enough information to spend paper afterwards.
pub fn prepare_damage(damage_info: *mut c_void, hp_before: i32) -> Option<PendingHit> {
    let guard = INSTANCE.get()?;
    if !guard.enabled || damage_info.is_null() || hp_before <= 0 {
        return None;
    }
    let damage_info_addr = damage_info as usize;
    if guard.last_damage_info.load(Ordering::Relaxed) != damage_info_addr {
        if !memory::is_valid_read_ptr(damage_info_addr, DAMAGE_INFO_SCAN_LEN) {
            return None;
        }
        guard.last_damage_info.store(damage_info_addr, Ordering::Relaxed);
    }

    let info = damage_info.cast::<u8>();
    let raw_damage = unsafe { memory::read_i32(info, DAMAGE_INFO_HP_DAMAGE_OFFSET) };
    if raw_damage <= 0 {
        return None;
    }

    let player = game::player_ins()?;
    let player_addr = player as u64;
    let target_a = unsafe { read_u64(info, DAMAGE_INFO_TARGET_OFFSET_A) };
    let target_b = unsafe { read_u64(info, DAMAGE_INFO_TARGET_OFFSET_B) };
    if target_a != player_addr && target_b != player_addr {
        return None;
    }

    let player_data = game::player_data()?;
    let max_hp = unsafe { memory::read_u32(player_data as *const u8, PLAYER_MAX_HP_OFFSET) } as i32;
    if max_hp <= 0 {
        return None;
    }

    let stock = guard.read_stock();
    let tier = tier_for_damage(raw_damage, max_hp);
    let Some(plan) = plan_for(tier, stock.total()) else {
        if guard.debug {
            log::info!(
                "PAPER_SKIP tier={tier:?} raw={raw_damage} maxHp={max_hp} temp={} normal={} total={}",
                stock.temp,
                stock.normal,
                stock.total()
            );
        }
        return None;
    };

    let reduced_damage = reduce_damage(raw_damage, plan.percent);
    unsafe { memory::write_i32(info, DAMAGE_INFO_HP_DAMAGE_OFFSET, reduced_damage) };

    if guard.debug {
        log::info!(
            "PAPER_READY tier={tier:?} raw={raw_damage} maxHp={max_hp} temp={} normal={} total={} cost={} percent={} reduced={reduced_damage}",
            stock.temp,
            stock.normal,
            stock.total(),
            plan.cost,
            plan.percent
        );
    }

    Some(PendingHit {
        raw_damage,
        reduced_damage,
        cost: plan.cost,
        percent: plan.percent,
        tier,
        total_emblems: stock.total(),
        temp_before: stock.temp,
        normal_before: stock.normal,
        max_hp,
        hp_before,
    })
}

/// Called after the game's damage reaction function. If HP was actually lost,
/// spend the paper dolls that paid for this hit.
pub fn finish_damage(pending: Option<PendingHit>, chr_data: *mut u8, hp_after: i32) {
    let Some(pending) = pending else {
        return;
    };
    let Some(guard) = INSTANCE.get() else {
        return;
    };

    let actual_drop = (pending.hp_before - hp_after).max(0);
    if actual_drop == 0 {
        return;
    }

    let expected_drop = pending.reduced_damage.min(pending.hp_before).max(0);
    let mut fallback_used = false;

    // Safety net: if the pre-scaled field was not honoured, restore the
    // difference immediately. This also covers the case where the hit was
    // lethal only because the unscaled damage was applied.
    if actual_drop > expected_drop {
        let target_hp = pending.hp_before.saturating_sub(expected_drop);
        if target_hp > hp_after && target_hp > 0 && !chr_data.is_null() {
            unsafe {
                memory::write_i32(chr_data, CHR_CURRENT_HP_OFFSET, target_hp);
                if let Some(player_data) = game::player_data() {
                    memory::write_i32(player_data as *mut u8, PLAYER_CURRENT_HP_OFFSET, target_hp);
                }
            }
            fallback_used = true;
        }
    }

    let spent = inventory::spend_emblems(pending.cost);
    if guard.debug {
        let after = guard.read_stock();
        log::info!(
            "PAPER_HIT tier={:?} raw={} maxHp={} total={} cost={} percent={} reduced={} hp={}->{} actualDrop={} expectedDrop={} fallback={} spent={} temp={}->{} normal={}->{}",
            pending.tier,
            pending.raw_damage,
            pending.max_hp,
            pending.total_emblems,
            pending.cost,
            pending.percent,
            pending.reduced_damage,
            pending.hp_before,
            hp_after,
            actual_drop,
            expected_drop,
            fallback_used,
            spent,
            pending.temp_before,
            after.temp,
            pending.normal_before,
            after.normal
        );
    }
    if spent < pending.cost {
        log::warn!(
            "paper guard: wanted to spend {} paper dolls but only spent {}",
            pending.cost,
            spent
        );
    }
}

fn tier_for_damage(raw_damage: i32, max_hp: i32) -> Tier {
    let raw = raw_damage.max(0) as u64;
    let max = max_hp.max(1) as u64;
    if raw.saturating_mul(100) <= max.saturating_mul(33) {
        Tier::One
    } else if raw.saturating_mul(100) <= max.saturating_mul(66) {
        Tier::Two
    } else {
        Tier::Three
    }
}

fn plan_for(tier: Tier, total_emblems: u16) -> Option<MitigationPlan> {
    match tier {
        Tier::One if total_emblems >= 1 => Some(MitigationPlan { cost: 1, percent: 60 }),
        Tier::Two if total_emblems >= 2 => Some(MitigationPlan { cost: 2, percent: 60 }),
        Tier::Two if total_emblems == 1 => Some(MitigationPlan { cost: 1, percent: 30 }),
        Tier::Three if total_emblems >= 3 => Some(MitigationPlan { cost: 3, percent: 60 }),
        Tier::Three if total_emblems == 2 => Some(MitigationPlan { cost: 2, percent: 30 }),
        Tier::Three if total_emblems == 1 => Some(MitigationPlan { cost: 1, percent: 20 }),
        _ => None,
    }
}

fn reduce_damage(raw_damage: i32, percent: u16) -> i32 {
    let raw = raw_damage.max(0) as i64;
    let mitigated = (raw * percent as i64 + 50) / 100;
    (raw - mitigated).clamp(0, raw) as i32
}

unsafe fn read_u64(base: *const u8, offset: usize) -> u64 {
    unsafe { u64::from_le((base.add(offset) as *const u64).read_unaligned()) }
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
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

    #[test]
    fn damage_tiers_use_raw_damage_over_max_hp() {
        assert_eq!(tier_for_damage(330, 1000), Tier::One);
        assert_eq!(tier_for_damage(331, 1000), Tier::Two);
        assert_eq!(tier_for_damage(660, 1000), Tier::Two);
        assert_eq!(tier_for_damage(661, 1000), Tier::Three);
        assert_eq!(tier_for_damage(1120, 1120), Tier::Three);
        assert_eq!(tier_for_damage(169, 1120), Tier::One);
        assert_eq!(tier_for_damage(394, 1120), Tier::Two);
    }

    #[test]
    fn full_paper_plan_uses_one_two_three_dolls() {
        assert_eq!(plan_for(Tier::One, 1).unwrap().cost, 1);
        assert_eq!(plan_for(Tier::Two, 2).unwrap().cost, 2);
        assert_eq!(plan_for(Tier::Three, 3).unwrap().cost, 3);
        assert_eq!(plan_for(Tier::One, 1).unwrap().percent, 60);
        assert_eq!(plan_for(Tier::Two, 5).unwrap().percent, 60);
        assert_eq!(plan_for(Tier::Three, 5).unwrap().percent, 60);
    }

    #[test]
    fn downgrade_plan_matches_specification() {
        assert_eq!(plan_for(Tier::One, 0), None);
        assert_eq!(plan_for(Tier::Two, 0), None);
        assert_eq!(plan_for(Tier::Three, 0), None);

        let p = plan_for(Tier::Two, 1).unwrap();
        assert_eq!((p.cost, p.percent), (1, 30));

        let p = plan_for(Tier::Three, 2).unwrap();
        assert_eq!((p.cost, p.percent), (2, 30));

        let p = plan_for(Tier::Three, 1).unwrap();
        assert_eq!((p.cost, p.percent), (1, 20));
    }

    #[test]
    fn reduction_rounds_to_nearest_integer() {
        assert_eq!(reduce_damage(169, 60), 68);
        assert_eq!(reduce_damage(394, 60), 158);
        assert_eq!(reduce_damage(169, 40), 101);
        assert_eq!(reduce_damage(169, 20), 135);
        assert_eq!(reduce_damage(1, 60), 0);
    }

    #[test]
    fn emblem_stock_total_includes_temporary_dolls() {
        assert_eq!(EmblemStock { temp: 9, normal: 30 }.total(), 39);
        assert_eq!(
            EmblemStock {
                temp: u16::MAX,
                normal: 1
            }
            .total(),
            u16::MAX
        );
    }
}
