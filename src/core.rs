use std::{fmt, num::NonZero, path::Path};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{
    config::{Config, ToolCombo, ToolFirst, ToolTail},
    device::{Gamepad, is_key_down},
    frame::Frames,
    game::{self},
    input::{ArtCombo, ArtComboWindow, ArtToken},
};

//----------------------------------------------------------------------------
//
//  Basic constants
//
//----------------------------------------------------------------------------

// MOD behavior
const BLOCK_INJECTION_DURATION: u8 = 10;
/// Block frames to inject when starting a combo-fired combat art.
const ART_BLOCK_INJECTION_DURATION: u8 = 2;
const ATTACK_SUPRESSION_DURATION: u8 = 4;
const PROSTHETIC_SUPRESSION_DURATION: u8 = 2;
/// After tool lock expires, wait ~1.4s before returning to bare-`t` default.
const PROSTHETIC_RETURN_DELAY: Frames = Frames::standard(84);
/// After arming a tool, block switching ~1s; pressing `t` refreshes (multi-hit tools).
const PROSTHETIC_TOOL_LOCK: Frames = Frames::standard(60);
/// Window for first key → q/t (~0.3s @60fps).
const TOOL_FIRST_MAX_AGE: u16 = 18;

// UIDs
const ASHINA_CROSS: UID = 5500;
const ONE_MIND: UID = 6100;
const SAKURA_DANCE: UID = 7700;
const ICHIMONJI: UID = 5300;
const ICHIMONJI_DOUBLE: UID = 7100;
const PRAYING_STRIKES: UID = 5900;
const PRAYING_STRIKES_EXORCISM: UID = 7500;
const SENPO_LEAPING_KICKS: UID = 5800;
const HIGH_MONK: UID = 7400;
const SHADOWRUSH: UID = 6000;
const SHADOWFALL: UID = 7600;
const MORTAL_DRAW: UID = 5700;
const EMPOWERED_MORTAL_DRAW: UID = 7300;

// action bitfields
const ATTACK: u64 = 0x1;
const BLOCK: u64 = 0x4;
const JUMP: u64 = 0x10;
/// 「动作、(长按)吸引」— verified in-game via action probe.
const INTERACT: u64 = 0x1000;
const DODGE: u64 = 0x2000;
const USE_PROSTHETIC: u64 = 0x40040002;
/// 「切换忍具」— verified in-game via action probe (2026-09-01).
const SWITCH_PROSTHETIC: u64 = 0x400;

// slot index
const COMBAT_ART_SLOT: u8 = 1;
const PROSTHETIC_SLOT_0: u8 = 0;
const PROSTHETIC_SLOT_1: u8 = 2;
const PROSTHETIC_SLOT_2: u8 = 4;

//----------------------------------------------------------------------------
//
//  Actual content of the mod
//
//----------------------------------------------------------------------------

pub struct Mod {
    config: Config,
    art_combo: ArtComboWindow,
    cur_art: Option<UID>,
    blocking_last_frame: bool,
    attacking_last_frame: bool,
    interacting_last_frame: bool,
    using_tool_last_frame: bool,
    swapout_countdown: Countdown,
    attack_delay: u8,
    prosthetic_delay: u8,
    injected_blocks: u8,
    disable_block: bool,
    /// After a fresh art swap, start rl fire once attack_delay ends.
    pending_rl_attack: bool,
    /// Keep injecting ATTACK while this combo's last token stays held.
    hold_for_attack: Option<ArtToken>,
    /// Remaining BLOCK-only prime frames (ATTACK suppressed) at art start.
    art_block_inject_left: u8,
    /// True after the first BLOCK|ATTACK frame that opens the art.
    art_attack_latched: bool,
    /// Latest combo art waiting until sustained fire ends (size 1, newest wins).
    queued_art: Option<(UID, ArtToken)>,
    /// When hold key is already up (typical after queue flush), keep firing this many frames.
    art_tap_frames: u8,
    /// Diagnostic: frames since current art fire armed (0 = idle).
    art_diag_n: u32,
    /// Diagnostic: last inject phase label (to log transitions only).
    art_diag_phase: &'static str,
    /// Pending first token for prosthetic combo (↑↓←→/r/l/f/q); waiting for q/t.
    tool_first: Option<ToolFirst>,
    tool_first_age: u16,
    /// While set, keep injecting USE_PROSTHETIC until this tail key is released.
    hold_tool_tail: Option<ToolTail>,
    /// Guarantee a few USE frames after equip (tap-friendly).
    tool_min_use: u8,
    /// Last successfully armed prosthetic UID (for return-to-default).
    cur_tool: Option<UID>,
    /// Frames left after releasing tool before returning to bare-`t` default.
    return_default_left: u16,
    /// Frames left where switching tools is blocked; `t` refreshes for multi-hit.
    tool_lock_left: u16,
    /// Last frame「切换忍具」held (after we observe, before swallow).
    switch_down_last: bool,
    tools_inited: bool,
    /// Edge memory for prosthetic first-key directions / r l f.
    tool_keys_down: [bool; 4],
    tool_block_down: bool,
    tool_attack_down: bool,
    tool_interact_down: bool,
    ejection: Option<(ItemID, ProstheticSlot)>,
    gamepad: Gamepad,
}

impl Mod {
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Mod> {
        let modification = Mod {
            config: Config::open(path)?,
            gamepad: Gamepad::new()?,
            art_combo: ArtComboWindow::new(),
            cur_art: None,
            blocking_last_frame: false,
            attacking_last_frame: false,
            interacting_last_frame: false,
            using_tool_last_frame: false,
            swapout_countdown: Countdown::zero(),
            attack_delay: 0,
            prosthetic_delay: 0,
            injected_blocks: 0,
            disable_block: false,
            pending_rl_attack: false,
            hold_for_attack: None,
            art_block_inject_left: 0,
            art_attack_latched: false,
            queued_art: None,
            art_tap_frames: 0,
            art_diag_n: 0,
            art_diag_phase: "idle",
            tool_first: None,
            tool_first_age: 0,
            hold_tool_tail: None,
            tool_min_use: 0,
            cur_tool: None,
            return_default_left: 0,
            tool_lock_left: 0,
            switch_down_last: false,
            tools_inited: false,
            tool_keys_down: [false; 4],
            tool_block_down: false,
            tool_attack_down: false,
            tool_interact_down: false,
            ejection: None,
        };
        Ok(modification)
    }

    /// True while a combo-fired art is still settling or hold-injecting.
    fn art_fire_busy(&self) -> bool {
        self.hold_for_attack.is_some()
            || self.pending_rl_attack
            || self.attack_delay > 0
            || self.art_block_inject_left > 0
            || self.art_tap_frames > 0
    }

    fn clear_art_fire(&mut self) {
        if self.hold_for_attack.is_some() || self.art_diag_n > 0 {
            log::info!(
                "ART_CLEAR n={} hold={:?} tap={} blk={} latch={} pending_rl={} delay={} cur={:?} phase={}",
                self.art_diag_n,
                self.hold_for_attack,
                self.art_tap_frames,
                self.art_block_inject_left,
                self.art_attack_latched,
                self.pending_rl_attack,
                self.attack_delay,
                self.cur_art,
                self.art_diag_phase,
            );
        }
        self.hold_for_attack = None;
        self.art_block_inject_left = 0;
        self.art_attack_latched = false;
        self.pending_rl_attack = false;
        self.art_tap_frames = 0;
        self.art_diag_n = 0;
        self.art_diag_phase = "idle";
    }

    fn arm_combo_fire(&mut self, hold: ArtToken) {
        let held = self.art_combo.token_held(hold);
        self.hold_for_attack = Some(hold);
        self.art_block_inject_left = 0;
        self.art_attack_latched = false;
        // If the last key is already up (common when flushing a queued tap), auto-fire briefly.
        self.art_tap_frames = if held {
            0
        } else {
            ART_BLOCK_INJECTION_DURATION.saturating_add(3)
        };
        self.art_diag_n = 0;
        self.art_diag_phase = "armed";
        log::info!(
            "ART_ARM hold={:?} held={} tap={} cur={:?} delay={} swapout_done={}",
            hold,
            held,
            self.art_tap_frames,
            self.cur_art,
            self.attack_delay,
            self.swapout_countdown.is_done(),
        );
    }

    fn art_diag_set_phase(&mut self, phase: &'static str) {
        if self.art_diag_phase != phase {
            log::info!(
                "ART_PHASE {} -> {} n={} hold={:?} delay={} blk={} tap={}",
                self.art_diag_phase,
                phase,
                self.art_diag_n,
                self.hold_for_attack,
                self.attack_delay,
                self.art_block_inject_left,
                self.art_tap_frames,
            );
            self.art_diag_phase = phase;
        }
    }

    fn revert_ejection_if_left_slot(&mut self) {
        let active_slot = get_active_prosthetic_slot();
        if let Some((ejected_tool, original_slot)) = self.ejection {
            if active_slot != original_slot {
                equip_prosthetic(ejected_tool, original_slot);
                self.ejection = None;
            }
        }
    }

    /// Equip `uid` into the **active** slot so the HUD shows it on the current highlight.
    fn equip_tool_uid(&mut self, uid: UID) -> bool {
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

    fn return_to_default_tool(&mut self) {
        self.return_default_left = 0;
        self.tool_lock_left = 0;
        self.hold_tool_tail = None;
        self.tool_min_use = 0;
        if let Some(uid) = self.config.tool_on_t {
            if self.equip_tool_uid(uid) {
                self.cur_tool = Some(uid);
                self.prosthetic_delay = PROSTHETIC_SUPRESSION_DURATION;
            }
        }
    }

    fn refresh_tool_lock(&mut self) {
        self.tool_lock_left = PROSTHETIC_TOOL_LOCK.as_actual();
        self.return_default_left = 0;
    }

    fn arm_tool(&mut self, uid: UID, tail: ToolTail) {
        // During lock, only allow re-arming the same tool (multi-hit via t).
        if self.tool_lock_left > 0 && self.cur_tool.is_some_and(|cur| cur != uid) {
            return;
        }
        if self.equip_tool_uid(uid) {
            self.cur_tool = Some(uid);
            self.hold_tool_tail = Some(tail);
            self.tool_min_use = 3;
            self.prosthetic_delay = PROSTHETIC_SUPRESSION_DURATION;
            self.refresh_tool_lock();
            self.tool_first = None;
            self.tool_first_age = 0;
        }
    }

    fn push_tool_first(&mut self, token: ToolFirst) {
        self.tool_first = Some(token);
        self.tool_first_age = 0;
    }

    /// Track ↑↓←→/r/l/f/q as first key; on q/t complete combo → equip + hold-inject USE.
    /// `t` alone (no pending first) arms the unique default. `q` may be first (e.g. `qt`).
    fn handle_prosthetic_input(
        &mut self,
        input_handler: &mut game::InputHandler,
        up: bool,
        right: bool,
        down: bool,
        left: bool,
        blocking: bool,
        attacking: bool,
        interacting: bool,
        using_tool: bool,
        used_tool_just_now: bool,
    ) {
        // First-key edges for directions / r / l / f (`t` is never first).
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
            self.push_tool_first(ToolFirst::Block);
        }
        self.tool_block_down = blocking;
        if attacking && !self.tool_attack_down {
            self.push_tool_first(ToolFirst::Attack);
        }
        self.tool_attack_down = attacking;
        if interacting && !self.tool_interact_down {
            self.push_tool_first(ToolFirst::Interact);
        }
        self.tool_interact_down = interacting;

        if self.tool_first.is_some() {
            self.tool_first_age = self.tool_first_age.saturating_add(1);
            if self.tool_first_age >= TOOL_FIRST_MAX_AGE {
                self.tool_first = None;
                self.tool_first_age = 0;
            }
        }

        // q = switch prosthetic: observe hold, swallow vanilla 3-slot cycle.
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
                self.tool_first = None;
            } else if let Some(first) = self.tool_first {
                // Pending first + q → try (first, Switch); else treat q as new first.
                if let Some(uid) = self.config.tool(ToolCombo {
                    first,
                    tail: ToolTail::Switch,
                }) {
                    self.arm_tool(uid, ToolTail::Switch);
                } else {
                    self.push_tool_first(ToolFirst::Switch);
                }
            } else {
                // Bare q: start a prosthetic first-key window (e.g. waiting for t → `qt`).
                self.push_tool_first(ToolFirst::Switch);
            }
        }

        if used_tool_just_now {
            if self.tool_lock_left > 0 {
                // Multi-hit window: refresh lock, keep current tool, inject more USE.
                self.refresh_tool_lock();
                self.tool_min_use = self.tool_min_use.max(3);
                self.hold_tool_tail = Some(ToolTail::Use);
                self.prosthetic_delay = 0;
                self.tool_first = None;
            } else if let Some(first) = self.tool_first {
                if let Some(uid) = self.config.tool(ToolCombo {
                    first,
                    tail: ToolTail::Use,
                }) {
                    self.arm_tool(uid, ToolTail::Use);
                } else {
                    self.tool_first = None;
                }
            } else if let Some(uid) = self.config.tool_on_t {
                self.arm_tool(uid, ToolTail::Use);
            }
        }

        // Sustain / release of armed tool.
        if self.hold_tool_tail.is_some() || self.tool_min_use > 0 {
            let held = match self.hold_tool_tail {
                Some(ToolTail::Switch) => switch_down,
                Some(ToolTail::Use) => using_tool || used_tool_just_now,
                None => false,
            };
            if self.prosthetic_delay > 0 || held || self.tool_min_use > 0 {
                self.return_default_left = 0;
            } else {
                self.hold_tool_tail = None;
                // Do not return to default while tool lock is active.
            }
        }

        if self.tool_lock_left > 0 {
            self.tool_lock_left -= 1;
            self.return_default_left = 0;
            if self.tool_lock_left == 0
                && self.hold_tool_tail.is_none()
                && self.tool_min_use == 0
            {
                let is_default =
                    self.config.tool_on_t.is_some() && self.cur_tool == self.config.tool_on_t;
                if !is_default {
                    self.return_default_left = PROSTHETIC_RETURN_DELAY.as_actual();
                }
            }
        } else if self.return_default_left > 0 {
            self.return_default_left -= 1;
            if self.return_default_left == 0 {
                self.return_to_default_tool();
            }
        }
    }

    pub fn process_input(&mut self, input_handler: &mut game::InputHandler) {
        /***** keystates *****/
        let w_down = is_key_down(VK_W);
        let a_down = is_key_down(VK_A);
        let s_down = is_key_down(VK_S);
        let d_down = is_key_down(VK_D);

        /***** update combat-art directions (WASD / left stick) *****/
        let stick = self.gamepad.get_left_pos().filter(|pos| *pos != (0.0, 0.0));
        let (combo_up, combo_right, combo_down, combo_left) = if let Some((x, y)) = stick {
            let x_abs = x.abs();
            let y_abs = y.abs();
            if y_abs >= x_abs {
                if y > 0.0 {
                    (true, false, false, false)
                } else {
                    (false, false, true, false)
                }
            } else if x > 0.0 {
                (false, true, false, false)
            } else {
                (false, false, false, true)
            }
        } else {
            (w_down, d_down, s_down, a_down)
        };

        /***** parse the action bitflags *****/
        let action_snapshot = input_handler.action;
        let attacking = action_snapshot & ATTACK != 0;
        let blocking = action_snapshot & BLOCK != 0;
        let interacting = action_snapshot & INTERACT != 0;
        let using_tool = action_snapshot & USE_PROSTHETIC != 0;
        let jumping = action_snapshot & JUMP != 0;
        let dodging = action_snapshot & DODGE != 0;
        let attacked_just_now = !self.attacking_last_frame && attacking;
        let blocked_just_now = !self.blocking_last_frame && blocking;
        let used_tool_just_now = !self.using_tool_last_frame && using_tool;

        /***** prosthetic: (dir|r|l|f)+q/t or bare t → equip + hold-inject USE *****/
        if !self.tools_inited {
            self.tools_inited = true;
            if self.config.tool_on_t.is_some() {
                self.return_to_default_tool();
            }
        }
        self.handle_prosthetic_input(
            input_handler,
            combo_up,
            combo_right,
            combo_down,
            combo_left,
            blocking,
            attacking,
            interacting,
            using_tool,
            used_tool_just_now,
        );
        let action = &mut input_handler.action;

        /***** update combat-art token buffer (r/l/f + directions) *****/
        // `f` = in-game「动作、(长按)吸引」(remap + pad), not a hardcoded VK_E.
        self.art_combo.tick(
            combo_up,
            combo_right,
            combo_down,
            combo_left,
            blocking,
            attacking,
            interacting,
        );

        /***** end sustained fire early when jump/dodge or last token released *****/
        if jumping || dodging {
            if self.art_fire_busy() {
                log::info!("ART_ABORT jump={} dodge={}", jumping, dodging);
            }
            self.clear_art_fire();
            self.queued_art = None;
        } else if let Some(token) = self.hold_for_attack {
            if !self.art_combo.token_held(token) && self.art_tap_frames == 0 {
                log::info!("ART_RELEASE hold={:?} (key up)", token);
                self.clear_art_fire();
            }
        }

        /***** query the desired combat art *****/
        let mut performed_block_free_art_just_now = false;
        let performed_art_just_now = blocking && attacked_just_now;
        let mut pending_rl = false;
        if !self.swapout_countdown.is_done() {
            // fix buggy behavior of sakura dance, ashina cross and one mind
            // One Mind has two windows for animation bugs to happen
            // one after pressing ATTACK (sheathing) and one after releasing ATTACK (drawing)
            // the current (ugly) solution is to apply the cooldown after pressing ATTACK,
            // but only start counting it down after ATTACK is released
            self.swapout_countdown.count_on(!attacking);
        }

        let desired_art = if let Some(combo) = self.art_combo.take_completed() {
            // Two consecutive tokens matched (e.g. r↑, ↑f, ff, rl, rf, fr, fl).
            if let (Some(uid), Some(hold)) = (self.config.art(combo), combo.second()) {
                self.art_combo.clear();
                if self.art_fire_busy() {
                    // Sustained fire still running: queue and flush when it ends.
                    log::info!(
                        "ART_QUEUE uid={} hold={:?} busy cur={:?}",
                        uid,
                        hold,
                        self.cur_art
                    );
                    self.queued_art = Some((uid, hold));
                    None
                } else {
                    log::info!(
                        "ART_HIT uid={} hold={:?} cur_before={:?} already={} swapout_done={}",
                        uid,
                        hold,
                        self.cur_art,
                        self.cur_art == Some(uid),
                        self.swapout_countdown.is_done(),
                    );
                    performed_block_free_art_just_now = true;
                    pending_rl = true;
                    self.arm_combo_fire(hold);
                    self.queued_art = None;
                    Some(uid)
                }
            } else {
                None
            }
        } else if !self.art_fire_busy() {
            if let Some((uid, hold)) = self.queued_art.take() {
                // Previous sustained fire just ended — start the queued art.
                log::info!("ART_FLUSH_QUEUE uid={} hold={:?}", uid, hold);
                performed_block_free_art_just_now = true;
                pending_rl = true;
                self.arm_combo_fire(hold);
                Some(uid)
            } else {
                None
            }
        } else {
            None
        };

        // Bare right-click (r) → default ∅ if configured; skip when `r` is first half of a combo.
        let desired_art = desired_art.or_else(|| {
            if !blocked_just_now || !self.swapout_countdown.is_done() || performed_block_free_art_just_now
            {
                return None;
            }
            if self.art_combo.awaiting_second() {
                None
            } else {
                self.art_combo.clear();
                self.config.art(ArtCombo::empty())
            }
        });

        // if combat art switching happens too quick after performing certain combat arts
        // animation of other unrelated combat arts can be triggered
        if performed_art_just_now || performed_block_free_art_just_now && self.swapout_countdown.is_done() {
            self.swapout_countdown = Countdown::new(self.cur_art.swapout_cooldown())
        }

        /***** equip the desired combat art (or its fallback version) *****/
        if let Some(desired_art) = desired_art {
            let mut desired_art = desired_art;
            let log_equip = performed_block_free_art_just_now;
            loop {
                if self.cur_art == Some(desired_art) {
                    if log_equip {
                        log::info!("ART_EQUIP uid={} same_slot=true (no set_combat_art)", desired_art);
                    }
                    break;
                }
                if set_combat_art(desired_art) {
                    if log_equip {
                        log::info!(
                            "ART_EQUIP uid={} same_slot=false delay={}",
                            desired_art,
                            ATTACK_SUPRESSION_DURATION
                        );
                    }
                    self.cur_art = Some(desired_art);
                    self.attack_delay = ATTACK_SUPRESSION_DURATION;
                    break;
                }
                desired_art = match desired_art {
                    ICHIMONJI_DOUBLE => ICHIMONJI,
                    PRAYING_STRIKES_EXORCISM => PRAYING_STRIKES,
                    HIGH_MONK => SENPO_LEAPING_KICKS,
                    SHADOWFALL => SHADOWRUSH,
                    EMPOWERED_MORTAL_DRAW => MORTAL_DRAW,
                    _ => {
                        break;
                    }
                }
            }
        }
        // Auto rl: after settle (attack_delay), fire with short BLOCK + hold-tied ATTACK.
        if pending_rl {
            self.pending_rl_attack = true;
        }

        /***** action injection *****/
        // Legacy block injection for paths that still set injected_blocks.
        // Combo-fired arts use hold_for_attack + art_block_inject_left instead.
        if performed_block_free_art_just_now {
            // Combo path: sustained fire starts after attack_delay (see below).
        } else if self.injected_blocks >= 1 {
            if jumping || dodging {
                // DODGE and JUMP cancel the injection because they cancel the combat art itself
                self.injected_blocks = 0
            } else if self.cur_art.is_sheathed() {
                // hold BLOCK for sheathing attacks as long as ATTACK is held until:
                // 1. the player decides to hold BLOCK by themself (that usually means cancelling)
                // 2. the player released the attack
                if attacking && !blocking {
                    *action |= BLOCK;
                } else {
                    self.injected_blocks = 0;
                }
            } else if self.injected_blocks < BLOCK_INJECTION_DURATION {
                // inject just a few frames for other art
                *action |= BLOCK;
                self.injected_blocks += 1;
            }
        }

        /***** action supression *****/
        if used_tool_just_now {
            self.disable_block = true;
        }
        if blocked_just_now || performed_block_free_art_just_now || !using_tool {
            self.disable_block = false;
        }
        if self.disable_block {
            *action &= !BLOCK;
        }

        // if ATTACK|BLOCK happens way too quick after combat art switching
        // Wirdwind Slash will be performed instead of the just equipped combat art
        // supressing the few ATTACK frames that happens right after combat art switching solves the bug
        if self.attack_delay > 0 {
            *action &= !ATTACK;
            self.attack_delay -= 1;
            if self.hold_for_attack.is_some() {
                self.art_diag_set_phase("delay");
            }
        }
        // Start BLOCK prime only after settle. Must run even on the frame delay hits 0,
        // otherwise sustained fire below opens B+A one frame early (then prime runs late),
        // which is the Shadowfall "first cast after swap → trailing R1" bug.
        if self.attack_delay == 0 && self.pending_rl_attack {
            self.pending_rl_attack = false;
            self.art_block_inject_left = ART_BLOCK_INJECTION_DURATION;
            self.art_diag_set_phase("prime_start");
        }

        // Sustained art fire:
        // 1) first N frames: BLOCK only + suppress ATTACK (prevents a stray R1/whirlwind)
        // 2) then: ATTACK while last combo token is held; include BLOCK on the first attack
        //    frame so the game sees Block+Attack to start the art.
        if self.attack_delay == 0 {
            if let Some(token) = self.hold_for_attack {
                let held = self.art_combo.token_held(token);
                let tapping = self.art_tap_frames > 0;
                self.art_diag_n = self.art_diag_n.saturating_add(1);
                if held || tapping {
                    if self.art_block_inject_left > 0 {
                        *action |= BLOCK;
                        *action &= !ATTACK;
                        self.art_block_inject_left -= 1;
                        self.art_diag_set_phase("prime_block");
                    } else {
                        *action |= ATTACK;
                        // First attack frame after priming still needs BLOCK to open the art.
                        if !self.art_attack_latched {
                            *action |= BLOCK;
                            self.art_attack_latched = true;
                            self.art_diag_set_phase("open_rl");
                            log::info!(
                                "ART_OPEN n={} hold={:?} held={} tap={} out=B+A cur={:?}",
                                self.art_diag_n,
                                token,
                                held,
                                self.art_tap_frames,
                                self.cur_art,
                            );
                        } else {
                            self.art_diag_set_phase("hold_attack");
                            // Sample while sustaining ATTACK (every 15f) to compare A vs B.
                            if self.art_diag_n % 15 == 0 {
                                log::info!(
                                    "ART_HOLD n={} hold={:?} held={} tap={} out=A cur={:?}",
                                    self.art_diag_n,
                                    token,
                                    held,
                                    self.art_tap_frames,
                                    self.cur_art,
                                );
                            }
                        }
                    }
                    if tapping && !held {
                        self.art_tap_frames -= 1;
                        if self.art_tap_frames == 0 {
                            log::info!("ART_TAP_END");
                            self.clear_art_fire();
                        }
                    }
                } else if self.art_diag_n % 15 == 0 {
                    log::info!(
                        "ART_IDLE_HOLD n={} hold={:?} held=false (waiting key?)",
                        self.art_diag_n,
                        token
                    );
                }
            }
        }
        // Prosthetic: settle a few frames after equip, then inject USE while tail held (or min tap).
        if self.prosthetic_delay != 0 {
            *action &= !USE_PROSTHETIC;
            self.prosthetic_delay -= 1;
        } else if self.hold_tool_tail.is_some() || self.tool_min_use > 0 {
            *action |= USE_PROSTHETIC;
            if self.tool_min_use > 0 {
                self.tool_min_use -= 1;
            }
        }

        /***** for next frame to refer to *****/
        self.attacking_last_frame = attacking;
        self.blocking_last_frame = blocking;
        self.interacting_last_frame = interacting;
        self.using_tool_last_frame = using_tool;
    }
}

trait CombatArt: Sized {
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

struct Countdown {
    value: u16,
    running: bool,
}

impl Countdown {
    const fn zero() -> Countdown {
        Countdown {
            value: 0,
            running: false,
        }
    }

    fn new(value: Frames) -> Countdown {
        Countdown {
            value: value.as_actual(),
            running: false,
        }
    }

    fn count(&mut self) {
        self.value -= 1;
        self.running = true;
    }

    fn count_on(&mut self, cond: bool) {
        if cond || self.running {
            self.count();
        }
    }

    fn is_done(&self) -> bool {
        self.value == 0
    }
}

//----------------------------------------------------------------------------
//
//  Wrappers of functions from the original game
//
//----------------------------------------------------------------------------

/// UIDs are consistent through different save files.
pub type UID = u32;

/// When players obtain skills(combat arts/prosthetic tools), skills become items in the inventory.
/// Thus a skill has 2 IDs: its original UID and its ID as an item in the inventory.
/// When putting things into item slots, the latter shall be used.
/// The mapping from UIDs to item IDs is not cached since it will change when player loads other save files.
/// Putting random items into the item slots can cause severe bugs like losing Kusabimaru permantly
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
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

// Conversion between UID and ItemId
trait ID: fmt::Display + Clone + Copy {
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
        let inventory = &inventory_data().inventory;
        let item_id = game::get_item_id(inventory, &self);
        ItemID::new(item_id).filter(|it| it.get() < 0xFFFF)
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProstheticSlot {
    S0 = PROSTHETIC_SLOT_0,
    S1 = PROSTHETIC_SLOT_1,
    S2 = PROSTHETIC_SLOT_2,
}

impl ProstheticSlot {
    #[inline(always)]
    fn as_slot_index(self) -> usize {
        self as usize
    }
    #[inline(always)]
    fn as_prosthetic_index(self) -> u32 {
        self as u32 / 2
    }
}

fn set_combat_art(art: impl ID) -> bool {
    set_slot(art, COMBAT_ART_SLOT as usize)
}

fn equip_prosthetic(tool: impl ID, slot: ProstheticSlot) -> bool {
    set_slot(tool, slot.as_slot_index())
}

fn set_slot(item: impl ID, slot_index: usize) -> bool {
    let Some(item_id) = item.get_item_id() else {
        return false;
    };
    let equip_data = &game::EquipData::new(item_id.get());
    game::set_slot(slot_index, equip_data, true);
    true
}

fn get_prosthetic_tool(slot: ProstheticSlot) -> Option<ItemID> {
    let items = &player_data().equiped_items;
    let item_id = items[slot.as_slot_index()];
    if item_id != 256 { ItemID::new(item_id) } else { None }
}

fn get_active_prosthetic_slot() -> ProstheticSlot {
    let active_prosthetic = player_data().activte_prosthetic;
    match active_prosthetic {
        0 => ProstheticSlot::S0,
        1 => ProstheticSlot::S1,
        2 => ProstheticSlot::S2,
        illegal_slot => unreachable!("Illegal prosthetic slot: {illegal_slot}"),
    }
}

fn locate_prosthetic_tool(tool: impl ID) -> Option<ProstheticSlot> {
    let items = &player_data().equiped_items;
    let Some(item_id) = tool.get_item_id() else {
        return None;
    };
    for slot in [ProstheticSlot::S0, ProstheticSlot::S1, ProstheticSlot::S2] {
        if items[slot.as_slot_index()] == item_id.get() {
            return Some(slot);
        }
    }
    None
}

fn activate_prosthetic_slot(slot: ProstheticSlot) {
    use std::ffi::c_void;
    let unknown = unsafe {
        let character_base: *const c_void =
            game::resolve_pointer_chain(game::WORLD_DATA, [0x88, 0x1F10, 0x10, 0xF8, 0x10, 0x18, 0x00]);
        *(character_base.byte_add(0x10) as *const *const c_void)
    };
    game::set_equipped_prosthetic(unknown, 0, slot.as_prosthetic_index());
}

fn game_data() -> &'static game::GameData {
    unsafe { game::game_data().as_ref().expect("`game_data` is null.") }
}

fn player_data() -> &'static game::PlayerData {
    unsafe { game_data().player_data.as_ref().expect("`player_data` is null.") }
}

fn inventory_data() -> &'static game::InventoryData {
    unsafe { player_data().inventory_data.as_ref().expect("`inventory_data` is null") }
}
