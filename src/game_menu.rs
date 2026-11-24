use std::{
    collections::HashMap,
    ffi::c_void,
    mem,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Result, anyhow};
use minhook::MinHook;

use crate::{
    bindings::{COMBAT_ART_UID_MAX, COMBAT_ART_UID_MIN, PROSTHETIC_UID_MAX, PROSTHETIC_UID_MIN},
    menu_probe,
};

// Main pause-menu components are static for Sekiro 1.6.0.0. Combat arts are
// resolved from the skill page's category table; equipment/prosthetic items are
// resolved from the highlighted InventoryUnit row.
const MAIN_MENU_DIALOG_CTOR: usize = 0x140cdfb50;

// Main pause-menu and skill-page fields, verified against Sekiro 1.6.0.0.
const PARAM_OFFSET: usize = 0xab8;
const MAIN_MENU_TAB_GRID_OFFSET: usize = 0xac0;
const GRID_SELECTED_INDEX_OFFSET: usize = 0xf4;
const MAIN_MENU_SKILL_PAGE_OFFSET: usize = 0x1258;
const PAGE_TYPE_STRIDE: usize = 0x40;
const PAGE_TYPE_OFFSET: usize = 0x38;
const PAGE_TYPE_COUNT_OFFSET: usize = 0x108;
// The skill page is the third page component (`dialog + 0x1258`); the page
// directory stores this logical type at entry +0x38. Do not compare the raw
// grid index: the visible tab order and the grid's internal index can differ.
const SKILL_PAGE_TYPE: u32 = 2;
const INVENTORY_PAGE_TYPE: u32 = 1;
const SKILL_CATEGORY_GRID_OFFSET: usize = 0x1198;
const SKILL_ITEM_GRID_OFFSET: usize = 0xac0;
// InventoryUnit item-row layout, verified in-game against the selected
// prosthetic UID. The game's own detail-update path uses the same formula.
const INVENTORY_CATEGORY_GRID_OFFSET: usize = 0x2458;
const INVENTORY_ITEM_TABLE_OFFSET: usize = 0x1128;
const INVENTORY_CATEGORY_STRIDE: usize = 0x30010;
const INVENTORY_ITEM_STRIDE: usize = 0x60;
const INVENTORY_ITEM_UID_OFFSET: usize = 0x40;
const ROW_CATEGORY_OFFSET: usize = 0x44;
// This is the category *directory* for the skill page, not one fixed item
// table. Its entries point at the category's item rows. The actual table is
// selected below from the currently highlighted category and identified by its
// content anchor.
const SKILL_CATEGORY_TABLE_OFFSET: usize = 0xc26f8;
const SKILL_CATEGORY_TABLE_COUNT_OFFSET: usize = 0x308;
const CATEGORY_ENTRY_STRIDE: usize = 0x60;
const CATEGORY_ENTRY_BEGIN_OFFSET: usize = 0x08;
const CATEGORY_ENTRY_END_OFFSET: usize = 0x10;

// Skill category item rows are 0x60 bytes. The row's +0x40 field is the
// SkillParam virtualWeaponId; it is translated to the shared acquireWeaponId
// anchor by the exact map below.
const TABLE_ROW_STRIDE: usize = 0x60;
const MAX_TABLE_ROWS: usize = 0x400;

const MAIN_MENU_DIALOG_CTOR_PREFIX: [u8; 16] = [
    0x40, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac, 0x24,
];

type MainMenuDialogCtorFn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;

static MAIN_MENU_DIALOG_CTOR_ORIG: OnceLock<MainMenuDialogCtorFn> = OnceLock::new();
static MAIN_MENU_DIALOG_POINTER: AtomicUsize = AtomicUsize::new(0);

// Exact `SkillParam.virtualWeaponId -> SkillParam.acquireWeaponId` anchors.
//
// This file is generated from Smithbox's SDT/SkillParam.xml and the game's
// gameparam.parambnd.dcx. `acquireWeaponId` is the same UID consumed by the
// existing combo/release path, so menu selection, key binding and release all
// share this single value. There is deliberately no name, prefix or fuzzy
// matching here.
const SKILLPARAM_ANCHOR_MAP: &str = include_str!("../res/skillparam_uid_map.tsv");
static SKILLPARAM_ANCHORS: OnceLock<HashMap<u32, u32>> = OnceLock::new();

fn skillparam_anchors() -> &'static HashMap<u32, u32> {
    SKILLPARAM_ANCHORS.get_or_init(|| {
        let mut anchors = HashMap::new();
        for line in SKILLPARAM_ANCHOR_MAP.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let (Some(display_id), Some(uid)) = (fields.next(), fields.next()) else {
                continue;
            };
            let (Ok(display_id), Ok(uid)) = (display_id.parse::<u32>(), uid.parse::<u32>()) else {
                continue;
            };
            if is_plausible_menu_uid(uid) {
                anchors.insert(display_id, uid);
            }
        }
        anchors
    })
}

fn anchor_uid_for_display_id(display_id: u32) -> Option<u32> {
    skillparam_anchors().get(&display_id).copied()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuSelection {
    /// The game/cfg UID, not the localized display id.
    pub uid: u32,
    pub category: u32,
}

/// True once the executable's menu constructor is mapped and matches the
/// supported Sekiro build. This lets startup install hooks as soon as the code
/// is usable instead of relying on a fixed delay.
pub fn code_ready() -> bool {
    has_expected_prefix(MAIN_MENU_DIALOG_CTOR, MAIN_MENU_DIALOG_CTOR_PREFIX)
}

pub fn install_hooks() -> Result<()> {
    if !has_expected_prefix(MAIN_MENU_DIALOG_CTOR, MAIN_MENU_DIALOG_CTOR_PREFIX)
        || !menu_probe::range_executable(MAIN_MENU_DIALOG_CTOR, MAIN_MENU_DIALOG_CTOR_PREFIX.len())
    {
        return Err(anyhow!(
            "unsupported Sekiro executable: menu dialog signature does not match 1.6.0.0"
        ));
    }
    unsafe {
        let dialog_ctor = MinHook::create_hook(
            MAIN_MENU_DIALOG_CTOR as *mut c_void,
            main_menu_dialog_ctor_detour as *mut c_void,
        )
        .map_err(|error| anyhow!("install main-menu dialog hook: {error:?}"))?;
        let dialog_ctor: MainMenuDialogCtorFn = mem::transmute(dialog_ctor);
        MAIN_MENU_DIALOG_CTOR_ORIG
            .set(dialog_ctor)
            .map_err(|_| anyhow!("set main-menu dialog original"))?;
    }
    Ok(())
}

fn has_expected_prefix(address: usize, expected: [u8; 16]) -> bool {
    if !menu_probe::range_readable(address, expected.len()) {
        return false;
    }
    let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) };
    bytes == expected
}

/// Read the highlighted equipment item directly from the InventoryUnit row
/// table. It does not depend on the game's input loop running while the pause
/// menu is open.
fn selected_inventory_item() -> Option<MenuSelection> {
    if active_tab() != Some(INVENTORY_PAGE_TYPE) {
        return None;
    }
    let page = current_page_component()?;
    let param = read_usize(page + PARAM_OFFSET)?;
    let category_index = read_u32(page + INVENTORY_CATEGORY_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)? as usize;
    let item_index = read_u32(page + MAIN_MENU_TAB_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)? as usize;
    let row = inventory_row_address(param, category_index, item_index)?;
    if !menu_probe::range_readable(row, ROW_CATEGORY_OFFSET + 4) {
        return None;
    }
    let raw_uid = read_u32(row + INVENTORY_ITEM_UID_OFFSET)?;
    let category = read_u32(row + ROW_CATEGORY_OFFSET).unwrap_or(0);
    let selection = decode_inventory_item(raw_uid, category)?;
    log::warn!(
        "menu: inventory category={} item={} row={:#x} uid={} category={}",
        category_index,
        item_index,
        row,
        selection.uid,
        selection.category
    );
    Some(selection)
}

fn decode_inventory_item(raw_uid: u32, category: u32) -> Option<MenuSelection> {
    let uid = if is_plausible_menu_uid(raw_uid) {
        raw_uid
    } else {
        anchor_uid_for_display_id(raw_uid)?
    };
    Some(MenuSelection { uid, category })
}

fn inventory_row_address(param: usize, category_index: usize, item_index: usize) -> Option<usize> {
    param
        .checked_add(INVENTORY_ITEM_TABLE_OFFSET)?
        .checked_add(category_index.checked_mul(INVENTORY_CATEGORY_STRIDE)?)?
        .checked_add(item_index.checked_mul(INVENTORY_ITEM_STRIDE)?)
}

fn current_page_component() -> Option<usize> {
    let dialog = MAIN_MENU_DIALOG_POINTER.load(Ordering::Acquire);
    if dialog == 0 {
        return None;
    }
    let raw_tab = read_u32(dialog + MAIN_MENU_TAB_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)? as usize;
    read_usize(dialog + 0x1248 + raw_tab.checked_mul(mem::size_of::<usize>())?)
}

fn active_tab() -> Option<u32> {
    let dialog = MAIN_MENU_DIALOG_POINTER.load(Ordering::Acquire);
    if dialog == 0 {
        return None;
    }
    let selected_index = read_u32(dialog + MAIN_MENU_TAB_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)? as usize;
    let param = read_usize(dialog + PARAM_OFFSET)?;
    let page_count = read_usize(param + PAGE_TYPE_COUNT_OFFSET)?;
    if page_count == 0 || page_count > 64 || selected_index >= page_count {
        return None;
    }
    let entry = param
        .checked_add(selected_index.checked_mul(PAGE_TYPE_STRIDE)?)?
        .checked_add(PAGE_TYPE_OFFSET)?;
    read_u32(entry)
}

/// Resolves the current menu row without relying on localized names or a
/// static display-id table. The page type chooses which UI source to try first;
/// a fallback covers transient menu-state differences while the page changes.
pub fn current_selection() -> Option<MenuSelection> {
    let page_type = active_tab();
    // Each page has a distinct selection source. Never fall back across page
    // types: a stale skill row on the inventory page was what made every
    // prosthetic display a combat-art name.
    let selection = match page_type {
        Some(SKILL_PAGE_TYPE) => selected_skill_item(),
        Some(INVENTORY_PAGE_TYPE) => selected_inventory_item(),
        _ => None,
    };
    if let Some(selection) = selection {
        log::warn!(
            "menu: selected page_type={:?} uid={} category={}",
            page_type,
            selection.uid,
            selection.category
        );
    } else {
        log_selection_diagnostics(page_type);
    }
    selection
}

fn log_selection_diagnostics(page_type: Option<u32>) {
    let dialog = MAIN_MENU_DIALOG_POINTER.load(Ordering::Acquire);
    log::warn!("menu diag: dialog={:#x} page_type={:?}", dialog, page_type);
    if dialog == 0 {
        return;
    }
    let raw_tab = read_u32(dialog + MAIN_MENU_TAB_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET);
    let param = read_usize(dialog + PARAM_OFFSET);
    log::warn!(
        "menu diag: raw_tab={:?} param={:?} page_count={:?}",
        raw_tab,
        param,
        param.and_then(|p| read_usize(p + PAGE_TYPE_COUNT_OFFSET))
    );
    let skill_page = read_usize(dialog + MAIN_MENU_SKILL_PAGE_OFFSET);
    log::warn!(
        "menu diag: skill_page={:?} cat_idx={:?} item_idx={:?}",
        skill_page,
        skill_page.and_then(|p| read_u32(p + SKILL_CATEGORY_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)),
        skill_page.and_then(|p| read_u32(p + SKILL_ITEM_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET))
    );
}

/// Reads the highlighted entry from the active skill category.
///
/// The skill page has a category grid at `+0x1198`, a category directory at
/// `param + 0xc26f8`, and an item grid at `+0xac0`. The selected directory
/// entry points at that category's row table. We identify the table by its
/// 旋风斩/手里剑 anchor, then read the selected row through its pointer chain.
fn selected_skill_item() -> Option<MenuSelection> {
    if active_tab() != Some(SKILL_PAGE_TYPE) {
        return None;
    }
    let dialog = MAIN_MENU_DIALOG_POINTER.load(Ordering::Acquire);
    if dialog == 0 {
        return None;
    }
    let skill_page = read_usize(dialog + MAIN_MENU_SKILL_PAGE_OFFSET)?;
    let param = read_usize(skill_page + PARAM_OFFSET)?;

    selected_skill_from_category_directory(dialog, skill_page, param)
        .or_else(|| selected_skill_from_page_descriptor(skill_page))
}

fn selected_skill_from_category_directory(dialog: usize, skill_page: usize, param: usize) -> Option<MenuSelection> {
    let _ = dialog;
    if active_tab() != Some(SKILL_PAGE_TYPE) {
        return None;
    }
    let category_table = param.checked_add(SKILL_CATEGORY_TABLE_OFFSET)?;
    let category_count = read_usize(category_table + SKILL_CATEGORY_TABLE_COUNT_OFFSET)?;
    if category_count == 0 || category_count > 64 {
        return None;
    }

    let category_index = read_u32(skill_page + SKILL_CATEGORY_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)? as usize;
    if category_index >= category_count {
        return None;
    }
    let category_entry = category_table.checked_add(category_index.checked_mul(CATEGORY_ENTRY_STRIDE)?)?;
    let begin = read_usize(category_entry + CATEGORY_ENTRY_BEGIN_OFFSET)?;
    let end = read_usize(category_entry + CATEGORY_ENTRY_END_OFFSET)?;
    let row_count = row_count(begin, end)?;

    let item_index = read_u32(skill_page + SKILL_ITEM_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)? as usize;
    if item_index >= row_count {
        return None;
    }
    let row = begin.checked_add(item_index.checked_mul(TABLE_ROW_STRIDE)?)?;
    let selection = resolve_skill_row(skill_page, row)?;
    log::warn!(
        "menu: skill category={} row={} uid={} category={}",
        category_index,
        item_index,
        selection.uid,
        selection.category
    );
    Some(selection)
}

/// Compatibility path for the skill page's current item list. This still uses
/// the pointers stored in the row to recover the real UID; it never maps the
/// display id at +0x40.
fn selected_skill_from_page_descriptor(skill_page: usize) -> Option<MenuSelection> {
    let descriptor = read_usize(skill_page + 0x10b8)?;
    let begin = read_usize(descriptor + 0x08)?;
    let end = read_usize(descriptor + 0x10)?;
    let count = row_count(begin, end)?;
    let item_index = read_u32(skill_page + SKILL_ITEM_GRID_OFFSET + GRID_SELECTED_INDEX_OFFSET)? as usize;
    if item_index >= count {
        return None;
    }
    let row = begin.checked_add(item_index.checked_mul(TABLE_ROW_STRIDE)?)?;
    let selection = resolve_skill_row(skill_page, row)?;
    log::warn!(
        "menu: skill descriptor fallback row={} uid={} category={}",
        item_index,
        selection.uid,
        selection.category
    );
    Some(selection)
}

/// Resolves a highlighted skill row to the exact release anchor.
///
/// The row's +0x40 field is the SkillParam `virtualWeaponId`. It is resolved
/// through the generated `SkillParam` map to `acquireWeaponId`; that UID is the
/// same value the existing combo/release code consumes. If the row already
/// contains a true UID it is accepted directly. No record-array backtracking,
/// display-name matching or category heuristic is used here.
fn resolve_skill_row(_skill_page: usize, row: usize) -> Option<MenuSelection> {
    let display_id = read_u32(row + 0x40)?;
    if is_plausible_menu_uid(display_id) {
        return Some(MenuSelection {
            uid: display_id,
            category: read_u32(row + ROW_CATEGORY_OFFSET).unwrap_or(0),
        });
    }
    let uid = anchor_uid_for_display_id(display_id)?;
    let selection = MenuSelection {
        uid,
        category: read_u32(row + ROW_CATEGORY_OFFSET).unwrap_or(0),
    };
    log::warn!(
        "menu: SkillParam anchor display_id={} uid={} category={}",
        display_id,
        selection.uid,
        selection.category
    );
    Some(selection)
}

fn row_count(begin: usize, end: usize) -> Option<usize> {
    let bytes = end.checked_sub(begin)?;
    if bytes == 0 || bytes % TABLE_ROW_STRIDE != 0 || bytes > MAX_TABLE_ROWS * TABLE_ROW_STRIDE {
        return None;
    }
    Some(bytes / TABLE_ROW_STRIDE)
}

fn is_plausible_menu_uid(uid: u32) -> bool {
    (COMBAT_ART_UID_MIN..=COMBAT_ART_UID_MAX).contains(&uid) || (PROSTHETIC_UID_MIN..=PROSTHETIC_UID_MAX).contains(&uid)
}

fn read_u32(address: usize) -> Option<u32> {
    if !menu_probe::range_readable(address, 4) {
        return None;
    }
    Some(unsafe { (address as *const u32).read_unaligned() })
}

fn read_usize(address: usize) -> Option<usize> {
    if !menu_probe::range_readable(address, mem::size_of::<usize>()) {
        return None;
    }
    Some(unsafe { (address as *const usize).read_unaligned() })
}

unsafe extern "system" fn main_menu_dialog_ctor_detour(
    this: *mut c_void,
    param: *mut c_void,
    third: *mut c_void,
) -> *mut c_void {
    let Some(original) = MAIN_MENU_DIALOG_CTOR_ORIG.get().copied() else {
        return this;
    };
    let result = unsafe { original(this, param, third) };
    if !this.is_null() {
        MAIN_MENU_DIALOG_POINTER.store(this as usize, Ordering::Release);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_art_and_prosthetic_uid_ranges() {
        assert!(is_plausible_menu_uid(7100));
        assert!(is_plausible_menu_uid(70500));
        assert!(!is_plausible_menu_uid(410100));
        assert!(!is_plausible_menu_uid(210000));
    }

    #[test]
    fn resolves_exact_skillparam_anchor_ids() {
        assert_eq!(anchor_uid_for_display_id(210000), Some(5100));
        assert_eq!(anchor_uid_for_display_id(410000), Some(5300));
        assert_eq!(anchor_uid_for_display_id(410100), Some(7100));
        assert_eq!(anchor_uid_for_display_id(701000), Some(70100));
        assert_eq!(anchor_uid_for_display_id(705000), Some(70500));
        assert_eq!(anchor_uid_for_display_id(410101), None);
    }

    #[test]
    fn inventory_row_address_uses_verified_layout() {
        assert_eq!(
            inventory_row_address(0x1000, 3, 2),
            Some(0x1000 + 0x1128 + 3 * 0x30010 + 2 * 0x60)
        );
    }

    #[test]
    fn decodes_inventory_uid_and_category() {
        assert_eq!(
            decode_inventory_item(70200, 5),
            Some(MenuSelection {
                uid: 70200,
                category: 5,
            })
        );
    }

    #[test]
    fn decodes_inventory_display_id_via_skillparam_anchor() {
        assert_eq!(
            decode_inventory_item(733000, 5),
            Some(MenuSelection {
                uid: 73300,
                category: 5,
            })
        );
    }
}
