use std::ffi::c_void;

//----------------------------------------------------------------------------
//
//  Addresses of functions and objects from the original program
//
//----------------------------------------------------------------------------

// functions
pub const PROCESS_INPUT: usize = 0x140B2C190;
pub const GET_ITEM_ID: usize = 0x140C3D680;
pub const SET_SLOT: usize = 0x140D592F0;
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

// objects
pub const GAME_DATA: usize = 0x143D5AAC0;
pub const WORLD_DATA: usize = 0x143D7A1E0;

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
    /// Primary action bitfield (ATTACK/BLOCK/JUMP/DODGE/USE_PROSTHETIC/…).
    pub action: u64,
    /// Neighboring words — used when probing undocumented binds (e.g. switch prosthetic).
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
    fn set_slot(equip_slot: usize, equip_data: *const EquipData, ignore_equip_lock: bool);
}
