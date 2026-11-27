use std::path::{Path, PathBuf};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::{
    action::{self, ATTACK, BLOCK, CROUCH, GRAPPLE, INTERACT, JUMP, SPRINT, USE_PROSTHETIC, VANILLA_ART_INPUT},
    bindings,
    config::{Config, ToolTail},
    device::{Gamepad, is_key_down},
    features::{
        combat_art::{
            ART_INPUT_PAUSE_DURATION, ATTACK_SUPRESSION_DURATION, BLOCK_INJECTION_DURATION, CombatArt, CombatArtState,
            Countdown, EMPOWERED_MORTAL_DRAW, HIGH_MONK, ICHIMONJI, ICHIMONJI_DOUBLE, MORTAL_DRAW, PRAYING_STRIKES,
            PRAYING_STRIKES_EXORCISM, SENPO_LEAPING_KICKS, SHADOWFALL, SHADOWRUSH, SLOT_SWAP_SETTLE_DURATION,
            art_fallback, combat_art_slot_matches, combo_action_mask, set_combat_art,
        },
        emblem_cap::Emblem,
        flow::Flow,
        paper_guard,
        prosthetic::ProstheticState,
    },
    game::{self, ID, UID},
    input::{ActionFrame, ArtCombo, ArtToken, DirectionBuffer},
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
    action_frame: ActionFrame,
    direction: DirectionBuffer,
    /// Prosthetic-tool state machine, grouped in its feature module.
    prosthetic: ProstheticState,
    /// `None` when `no_gamepad.enable` is present: keyboard-only players and
    /// startup diagnosis can skip the gilrs HID enumeration entirely.
    gamepad: Option<Gamepad>,
}

impl Mod {
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Mod> {
        let cfg_path = path.as_ref().to_path_buf();
        let no_gamepad = cfg_path
            .parent()
            .map(|dir| dir.join("no_gamepad.enable").is_file())
            .unwrap_or(false);
        let cfg_text = std::fs::read_to_string(&cfg_path)?;
        let base_config = bindings::load_effective_config_from_text(&cfg_path, &cfg_text)?;
        let config = base_config.clone();
        let availability_uids = collect_availability_uids(&base_config);
        let emblem = Emblem::from_text(&cfg_text);
        let flow = Flow::from_text(&cfg_text);
        paper_guard::init_from_text(&cfg_text);
        let combat_art = CombatArtState::new(config.rl_window());
        let modification = Mod {
            cfg_path: cfg_path.clone(),
            config,
            base_config,
            availability_signature: None,
            availability_menu_generation: menu_probe::menu_close_generation(),
            availability_uids,
            emblem,
            flow,
            gamepad: if no_gamepad {
                log::warn!("gamepad: disabled by no_gamepad.enable");
                None
            } else {
                Some(Gamepad::new()?)
            },
            combat_art,
            action_frame: ActionFrame::default(),
            direction: DirectionBuffer::new(),
            prosthetic: ProstheticState::new(),
            disable_block: false,
        };
        Ok(modification)
    }

    fn refresh_available_bindings(&mut self, force: bool) {
        // Do not call the game's native inventory lookup on a timer. A menu
        // close is the point where learned arts / newly acquired tools can
        // change; until then the previous snapshot is still valid.
        let menu_generation = menu_probe::menu_close_generation();
        if self.availability_menu_generation != menu_generation {
            game::invalidate_pointer_cache();
        }
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
        let tools = config.tools.len()
            + config.tools3.len()
            + usize::from(config.tool_on_t.is_some())
            + usize::from(config.tool_on_q.is_some());
        log::warn!("keybinds: available filter arts={arts} tools={tools}");
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
        self.prosthetic.tool_middle = None;
        self.prosthetic.tool_middle_age = 0;
        self.prosthetic.hold_tool_tail = None;
        self.prosthetic.tool_min_use = 0;
        self.prosthetic.cur_tool = None;
        self.prosthetic.return_default_left = 0;
        self.prosthetic.tool_lock_left = 0;
        self.prosthetic.tools_inited = false;
    }
    fn instant_art_candidate(&self, frame: ActionFrame) -> Option<(ArtCombo, UID)> {
        const DIRECTIONS: [ArtToken; 4] = [ArtToken::Up, ArtToken::Right, ArtToken::Down, ArtToken::Left];
        let mut candidates = [ArtCombo::empty(); 16];
        let mut count = 0usize;
        let mut push = |combo: ArtCombo| {
            if count < candidates.len() {
                candidates[count] = combo;
                count += 1;
            }
        };

        if frame.was_held(ArtToken::Block) {
            for direction in DIRECTIONS {
                if frame.pressed(direction) {
                    push(ArtCombo::pair(ArtToken::Block, direction));
                }
            }
        }
        if frame.pressed(ArtToken::Attack) {
            for direction in DIRECTIONS {
                if frame.held(direction) {
                    push(ArtCombo::pair(direction, ArtToken::Attack));
                }
            }
            if frame.was_held(ArtToken::Interact) {
                push(ArtCombo::pair(ArtToken::Interact, ArtToken::Attack));
            }
            if frame.was_held(ArtToken::Grapple) {
                push(ArtCombo::pair(ArtToken::Grapple, ArtToken::Attack));
            }
        }
        if frame.held(ArtToken::Attack) {
            for direction in DIRECTIONS {
                if frame.pressed(direction) {
                    push(ArtCombo::pair(ArtToken::Attack, direction));
                }
            }
        }

        candidates[..count]
            .iter()
            .find_map(|combo| self.config.instant_arts.get(combo).copied().map(|uid| (*combo, uid)))
    }

    /// Instant arts (cfg name contains 瞬发 / instant): fire same-frame B+A, no combo settle.
    fn try_instant_combat_art(&mut self, frame: ActionFrame) -> Option<UID> {
        if !self.combat_art.swapout_countdown.is_done() || self.combat_art.fire_busy() {
            return None;
        }

        // Every instant-art matcher needs either an attack edge or a direction
        // edge. Skipping the scan when neither exists keeps this hot path cheap.
        if !frame.pressed(ArtToken::Attack) && !frame.pressed_direction() {
            return None;
        }

        let (combo, uid) = self.instant_art_candidate(frame)?;
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
        let stick = self.gamepad.as_mut().and_then(|gamepad| gamepad.get_left_pos());
        let (combo_up, combo_right, combo_down, combo_left) =
            self.direction.update(stick, w_down, d_down, s_down, a_down);

        // Single edge-detection pass. Combat art and prosthetic both consume
        // this frame, so the 11 token edges are no longer computed twice.
        self.action_frame
            .update(input_handler, combo_up, combo_right, combo_down, combo_left);

        /***** parse the action bitflags from the shared frame *****/
        let attacking = self.action_frame.held(ArtToken::Attack);
        let blocking = self.action_frame.held(ArtToken::Block);
        // Both tokens follow the game's logical action bits, so in-game key remaps
        // and gamepad bindings keep working. `e` = interact/action, `f` = grapple/hook.
        let using_tool = self.action_frame.use_prosthetic();
        let jumping = self.action_frame.held(ArtToken::Jump);
        let dodging = self.action_frame.dodging();
        let attacked_just_now = self.action_frame.pressed(ArtToken::Attack);
        let blocked_just_now = self.action_frame.pressed(ArtToken::Block);
        let used_tool_just_now = self.action_frame.use_prosthetic_pressed();

        /***** prosthetic: normal token + tail, or bare t → equip + hold-inject USE *****/
        if !self.prosthetic.tools_inited {
            self.prosthetic.tools_inited = true;
            if self.config.tool_on_t.is_some() {
                self.prosthetic.return_to_default_tool(&self.config);
            }
        }
        let tool_armed_via_art_tail =
            self.prosthetic
                .handle_prosthetic_input(&self.config, input_handler, &self.action_frame);

        /***** update combat-art token buffer (normal action tokens + directions) *****/
        // Logical action bits keep in-game remaps and gamepad bindings working.
        if has_available_arts {
            self.combat_art
                .combo
                .tick_frame_with(&self.action_frame, |combo| self.config.art_or_instant(combo).is_some());
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
        let instant_art_fired = self.try_instant_combat_art(self.action_frame).is_some();

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
                    self.combat_art.art_tap_frames = ART_INPUT_PAUSE_DURATION.saturating_add(3);
                    log::debug!(
                        "ART_RELEASE→TAP hold={:?} tap={}",
                        token,
                        self.combat_art.art_tap_frames
                    );
                } else {
                    log::debug!("ART_RELEASE hold={:?} (key up)", token);
                    self.combat_art.end_fire_for_queue(self.config.art_queue_delay());
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
            // Two or three consecutive tokens matched (e.g. r↑, ↑e, ee, rl, or a triple).
            if let (Some(uid), Some(hold)) = (self.config.art_or_instant(combo), combo.last()) {
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
        // activate arts. While waiting for the next key of block-first combos, also strip
        // stray ATTACK so held-l + new block cannot vanilla-fire before the combo completes.
        if has_available_arts && self.combat_art.combo.awaiting_block_combo() && blocking {
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
            self.combat_art.art_input_pause_left = ART_INPUT_PAUSE_DURATION;
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
        // 1) first N frames: pause synthetic input (all combo keys suppressed)
        // 2) then: inject BLOCK+ATTACK while the last combo token is held
        //    (charge / multi-hit). For `ee`, the last token is Interact.
        if self.combat_art.attack_delay == 0 {
            if let Some(token) = self.combat_art.hold_for_attack {
                let held = self.combat_art.combo.token_held(token);
                let tapping = self.combat_art.art_tap_frames > 0;
                if held || tapping {
                    if self.combat_art.art_input_pause_left > 0 {
                        // Hold both keys for a short pause so the game does not
                        // see an early attack edge, then inject B+A directly.
                        self.combat_art.art_input_pause_left -= 1;
                    } else {
                        action::or_input(input_handler, VANILLA_ART_INPUT);
                        self.combat_art.art_attack_latched = true;
                    }
                    if tapping && !held {
                        self.combat_art.art_tap_frames -= 1;
                        if self.combat_art.art_tap_frames == 0 {
                            self.combat_art.end_fire_for_queue(self.config.art_queue_delay());
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
    }
}

fn collect_availability_uids(config: &Config) -> Vec<UID> {
    let mut uids: Vec<UID> = Vec::new();
    uids.extend(config.arts.values().copied());
    uids.extend(config.instant_arts.values().copied());
    uids.extend(config.tools.values().copied());
    uids.extend(config.tools3.values().copied());
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
    config.tools3.retain(|_, uid| available(*uid));
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
    use crate::action::DODGE;
    use crate::config::{ToolCombo, ToolCombo3, ToolFirst};
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
    fn three_key_tool_availability_is_filtered_and_collected() {
        let mut config = Config::default();
        let combo = ToolCombo3 {
            first: ToolFirst::Up,
            middle: ToolFirst::Right,
            tail: ToolTail::Use,
        };
        config.tools3.insert(combo, 75300);
        assert!(collect_availability_uids(&config).contains(&75300));

        filter_config_by_availability_with(&mut config, |uid| uid != 75300);
        assert!(config.tools3.is_empty());
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
