use std::path::{Path, PathBuf};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{
    action::{self, ATTACK, BLOCK, CROUCH, DODGE, GRAPPLE, INTERACT, JUMP, SPRINT, USE_PROSTHETIC, VANILLA_ART_INPUT},
    bindings,
    config::{Config, ToolTail},
    device::{Gamepad, is_key_down},
    features::{
        combat_art::{
            ART_BLOCK_INJECTION_DURATION, ATTACK_SUPRESSION_DURATION, BLOCK_INJECTION_DURATION, CombatArt,
            CombatArtState, Countdown, EMPOWERED_MORTAL_DRAW, HIGH_MONK, ICHIMONJI, ICHIMONJI_DOUBLE, MORTAL_DRAW,
            PRAYING_STRIKES, PRAYING_STRIKES_EXORCISM, SENPO_LEAPING_KICKS, SHADOWFALL, SHADOWRUSH,
            SLOT_SWAP_SETTLE_DURATION, art_fallback, combat_art_slot_matches, combo_action_mask, set_combat_art,
        },
        emblem_cap::Emblem,
        flow::Flow,
        paper_guard,
        prosthetic::ProstheticState,
    },
    game::{self, ID, UID},
    input::{ArtCombo, ArtToken},
    menu_probe,
};

//----------------------------------------------------------------------------
//
//  Basic constants
//
//----------------------------------------------------------------------------

// MOD behavior
/// Block frames to inject when starting a combo-fired combat art.
/// Extra settle frames after swapping combat-art slot before mod injects B+A.
/// After tool lock expires, wait ~1.4s before returning to bare-`t` default.
/// Window for first key → q/t; default ~0.3s @60fps, overridable via cfg.

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
    /// Last pause-menu close observed by this instance. Availability changes
    /// (learned combat arts, picked-up prosthetics) happen through the menu, so
    /// this edge is the only periodic reason to re-check the inventory.
    availability_menu_generation: u64,
    /// Sorted, deduplicated UIDs to check. Rebuilt only when the base config
    /// changes instead of allocating/sorting on every availability refresh.
    availability_uids: Vec<UID>,
    emblem: Emblem,
    flow: Flow,
    combat_art: CombatArtState,
    disable_block: bool,
    blocking_last_frame: bool,
    attacking_last_frame: bool,
    interacting_last_frame: bool,
    grappling_last_frame: bool,
    using_tool_last_frame: bool,
    /// Prosthetic-tool state machine, grouped in its feature module.
    prosthetic: ProstheticState,
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
        let combat_art = CombatArtState::new(config.rl_combo_max_age);
        let modification = Mod {
            cfg_path: cfg_path.clone(),
            config,
            base_config,
            availability_signature: None,
            availability_menu_generation: menu_probe::menu_close_generation(),
            availability_uids,
            emblem,
            flow,
            gamepad: Gamepad::new()?,
            combat_art,
            blocking_last_frame: false,
            attacking_last_frame: false,
            interacting_last_frame: false,
            grappling_last_frame: false,
            using_tool_last_frame: false,
            prosthetic: ProstheticState::new(),
            disable_block: false,
            combo_dirs_last: [false; 4],
        };
        Ok(modification)
    }

    fn refresh_available_bindings(&mut self, force: bool) {
        // Do not call the game's native inventory lookup on a timer. A menu
        // close is the point where learned arts / newly acquired tools can
        // change; until then the previous snapshot is still valid.
        let menu_generation = menu_probe::menu_close_generation();
        if !force && self.availability_signature.is_some() && self.availability_menu_generation == menu_generation {
            return;
        }
        self.availability_menu_generation = menu_generation;

        let Some(signature) = availability_signature(&self.availability_uids) else {
            // Inventory is not ready yet (loading screen). Keep retrying, but
            // the failure path is only a handful of pointer reads.
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
        crate::frame_perf::mark(crate::frame_perf::EventId::AvailabilityRefresh);
        self.config = config;
        self.reset_binding_runtime_state();
    }

    fn reset_binding_runtime_state(&mut self) {
        self.combat_art.combo.clear();
        self.combat_art.clear_fire();
        self.combat_art.queued_art = None;
        self.combat_art.queue_flush_left = 0;
        self.combat_art.queue_flush_skip_tick = false;
        self.combat_art.cur_art = None;
        self.combat_art.attack_delay = 0;
        self.prosthetic.prosthetic_delay = 0;
        self.combat_art.injected_blocks = 0;
        self.disable_block = false;
        self.combat_art.pending_rl_attack = false;
        self.combat_art.suppress_late_rl_skill = false;
        self.prosthetic.tool_first = None;
        self.prosthetic.tool_first_age = 0;
        self.prosthetic.hold_tool_tail = None;
        self.prosthetic.tool_min_use = 0;
        self.prosthetic.cur_tool = None;
        self.prosthetic.return_default_left = 0;
        self.prosthetic.tool_lock_left = 0;
        self.prosthetic.switch_down_last = false;
        self.prosthetic.tools_inited = false;
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
        if !self.combat_art.swapout_countdown.is_done() || self.combat_art.fire_busy() {
            return None;
        }

        // Every instant-art matcher needs either an attack edge or a direction
        // edge. Skipping the scan when neither exists keeps this hot path cheap
        // even when many multi-scheme instant bindings are configured.
        let direction_edge = (combo_up && !self.combo_dirs_last[0])
            || (combo_right && !self.combo_dirs_last[1])
            || (combo_down && !self.combo_dirs_last[2])
            || (combo_left && !self.combo_dirs_last[3]);
        if !attacked_just_now && !direction_edge {
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
        if self.combat_art.equip_no_delay(uid) {
            log::debug!("ART_INSTANT uid={} combo={:?}", uid, combo);
            self.combat_art.clear_fire();
            self.combat_art.clear_queued();
            self.combat_art.combo.clear();
            self.combat_art.injected_blocks = 1;
            self.combat_art.swapout_countdown = Countdown::new(self.combat_art.cur_art.swapout_cooldown());
            return Some(uid);
        }
        None
    }

    pub fn process_input(&mut self, input_handler: &mut game::InputHandler) {
        if bindings::take_reload_request() {
            match bindings::load_effective_config(&self.cfg_path) {
                Ok(config) => {
                    self.base_config = config;
                    self.availability_uids = collect_availability_uids(&self.base_config);
                    self.availability_signature = None;
                    self.refresh_available_bindings(true);
                    crate::frame_perf::mark(crate::frame_perf::EventId::ConfigReload);
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

        // All of these are game-mapped action bits. Do not mix in physical
        // keyboard state here: that would diverge from in-game remaps and from
        // gamepad bindings.
        let jumping = jump_action;
        let sprinting = sprint_action;
        let crouching = crouch_action;
        let attacked_just_now = !self.attacking_last_frame && attacking;
        let blocked_just_now = !self.blocking_last_frame && blocking;
        let used_tool_just_now = !self.using_tool_last_frame && using_tool;

        /***** prosthetic: normal token + tail, or bare t → equip + hold-inject USE *****/
        if !self.prosthetic.tools_inited {
            self.prosthetic.tools_inited = true;
            if self.config.tool_on_t.is_some() {
                self.prosthetic.return_to_default_tool(&self.config);
            }
        }
        let tool_armed_via_art_tail = self.prosthetic.handle_prosthetic_input(
            &self.config,
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
            self.combat_art.combo.tick(
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
            self.combat_art.combo.clear();
        }
        // Tool took this r/l/e/f completion — do not also fire a combat art on the same keys.
        if tool_armed_via_art_tail {
            self.combat_art.combo.clear();
        }
        let late_rl_rejected = self.combat_art.combo.take_late_rl_rejected();
        if late_rl_rejected {
            self.combat_art.suppress_late_rl_skill = true;
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
        let combo_owns_jump = self.combat_art.combo_suppress_mask & JUMP != 0;
        let combo_owns_sprint = self.combat_art.combo_suppress_mask & SPRINT != 0;
        let external_jump = jumping && !combo_owns_jump;
        let external_dodge = dodging && !combo_owns_sprint;
        if external_jump || external_dodge {
            if self.combat_art.fire_busy() {
                log::debug!(
                    "ART_ABORT external_jump={} external_dodge={}",
                    external_jump,
                    external_dodge
                );
            }
            self.combat_art.clear_fire();
            self.combat_art.clear_queued();
        } else if let Some(token) = self.combat_art.hold_for_attack {
            if !self.combat_art.combo.token_held(token) && self.combat_art.art_tap_frames == 0 {
                if !self.combat_art.art_attack_latched {
                    // Released during settle / prime (common for quick `ee`): still fire a short
                    // B+A inject instead of cancelling into a bare block.
                    self.combat_art.art_tap_frames = ART_BLOCK_INJECTION_DURATION.saturating_add(3);
                    log::debug!(
                        "ART_RELEASE→TAP hold={:?} tap={}",
                        token,
                        self.combat_art.art_tap_frames
                    );
                } else {
                    log::debug!("ART_RELEASE hold={:?} (key up)", token);
                    self.combat_art.end_fire_for_queue(self.config.art_queue_delay_frames);
                }
            }
        }

        /***** query the desired combat art *****/
        let mut performed_block_free_art_just_now = false;
        let performed_art_just_now = blocking && attacked_just_now && !late_rl_rejected;
        let mut pending_rl = false;
        if !self.combat_art.swapout_countdown.is_done() {
            // fix buggy behavior of sakura dance, ashina cross and one mind
            // One Mind has two windows for animation bugs to happen
            // one after pressing ATTACK (sheathing) and one after releasing ATTACK (drawing)
            // the current (ugly) solution is to apply the cooldown after pressing ATTACK,
            // but only start counting it down after ATTACK is released
            self.combat_art.swapout_countdown.count_on(!attacking);
        }

        let completed_combo = self.combat_art.combo.take_completed();
        let desired_art = if let Some(combo) = completed_combo {
            // Two consecutive tokens matched (e.g. r↑, ↑e, ↑f, ee, ff, rl, rf, fr, fl).
            if let (Some(uid), Some(hold)) = (self.config.art_or_instant(combo), combo.second()) {
                self.combat_art.combo.clear();
                let is_rl = combo.is_rl();
                if self.combat_art.fire_busy() && !is_rl {
                    // Sustained fire still running: queue and flush after release + delay.
                    log::debug!(
                        "ART_QUEUE uid={} hold={:?} busy cur={:?}",
                        uid,
                        hold,
                        self.combat_art.cur_art
                    );
                    self.combat_art.queued_art = Some((uid, hold));
                    // Delay starts when current fire ends, not when queued.
                    self.combat_art.queue_flush_left = 0;
                    self.combat_art.queue_flush_skip_tick = false;
                    None
                } else {
                    if is_rl && self.combat_art.fire_busy() {
                        log::debug!("ART_RL_PREEMPT busy cur={:?} -> uid={}", self.combat_art.cur_art, uid);
                        self.combat_art.clear_fire();
                        self.combat_art.clear_queued();
                    }
                    log::debug!(
                        "ART_HIT uid={} hold={:?} cur_before={:?} already={} swapout_done={}",
                        uid,
                        hold,
                        self.combat_art.cur_art,
                        self.combat_art.cur_art == Some(uid),
                        self.combat_art.swapout_countdown.is_done(),
                    );
                    performed_block_free_art_just_now = true;
                    pending_rl = true;
                    self.combat_art.combo_suppress_mask = combo_action_mask(combo);
                    let held = self.combat_art.combo.token_held(hold);
                    self.combat_art.arm_combo_fire(hold, held);
                    self.combat_art.clear_queued();
                    Some(uid)
                }
            } else {
                None
            }
        } else if !self.combat_art.fire_busy() {
            if let Some((uid, hold)) = self.combat_art.try_flush_queued() {
                log::debug!("ART_FLUSH_QUEUE uid={} hold={:?}", uid, hold);
                performed_block_free_art_just_now = true;
                pending_rl = true;
                let held = self.combat_art.combo.token_held(hold);
                self.combat_art.arm_combo_fire(hold, held);
                Some(uid)
            } else {
                None
            }
        } else {
            None
        };

        // Bare right-click (r) → default ∅ if configured; skip when `r` is first half of a combo.
        let desired_art = desired_art.or_else(|| {
            if !blocked_just_now || !self.combat_art.swapout_countdown.is_done() || performed_block_free_art_just_now {
                return None;
            }
            if self.combat_art.combo.awaiting_second() {
                None
            } else {
                self.combat_art.combo.clear();
                self.config.art(ArtCombo::empty())
            }
        });

        // if combat art switching happens too quick after performing certain combat arts
        // animation of other unrelated combat arts can be triggered
        if performed_art_just_now || performed_block_free_art_just_now && self.combat_art.swapout_countdown.is_done() {
            self.combat_art.swapout_countdown = Countdown::new(self.combat_art.cur_art.swapout_cooldown())
        }

        /***** equip the desired combat art (or its fallback version) *****/
        let mut art_swapped_this_hit = false;
        if let Some(desired_art) = desired_art {
            let mut desired_art = desired_art;
            loop {
                if combat_art_slot_matches(desired_art) {
                    self.combat_art.cur_art = Some(desired_art);
                    break;
                }
                if set_combat_art(desired_art) {
                    art_swapped_this_hit = true;
                    self.combat_art.cur_art = Some(desired_art);
                    self.combat_art.attack_delay = ATTACK_SUPRESSION_DURATION;
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
            self.combat_art.pending_rl_attack = true;
            let settle = if art_swapped_this_hit {
                SLOT_SWAP_SETTLE_DURATION
            } else {
                ATTACK_SUPRESSION_DURATION
            };
            if self.combat_art.attack_delay < settle {
                self.combat_art.attack_delay = settle;
                log::debug!(
                    "ART_SETTLE swapped={} delay={} hold={:?}",
                    art_swapped_this_hit,
                    self.combat_art.attack_delay,
                    self.combat_art.hold_for_attack
                );
            }
        }

        /***** action injection *****/
        // Legacy block injection for paths that still set injected_blocks.
        // Combo-fired arts use hold_for_attack + art_block_inject_left instead.
        if performed_block_free_art_just_now {
            // Combo path: sustained fire starts after attack_delay (see below).
        } else if self.combat_art.injected_blocks >= 1 {
            if external_jump || external_dodge {
                // External DODGE/JUMP cancels the art; combo-owned jump/sprint bits are
                // suppressed instead of being allowed to preempt the injection.
                self.combat_art.injected_blocks = 0
            } else if self.combat_art.cur_art.is_sheathed() {
                // hold BLOCK for sheathing attacks as long as ATTACK is held until:
                // 1. the player decides to hold BLOCK by themself (that usually means cancelling)
                // 2. the player released the attack
                if attacking && !blocking {
                    input_handler.action |= BLOCK;
                } else {
                    self.combat_art.injected_blocks = 0;
                }
            } else if self.combat_art.injected_blocks < BLOCK_INJECTION_DURATION {
                // inject just a few frames for other art
                input_handler.action |= BLOCK;
                self.combat_art.injected_blocks += 1;
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
        if has_available_arts && self.combat_art.combo.awaiting_block_second() && blocking {
            action::strip_input(input_handler, ATTACK);
        }
        if has_available_arts && blocking && attacking {
            action::strip_input(input_handler, VANILLA_ART_INPUT);
        }
        if instant_art_fired {
            action::or_input(input_handler, VANILLA_ART_INPUT);
        }

        if self.combat_art.suppress_late_rl_skill && !blocking {
            self.combat_art.suppress_late_rl_skill = false;
        }

        // Post-swap: suppress stray ATTACK without B+A (whirlwind-on-switch bug).
        if self.combat_art.attack_delay > 0 {
            action::strip_input(input_handler, ATTACK);
            self.combat_art.attack_delay -= 1;
        }
        // Start BLOCK prime only after settle. Must run even on the frame delay hits 0,
        // otherwise sustained fire below opens B+A one frame early (then prime runs late),
        // which is the Shadowfall "first cast after swap → trailing R1" bug.
        if self.combat_art.attack_delay == 0 && self.combat_art.pending_rl_attack {
            self.combat_art.pending_rl_attack = false;
            self.combat_art.art_block_inject_left = ART_BLOCK_INJECTION_DURATION;
        }

        // The combo keys themselves must never reach the game's native action
        // handling once the combo has been recognized (guard/sprint/jump/crouch
        // animations can otherwise preempt the MOD's BLOCK+ATTACK injection).
        if self.combat_art.combo_suppress_mask != 0 {
            // Suppress only the current logical action. Do NOT clear action_b/
            // action_c here: the game uses them as press/hold history, and
            // clearing them every frame would turn the injected B+A into a new
            // attack edge on every frame (repeated sheathing for Ashina Cross /
            // One Mind instead of a sustained ready-to-draw state).
            input_handler.action &= !self.combat_art.combo_suppress_mask;
        }

        // Sustained art fire:
        // 1) first N frames: BLOCK only + suppress ATTACK (prevents a stray R1/whirlwind)
        // 2) then: BLOCK+ATTACK while last combo token is held (charge / multi-hit)
        //    For `ee`, the last token is Interact — hold e to keep injecting B+A (charge).
        if self.combat_art.attack_delay == 0 {
            if let Some(token) = self.combat_art.hold_for_attack {
                let held = self.combat_art.combo.token_held(token);
                let tapping = self.combat_art.art_tap_frames > 0;
                if held || tapping {
                    if self.combat_art.art_block_inject_left > 0 {
                        action::or_input(input_handler, BLOCK);
                        self.combat_art.art_block_inject_left -= 1;
                    } else {
                        action::or_input(input_handler, VANILLA_ART_INPUT);
                        self.combat_art.art_attack_latched = true;
                    }
                    if tapping && !held {
                        self.combat_art.art_tap_frames -= 1;
                        if self.combat_art.art_tap_frames == 0 {
                            self.combat_art.end_fire_for_queue(self.config.art_queue_delay_frames);
                        }
                    }
                }
            }
        }
        // Prosthetic: settle a few frames after equip, then inject USE while tail held (or min tap).
        // When the combo tail is r/l/e/f, strip that vanilla action so ↑r / ↑e / ↑f fires the
        // tool directly (like swallowing SWITCH for q), not "block first, then use".
        if self.prosthetic.prosthetic_delay != 0 {
            input_handler.action &= !USE_PROSTHETIC;
            self.prosthetic.prosthetic_delay -= 1;
        } else if self.prosthetic.hold_tool_tail.is_some() || self.prosthetic.tool_min_use > 0 {
            input_handler.action |= USE_PROSTHETIC;
            if self.prosthetic.tool_min_use > 0 {
                self.prosthetic.tool_min_use -= 1;
            }
        }
        match self.prosthetic.hold_tool_tail {
            Some(ToolTail::Block) => input_handler.action &= !BLOCK,
            Some(ToolTail::Attack) => input_handler.action &= !ATTACK,
            Some(ToolTail::Interact) => input_handler.action &= !INTERACT,
            Some(ToolTail::Grapple) => {
                input_handler.action &= !GRAPPLE;
                input_handler.action_b &= !GRAPPLE;
                input_handler.action_c &= !GRAPPLE;
            }
            Some(ToolTail::Jump) => action::strip_input(input_handler, JUMP),
            Some(ToolTail::Sprint) => action::strip_input(input_handler, SPRINT),
            Some(ToolTail::Crouch) => action::strip_input(input_handler, CROUCH),
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
//----------------------------------------------------------------------------
//
//  Wrappers of functions from the original game
//
//----------------------------------------------------------------------------

#[cfg(test)]
mod availability_tests {
    use super::*;
    use crate::config::{ToolCombo, ToolFirst};
    use crate::features::combat_art::SAKURA_DANCE;

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
