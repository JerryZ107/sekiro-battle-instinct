//! Prosthetic-tool feature primitives and state.
//!
//! The slot layout and all per-frame prosthetic edge/queue state live here so
//! `core.rs` can orchestrate the feature without carrying a second copy of the
//! prosthetic state machine. The actual game calls remain in `core.rs` for now;
//! this is the same separation dec32/sekiro-battle-instinct uses between
//! `game.rs` and `core.rs`.

use crate::{
    action::SWITCH_PROSTHETIC,
    config::{Config, ToolCombo, ToolCombo3, ToolFirst, ToolTail},
    frame::Frames,
    game::{self, ID, InputHandler, ItemID, UID},
    input::{ActionFrame, ArtToken},
};

pub const PROSTHETIC_SUPRESSION_DURATION: u8 = 2;
pub const PROSTHETIC_RETURN_DELAY: Frames = Frames::standard(84);

/// Middle -> tail stage of a three-key prosthetic binding (1s). The first
/// and middle keys must both still be held when the tail is pressed.
const TOOL_TRIPLE_TAIL_WINDOW: Frames = Frames::standard(60);

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

    #[inline(always)]
    pub fn as_prosthetic_index(self) -> u32 {
        self as u32 / 2
    }
}

/// All prosthetic-specific state that used to be scattered across `Mod`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProstheticState {
    /// Pending first token for a prosthetic combo; waiting for q/t.
    pub tool_first: Option<ToolFirst>,
    pub tool_first_age: u16,
    /// Middle key of a three-key prosthetic sequence.
    pub tool_middle: Option<ToolFirst>,
    pub tool_middle_age: u16,
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
    pub tools_inited: bool,
    pub ejection: Option<(ItemID, ProstheticSlot)>,
    pub prosthetic_delay: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolTailMatch {
    Arm(UID),
    Middle(ToolFirst),
    None,
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

/// Activate an already-equipped prosthetic slot through the game's native
/// function, avoiding an inventory-row swap and the associated HUD flicker.
pub fn activate_prosthetic_slot(slot: ProstheticSlot) -> bool {
    use std::ffi::c_void;
    let character_base: *const c_void =
        unsafe { game::resolve_pointer_chain(game::WORLD_DATA, [0x88, 0x1F10, 0x10, 0xF8, 0x10, 0x18, 0x00]) };
    if character_base.is_null() {
        return false;
    }
    let unknown = unsafe { *(character_base.byte_add(0x10) as *const *const c_void) };
    if unknown.is_null() {
        return false;
    }
    game::set_equipped_prosthetic(unknown, 0, slot.as_prosthetic_index());
    true
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
            // Tool is already equipped: activate its native slot instead of
            // swapping inventory rows. This is the upstream approach and avoids
            // HUD flicker plus the displaced-tool round trip.
            if activate_prosthetic_slot(other_slot) {
                return true;
            }

            // Fallback for a build where the native slot-activation chain is not
            // available: preserve the old swap-into-active-slot behavior.
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
        self.tool_lock_left = config.tool_lock(self.cur_tool).as_actual();
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
            self.clear_tool_sequence();
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
        self.clear_tool_sequence();
    }

    fn clear_tool_sequence(&mut self) {
        self.tool_first = None;
        self.tool_first_age = 0;
        self.tool_middle = None;
        self.tool_middle_age = 0;
    }

    /// Try to complete a pending first/middle sequence with `tail`.
    /// Returns true if a tool was armed.
    pub fn try_complete_tool_tail(&mut self, tail: ToolTail, config: &Config, frame: &ActionFrame) -> bool {
        if self.tool_lock_left > 0 {
            // Multi-hit: refresh when the arming tail (or use/switch) is pressed again.
            let same_tail = self.hold_tool_tail == Some(tail);
            let use_or_switch = matches!(tail, ToolTail::Use | ToolTail::Switch);
            if same_tail || use_or_switch {
                self.refresh_tool_hold(tail, config);
                return true;
            }
        }

        match self.classify_tool_tail(tail, config, frame) {
            ToolTailMatch::Arm(uid) => self.arm_tool(uid, tail, config),
            ToolTailMatch::Middle(middle) => {
                self.tool_middle = Some(middle);
                self.tool_middle_age = 0;
                false
            }
            ToolTailMatch::None => {
                self.clear_tool_sequence();
                false
            }
        }
    }

    /// Pure trigger resolution for tests and for keeping pair priority explicit:
    /// an exact two-key match always wins over using the edge as a triple middle.
    fn classify_tool_tail(&self, tail: ToolTail, config: &Config, frame: &ActionFrame) -> ToolTailMatch {
        let Some(first) = self.tool_first else {
            return ToolTailMatch::None;
        };
        let first_window = config.tool_window().as_actual();

        // Once a middle key exists, only the exact three-key sequence may fire.
        if let Some(middle) = self.tool_middle {
            let chain_held = tool_first_held(first, frame) && tool_first_held(middle, frame);
            if chain_held && self.tool_middle_age < TOOL_TRIPLE_TAIL_WINDOW.as_actual() {
                if let Some(uid) = config.tool3(ToolCombo3 { first, middle, tail }) {
                    return ToolTailMatch::Arm(uid);
                }
            }
            return ToolTailMatch::None;
        }

        // Two-key trigger priority: if this tail completes a configured pair
        // while the first key is still held, fire it and never treat the same
        // edge as a triple middle key. This mirrors the three-key hold chain.
        if tool_first_held(first, frame) {
            if let Some(uid) = config.tool(ToolCombo { first, tail }) {
                return ToolTailMatch::Arm(uid);
            }
        }

        // No pair matched. Keep this edge as the middle key only when the
        // first key is still held and the 0.3s first-key window is valid.
        if tool_first_held(first, frame) && self.tool_first_age < first_window {
            if let Some(middle) = middle_for_tail(tail) {
                return ToolTailMatch::Middle(middle);
            }
        }
        ToolTailMatch::None
    }

    /// A direction cannot be a prosthetic tail, but it can be the middle key
    /// of a triple. This keeps the first -> middle hold requirement.
    fn try_push_tool_middle(&mut self, middle: ToolFirst, config: &Config, frame: &ActionFrame) -> bool {
        if self.tool_middle.is_some() {
            self.clear_tool_sequence();
            return false;
        }
        let Some(first) = self.tool_first else {
            return false;
        };
        if tool_first_held(first, frame) && self.tool_first_age < config.tool_window().as_actual() {
            self.tool_middle = Some(middle);
            self.tool_middle_age = 0;
            true
        } else {
            self.clear_tool_sequence();
            false
        }
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
        frame: &ActionFrame,
    ) -> bool {
        let mut armed_via_art_tail = false;

        // Directions can start a sequence or become its middle key; they can
        // never be the tail of a prosthetic bind.
        for (token, first) in [
            (ArtToken::Up, ToolFirst::Up),
            (ArtToken::Right, ToolFirst::Right),
            (ArtToken::Down, ToolFirst::Down),
            (ArtToken::Left, ToolFirst::Left),
        ] {
            if frame.pressed(token) {
                if !self.try_push_tool_middle(first, config, frame) && self.tool_middle.is_none() {
                    self.push_tool_first(first);
                }
            }
        }

        for (token, tail) in [
            (ArtToken::Block, ToolTail::Block),
            (ArtToken::Attack, ToolTail::Attack),
            (ArtToken::Interact, ToolTail::Interact),
            (ArtToken::Grapple, ToolTail::Grapple),
            (ArtToken::Jump, ToolTail::Jump),
            (ArtToken::Sprint, ToolTail::Sprint),
            (ArtToken::Crouch, ToolTail::Crouch),
        ] {
            if frame.pressed(token) {
                if self.try_complete_tool_tail(tail, config, frame) {
                    armed_via_art_tail = true;
                } else if self.tool_middle.is_none() {
                    if let Some(first) = middle_for_tail(tail) {
                        self.push_tool_first(first);
                    }
                }
            }
        }

        if self.tool_first.is_some() {
            let first_window = config.tool_window().as_actual();
            if self.tool_middle.is_some() {
                self.tool_middle_age = self.tool_middle_age.saturating_add(1);
                if self.tool_middle_age >= TOOL_TRIPLE_TAIL_WINDOW.as_actual() {
                    self.clear_tool_sequence();
                }
            } else {
                self.tool_first_age = self.tool_first_age.saturating_add(1);
                if self.tool_first_age >= first_window {
                    self.clear_tool_sequence();
                }
            }
        }

        // q is reserved as a trigger key in this MOD. Always swallow the native
        // 3-slot switch action, even when no available binding currently uses q.
        let switch_down = frame.switch_prosthetic();
        if switch_down {
            input_handler.action &= !SWITCH_PROSTHETIC;
            input_handler.action_b &= !SWITCH_PROSTHETIC;
            input_handler.action_c &= !SWITCH_PROSTHETIC;
        }

        if frame.switch_prosthetic_pressed() {
            let had_pending = self.tool_first.is_some();
            if self.tool_lock_left > 0 {
                self.refresh_tool_hold(ToolTail::Switch, config);
            } else if self.try_complete_tool_tail(ToolTail::Switch, config, frame) {
                // armed through a two- or three-key sequence
            } else if !had_pending {
                if let Some(uid) = config.tool_on_q {
                    let _ = self.arm_tool(uid, ToolTail::Switch, config);
                } else {
                    self.push_tool_first(ToolFirst::Switch);
                }
            }
        }

        if frame.use_prosthetic_pressed() {
            let had_pending = self.tool_first.is_some();
            if self.tool_lock_left > 0 {
                self.refresh_tool_hold(ToolTail::Use, config);
            } else if self.try_complete_tool_tail(ToolTail::Use, config, frame) {
                // armed through a two- or three-key sequence
            } else if !had_pending {
                if let Some(uid) = config.tool_on_t {
                    let _ = self.arm_tool(uid, ToolTail::Use, config);
                }
            }
        }

        // Sustain / release of armed tool.
        if self.hold_tool_tail.is_some() || self.tool_min_use > 0 {
            let held = match self.hold_tool_tail {
                Some(ToolTail::Switch) => switch_down,
                Some(ToolTail::Use) => frame.use_prosthetic(),
                Some(ToolTail::Block) => frame.held(ArtToken::Block),
                Some(ToolTail::Attack) => frame.held(ArtToken::Attack),
                Some(ToolTail::Interact) => frame.held(ArtToken::Interact),
                Some(ToolTail::Grapple) => frame.held(ArtToken::Grapple),
                Some(ToolTail::Jump) => frame.held(ArtToken::Jump),
                Some(ToolTail::Sprint) => frame.held(ArtToken::Sprint),
                Some(ToolTail::Crouch) => frame.held(ArtToken::Crouch),
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

    fn held_frame(tokens: &[ArtToken]) -> ActionFrame {
        let mut bits = 0u16;
        for token in tokens {
            bits |= token.bit();
        }
        ActionFrame::from_held_for_test(bits, 0)
    }

    #[test]
    fn three_key_tool_requires_hold_chain_and_stage_window() {
        let mut config = Config::default();
        config.tools3.insert(
            ToolCombo3 {
                first: ToolFirst::Up,
                middle: ToolFirst::Right,
                tail: ToolTail::Block,
            },
            75300,
        );
        let mut state = ProstheticState::new();
        state.tool_first = Some(ToolFirst::Up);
        state.tool_first_age = 0;
        state.tool_middle = Some(ToolFirst::Right);
        state.tool_middle_age = 0;

        let both_held = held_frame(&[ArtToken::Up, ArtToken::Right]);
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &both_held),
            ToolTailMatch::Arm(75300)
        );

        let first_released = held_frame(&[ArtToken::Right]);
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &first_released),
            ToolTailMatch::None
        );

        let middle_released = held_frame(&[ArtToken::Up]);
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &middle_released),
            ToolTailMatch::None
        );

        // A long middle -> tail pause is allowed (up to 1s).
        state.tool_middle_age = 45;
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &both_held),
            ToolTailMatch::Arm(75300)
        );

        state.tool_middle_age = TOOL_TRIPLE_TAIL_WINDOW.as_actual();
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &both_held),
            ToolTailMatch::None
        );
    }

    #[test]
    fn two_key_tool_requires_first_key_held_when_tail_is_pressed() {
        let mut config = Config::default();
        config.tools.insert(
            ToolCombo {
                first: ToolFirst::Up,
                tail: ToolTail::Block,
            },
            75300,
        );
        let mut state = ProstheticState::new();
        state.push_tool_first(ToolFirst::Up);

        // Tail edge after the first key was released must not arm the tool.
        let tail_only = held_frame(&[ArtToken::Block]);
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &tail_only),
            ToolTailMatch::None
        );

        // With the first key still held, the pair fires.
        let both_held = held_frame(&[ArtToken::Up, ArtToken::Block]);
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &both_held),
            ToolTailMatch::Arm(75300)
        );
    }

    #[test]
    fn double_tool_match_wins_over_becoming_middle_key() {
        let mut config = Config::default();
        config.tools.insert(
            ToolCombo {
                first: ToolFirst::Up,
                tail: ToolTail::Block,
            },
            75300,
        );
        let mut state = ProstheticState::new();
        state.tool_first = Some(ToolFirst::Up);
        state.tool_first_age = 0;
        let frame = held_frame(&[ArtToken::Up, ArtToken::Block]);
        assert_eq!(
            state.classify_tool_tail(ToolTail::Block, &config, &frame),
            ToolTailMatch::Arm(75300)
        );
    }

    #[test]
    fn direction_can_be_middle_key_but_not_tail() {
        let config = Config::default();
        let mut state = ProstheticState::new();
        state.push_tool_first(ToolFirst::Up);
        let frame = held_frame(&[ArtToken::Up, ArtToken::Right]);
        assert!(state.try_push_tool_middle(ToolFirst::Right, &config, &frame));
        assert_eq!(state.tool_middle, Some(ToolFirst::Right));
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

fn middle_for_tail(tail: ToolTail) -> Option<ToolFirst> {
    Some(match tail {
        ToolTail::Block => ToolFirst::Block,
        ToolTail::Attack => ToolFirst::Attack,
        ToolTail::Interact => ToolFirst::Interact,
        ToolTail::Grapple => ToolFirst::Grapple,
        ToolTail::Jump => ToolFirst::Jump,
        ToolTail::Sprint => ToolFirst::Sprint,
        ToolTail::Crouch => ToolFirst::Crouch,
        ToolTail::Switch => ToolFirst::Switch,
        ToolTail::Use => return None,
    })
}

fn tool_first_held(first: ToolFirst, frame: &ActionFrame) -> bool {
    match first {
        ToolFirst::Up => frame.held(ArtToken::Up),
        ToolFirst::Right => frame.held(ArtToken::Right),
        ToolFirst::Down => frame.held(ArtToken::Down),
        ToolFirst::Left => frame.held(ArtToken::Left),
        ToolFirst::Block => frame.held(ArtToken::Block),
        ToolFirst::Attack => frame.held(ArtToken::Attack),
        ToolFirst::Interact => frame.held(ArtToken::Interact),
        ToolFirst::Grapple => frame.held(ArtToken::Grapple),
        ToolFirst::Jump => frame.held(ArtToken::Jump),
        ToolFirst::Sprint => frame.held(ArtToken::Sprint),
        ToolFirst::Crouch => frame.held(ArtToken::Crouch),
        ToolFirst::Switch => frame.switch_prosthetic(),
    }
}
