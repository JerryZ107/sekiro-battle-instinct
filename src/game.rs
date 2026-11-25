use std::{
    ffi::c_void,
    fmt,
    num::NonZero,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use windows::Win32::System::LibraryLoader::GetModuleHandleW;

//----------------------------------------------------------------------------
//
//  Addresses of functions and objects from the original program
//
//----------------------------------------------------------------------------

// functions
pub const PROCESS_INPUT: usize = 0x140B2C190;
pub const GET_ITEM_ID: usize = 0x140C3D680;
pub const SET_SLOT: usize = 0x140D592F0;
pub const SET_EQUIPED_PROTHSETIC: usize = 0x140A26150;
pub const PLAYER_DAMAGE_REACTION: usize = 0x140B6E6A0;

/// First 16 bytes of the supported Sekiro 1.6.0.0 function prologues. These are
/// used as a readiness/safety signature before MinHook is allowed to patch the
/// pages; a page can be executable while the loader is still unpacking code.
pub const PROCESS_INPUT_PREFIX: [u8; 16] = [
    0x48, 0x89, 0x5C, 0x24, 0x18, 0x55, 0x48, 0x83, 0xEC, 0x20, 0x4C, 0x8B, 0x41, 0x10, 0x48, 0x8B,
];
pub const PLAYER_DAMAGE_REACTION_PREFIX: [u8; 16] = [
    0x48, 0x8B, 0xC4, 0x55, 0x53, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D, 0x68, 0xB8, 0x48,
];

/// Base address of the running `sekiro.exe`. Data-patch modules use
/// `base + RVA`; the function/object constants above are absolute for the
/// usual 0x140000000 image base.
pub fn module_base() -> usize {
    static BASE: OnceLock<usize> = OnceLock::new();
    *BASE.get_or_init(|| unsafe { GetModuleHandleW(None).map(|module| module.0 as usize).unwrap_or(0) })
}

// objects
pub const GAME_DATA: usize = 0x143D5AAC0;
pub const WORLD_DATA: usize = 0x143D7A1E0;

// PlayerIns -> module registry -> SprjChrDataModule.
pub const PLAYER_MODULE_REGISTRY_OFFSET: usize = 0x1ff8;
pub const CHR_DATA_MODULE_OFFSET: usize = 0x18;

//----------------------------------------------------------------------------
//
//  Structs from the original program
//
//----------------------------------------------------------------------------

pub fn game_data() -> *const GameData {
    unsafe { *(GAME_DATA as *const *const GameData) }
}

#[rustfmt::skip]
#[repr(C)]
pub struct GameData { 
    _0: [u8;8], pub player_data: *const PlayerData 
}

#[rustfmt::skip]
#[repr(C)]
pub struct PlayerData { 
    _0: [u8;0x29c], pub equiped_items: [u32; 5],
    _1: [u8;0x06c], pub activte_prosthetic: u8,
    _2: [u8;0x293], pub inventory_data: *const InventoryData 
}

#[rustfmt::skip]
#[repr(C)]
pub struct InventoryData { 
    _0: [u8;16], pub inventory: c_void 
}

#[rustfmt::skip]
#[repr(C)]
pub struct InputHandler {
    _0: [u8;16],
    /// Primary action bitfield (ATTACK/BLOCK/JUMP/DODGE/USE_PROSTHETIC/鈥?.
    pub action: u64,
    /// Neighboring words 鈥?used when probing undocumented binds (e.g. switch prosthetic).
    pub action_b: u64,
    pub action_c: u64,
}

#[rustfmt::skip]
#[repr(C)]
pub struct EquipData { 
    _0: [u8;56], pub item_id: u32,
}

impl EquipData {
    pub fn new(item_id: u32) -> EquipData {
        EquipData { _0: [0; 56], item_id }
    }
}

//----------------------------------------------------------------------------
//
//  Functions from the original program
//
//----------------------------------------------------------------------------

macro_rules! forward {
    (
        $(
            @[$address:expr]
            fn $name:tt($($arg:tt: $arg_ty:ty),*) $(-> $ret_ty:ty)?
        );*;
    ) => {
        $(
            #[inline(always)]
            #[allow(unused)]
            pub fn $name($($arg: $arg_ty),*) $(-> $ret_ty)? {
                unsafe { std::mem::transmute::<_, extern "system" fn($($arg: $arg_ty),*)$(-> $ret_ty)?>($address as *const ())($($arg),*) }
            }
        )*
    };
}

forward! {
    @[GET_ITEM_ID]
    fn get_item_id(inventory: *const c_void, uid: *const u32) -> u32;

    @[SET_SLOT]
    fn set_slot_raw(equip_slot: usize, equip_data: *const EquipData, ignore_equip_lock: bool);

    @[SET_EQUIPED_PROTHSETIC]
    fn set_equipped_prosthetic(unknown: *const c_void, zero: u32, prosthetic_index: u32);
}

//----------------------------------------------------------------------------
//
//  UID / ItemID primitives, shared by every feature module
//
//----------------------------------------------------------------------------

/// UIDs are consistent through different save files.
pub type UID = u32;

pub const COMBAT_ART_SLOT: u8 = 1;

/// When players obtain skills (combat arts/prosthetic tools), the skill becomes
/// an item in the inventory. Thus a skill has two IDs: its original UID and its
/// ID as an inventory item. Putting random items into item slots can cause
/// severe bugs such as losing Kusabimaru permanently, so the conversion is
/// centralised here and never guessed by feature code.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemID(NonZero<u32>);

impl ItemID {
    #[inline(always)]
    pub fn new(value: u32) -> Option<ItemID> {
        NonZero::<u32>::new(value).map(ItemID)
    }

    #[inline(always)]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for ItemID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.get().fmt(f)
    }
}

pub trait ID: fmt::Display + Clone + Copy {
    fn get_item_id(self) -> Option<ItemID>;
}

impl ID for ItemID {
    #[inline(always)]
    fn get_item_id(self) -> Option<ItemID> {
        Some(self)
    }
}

impl ID for UID {
    #[inline(always)]
    fn get_item_id(self) -> Option<ItemID> {
        let inventory = inventory_data()?;
        let inventory = unsafe { &(*inventory).inventory as *const c_void };
        let item_id = get_item_id(inventory, &self);
        ItemID::new(item_id).filter(|it| it.get() < 0xFFFF)
    }
}

/// Place an item into a game equip slot. The caller must already have resolved
/// a real inventory item id; `ItemID` enforces that at the type level.
pub fn set_slot(item: impl ID, slot_index: usize) -> bool {
    let Some(item_id) = item.get_item_id() else {
        return false;
    };
    let equip_data = EquipData::new(item_id.get());
    set_slot_raw(slot_index, &equip_data, true);
    true
}

/// Resolve a chain of pointer dereferences followed by byte offsets.
/// Mirrors dec32/sekiro-battle-instinct's `game::resolve_pointer_chain`.
///
/// Safety: every dereference must target readable game memory.
pub unsafe fn resolve_pointer_chain<R, const N: usize>(root: usize, offsets: [usize; N]) -> *mut R {
    unsafe {
        let mut pointer = root as *mut ();
        for offset in offsets {
            pointer = *(pointer as *mut *mut ());
            if pointer.is_null() {
                return pointer as *mut R;
            }
            pointer = pointer.byte_add(offset);
        }
        pointer as *mut R
    }
}

//----------------------------------------------------------------------------
//
//  Cached player / ChrData pointer chains
//
//----------------------------------------------------------------------------

static PLAYER_CACHE_WORLD: AtomicUsize = AtomicUsize::new(0);
static PLAYER_CACHE: AtomicUsize = AtomicUsize::new(0);
static CHR_CACHE_PLAYER: AtomicUsize = AtomicUsize::new(0);
static CHR_CACHE: AtomicUsize = AtomicUsize::new(0);

/// Clear cached player/module pointers after a menu close/save load. The next
/// damage event or flow tick resolves the chain again.
pub fn invalidate_pointer_cache() {
    PLAYER_CACHE_WORLD.store(0, Ordering::Release);
    PLAYER_CACHE.store(0, Ordering::Release);
    CHR_CACHE_PLAYER.store(0, Ordering::Release);
    CHR_CACHE.store(0, Ordering::Release);
}

/// PlayerIns from `WORLD_DATA -> +0x88`.
#[inline]
pub fn player_ins() -> Option<*mut u8> {
    let world_data = unsafe { *(WORLD_DATA as *const *const u8) };
    if world_data.is_null() {
        return None;
    }
    let cached_world = PLAYER_CACHE_WORLD.load(Ordering::Acquire);
    let cached_player = PLAYER_CACHE.load(Ordering::Acquire) as *mut u8;
    if cached_world == world_data as usize && !cached_player.is_null() {
        return Some(cached_player);
    }

    let player = unsafe { *(world_data.add(0x88) as *const *mut u8) };
    if player.is_null() {
        return None;
    }
    PLAYER_CACHE_WORLD.store(world_data as usize, Ordering::Release);
    PLAYER_CACHE.store(player as usize, Ordering::Release);
    Some(player)
}

/// SprjChrDataModule from PlayerIns.
#[inline]
pub fn chr_data_module(player: *mut u8) -> Option<*mut u8> {
    if player.is_null() {
        return None;
    }
    let cached_player = CHR_CACHE_PLAYER.load(Ordering::Acquire);
    let cached_module = CHR_CACHE.load(Ordering::Acquire) as *mut u8;
    if cached_player == player as usize && !cached_module.is_null() {
        return Some(cached_module);
    }

    let registry = unsafe { *(player.add(PLAYER_MODULE_REGISTRY_OFFSET) as *const *mut u8) };
    if registry.is_null() {
        return None;
    }
    let module = unsafe { *(registry.add(CHR_DATA_MODULE_OFFSET) as *const *mut u8) };
    if module.is_null() {
        return None;
    }
    CHR_CACHE_PLAYER.store(player as usize, Ordering::Release);
    CHR_CACHE.store(module as usize, Ordering::Release);
    Some(module)
}

#[inline]
pub fn player_chr_data_module() -> Option<*mut u8> {
    player_ins().and_then(chr_data_module)
}

//----------------------------------------------------------------------------
//
//  Shared pointer-chain helpers
//
//  Keep these in one place, like dec32/sekiro-battle-instinct's game.rs. The
//  inventory scanner and the paper guard must resolve exactly the same chain;
//  duplicating it invites one copy to read a stale or wrong pointer.
//
//----------------------------------------------------------------------------

#[inline]
pub fn player_data() -> Option<*const PlayerData> {
    unsafe {
        let game_data = game_data();
        if game_data.is_null() {
            return None;
        }
        let player = (*game_data).player_data;
        if player.is_null() { None } else { Some(player) }
    }
}

#[inline]
pub fn inventory_data() -> Option<*const InventoryData> {
    unsafe {
        let player = player_data()?;
        let inventory = (*player).inventory_data;
        if inventory.is_null() { None } else { Some(inventory) }
    }
}

/// Start of the runtime inventory rows (`InventoryData + 0x10`).
#[inline]
pub fn inventory_base() -> Option<usize> {
    inventory_data().map(|inventory| unsafe { (inventory as *const u8).add(0x10) as usize })
}
