//! Cached, single-pass inventory scanner.
//!
//! The upstream projects solve this in two complementary ways:
//!
//! * dec32/sekiro-battle-instinct keeps the game pointer chain and the native
//!   item lookup in one `game.rs`, and only calls the game when a value is
//!   actually needed.
//! * kargoshakar/SekiroImGuiCustom resolves and validates addresses once during
//!   worker-thread initialisation, caches the resulting pointers, and updates
//!   them only on explicit state edges (e.g. loading-screen transitions).
//!
//! This module applies both ideas to the fork's inventory needs. One pass over
//! the inventory collects everything the paper guard and emblem-cap fix need:
//!
//! * learned "paper-doll cap" skill rows;
//! * normal/temporary paper-doll rows and quantities.
//!
//! Scans are cached and invalidated by inventory base changes or pause-menu
//! closes. Damage-path lookups may additionally use a low-frequency refresh
//! (default 1 s) so a pickup is noticed without ever scanning on every hit.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{game, memory, menu_probe};

pub const ENTRY_SIZE: usize = 16;
pub const SCAN_LEN: usize = 0x2000;
const FALLBACK_SCAN_LEN: usize = 0x1000;
pub const MAX_EMBLEM_ROWS: usize = 8;

pub const NORMAL_EMBLEM_GOODS_ID: u32 = 1000;
pub const TEMP_EMBLEM_GOODS_ID: u32 = 1001;

/// `virtualWeaponId` of SkillParam rows 75, 76, 175, 176 and 603
/// ("形代所持上限+1A" ... "+1E"). Learning one adds the row to the inventory.
pub const CAP_SKILL_ITEM_IDS: [u32; 5] = [650000, 650100, 650200, 650300, 650400];

/// How often the damage path may force a rescan when no explicit invalidation
/// happened. Cap-skill changes come from menu closes and are event-driven; this
/// throttle exists only to notice pickups made in the world.
const DAMAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InventorySnapshot {
    /// Monotonic id assigned by the cache when this scan was produced.
    pub scan_id: u64,
    pub base: usize,
    pub cap_skills: [bool; 5],
    pub temp: u16,
    pub normal: u16,
    pub temp_offsets: [usize; MAX_EMBLEM_ROWS],
    pub normal_offsets: [usize; MAX_EMBLEM_ROWS],
    pub temp_count: u8,
    pub normal_count: u8,
    pub overflow: bool,
    /// A complete scan ran and the paper-row list did not overflow.
    pub valid: bool,
}

impl InventorySnapshot {
    #[inline]
    #[allow(dead_code)]
    pub fn total(&self) -> u16 {
        self.temp.saturating_add(self.normal)
    }
}

struct CachedInventory {
    snapshot: InventorySnapshot,
    menu_generation: u64,
    scanned_at: Instant,
}

static CACHE: Mutex<Option<CachedInventory>> = Mutex::new(None);
static NEXT_SCAN_ID: AtomicU64 = AtomicU64::new(1);

/// Scan the inventory rows once and return a snapshot. `base` is
/// `InventoryData + 0x10`, exactly the address the old separate scanners used.
pub fn scan(base: usize) -> Option<InventorySnapshot> {
    let mut buffer = [0u8; SCAN_LEN];
    let scan_len = if memory::read_bytes(base, &mut buffer) {
        SCAN_LEN
    } else if memory::read_bytes(base, &mut buffer[..FALLBACK_SCAN_LEN]) {
        FALLBACK_SCAN_LEN
    } else {
        return None;
    };
    Some(scan_at(base, &buffer[..scan_len]))
}

fn scan_at(base: usize, bytes: &[u8]) -> InventorySnapshot {
    let mut snapshot = InventorySnapshot {
        base,
        valid: true,
        ..InventorySnapshot::default()
    };

    for off in (0..bytes.len()).step_by(ENTRY_SIZE) {
        let item_id = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().expect("inventory item id")) & 0x00ff_ffff;
        let quantity = u16::from_le_bytes(bytes[off + 8..off + 10].try_into().expect("inventory quantity"));

        if let Some(slot) = CAP_SKILL_ITEM_IDS.iter().position(|&id| id == item_id) {
            snapshot.cap_skills[slot] = true;
        }

        match item_id {
            TEMP_EMBLEM_GOODS_ID => {
                snapshot.temp = snapshot.temp.saturating_add(quantity);
                let index = snapshot.temp_count as usize;
                if index < MAX_EMBLEM_ROWS {
                    snapshot.temp_offsets[index] = off;
                    snapshot.temp_count += 1;
                } else {
                    snapshot.overflow = true;
                }
            }
            NORMAL_EMBLEM_GOODS_ID => {
                snapshot.normal = snapshot.normal.saturating_add(quantity);
                let index = snapshot.normal_count as usize;
                if index < MAX_EMBLEM_ROWS {
                    snapshot.normal_offsets[index] = off;
                    snapshot.normal_count += 1;
                } else {
                    snapshot.overflow = true;
                }
            }
            _ => {}
        }
    }

    snapshot.valid = !snapshot.overflow;
    snapshot
}

fn refresh(force: bool, max_age: Option<Duration>) -> Option<InventorySnapshot> {
    let base = game::inventory_base()?;
    let menu_generation = menu_probe::menu_close_generation();
    let mut cache = CACHE.lock().ok()?;

    let stale = force
        || cache.as_ref().is_none_or(|cached| {
            cached.snapshot.base != base
                || cached.menu_generation != menu_generation
                || max_age.is_some_and(|max_age| cached.scanned_at.elapsed() >= max_age)
        });

    if stale {
        crate::frame_perf::mark(crate::frame_perf::EventId::PaperInventoryRescan);
        let mut snapshot = scan(base)?;
        snapshot.scan_id = NEXT_SCAN_ID.fetch_add(1, Ordering::Relaxed);
        *cache = Some(CachedInventory {
            snapshot,
            menu_generation,
            scanned_at: Instant::now(),
        });
    }

    cache.as_ref().map(|cached| cached.snapshot)
}

/// Event-driven refresh: only when the inventory object or pause-menu state
/// changed. Safe to call from `process_input`; it is a few pointer reads plus
/// one uncontended mutex when cached.
#[inline]
pub fn ensure_fresh() -> Option<InventorySnapshot> {
    refresh(false, None)
}

/// Damage-path refresh with a one-second pickup fallback. The common zero-paper
/// path returns the cached snapshot and never rescans the inventory.
#[inline]
pub fn ensure_fresh_for_damage() -> Option<InventorySnapshot> {
    refresh(false, Some(DAMAGE_REFRESH_INTERVAL))
}

/// Drop the cache after an explicit external change (currently used by tests
/// and available for future pickup/menu hooks).
#[allow(dead_code)]
pub fn invalidate() {
    if let Ok(mut cache) = CACHE.lock() {
        *cache = None;
    }
}

/// Spend paper dolls from a snapshot's cached row offsets and update the
/// snapshot totals. The caller already decided that a mitigation plan is
/// affordable, so this function never scans. Row IDs are re-read cheaply before
/// any write, so a stale layout cannot spend an unrelated item.
fn spend_from_rows(base: *mut u8, snapshot: &mut InventorySnapshot, cost: u16) -> u16 {
    if cost == 0 || !snapshot.valid {
        return 0;
    }

    let base = base as *const u8;
    let mut remaining = cost;
    let mut spent = 0u16;

    for (id, offsets, count) in [
        (
            TEMP_EMBLEM_GOODS_ID,
            snapshot.temp_offsets,
            snapshot.temp_count as usize,
        ),
        (
            NORMAL_EMBLEM_GOODS_ID,
            snapshot.normal_offsets,
            snapshot.normal_count as usize,
        ),
    ] {
        for &off in &offsets[..count] {
            if remaining == 0 {
                break;
            }
            let item_id = unsafe { memory::read_u32(base, off + 4) } & 0x00ff_ffff;
            if item_id != id {
                // The cached row layout is stale. Do not mutate unrelated rows;
                // the next menu close/base change will trigger a fresh scan.
                break;
            }
            let quantity = unsafe { memory::read_u16(base, off + 8) };
            if quantity == 0 {
                continue;
            }
            let take = quantity.min(remaining);
            unsafe { memory::write_u16(base as *mut u8, off + 8, quantity - take) };
            remaining -= take;
            spent = spent.saturating_add(take);
            match id {
                TEMP_EMBLEM_GOODS_ID => snapshot.temp = snapshot.temp.saturating_sub(take),
                NORMAL_EMBLEM_GOODS_ID => snapshot.normal = snapshot.normal.saturating_sub(take),
                _ => {}
            }
        }
    }

    spent
}

/// Spend from the global cached snapshot without scanning.
pub fn spend_emblems(cost: u16) -> u16 {
    if cost == 0 {
        return 0;
    }
    let Ok(mut cache) = CACHE.lock() else {
        return 0;
    };
    let Some(cached) = cache.as_mut() else {
        return 0;
    };
    let mut snapshot = cached.snapshot;
    let spent = spend_from_rows(snapshot.base as *mut u8, &mut snapshot, cost);
    cached.snapshot = snapshot;
    spent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pass_collects_cap_skills_and_paper_rows() {
        let mut inventory = vec![0u8; SCAN_LEN];
        let base = inventory.as_mut_ptr() as usize;
        let write_row = |inventory: &mut [u8], index: usize, item_id: u32, quantity: u16| {
            let off = index * ENTRY_SIZE;
            inventory[off + 4..off + 8].copy_from_slice(&item_id.to_le_bytes());
            inventory[off + 8..off + 10].copy_from_slice(&quantity.to_le_bytes());
        };

        write_row(&mut inventory, 0, CAP_SKILL_ITEM_IDS[0], 1);
        write_row(&mut inventory, 3, NORMAL_EMBLEM_GOODS_ID, 12);
        write_row(&mut inventory, 5, TEMP_EMBLEM_GOODS_ID, 7);
        write_row(&mut inventory, 7, CAP_SKILL_ITEM_IDS[4], 1);

        let snapshot = scan_at(base, &inventory);
        assert!(snapshot.cap_skills[0]);
        assert!(snapshot.cap_skills[4]);
        assert_eq!(snapshot.normal, 12);
        assert_eq!(snapshot.temp, 7);
        assert_eq!(snapshot.total(), 19);
        assert!(snapshot.valid);
    }

    #[test]
    fn zero_paper_is_a_valid_snapshot_not_a_reason_to_rescan() {
        let inventory = vec![0u8; SCAN_LEN];
        let snapshot = scan_at(inventory.as_ptr() as usize, &inventory);
        assert_eq!(snapshot.total(), 0);
        assert!(!snapshot.overflow);
        assert!(snapshot.valid);
    }

    #[test]
    fn spending_prefers_temporary_rows_and_updates_totals() {
        let mut inventory = vec![0u8; SCAN_LEN];
        let base = inventory.as_mut_ptr() as usize;
        let write_row = |inventory: &mut [u8], index: usize, item_id: u32, quantity: u16| {
            let off = index * ENTRY_SIZE;
            inventory[off + 4..off + 8].copy_from_slice(&item_id.to_le_bytes());
            inventory[off + 8..off + 10].copy_from_slice(&quantity.to_le_bytes());
        };
        write_row(&mut inventory, 1, TEMP_EMBLEM_GOODS_ID, 2);
        write_row(&mut inventory, 2, NORMAL_EMBLEM_GOODS_ID, 5);

        let mut snapshot = scan_at(base, &inventory);
        let spent = spend_from_rows(base as *mut u8, &mut snapshot, 3);
        assert_eq!(spent, 3);
        assert_eq!(snapshot.temp, 0);
        assert_eq!(snapshot.normal, 4);
        assert_eq!(snapshot.total(), 4);
    }
}
