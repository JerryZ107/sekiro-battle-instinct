//! Combat-art feature primitives.
//!
//! UID aliases, fallback tables and the swap-out countdown live here, separated
//! from the input orchestration in `core.rs`. This mirrors the upstream
//! dec32/sekiro-battle-instinct layout, where `game.rs` owns IDs and `core.rs`
//! owns only the per-tick decisions.

use crate::{
    action::{ATTACK, BLOCK, CROUCH, DODGE, GRAPPLE, INTERACT, JUMP, SPRINT},
    frame::Frames,
    game::{self, COMBAT_ART_SLOT, ID, ItemID, UID},
    input::{ArtCombo, ArtComboWindow, ArtToken},
};

// Combat-art injection timing (frames at 60 fps; Frames/actual conversion is
// applied by the caller where needed).
pub const BLOCK_INJECTION_DURATION: u8 = 10;
pub const ART_BLOCK_INJECTION_DURATION: u8 = 2;
pub const ATTACK_SUPRESSION_DURATION: u8 = 4;
pub const SLOT_SWAP_SETTLE_DURATION: u8 = 6;

// UIDs for vanilla combat arts with special swap-out timing/fallbacks.
pub const ASHINA_CROSS: UID = 5500;
pub const ONE_MIND: UID = 6100;
pub const SAKURA_DANCE: UID = 7700;
pub const ICHIMONJI: UID = 5300;
pub const ICHIMONJI_DOUBLE: UID = 7100;
pub const PRAYING_STRIKES: UID = 5900;
pub const PRAYING_STRIKES_EXORCISM: UID = 7500;
pub const SENPO_LEAPING_KICKS: UID = 5800;
pub const HIGH_MONK: UID = 7400;
pub const SHADOWRUSH: UID = 6000;
pub const SHADOWFALL: UID = 7600;
pub const MORTAL_DRAW: UID = 5700;
pub const EMPOWERED_MORTAL_DRAW: UID = 7300;

/// Fallback UID if the upgraded art is not owned by the current save.
pub fn art_fallback(uid: UID) -> Option<UID> {
    match uid {
        ICHIMONJI_DOUBLE => Some(ICHIMONJI),
        PRAYING_STRIKES_EXORCISM => Some(PRAYING_STRIKES),
        HIGH_MONK => Some(SENPO_LEAPING_KICKS),
        SHADOWFALL => Some(SHADOWRUSH),
        EMPOWERED_MORTAL_DRAW => Some(MORTAL_DRAW),
        _ => None,
    }
}

/// Cooldown/timing table for arts that need a settle window after swapping.
pub trait CombatArt: Sized {
    fn is_sheathed(self) -> bool;
    fn swapout_cooldown(self) -> Frames;
}

impl CombatArt for UID {
    fn is_sheathed(self) -> bool {
        matches!(self, ASHINA_CROSS | ONE_MIND)
    }

    fn swapout_cooldown(self) -> Frames {
        let frames = match self {
            ASHINA_CROSS => 75,
            ONE_MIND => 240,
            SAKURA_DANCE => 60,
            _ => 40,
        };
        Frames::standard(frames)
    }
}

impl CombatArt for Option<UID> {
    fn is_sheathed(self) -> bool {
        self.map(CombatArt::is_sheathed).unwrap_or(false)
    }

    fn swapout_cooldown(self) -> Frames {
        self.map(CombatArt::swapout_cooldown).unwrap_or(Frames::standard(0))
    }
}

/// Small frame countdown used by the combat-art settle logic. Keeping it here
/// avoids allocating and makes the countdown state part of the feature rather
/// than the global `Mod` state.
pub struct Countdown {
    value: u16,
    running: bool,
}

impl Countdown {
    pub const fn zero() -> Countdown {
        Countdown {
            value: 0,
            running: false,
        }
    }

    pub fn new(value: Frames) -> Countdown {
        Countdown {
            value: value.as_actual(),
            running: false,
        }
    }

    pub fn count(&mut self) {
        self.value -= 1;
        self.running = true;
    }

    pub fn count_on(&mut self, cond: bool) {
        if cond || self.running {
            self.count();
        }
    }

    pub fn is_done(&self) -> bool {
        self.value == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_starts_idle_and_runs_to_zero() {
        let mut countdown = Countdown::new(Frames::standard(3));
        assert!(!countdown.is_done());
        countdown.count_on(true);
        countdown.count_on(true);
        countdown.count_on(true);
        assert!(countdown.is_done());
    }

    #[test]
    fn combo_mask_includes_sprint_dodge_companion() {
        assert_eq!(
            combo_action_mask(ArtCombo::pair(ArtToken::Block, ArtToken::Sprint)),
            BLOCK | SPRINT | DODGE
        );
        assert_eq!(
            combo_action_mask(ArtCombo::pair(ArtToken::Block, ArtToken::Attack)),
            BLOCK | ATTACK
        );
        assert_eq!(combo_action_mask(ArtCombo::empty()), 0);
    }

    #[test]
    fn queued_fire_waits_for_release_then_flushes_after_delay() {
        let mut state = CombatArtState::new(18);
        state.queued_art = Some((SAKURA_DANCE, ArtToken::Attack));
        state.queue_flush_left = 2;
        state.queue_flush_skip_tick = true;
        assert!(state.try_flush_queued().is_none()); // skip tick
        assert!(state.try_flush_queued().is_none()); // decrement 2 -> 1
        assert_eq!(state.try_flush_queued(), Some((SAKURA_DANCE, ArtToken::Attack)));
    }

    #[test]
    fn upgraded_arts_fall_back_to_base_versions() {
        assert_eq!(art_fallback(ICHIMONJI_DOUBLE), Some(ICHIMONJI));
        assert_eq!(art_fallback(PRAYING_STRIKES_EXORCISM), Some(PRAYING_STRIKES));
        assert_eq!(art_fallback(HIGH_MONK), Some(SENPO_LEAPING_KICKS));
        assert_eq!(art_fallback(SHADOWFALL), Some(SHADOWRUSH));
        assert_eq!(art_fallback(EMPOWERED_MORTAL_DRAW), Some(MORTAL_DRAW));
        assert_eq!(art_fallback(ICHIMONJI), None);
    }
}

/// All combat-art-specific state that used to be scattered across `Mod`.
pub struct CombatArtState {
    pub combo: ArtComboWindow,
    pub cur_art: Option<UID>,
    pub swapout_countdown: Countdown,
    pub attack_delay: u8,
    pub injected_blocks: u8,
    /// After a fresh art swap, start rl fire once attack_delay ends.
    pub pending_rl_attack: bool,
    /// Strip vanilla B+A until block released after a late `l` outside the rl window.
    pub suppress_late_rl_skill: bool,
    /// Keep injecting ATTACK while this combo's last token stays held.
    pub hold_for_attack: Option<ArtToken>,
    /// Remaining BLOCK-only prime frames (ATTACK suppressed) at art start.
    pub art_block_inject_left: u8,
    /// True after the first BLOCK|ATTACK frame that opens the art.
    pub art_attack_latched: bool,
    /// Native action bits of the combo that fired; stripped until injection ends.
    pub combo_suppress_mask: u64,
    /// Latest combo art waiting until sustained fire ends (size 1, newest wins).
    pub queued_art: Option<(UID, ArtToken)>,
    /// Frames left after last-key release before flushing `queued_art` (from cfg).
    pub queue_flush_left: u16,
    /// Skip one flush tick so delay starts counting from the frame after release.
    pub queue_flush_skip_tick: bool,
    /// When hold key is already up (typical after queue flush), keep firing this many frames.
    pub art_tap_frames: u8,
}

impl CombatArtState {
    pub fn new(rl_combo_max_age: u16) -> Self {
        Self {
            combo: ArtComboWindow::new(rl_combo_max_age),
            cur_art: None,
            swapout_countdown: Countdown::zero(),
            attack_delay: 0,
            injected_blocks: 0,
            pending_rl_attack: false,
            suppress_late_rl_skill: false,
            hold_for_attack: None,
            art_block_inject_left: 0,
            art_attack_latched: false,
            combo_suppress_mask: 0,
            queued_art: None,
            queue_flush_left: 0,
            queue_flush_skip_tick: false,
            art_tap_frames: 0,
        }
    }
}

impl CombatArtState {
    /// True while a combo-fired art is still settling or hold-injecting.
    pub fn fire_busy(&self) -> bool {
        self.hold_for_attack.is_some()
            || self.pending_rl_attack
            || self.attack_delay > 0
            || self.art_block_inject_left > 0
            || self.art_tap_frames > 0
    }

    pub fn clear_fire(&mut self) {
        self.hold_for_attack = None;
        self.art_block_inject_left = 0;
        self.art_attack_latched = false;
        self.pending_rl_attack = false;
        self.art_tap_frames = 0;
        self.combo_suppress_mask = 0;
    }

    /// End sustained fire; if a queued art is waiting, start the cfg flush delay.
    pub fn end_fire_for_queue(&mut self, queue_delay_frames: u16) {
        self.clear_fire();
        if self.queued_art.is_some() {
            self.queue_flush_left = queue_delay_frames;
            self.queue_flush_skip_tick = true;
            log::debug!(
                "ART_QUEUE_WAIT delay={} uid={:?}",
                self.queue_flush_left,
                self.queued_art.as_ref().map(|(uid, _)| *uid),
            );
        }
    }

    pub fn clear_queued(&mut self) {
        self.queued_art = None;
        self.queue_flush_left = 0;
        self.queue_flush_skip_tick = false;
    }

    pub fn try_flush_queued(&mut self) -> Option<(UID, ArtToken)> {
        if self.queued_art.is_none() {
            return None;
        }
        if self.queue_flush_skip_tick {
            self.queue_flush_skip_tick = false;
            if self.queue_flush_left == 0 {
                return self.queued_art.take();
            }
            return None;
        }
        if self.queue_flush_left > 0 {
            self.queue_flush_left -= 1;
            if self.queue_flush_left == 0 {
                return self.queued_art.take();
            }
            return None;
        }
        self.queued_art.take()
    }

    pub fn arm_combo_fire(&mut self, hold: ArtToken, held: bool) {
        self.hold_for_attack = Some(hold);
        self.art_block_inject_left = 0;
        self.art_attack_latched = false;
        // If the last key is already up (common when flushing a queued tap), auto-fire briefly.
        self.art_tap_frames = if held {
            0
        } else {
            ART_BLOCK_INJECTION_DURATION.saturating_add(3)
        };
    }
}

pub fn get_equipped_combat_art_item_id() -> Option<ItemID> {
    let items = &unsafe { game::player_data()?.as_ref()? }.equiped_items;
    let item_id = items[COMBAT_ART_SLOT as usize];
    if item_id != 256 { ItemID::new(item_id) } else { None }
}

pub fn combat_art_slot_matches(art: UID) -> bool {
    match (art.get_item_id(), get_equipped_combat_art_item_id()) {
        (Some(want), Some(have)) => want == have,
        _ => false,
    }
}

pub fn set_combat_art(art: impl ID) -> bool {
    game::set_slot(art, COMBAT_ART_SLOT as usize)
}

/// Native action bits owned by a two-token combo. `Sprint` also suppresses the
/// companion dodge/step action so the combo does not get preempted.
pub fn combo_action_mask(combo: ArtCombo) -> u64 {
    [combo.first(), combo.second()]
        .into_iter()
        .flatten()
        .map(|token| match token {
            ArtToken::Block => BLOCK,
            ArtToken::Attack => ATTACK,
            ArtToken::Interact => INTERACT,
            ArtToken::Grapple => GRAPPLE,
            ArtToken::Jump => JUMP,
            ArtToken::Sprint => SPRINT | DODGE,
            ArtToken::Crouch => CROUCH,
            ArtToken::Up | ArtToken::Right | ArtToken::Down | ArtToken::Left => 0,
        })
        .fold(0, |mask, bit| mask | bit)
}

impl CombatArtState {
    /// Equip `desired_art` immediately, following the vanilla fallback chain if
    /// the current save does not own the upgraded art.
    pub fn equip_no_delay(&mut self, desired_art: UID) -> bool {
        let mut desired_art = desired_art;
        loop {
            if combat_art_slot_matches(desired_art) {
                self.cur_art = Some(desired_art);
                return true;
            }
            if set_combat_art(desired_art) {
                self.cur_art = Some(desired_art);
                return true;
            }
            desired_art = match art_fallback(desired_art) {
                Some(fallback) => fallback,
                None => return false,
            };
        }
    }
}
