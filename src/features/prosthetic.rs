//! Prosthetic-tool feature primitives and state.
//!
//! The slot layout and all per-frame prosthetic edge/queue state live here so
//! `core.rs` can orchestrate the feature without carrying a second copy of the
//! prosthetic state machine. The actual game calls remain in `core.rs` for now;
//! this is the same separation dec32/sekiro-battle-instinct uses between
//! `game.rs` and `core.rs`.

use crate::{
    action::SWITCH_PROSTHETIC,
    config::{Config, ToolCombo, ToolFirst, ToolTail},
    frame::Frames,
    game::{self, ID, InputHandler, ItemID, UID},
    input::ArtToken,
};

pub const PROSTHETIC_SUPRESSION_DURATION: u8 = 2;
pub const PROSTHETIC_RETURN_DELAY: Frames = Frames::standard(84);

// Slot index values used by the game's equip array.
pub const PROSTHETIC_SLOT_0: u8 = 0;
pub const PROSTHETIC_SLOT_1: u8 = 2;
pub const PROSTHETIC_SLOT_2: u8 = 4;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProstheticSlot {
    S0 = PROSTHETIC_SLOT_0,
    S1 = PROSTHETIC_SLOT_1,
    S2 = PROSTHETIC_SLOT_2,
}

impl ProstheticSlot {
    #[inline(always)]
    pub fn as_slot_index(self) -> usize {
        self as usize
    }
}

/// All prosthetic-specific state that used to be scattered across `Mod`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProstheticState {
    /// Pending first token for a prosthetic combo; waiting for q/t.
    pub tool_first: Option<ToolFirst>,
    pub tool_first_age: u16,
    /// While set, keep injecting USE_PROSTHETIC until this tail key is released.
    pub hold_tool_tail: Option<ToolTail>,
    /// Guarantee a few USE frames after equip (tap-friendly).
    pub tool_min_use: u8,
    /// Last successfully armed prosthetic UID (for return-to-default).
    pub cur_tool: Option<UID>,
    /// Frames left after releasing tool before returning to bare-`t` default.
    pub return_default_left: u16,
    /// Frames left where switching tools is blocked; `t` refreshes for multi-hit.
    pub tool_lock_left: u16,
    /// Last frame's switch-prosthetic field (after observing, before swallowing).
    pub switch_down_last: bool,
    pub tools_inited: bool,
    /// Edge memory for prosthetic first-key directions / r l e f.
    pub tool_keys_down: [bool; 4],
    pub tool_block_down: bool,
    pub tool_attack_down: bool,
    pub tool_interact_down: bool,
    pub tool_grapple_down: bool,
    pub tool_jump_down: bool,
    pub tool_sprint_down: bool,
    pub tool_crouch_down: bool,
    pub ejection: Option<(ItemID, ProstheticSlot)>,
    pub prosthetic_delay: u8,
}

impl ProstheticState {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn equip_prosthetic(tool: impl ID, slot: ProstheticSlot) -> bool {
    game::set_slot(tool, slot.as_slot_index())
}

pub fn get_prosthetic_tool(slot: ProstheticSlot) -> Option<ItemID> {
    let items = &unsafe { game::player_data()?.as_ref()? }.equiped_items;
    let item_id = items[slot.as_slot_index()];
    if item_id != 256 { ItemID::new(item_id) } else { None }
}

pub fn get_active_prosthetic_slot() -> ProstheticSlot {
    let active_prosthetic = unsafe { game::player_data().unwrap().as_ref().unwrap() }.activte_prosthetic;
    match active_prosthetic {
        0 => ProstheticSlot::S0,
        1 => ProstheticSlot::S1,
        2 => ProstheticSlot::S2,
        illegal_slot => unreachable!("Illegal prosthetic slot: {illegal_slot}"),
    }
}

pub fn locate_prosthetic_tool(tool: impl ID) -> Option<ProstheticSlot> {
    let items = &unsafe { game::player_data()?.as_ref()? }.equiped_items;
    let item_id = tool.get_item_id()?;
    [ProstheticSlot::S0, ProstheticSlot::S1, ProstheticSlot::S2]
        .into_iter()
        .find(|slot| items[slot.as_slot_index()] == item_id.get())
}

impl ProstheticState {
    pub fn revert_ejection_if_left_slot(&mut self) {
        let active_slot = get_active_prosthetic_slot();
        if let Some((ejected_tool, original_slot)) = self.ejection {
            if active_slot != original_slot {
                equip_prosthetic(ejected_tool, original_slot);
                self.ejection = None;
            }
        }
    }

    /// Equip `uid` into the **active** slot so the HUD shows it on the current highlight.
    pub fn equip_tool_uid(&mut self, uid: UID) -> bool {
        self.revert_ejection_if_left_slot();
        let active_slot = get_active_prosthetic_slot();
        let Some(want_id) = uid.get_item_id() else {
            return false;
        };

        if get_prosthetic_tool(active_slot) == Some(want_id) {
            return true;
        }

        if let Some(other_slot) = locate_prosthetic_tool(uid) {
            // Tool already in another slot: swap into the active (highlighted) slot.
            let displaced = get_prosthetic_tool(active_slot);
            if equip_prosthetic(uid, active_slot) {
                if let Some(displaced) = displaced {
                    let _ = equip_prosthetic(displaced, other_slot);
                }
                return true;
            }
            return false;
        }

        // Not equipped anywhere: inject into active slot, remember what we ejected.
        let active_tool = get_prosthetic_tool(active_slot);
        if equip_prosthetic(uid, active_slot) {
            if let Some(active_tool) = active_tool {
                self.ejection.get_or_insert((active_tool, active_slot));
            }
            true
        } else {
            false
        }
    }

    pub fn return_to_default_tool(&mut self, config: &Config) {
        self.return_default_left = 0;
        self.tool_lock_left = 0;
        self.hold_tool_tail = None;
        self.tool_min_use = 0;
        if let Some(uid) = config.tool_on_t {
            if self.equip_tool_uid(uid) {
                self.cur_tool = Some(uid);
                self.prosthetic_delay = PROSTHETIC_SUPRESSION_DURATION;
            }
        }
    }

    pub fn refresh_tool_lock(&mut self, config: &Config) {
        self.tool_lock_left = config.tool_lock_frames(self.cur_tool);
        self.return_default_left = 0;
    }

    pub fn arm_tool(&mut self, uid: UID, tail: ToolTail, config: &Config) -> bool {
        // During lock, only allow re-arming the same tool (multi-hit via t).
        if self.tool_lock_left > 0 && self.cur_tool.is_some_and(|cur| cur != uid) {
            return false;
        }
        if self.equip_tool_uid(uid) {
            self.cur_tool = Some(uid);
            self.hold_tool_tail = Some(tail);
            self.tool_min_use = 3;
            self.prosthetic_delay = PROSTHETIC_SUPRESSION_DURATION;
            self.refresh_tool_lock(config);
            self.tool_first = None;
            self.tool_first_age = 0;
            true
        } else {
            false
        }
    }

    pub fn refresh_tool_hold(&mut self, tail: ToolTail, config: &Config) {
        self.refresh_tool_lock(config);
        self.tool_min_use = self.tool_min_use.max(3);
        self.hold_tool_tail = Some(tail);
        self.prosthetic_delay = 0;
        self.tool_first = None;
    }

    /// Try to complete a pending first-key with `tail`. Returns true if a tool was armed.
    pub fn try_complete_tool_tail(&mut self, tail: ToolTail, config: &Config) -> bool {
        if self.tool_lock_left > 0 {
            // Multi-hit: refresh when the arming tail (or use/switch) is pressed again.
            let same_tail = self.hold_tool_tail == Some(tail);
            let use_or_switch = matches!(tail, ToolTail::Use | ToolTail::Switch);
            if same_tail || use_or_switch {
                self.refresh_tool_hold(tail, config);
                return true;
            }
        }
        if let Some(first) = self.tool_first {
            if let Some(uid) = config.tool(ToolCombo { first, tail }) {
                return self.arm_tool(uid, tail, config);
            }
            // Pending first did not match this tail — clear so it does not stale.
            self.tool_first = None;
            self.tool_first_age = 0;
        }
        false
    }

    pub fn push_tool_first(&mut self, token: ToolFirst) {
        self.tool_first = Some(token);
        self.tool_first_age = 0;
    }

    /// Track normal action tokens as the prosthetic first key; on another normal
    /// token or q/t complete the combo → equip + hold-inject USE.
    /// Bare `t` arms `tool_on_t`. Bare `q` arms `tool_on_q` (same fire style; return-default stays `t`).
    /// Returns true when a tool was armed via a normal action tail (caller should clear art combo).
    pub fn handle_prosthetic_input(
        &mut self,
        config: &Config,
        input_handler: &mut InputHandler,
        up: bool,
        right: bool,
        down: bool,
        left: bool,
        blocking: bool,
        attacking: bool,
        interacting: bool,
        grappling: bool,
        jumping: bool,
        sprinting: bool,
        crouching: bool,
        using_tool: bool,
        used_tool_just_now: bool,
    ) -> bool {
        let mut armed_via_art_tail = false;

        // First-key edges for directions only (r/l/e/f may complete a combo as tail first).
        for (i, (held, token)) in [
            (up, ArtToken::Up),
            (right, ArtToken::Right),
            (down, ArtToken::Down),
            (left, ArtToken::Left),
        ]
        .into_iter()
        .enumerate()
        {
            if held && !self.tool_keys_down[i] {
                self.push_tool_first(ToolFirst::from_art(token));
            }
            self.tool_keys_down[i] = held;
        }

        if blocking && !self.tool_block_down {
            if self.try_complete_tool_tail(ToolTail::Block, config) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Block);
            }
        }
        self.tool_block_down = blocking;

        if attacking && !self.tool_attack_down {
            if self.try_complete_tool_tail(ToolTail::Attack, config) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Attack);
            }
        }
        self.tool_attack_down = attacking;

        if interacting && !self.tool_interact_down {
            if self.try_complete_tool_tail(ToolTail::Interact, config) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Interact);
            }
        }
        self.tool_interact_down = interacting;

        if grappling && !self.tool_grapple_down {
            if self.try_complete_tool_tail(ToolTail::Grapple, config) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Grapple);
            }
        }
        self.tool_grapple_down = grappling;

        if jumping && !self.tool_jump_down {
            if self.try_complete_tool_tail(ToolTail::Jump, config) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Jump);
            }
        }
        self.tool_jump_down = jumping;

        if sprinting && !self.tool_sprint_down {
            if self.try_complete_tool_tail(ToolTail::Sprint, config) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Sprint);
            }
        }
        self.tool_sprint_down = sprinting;

        if crouching && !self.tool_crouch_down {
            if self.try_complete_tool_tail(ToolTail::Crouch, config) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Crouch);
            }
        }
        self.tool_crouch_down = crouching;

        if self.tool_first.is_some() {
            self.tool_first_age = self.tool_first_age.saturating_add(1);
            if self.tool_first_age >= config.tool_combo_max_age {
                self.tool_first = None;
                self.tool_first_age = 0;
            }
        }

        // q is reserved as a trigger key in this MOD. Always swallow the native
        // 3-slot switch action, even when no available binding currently uses q.
        let switch_down = input_handler.action & SWITCH_PROSTHETIC != 0;
        if switch_down {
            input_handler.action &= !SWITCH_PROSTHETIC;
            input_handler.action_b &= !SWITCH_PROSTHETIC;
            input_handler.action_c &= !SWITCH_PROSTHETIC;
        }
        let switch_rising = switch_down && !self.switch_down_last;
        self.switch_down_last = switch_down;

        if switch_rising {
            if self.tool_lock_left > 0 {
                self.refresh_tool_hold(ToolTail::Switch, config);
            } else if let Some(first) = self.tool_first {
                if let Some(uid) = config.tool(ToolCombo {
                    first,
                    tail: ToolTail::Switch,
                }) {
                    let _ = self.arm_tool(uid, ToolTail::Switch, config);
                } else {
                    self.tool_first = None;
                }
            } else if let Some(uid) = config.tool_on_q {
                let _ = self.arm_tool(uid, ToolTail::Switch, config);
            } else {
                self.push_tool_first(ToolFirst::Switch);
            }
        }

        if used_tool_just_now {
            if self.tool_lock_left > 0 {
                self.refresh_tool_hold(ToolTail::Use, config);
            } else if let Some(first) = self.tool_first {
                if let Some(uid) = config.tool(ToolCombo {
                    first,
                    tail: ToolTail::Use,
                }) {
                    let _ = self.arm_tool(uid, ToolTail::Use, config);
                } else {
                    self.tool_first = None;
                }
            } else if let Some(uid) = config.tool_on_t {
                let _ = self.arm_tool(uid, ToolTail::Use, config);
            }
        }

        // Sustain / release of armed tool.
        if self.hold_tool_tail.is_some() || self.tool_min_use > 0 {
            let held = match self.hold_tool_tail {
                Some(ToolTail::Switch) => switch_down,
                Some(ToolTail::Use) => using_tool || used_tool_just_now,
                Some(ToolTail::Block) => blocking,
                Some(ToolTail::Attack) => attacking,
                Some(ToolTail::Interact) => interacting,
                Some(ToolTail::Grapple) => grappling,
                Some(ToolTail::Jump) => jumping,
                Some(ToolTail::Sprint) => sprinting,
                Some(ToolTail::Crouch) => crouching,
                None => false,
            };
            if self.prosthetic_delay > 0 || held || self.tool_min_use > 0 {
                self.return_default_left = 0;
            } else {
                self.hold_tool_tail = None;
            }
        }

        if self.tool_lock_left > 0 {
            self.return_default_left = 0;
            let still_firing = self.hold_tool_tail.is_some() || self.tool_min_use > 0 || self.prosthetic_delay > 0;
            if !still_firing {
                self.tool_lock_left -= 1;
                if self.tool_lock_left == 0 {
                    let is_default = config.tool_on_t.is_some() && self.cur_tool == config.tool_on_t;
                    if !is_default {
                        self.return_default_left = PROSTHETIC_RETURN_DELAY.as_actual();
                    }
                }
            }
        } else if self.return_default_left > 0 {
            self.return_default_left -= 1;
            if self.return_default_left == 0 {
                self.return_to_default_tool(config);
            }
        }

        armed_via_art_tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_indices_match_the_game_equip_array() {
        assert_eq!(ProstheticSlot::S0.as_slot_index(), 0);
        assert_eq!(ProstheticSlot::S1.as_slot_index(), 2);
        assert_eq!(ProstheticSlot::S2.as_slot_index(), 4);
    }

    #[test]
    fn pushing_a_first_token_resets_its_age() {
        let mut state = ProstheticState::new();
        state.tool_first_age = 12;
        state.push_tool_first(ToolFirst::Up);
        assert_eq!(state.tool_first, Some(ToolFirst::Up));
        assert_eq!(state.tool_first_age, 0);
    }
}
