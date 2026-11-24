//----------------------------------------------------------------------------
//
//  Paper-doll (紙人 / Spirit Emblem) cap
//
//  Strict rule:
//      i = number of learned "形代所持上限" skills (A..E, SkillParam 75/76/175/176/603)
//      cap = 20 + 2 * i
//
//  The learned state is read from the live inventory: learning one of these
//  skills immediately adds its dummy weapon (650000..650400) to the runtime
//  inventory. There is no state file, no cfg override and no inference from
//  the old cap value.
//
//----------------------------------------------------------------------------

use std::{fs, path::Path};

use crate::{game, menu_probe};

/// Fixed acceptance rule: initial cap 20, +2 for each of the five skills.
const BASE_CAP: u32 = 20;
const PER_SKILL: u32 = 2;
const SKILL_COUNT: usize = 5;

/// `virtualWeaponId` of SkillParam rows 75, 76, 175, 176 and 603
/// (形代所持上限+1A ... +1E). Learning a skill adds this weapon to the live
/// inventory; unlearned dummy entries are not present in the runtime inventory.
const CAP_SKILL_ITEM_IDS: [u32; SKILL_COUNT] = [650000, 650100, 650200, 650300, 650400];

/// PlayerData field used by the game for the live emblem cap.
const CAP_OFFSET: usize = 0x146;
const INVENTORY_ITEM_SIZE: usize = 16;
const INVENTORY_SCAN_LEN: usize = 0x2000;

pub struct Emblem {
    enabled: bool,
    last_i: Option<u32>,
    warned_bad_cap: bool,
    last_inventory: Option<usize>,
    menu_was_open: bool,
    needs_scan: bool,
}

impl Emblem {
    pub fn new(cfg: impl AsRef<Path>) -> Emblem {
        let mut emblem = Emblem {
            enabled: true,
            last_i: None,
            warned_bad_cap: false,
            last_inventory: None,
            menu_was_open: false,
            needs_scan: true,
        };
        if let Ok(text) = fs::read_to_string(cfg.as_ref()) {
            emblem.load(&text);
        }
        log::warn!(
            "emblem settings: enabled={} rule=cap {} + {} * i (i from live inventory)",
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

        let Some(inventory) = inventory_identity() else {
            self.last_inventory = None;
            self.needs_scan = true;
            return;
        };
        if self.last_inventory != Some(inventory) {
            self.last_inventory = Some(inventory);
            self.needs_scan = true;
        }

        // Cap skills are learned from the pause menu. Refresh once when the
        // menu closes instead of polling the inventory on a timer.
        let menu_open = menu_probe::menu_open();
        if self.menu_was_open && !menu_open {
            self.needs_scan = true;
        }
        self.menu_was_open = menu_open;

        if !self.needs_scan {
            return;
        }
        self.needs_scan = false;

        let Some(player) = player_data() else {
            self.needs_scan = true;
            return;
        };
        self.apply(player as *mut u8);
    }

    fn apply(&mut self, player: *mut u8) {
        let Some(found) = detect_cap_skills() else {
            // Inventory not ready (loading screen / no save). Do not write a
            // guessed value; wait for the real skill state.
            return;
        };

        let i = count_found(&found);
        let target = BASE_CAP + PER_SKILL * i;
        let cur = unsafe { read_u16(player, CAP_OFFSET) } as u32;

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
                found[0] as u8,
                found[1] as u8,
                found[2] as u8,
                found[3] as u8,
                found[4] as u8,
                i,
                target
            );
            self.last_i = Some(i);
        }

        if cur != target {
            unsafe { write_u16(player, CAP_OFFSET, target as u16) };
            log::warn!("emblem: cap {} -> {} (i={})", cur, target, i);
        }
    }
}

fn count_found(found: &[bool; SKILL_COUNT]) -> u32 {
    found.iter().filter(|&&v| v).count() as u32
}

/// Read the five learned-skill flags from the **live** inventory.
///
/// Runtime inventory entries are 16 bytes: `handle, item_id, quantity, index`.
/// A learned skill adds `650000/650100/650200/650300/650400` as an item; an
/// unlearned skill has no entry. This is a direct possession check, not an
/// inference from the emblem cap.
fn inventory_identity() -> Option<usize> {
    unsafe {
        let game_data = game::game_data();
        if game_data.is_null() {
            return None;
        }
        let player = (*game_data).player_data;
        if player.is_null() {
            return None;
        }
        let inventory = (*player).inventory_data;
        if inventory.is_null() {
            return None;
        }
        Some(inventory as usize)
    }
}

fn detect_cap_skills() -> Option<[bool; SKILL_COUNT]> {
    unsafe {
        let inventory = inventory_identity()? as *const u8;

        let base = inventory.add(0x10);
        let scan_len = if range_readable(base as usize, INVENTORY_SCAN_LEN) {
            INVENTORY_SCAN_LEN
        } else if range_readable(base as usize, 0x1000) {
            0x1000
        } else {
            return None;
        };

        let mut found = [false; SKILL_COUNT];
        for off in (0..scan_len).step_by(INVENTORY_ITEM_SIZE) {
            let item_id = read_u32(base, off + 4) & 0x00FF_FFFF;
            if let Some(slot) = CAP_SKILL_ITEM_IDS.iter().position(|&id| id == item_id) {
                found[slot] = true;
                if found.iter().all(|&v| v) {
                    break;
                }
            }
        }
        Some(found)
    }
}

fn range_readable(addr: usize, len: usize) -> bool {
    use core::ffi::c_void;
    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };

    let mut off = 0usize;
    while off < len {
        let mut mbi = MEMORY_BASIC_INFORMATION::default();
        let w = unsafe {
            VirtualQuery(
                Some((addr + off) as *const c_void),
                &mut mbi,
                core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if w == 0 || mbi.State != MEM_COMMIT {
            return false;
        }
        let p = mbi.Protect.0;
        if p & PAGE_GUARD.0 != 0 || p & PAGE_NOACCESS.0 != 0 {
            return false;
        }
        off = off.saturating_add(mbi.RegionSize.max(1));
    }
    true
}

fn player_data() -> Option<*const u8> {
    unsafe {
        let game_data = game::game_data();
        if game_data.is_null() {
            return None;
        }
        let player = (*game_data).player_data;
        if player.is_null() {
            return None;
        }
        Some(player as *const u8)
    }
}

unsafe fn read_u16(base: *const u8, offset: usize) -> u16 {
    unsafe { u16::from_le((base.add(offset) as *const u16).read_unaligned()) }
}

unsafe fn read_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { u32::from_le((base.add(offset) as *const u32).read_unaligned()) }
}

unsafe fn write_u16(base: *mut u8, offset: usize, value: u16) {
    unsafe {
        (base.add(offset) as *mut u16).write_unaligned(value.to_le());
    }
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
