//! Sekiro action-bit fields and input rewriting helpers.
//!
//! Keeping the bitmasks and the three input words in one place mirrors the
//! upstream projects' small `game.rs` surface: feature code asks for an action
//! mask instead of re-declaring magic constants.

use crate::game::InputHandler;

pub const ATTACK: u64 = 0x1;
pub const BLOCK: u64 = 0x4;
pub const JUMP: u64 = 0x10;
pub const SPRINT: u64 = 0x20;
pub const INTERACT: u64 = 0x1000;
pub const GRAPPLE: u64 = 0x20000000;
pub const DODGE: u64 = 0x2000;
pub const CROUCH: u64 = 0x8000;
pub const USE_PROSTHETIC: u64 = 0x40040002;
pub const SWITCH_PROSTHETIC: u64 = 0x400;

/// Vanilla combat-art activation (skill slot reads real keyboard B+A).
pub const VANILLA_ART_INPUT: u64 = BLOCK | ATTACK;

#[inline]
pub fn strip_input(input: &mut InputHandler, mask: u64) {
    input.action &= !mask;
    input.action_b &= !mask;
    input.action_c &= !mask;
}

#[inline]
pub fn or_input(input: &mut InputHandler, mask: u64) {
    input.action |= mask;
}
