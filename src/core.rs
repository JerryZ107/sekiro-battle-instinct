use std::{
    fmt,
    num::NonZero,
    path::{Path, PathBuf},
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{
    bindings,
    config::{Config, ToolCombo, ToolFirst, ToolTail},
    device::{Gamepad, KeyHoldTracker, is_key_down},
    emblem::Emblem,
    flow::Flow,
    frame::Frames,
    game::{self},
    input::{ArtCombo, ArtComboWindow, ArtToken},
    paper_guard,
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
/// Extra settle frames after swapping combat-art slot before mod injects B+A.
const SLOT_SWAP_SETTLE_DURATION: u8 = 6;
/// Vanilla combat-art activation (skill slot reads real keyboard B+A).
const VANILLA_ART_INPUT: u64 = BLOCK | ATTACK;
const PROSTHETIC_SUPRESSION_DURATION: u8 = 2;
/// After tool lock expires, wait ~1.4s before returning to bare-`t` default.
const PROSTHETIC_RETURN_DELAY: Frames = Frames::standard(84);
/// Window for first key → q/t; default ~0.3s @60fps, overridable via cfg.
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

/// Availability only changes after acquiring an item, loading a save, or a
/// config reload. A one-second refresh is responsive without polling the game
/// inventory several times per second.
const AVAILABILITY_REFRESH_FRAMES: u8 = 60;

// action bitfields
const ATTACK: u64 = 0x1;
const BLOCK: u64 = 0x4;
const JUMP: u64 = 0x10;
/// 「冲刺」action bit (SpMove, action index 5; verified in-game 2026-11-22).
const SPRINT: u64 = 0x20;
/// 「动作」— verified in-game via action probe.
const INTERACT: u64 = 0x1000;
/// Grapple/hook action — verified in-game via action probe (E rebound to grapple reads 0x20000008).
const GRAPPLE: u64 = 0x20000000;
const DODGE: u64 = 0x2000;
/// 「下蹲」action bit (Crouch, action index 15; verified in-game 2026-11-22).
const CROUCH: u64 = 0x8000;
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
    cfg_path: PathBuf,
    config: Config,
    base_config: Config,
    availability_signature: Option<u64>,
    availability_tick: u8,
    /// Sorted, deduplicated UIDs to check. Rebuilt only when the base config
    /// changes instead of allocating/sorting on every availability refresh.
    availability_uids: Vec<UID>,
    emblem: Emblem,
    flow: Flow,
    art_combo: ArtComboWindow,
    cur_art: Option<UID>,
    blocking_last_frame: bool,
    attacking_last_frame: bool,
    interacting_last_frame: bool,
    grappling_last_frame: bool,
    using_tool_last_frame: bool,
    swapout_countdown: Countdown,
    attack_delay: u8,
    prosthetic_delay: u8,
    injected_blocks: u8,
    disable_block: bool,
    /// After a fresh art swap, start rl fire once attack_delay ends.
    pending_rl_attack: bool,
    /// Strip vanilla B+A until block released after a late `l` outside the rl window.
    suppress_late_rl_skill: bool,
    /// Keep injecting ATTACK while this combo's last token stays held.
    hold_for_attack: Option<ArtToken>,
    /// Remaining BLOCK-only prime frames (ATTACK suppressed) at art start.
    art_block_inject_left: u8,
    /// True after the first BLOCK|ATTACK frame that opens the art.
    art_attack_latched: bool,
    /// Native action bits of the combo that fired; stripped until injection ends.
    combo_suppress_mask: u64,
    /// Latest combo art waiting until sustained fire ends (size 1, newest wins).
    queued_art: Option<(UID, ArtToken)>,
    /// Frames left after last-key release before flushing `queued_art` (from cfg).
    queue_flush_left: u16,
    /// Skip one flush tick so delay starts counting from the frame after release.
    queue_flush_skip_tick: bool,
    /// When hold key is already up (typical after queue flush), keep firing this many frames.
    art_tap_frames: u8,
    /// Pending first token for prosthetic combo (↑↓←→/r/l/e/f/q); waiting for q/t.
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
    /// Edge memory for prosthetic first-key directions / r l e f.
    tool_keys_down: [bool; 4],
    tool_block_down: bool,
    tool_attack_down: bool,
    tool_interact_down: bool,
    tool_grapple_down: bool,
    tool_jump_down: bool,
    tool_sprint_down: bool,
    tool_crouch_down: bool,
    /// Physical-key hold supplement learned from game action edges.
    key_hold: KeyHoldTracker,
    prev_jump_action: bool,
    prev_sprint_action: bool,
    prev_crouch_action: bool,
    ejection: Option<(ItemID, ProstheticSlot)>,
    gamepad: Gamepad,
    combo_dirs_last: [bool; 4],
}

impl Mod {
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Mod> {
        let cfg_path = path.as_ref().to_path_buf();
        let base_config = bindings::load_effective_config(&cfg_path)?;
        let config = base_config.clone();
        let availability_uids = collect_availability_uids(&base_config);
        let emblem = Emblem::new(&cfg_path);
        let flow = Flow::new(&cfg_path);
        paper_guard::init(&cfg_path);
        let art_combo = ArtComboWindow::new(config.rl_combo_max_age);
        let modification = Mod {
            cfg_path: cfg_path.clone(),
            config,
            base_config,
            availability_signature: None,
            availability_tick: 0,
            availability_uids,
            emblem,
            flow,
            gamepad: Gamepad::new()?,
            art_combo,
            cur_art: None,
            blocking_last_frame: false,
            attacking_last_frame: false,
            interacting_last_frame: false,
            grappling_last_frame: false,
            using_tool_last_frame: false,
            swapout_countdown: Countdown::zero(),
            attack_delay: 0,
            prosthetic_delay: 0,
            injected_blocks: 0,
            disable_block: false,
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
            tool_grapple_down: false,
            tool_jump_down: false,
            tool_sprint_down: false,
            tool_crouch_down: false,
            key_hold: KeyHoldTracker::new(),
            prev_jump_action: false,
            prev_sprint_action: false,
            prev_crouch_action: false,
            ejection: None,
            combo_dirs_last: [false; 4],
        };
        Ok(modification)
    }

    fn refresh_available_bindings(&mut self, force: bool) {
        if !force {
            self.availability_tick = self.availability_tick.saturating_add(1);
            if self.availability_signature.is_some() && self.availability_tick < AVAILABILITY_REFRESH_FRAMES {
                return;
            }
        }
        self.availability_tick = 0;

        let Some(signature) = availability_signature(&self.availability_uids) else {
            return;
        };
        if !force && self.availability_signature == Some(signature) {
            return;
        }
        self.availability_signature = Some(signature);

        let mut config = self.base_config.clone();
        filter_config_by_availability(&mut config);
        let arts = config.arts.len() + config.instant_arts.len();
        let tools =
            config.tools.len() + usize::from(config.tool_on_t.is_some()) + usize::from(config.tool_on_q.is_some());
        log::warn!("keybinds: available filter arts={arts} tools={tools}");
        self.config = config;
        self.reset_binding_runtime_state();
    }

    fn reset_binding_runtime_state(&mut self) {
        self.art_combo.clear();
        self.clear_art_fire();
        self.queued_art = None;
        self.queue_flush_left = 0;
        self.queue_flush_skip_tick = false;
        self.cur_art = None;
        self.attack_delay = 0;
        self.prosthetic_delay = 0;
        self.injected_blocks = 0;
        self.disable_block = false;
        self.pending_rl_attack = false;
        self.suppress_late_rl_skill = false;
        self.tool_first = None;
        self.tool_first_age = 0;
        self.hold_tool_tail = None;
        self.tool_min_use = 0;
        self.cur_tool = None;
        self.return_default_left = 0;
        self.tool_lock_left = 0;
        self.switch_down_last = false;
        self.tools_inited = false;
    }
    fn instant_combo_matches(
        &self,
        combo: ArtCombo,
        blocking_last_frame: bool,
        interacting_last_frame: bool,
        grappling_last_frame: bool,
        attacked_just_now: bool,
        attacking: bool,
        combo_up: bool,
        combo_right: bool,
        combo_down: bool,
        combo_left: bool,
        combo_dirs_last: [bool; 4],
    ) -> bool {
        let (Some(first), Some(second)) = (combo.first(), combo.second()) else {
            return false;
        };
        let dir_held = |t: ArtToken| match t {
            ArtToken::Up => combo_up,
            ArtToken::Right => combo_right,
            ArtToken::Down => combo_down,
            ArtToken::Left => combo_left,
            _ => false,
        };
        let dir_edge = |t: ArtToken| match t {
            ArtToken::Up => combo_up && !combo_dirs_last[0],
            ArtToken::Right => combo_right && !combo_dirs_last[1],
            ArtToken::Down => combo_down && !combo_dirs_last[2],
            ArtToken::Left => combo_left && !combo_dirs_last[3],
            _ => false,
        };
        match (first, second) {
            (ArtToken::Block, dir) if dir.is_direction() => blocking_last_frame && dir_edge(dir),
            (dir, ArtToken::Attack) if dir.is_direction() => dir_held(dir) && attacked_just_now,
            (ArtToken::Attack, dir) if dir.is_direction() => attacking && dir_edge(dir),
            (ArtToken::Interact, ArtToken::Attack) => interacting_last_frame && attacked_just_now,
            (ArtToken::Grapple, ArtToken::Attack) => grappling_last_frame && attacked_just_now,
            _ => false,
        }
    }

    /// Instant arts (cfg name contains 瞬发 / instant): fire same-frame B+A, no combo settle.
    fn try_instant_combat_art(
        &mut self,
        combo_up: bool,
        combo_right: bool,
        combo_down: bool,
        combo_left: bool,
        attacked_just_now: bool,
        attacking: bool,
    ) -> Option<UID> {
        if !self.swapout_countdown.is_done() || self.art_fire_busy() {
            return None;
        }
        // Find the match without allocating a temporary Vec on every input tick.
        let matched = self.config.instant_arts.iter().find_map(|(&combo, &uid)| {
            self.instant_combo_matches(
                combo,
                self.blocking_last_frame,
                self.interacting_last_frame,
                self.grappling_last_frame,
                attacked_just_now,
                attacking,
                combo_up,
                combo_right,
                combo_down,
                combo_left,
                self.combo_dirs_last,
            )
            .then_some((combo, uid))
        });
        let (combo, uid) = matched?;
        if self.equip_combat_art_no_delay(uid) {
            log::debug!("ART_INSTANT uid={} combo={:?}", uid, combo);
            self.clear_art_fire();
            self.clear_queued_art();
            self.art_combo.clear();
            self.injected_blocks = 1;
            self.swapout_countdown = Countdown::new(self.cur_art.swapout_cooldown());
            return Some(uid);
        }
        None
    }

    fn equip_combat_art_no_delay(&mut self, desired_art: UID) -> bool {
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
            desired_art = match desired_art {
                ICHIMONJI_DOUBLE => ICHIMONJI,
                PRAYING_STRIKES_EXORCISM => PRAYING_STRIKES,
                HIGH_MONK => SENPO_LEAPING_KICKS,
                SHADOWFALL => SHADOWRUSH,
                EMPOWERED_MORTAL_DRAW => MORTAL_DRAW,
                _ => return false,
            };
        }
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
        self.hold_for_attack = None;
        self.art_block_inject_left = 0;
        self.art_attack_latched = false;
        self.pending_rl_attack = false;
        self.art_tap_frames = 0;
        self.combo_suppress_mask = 0;
    }

    /// End sustained fire; if a queued art is waiting, start the cfg flush delay.
    fn end_art_fire_for_queue(&mut self) {
        self.clear_art_fire();
        if self.queued_art.is_some() {
            self.queue_flush_left = self.config.art_queue_delay_frames;
            self.queue_flush_skip_tick = true;
            log::debug!(
                "ART_QUEUE_WAIT delay={} uid={:?}",
                self.queue_flush_left,
                self.queued_art.as_ref().map(|(uid, _)| *uid),
            );
        }
    }

    fn clear_queued_art(&mut self) {
        self.queued_art = None;
        self.queue_flush_left = 0;
        self.queue_flush_skip_tick = false;
    }

    fn try_flush_queued_art(&mut self) -> Option<(UID, ArtToken)> {
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
        self.tool_lock_left = self.config.tool_lock_frames(self.cur_tool);
        self.return_default_left = 0;
    }

    fn arm_tool(&mut self, uid: UID, tail: ToolTail) -> bool {
        // During lock, only allow re-arming the same tool (multi-hit via t).
        if self.tool_lock_left > 0 && self.cur_tool.is_some_and(|cur| cur != uid) {
            return false;
        }
        if self.equip_tool_uid(uid) {
            self.cur_tool = Some(uid);
            self.hold_tool_tail = Some(tail);
            self.tool_min_use = 3;
            self.prosthetic_delay = PROSTHETIC_SUPRESSION_DURATION;
            self.refresh_tool_lock();
            self.tool_first = None;
            self.tool_first_age = 0;
            true
        } else {
            false
        }
    }

    fn refresh_tool_hold(&mut self, tail: ToolTail) {
        self.refresh_tool_lock();
        self.tool_min_use = self.tool_min_use.max(3);
        self.hold_tool_tail = Some(tail);
        self.prosthetic_delay = 0;
        self.tool_first = None;
    }

    /// Try to complete a pending first-key with `tail`. Returns true if a tool was armed.
    fn try_complete_tool_tail(&mut self, tail: ToolTail) -> bool {
        if self.tool_lock_left > 0 {
            // Multi-hit: refresh when the arming tail (or use/switch) is pressed again.
            let same_tail = self.hold_tool_tail == Some(tail);
            let use_or_switch = matches!(tail, ToolTail::Use | ToolTail::Switch);
            if same_tail || use_or_switch {
                self.refresh_tool_hold(tail);
                return true;
            }
        }
        if let Some(first) = self.tool_first {
            if let Some(uid) = self.config.tool(ToolCombo { first, tail }) {
                return self.arm_tool(uid, tail);
            }
            // Pending first did not match this tail — clear so it does not stale.
            self.tool_first = None;
            self.tool_first_age = 0;
        }
        false
    }

    fn push_tool_first(&mut self, token: ToolFirst) {
        self.tool_first = Some(token);
        self.tool_first_age = 0;
    }

    /// Track normal action tokens as the prosthetic first key; on another normal
    /// token or q/t complete the combo → equip + hold-inject USE.
    /// Bare `t` arms `tool_on_t`. Bare `q` arms `tool_on_q` (same fire style; return-default stays `t`).
    /// Returns true when a tool was armed via a normal action tail (caller should clear art combo).
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
            if self.try_complete_tool_tail(ToolTail::Block) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Block);
            }
        }
        self.tool_block_down = blocking;

        if attacking && !self.tool_attack_down {
            if self.try_complete_tool_tail(ToolTail::Attack) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Attack);
            }
        }
        self.tool_attack_down = attacking;

        if interacting && !self.tool_interact_down {
            if self.try_complete_tool_tail(ToolTail::Interact) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Interact);
            }
        }
        self.tool_interact_down = interacting;

        if grappling && !self.tool_grapple_down {
            if self.try_complete_tool_tail(ToolTail::Grapple) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Grapple);
            }
        }
        self.tool_grapple_down = grappling;

        if jumping && !self.tool_jump_down {
            if self.try_complete_tool_tail(ToolTail::Jump) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Jump);
            }
        }
        self.tool_jump_down = jumping;

        if sprinting && !self.tool_sprint_down {
            if self.try_complete_tool_tail(ToolTail::Sprint) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Sprint);
            }
        }
        self.tool_sprint_down = sprinting;

        if crouching && !self.tool_crouch_down {
            if self.try_complete_tool_tail(ToolTail::Crouch) {
                armed_via_art_tail = true;
            } else {
                self.push_tool_first(ToolFirst::Crouch);
            }
        }
        self.tool_crouch_down = crouching;

        if self.tool_first.is_some() {
            self.tool_first_age = self.tool_first_age.saturating_add(1);
            if self.tool_first_age >= self.config.tool_combo_max_age {
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
                self.refresh_tool_hold(ToolTail::Switch);
            } else if let Some(first) = self.tool_first {
                if let Some(uid) = self.config.tool(ToolCombo {
                    first,
                    tail: ToolTail::Switch,
                }) {
                    let _ = self.arm_tool(uid, ToolTail::Switch);
                } else {
                    self.tool_first = None;
                }
            } else if let Some(uid) = self.config.tool_on_q {
                let _ = self.arm_tool(uid, ToolTail::Switch);
            } else {
                self.push_tool_first(ToolFirst::Switch);
            }
        }

        if used_tool_just_now {
            if self.tool_lock_left > 0 {
                self.refresh_tool_hold(ToolTail::Use);
            } else if let Some(first) = self.tool_first {
                if let Some(uid) = self.config.tool(ToolCombo {
                    first,
                    tail: ToolTail::Use,
                }) {
                    let _ = self.arm_tool(uid, ToolTail::Use);
                } else {
                    self.tool_first = None;
                }
            } else if let Some(uid) = self.config.tool_on_t {
                let _ = self.arm_tool(uid, ToolTail::Use);
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
                    let is_default = self.config.tool_on_t.is_some() && self.cur_tool == self.config.tool_on_t;
                    if !is_default {
                        self.return_default_left = PROSTHETIC_RETURN_DELAY.as_actual();
                    }
                }
            }
        } else if self.return_default_left > 0 {
            self.return_default_left -= 1;
            if self.return_default_left == 0 {
                self.return_to_default_tool();
            }
        }

        armed_via_art_tail
    }

    pub fn process_input(&mut self, input_handler: &mut game::InputHandler) {
        if bindings::take_reload_request() {
            match bindings::load_effective_config(&self.cfg_path) {
                Ok(config) => {
                    self.base_config = config;
                    self.availability_uids = collect_availability_uids(&self.base_config);
                    self.availability_signature = None;
                    self.refresh_available_bindings(true);
                    log::warn!("keybinds: runtime config reloaded");
                }
                Err(error) => {
                    log::error!("keybinds: reload failed: {error:#}");
                }
            }
        }
        self.refresh_available_bindings(false);
        let has_available_arts = !self.config.arts.is_empty() || !self.config.instant_arts.is_empty();

        self.emblem.tick();
        self.flow.tick();

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
        // Both tokens follow the game's logical action bits, so in-game key remaps
        // and gamepad bindings keep working. `e` = interact/action, `f` = grapple/hook.
        let interacting = action_snapshot & INTERACT != 0;
        let grappling = action_snapshot & GRAPPLE != 0;
        let using_tool = action_snapshot & USE_PROSTHETIC != 0;
        let jump_action = action_snapshot & JUMP != 0;
        let sprint_action = action_snapshot & SPRINT != 0;
        let crouch_action = action_snapshot & CROUCH != 0;
        let dodging = action_snapshot & DODGE != 0;

        // Jump/crouch are emitted by the game as short action pulses in some
        // contexts. Learn the physical key at the action edge and OR that key's
        // held state into the logical token for the rest of the press.
        let newly_down = self.key_hold.update();
        if jump_action && !self.prev_jump_action {
            self.key_hold.learn(0, newly_down);
        }
        if sprint_action && !self.prev_sprint_action {
            self.key_hold.learn(1, newly_down);
        }
        if crouch_action && !self.prev_crouch_action {
            self.key_hold.learn(2, newly_down);
        }
        self.prev_jump_action = jump_action;
        self.prev_sprint_action = sprint_action;
        self.prev_crouch_action = crouch_action;
        let jumping = jump_action || self.key_hold.held(0);
        let sprinting = sprint_action || self.key_hold.held(1);
        let crouching = crouch_action || self.key_hold.held(2);
        let attacked_just_now = !self.attacking_last_frame && attacking;
        let blocked_just_now = !self.blocking_last_frame && blocking;
        let used_tool_just_now = !self.using_tool_last_frame && using_tool;

        /***** prosthetic: normal token + tail, or bare t → equip + hold-inject USE *****/
        if !self.tools_inited {
            self.tools_inited = true;
            if self.config.tool_on_t.is_some() {
                self.return_to_default_tool();
            }
        }
        let tool_armed_via_art_tail = self.handle_prosthetic_input(
            input_handler,
            combo_up,
            combo_right,
            combo_down,
            combo_left,
            blocking,
            attacking,
            interacting,
            grappling,
            jumping,
            sprinting,
            crouching,
            using_tool,
            used_tool_just_now,
        );

        /***** update combat-art token buffer (normal action tokens + directions) *****/
        // Logical action bits keep in-game remaps and gamepad bindings working.
        if has_available_arts {
            self.art_combo.tick(
                combo_up,
                combo_right,
                combo_down,
                combo_left,
                blocking,
                attacking,
                interacting,
                grappling,
                jumping,
                sprinting,
                crouching,
            );
        } else {
            self.art_combo.clear();
        }
        // Tool took this r/l/e/f completion — do not also fire a combat art on the same keys.
        if tool_armed_via_art_tail {
            self.art_combo.clear();
        }
        let late_rl_rejected = self.art_combo.take_late_rl_rejected();
        if late_rl_rejected {
            self.suppress_late_rl_skill = true;
            log::debug!("ART_RL_LATE reject — suppress B+A until block released");
        }
        let instant_art_fired = self
            .try_instant_combat_art(
                combo_up,
                combo_right,
                combo_down,
                combo_left,
                attacked_just_now,
                attacking,
            )
            .is_some();

        /***** end sustained fire early on external jump/dodge or last token release *****/
        let combo_owns_jump = self.combo_suppress_mask & JUMP != 0;
        let combo_owns_sprint = self.combo_suppress_mask & SPRINT != 0;
        let external_jump = jumping && !combo_owns_jump;
        let external_dodge = dodging && !combo_owns_sprint;
        if external_jump || external_dodge {
            if self.art_fire_busy() {
                log::debug!(
                    "ART_ABORT external_jump={} external_dodge={}",
                    external_jump,
                    external_dodge
                );
            }
            self.clear_art_fire();
            self.clear_queued_art();
        } else if let Some(token) = self.hold_for_attack {
            if !self.art_combo.token_held(token) && self.art_tap_frames == 0 {
                if !self.art_attack_latched {
                    // Released during settle / prime (common for quick `ee`): still fire a short
                    // B+A inject instead of cancelling into a bare block.
                    self.art_tap_frames = ART_BLOCK_INJECTION_DURATION.saturating_add(3);
                    log::debug!("ART_RELEASE→TAP hold={:?} tap={}", token, self.art_tap_frames);
                } else {
                    log::debug!("ART_RELEASE hold={:?} (key up)", token);
                    self.end_art_fire_for_queue();
                }
            }
        }

        /***** query the desired combat art *****/
        let mut performed_block_free_art_just_now = false;
        let performed_art_just_now = blocking && attacked_just_now && !late_rl_rejected;
        let mut pending_rl = false;
        if !self.swapout_countdown.is_done() {
            // fix buggy behavior of sakura dance, ashina cross and one mind
            // One Mind has two windows for animation bugs to happen
            // one after pressing ATTACK (sheathing) and one after releasing ATTACK (drawing)
            // the current (ugly) solution is to apply the cooldown after pressing ATTACK,
            // but only start counting it down after ATTACK is released
            self.swapout_countdown.count_on(!attacking);
        }

        let completed_combo = self.art_combo.take_completed();
        let desired_art = if let Some(combo) = completed_combo {
            // Two consecutive tokens matched (e.g. r↑, ↑e, ↑f, ee, ff, rl, rf, fr, fl).
            if let (Some(uid), Some(hold)) = (self.config.art_or_instant(combo), combo.second()) {
                self.art_combo.clear();
                let is_rl = combo.is_rl();
                if self.art_fire_busy() && !is_rl {
                    // Sustained fire still running: queue and flush after release + delay.
                    log::debug!("ART_QUEUE uid={} hold={:?} busy cur={:?}", uid, hold, self.cur_art);
                    self.queued_art = Some((uid, hold));
                    // Delay starts when current fire ends, not when queued.
                    self.queue_flush_left = 0;
                    self.queue_flush_skip_tick = false;
                    None
                } else {
                    if is_rl && self.art_fire_busy() {
                        log::debug!("ART_RL_PREEMPT busy cur={:?} -> uid={}", self.cur_art, uid);
                        self.clear_art_fire();
                        self.clear_queued_art();
                    }
                    log::debug!(
                        "ART_HIT uid={} hold={:?} cur_before={:?} already={} swapout_done={}",
                        uid,
                        hold,
                        self.cur_art,
                        self.cur_art == Some(uid),
                        self.swapout_countdown.is_done(),
                    );
                    performed_block_free_art_just_now = true;
                    pending_rl = true;
                    self.combo_suppress_mask = combo_action_mask(combo);
                    self.arm_combo_fire(hold);
                    self.clear_queued_art();
                    Some(uid)
                }
            } else {
                None
            }
        } else if !self.art_fire_busy() {
            if let Some((uid, hold)) = self.try_flush_queued_art() {
                log::debug!("ART_FLUSH_QUEUE uid={} hold={:?}", uid, hold);
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
            if !blocked_just_now || !self.swapout_countdown.is_done() || performed_block_free_art_just_now {
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
        let mut art_swapped_this_hit = false;
        if let Some(desired_art) = desired_art {
            let mut desired_art = desired_art;
            loop {
                if combat_art_slot_matches(desired_art) {
                    self.cur_art = Some(desired_art);
                    break;
                }
                if set_combat_art(desired_art) {
                    art_swapped_this_hit = true;
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
            let settle = if art_swapped_this_hit {
                SLOT_SWAP_SETTLE_DURATION
            } else {
                ATTACK_SUPRESSION_DURATION
            };
            if self.attack_delay < settle {
                self.attack_delay = settle;
                log::debug!(
                    "ART_SETTLE swapped={} delay={} hold={:?}",
                    art_swapped_this_hit,
                    self.attack_delay,
                    self.hold_for_attack
                );
            }
        }

        /***** action injection *****/
        // Legacy block injection for paths that still set injected_blocks.
        // Combo-fired arts use hold_for_attack + art_block_inject_left instead.
        if performed_block_free_art_just_now {
            // Combo path: sustained fire starts after attack_delay (see below).
        } else if self.injected_blocks >= 1 {
            if external_jump || external_dodge {
                // External DODGE/JUMP cancels the art; combo-owned jump/sprint bits are
                // suppressed instead of being allowed to preempt the injection.
                self.injected_blocks = 0
            } else if self.cur_art.is_sheathed() {
                // hold BLOCK for sheathing attacks as long as ATTACK is held until:
                // 1. the player decides to hold BLOCK by themself (that usually means cancelling)
                // 2. the player released the attack
                if attacking && !blocking {
                    input_handler.action |= BLOCK;
                } else {
                    self.injected_blocks = 0;
                }
            } else if self.injected_blocks < BLOCK_INJECTION_DURATION {
                // inject just a few frames for other art
                input_handler.action |= BLOCK;
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
            input_handler.action &= !BLOCK;
        }

        // --- Combat-art input takeover ---
        // Real keyboard B+A must never reach the skill slot; only mod-injected B+A (below) may
        // activate arts. While waiting for the second key of block-first combos, also strip
        // stray ATTACK so held-l + new block cannot vanilla-fire before the combo completes.
        if has_available_arts && self.art_combo.awaiting_block_second() && blocking {
            strip_input(input_handler, ATTACK);
        }
        if has_available_arts && blocking && attacking {
            strip_input(input_handler, VANILLA_ART_INPUT);
        }
        if instant_art_fired {
            or_input(input_handler, VANILLA_ART_INPUT);
        }

        if self.suppress_late_rl_skill && !blocking {
            self.suppress_late_rl_skill = false;
        }

        // Post-swap: suppress stray ATTACK without B+A (whirlwind-on-switch bug).
        if self.attack_delay > 0 {
            strip_input(input_handler, ATTACK);
            self.attack_delay -= 1;
        }
        // Start BLOCK prime only after settle. Must run even on the frame delay hits 0,
        // otherwise sustained fire below opens B+A one frame early (then prime runs late),
        // which is the Shadowfall "first cast after swap → trailing R1" bug.
        if self.attack_delay == 0 && self.pending_rl_attack {
            self.pending_rl_attack = false;
            self.art_block_inject_left = ART_BLOCK_INJECTION_DURATION;
        }

        // The combo keys themselves must never reach the game's native action
        // handling once the combo has been recognized (guard/sprint/jump/crouch
        // animations can otherwise preempt the MOD's BLOCK+ATTACK injection).
        if self.combo_suppress_mask != 0 {
            // Suppress only the current logical action. Do NOT clear action_b/
            // action_c here: the game uses them as press/hold history, and
            // clearing them every frame would turn the injected B+A into a new
            // attack edge on every frame (repeated sheathing for Ashina Cross /
            // One Mind instead of a sustained ready-to-draw state).
            input_handler.action &= !self.combo_suppress_mask;
        }

        // Sustained art fire:
        // 1) first N frames: BLOCK only + suppress ATTACK (prevents a stray R1/whirlwind)
        // 2) then: BLOCK+ATTACK while last combo token is held (charge / multi-hit)
        //    For `ee`, the last token is Interact — hold e to keep injecting B+A (charge).
        if self.attack_delay == 0 {
            if let Some(token) = self.hold_for_attack {
                let held = self.art_combo.token_held(token);
                let tapping = self.art_tap_frames > 0;
                if held || tapping {
                    if self.art_block_inject_left > 0 {
                        or_input(input_handler, BLOCK);
                        self.art_block_inject_left -= 1;
                    } else {
                        or_input(input_handler, VANILLA_ART_INPUT);
                        self.art_attack_latched = true;
                    }
                    if tapping && !held {
                        self.art_tap_frames -= 1;
                        if self.art_tap_frames == 0 {
                            self.end_art_fire_for_queue();
                        }
                    }
                }
            }
        }
        // Prosthetic: settle a few frames after equip, then inject USE while tail held (or min tap).
        // When the combo tail is r/l/e/f, strip that vanilla action so ↑r / ↑e / ↑f fires the
        // tool directly (like swallowing SWITCH for q), not "block first, then use".
        if self.prosthetic_delay != 0 {
            input_handler.action &= !USE_PROSTHETIC;
            self.prosthetic_delay -= 1;
        } else if self.hold_tool_tail.is_some() || self.tool_min_use > 0 {
            input_handler.action |= USE_PROSTHETIC;
            if self.tool_min_use > 0 {
                self.tool_min_use -= 1;
            }
        }
        match self.hold_tool_tail {
            Some(ToolTail::Block) => input_handler.action &= !BLOCK,
            Some(ToolTail::Attack) => input_handler.action &= !ATTACK,
            Some(ToolTail::Interact) => input_handler.action &= !INTERACT,
            Some(ToolTail::Grapple) => {
                input_handler.action &= !GRAPPLE;
                input_handler.action_b &= !GRAPPLE;
                input_handler.action_c &= !GRAPPLE;
            }
            Some(ToolTail::Jump) => strip_input(input_handler, JUMP),
            Some(ToolTail::Sprint) => strip_input(input_handler, SPRINT),
            Some(ToolTail::Crouch) => strip_input(input_handler, CROUCH),
            _ => {}
        }

        /***** for next frame to refer to *****/
        self.attacking_last_frame = attacking;
        self.blocking_last_frame = blocking;
        self.interacting_last_frame = interacting;
        self.grappling_last_frame = grappling;
        self.using_tool_last_frame = using_tool;
        self.combo_dirs_last = [combo_up, combo_right, combo_down, combo_left];
    }
}

fn collect_availability_uids(config: &Config) -> Vec<UID> {
    let mut uids: Vec<UID> = Vec::new();
    uids.extend(config.arts.values().copied());
    uids.extend(config.instant_arts.values().copied());
    uids.extend(config.tools.values().copied());
    uids.extend(config.tool_on_t);
    uids.extend(config.tool_on_q);
    uids.sort_unstable();
    uids.dedup();
    uids
}

fn availability_signature(uids: &[UID]) -> Option<u64> {
    if !inventory_ready() {
        return None;
    }

    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &uid in uids {
        hash ^= uid as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        hash ^= u64::from(is_uid_available(uid));
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(hash)
}

fn inventory_ready() -> bool {
    unsafe {
        let game_data = game::game_data();
        if game_data.is_null() {
            return false;
        }
        let player = (*game_data).player_data;
        if player.is_null() {
            return false;
        }
        !(*player).inventory_data.is_null()
    }
}

fn is_uid_available(uid: UID) -> bool {
    uid.get_item_id().is_some()
}

fn art_fallback(uid: UID) -> Option<UID> {
    match uid {
        ICHIMONJI_DOUBLE => Some(ICHIMONJI),
        PRAYING_STRIKES_EXORCISM => Some(PRAYING_STRIKES),
        HIGH_MONK => Some(SENPO_LEAPING_KICKS),
        SHADOWFALL => Some(SHADOWRUSH),
        EMPOWERED_MORTAL_DRAW => Some(MORTAL_DRAW),
        _ => None,
    }
}

fn resolve_available_art_with<F>(uid: UID, available: &mut F) -> Option<UID>
where
    F: FnMut(UID) -> bool,
{
    let mut current = uid;
    loop {
        if available(current) {
            return Some(current);
        }
        current = art_fallback(current)?;
    }
}

fn filter_config_by_availability_with<F>(config: &mut Config, mut available: F)
where
    F: FnMut(UID) -> bool,
{
    let arts = std::mem::take(&mut config.arts);
    config.arts = arts
        .into_iter()
        .filter_map(|(combo, uid)| resolve_available_art_with(uid, &mut available).map(|uid| (combo, uid)))
        .collect();

    let instant_arts = std::mem::take(&mut config.instant_arts);
    config.instant_arts = instant_arts
        .into_iter()
        .filter_map(|(combo, uid)| resolve_available_art_with(uid, &mut available).map(|uid| (combo, uid)))
        .collect();

    config.tools.retain(|_, uid| available(*uid));
    if config.tool_on_t.is_some_and(|uid| !available(uid)) {
        config.tool_on_t = None;
    }
    if config.tool_on_q.is_some_and(|uid| !available(uid)) {
        config.tool_on_q = None;
    }
    config.tool_multi_lock_secs.retain(|uid, _| available(*uid));
}

fn filter_config_by_availability(config: &mut Config) {
    filter_config_by_availability_with(config, is_uid_available);
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
}

fn get_equipped_combat_art_item_id() -> Option<ItemID> {
    let items = &player_data().equiped_items;
    let item_id = items[COMBAT_ART_SLOT as usize];
    if item_id != 256 { ItemID::new(item_id) } else { None }
}

fn combat_art_slot_matches(art: UID) -> bool {
    match (art.get_item_id(), get_equipped_combat_art_item_id()) {
        (Some(want), Some(have)) => want == have,
        _ => false,
    }
}

fn combo_action_mask(combo: ArtCombo) -> u64 {
    [combo.first(), combo.second()]
        .into_iter()
        .flatten()
        .map(|token| match token {
            ArtToken::Block => BLOCK,
            ArtToken::Attack => ATTACK,
            ArtToken::Interact => INTERACT,
            ArtToken::Grapple => GRAPPLE,
            ArtToken::Jump => JUMP,
            // The sprint key also drives native dodge/step; swallow that companion
            // action while the combo owns the sprint key.
            ArtToken::Sprint => SPRINT | DODGE,
            ArtToken::Crouch => CROUCH,
            ArtToken::Up | ArtToken::Right | ArtToken::Down | ArtToken::Left => 0,
        })
        .fold(0, |mask, bit| mask | bit)
}

fn strip_input(input: &mut game::InputHandler, mask: u64) {
    input.action &= !mask;
    input.action_b &= !mask;
    input.action_c &= !mask;
}

fn or_input(input: &mut game::InputHandler, mask: u64) {
    input.action |= mask;
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

fn game_data() -> &'static game::GameData {
    unsafe { game::game_data().as_ref().expect("`game_data` is null.") }
}

fn player_data() -> &'static game::PlayerData {
    unsafe { game_data().player_data.as_ref().expect("`player_data` is null.") }
}

fn inventory_data() -> &'static game::InventoryData {
    unsafe { player_data().inventory_data.as_ref().expect("`inventory_data` is null") }
}

#[cfg(test)]
mod availability_tests {
    use super::*;

    #[test]
    fn unavailable_arts_and_tools_are_filtered_and_arts_fall_back() {
        let mut config = Config::default();
        let fallback_combo = ArtCombo::pair(ArtToken::Block, ArtToken::Attack);
        let missing_combo = ArtCombo::pair(ArtToken::Interact, ArtToken::Attack);
        let tool_combo = ToolCombo {
            first: ToolFirst::Up,
            tail: ToolTail::Use,
        };

        config.arts.insert(fallback_combo, ICHIMONJI_DOUBLE);
        config.arts.insert(missing_combo, PRAYING_STRIKES_EXORCISM);
        config.instant_arts.insert(ArtCombo::empty(), SAKURA_DANCE);
        config.tools.insert(tool_combo, 78300);
        config.tool_on_t = Some(70500);
        config.tool_on_q = Some(70400);
        config.tool_multi_lock_secs.insert(78300, 0.5);
        config.tool_multi_lock_secs.insert(70500, 0.5);

        let available = [ICHIMONJI, SAKURA_DANCE, 78300, 70400];
        filter_config_by_availability_with(&mut config, |uid| available.contains(&uid));

        assert_eq!(config.arts.get(&fallback_combo), Some(&ICHIMONJI));
        assert!(!config.arts.contains_key(&missing_combo));
        assert_eq!(config.instant_arts.get(&ArtCombo::empty()), Some(&SAKURA_DANCE));
        assert_eq!(config.tools.get(&tool_combo), Some(&78300));
        assert_eq!(config.tool_on_t, None);
        assert_eq!(config.tool_on_q, Some(70400));
        assert_eq!(config.tool_multi_lock_secs.len(), 1);
        assert!(config.tool_multi_lock_secs.contains_key(&78300));
    }

    #[test]
    fn available_upgraded_art_keeps_upgraded_uid() {
        let mut config = Config::default();
        let combo = ArtCombo::pair(ArtToken::Block, ArtToken::Attack);
        config.arts.insert(combo, ICHIMONJI_DOUBLE);

        filter_config_by_availability_with(&mut config, |uid| uid == ICHIMONJI_DOUBLE || uid == ICHIMONJI);

        assert_eq!(config.arts.get(&combo), Some(&ICHIMONJI_DOUBLE));
    }

    #[test]
    fn availability_uid_cache_is_sorted_and_deduplicated() {
        let mut config = Config::default();
        let combo = ArtCombo::pair(ArtToken::Block, ArtToken::Attack);
        config.arts.insert(combo, 7100);
        config.instant_arts.insert(ArtCombo::empty(), 5500);
        config.tools.insert(
            ToolCombo {
                first: ToolFirst::Interact,
                tail: ToolTail::Use,
            },
            78300,
        );
        config.tool_on_t = Some(70500);
        config.tool_on_q = Some(70500);

        let uids = collect_availability_uids(&config);
        assert!(uids.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(uids.iter().filter(|&&uid| uid == 70500).count(), 1);
        assert_eq!(uids, vec![5500, 7100, 70500, 78300]);
    }

    #[test]
    fn sprint_combo_mask_also_suppresses_native_dodge() {
        let combo = ArtCombo::pair(ArtToken::Block, ArtToken::Sprint);
        assert_eq!(combo_action_mask(combo), BLOCK | SPRINT | DODGE);

        let combo = ArtCombo::pair(ArtToken::Block, ArtToken::Jump);
        assert_eq!(combo_action_mask(combo), BLOCK | JUMP);

        let combo = ArtCombo::pair(ArtToken::Block, ArtToken::Crouch);
        assert_eq!(combo_action_mask(combo), BLOCK | CROUCH);
    }
}
