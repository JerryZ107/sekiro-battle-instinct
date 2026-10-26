use std::{
    ffi::c_void,
    fs, mem,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::anyhow;
use minhook::MinHook;

use crate::{frame::Frames, game};

// SprjEventDamageInfo::DAMAGE_TYPE_PARRY.
const DAMAGE_TYPE_PARRY: u32 = 5;

// SprjEventDamageInfo fields.
const DAMAGE_TYPE_OFFSET: usize = 0x24;
const DAMAGE_LEVEL_OFFSET: usize = 0x28;

// PlayerIns -> module registry -> SprjChrDataModule.
const PLAYER_MODULE_REGISTRY_OFFSET: usize = 0x1ff8;
const CHR_DATA_MODULE_OFFSET: usize = 0x18;

// SprjChrDataModule fields.
const CURRENT_HP_OFFSET: usize = 0x130;
const CURRENT_STAMINA_OFFSET: usize = 0x148;
const MAX_STAMINA_OFFSET: usize = 0x14c;

const DEFAULT_RATIOS: [u8; 10] = [20, 30, 40, 50, 60, 70, 80, 90, 100, 110];

#[derive(Clone, Debug)]
struct FlowConfig {
    enabled: bool,
    max_stacks: u8,
    duration_frames: u32,
    ratios: [u8; 10],
    debug: bool,
}

impl Default for FlowConfig {
    fn default() -> FlowConfig {
        FlowConfig {
            enabled: true,
            max_stacks: 10,
            duration_frames: seconds_to_frames(6.0),
            ratios: DEFAULT_RATIOS,
            debug: false,
        }
    }
}

impl FlowConfig {
    fn load(path: &Path) -> FlowConfig {
        let mut config = FlowConfig::default();
        let Ok(text) = fs::read_to_string(path) else {
            return config;
        };

        for line in text.lines() {
            let Some((key, value)) = split_kv(line) else {
                continue;
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "心流功能" | "flow" | "flow enabled" => config.enabled = parse_bool(value),
                "心流最大层数" | "flow max stacks" => {
                    if let Some(v) = parse_u8(value) {
                        config.max_stacks = v.clamp(1, 10);
                    }
                }
                "心流持续时间" | "flow duration" => {
                    config.duration_frames = parse_seconds_frames(value, config.duration_frames);
                }
                "心流回复比例" | "flow refund ratios" => {
                    if let Some(ratios) = parse_ratios(value) {
                        config.ratios = ratios;
                    }
                }
                "心流调试日志" | "flow debug log" => config.debug = parse_bool(value),
                _ => {}
            }
        }

        config.duration_frames = config.duration_frames.max(1);
        config
    }

    fn ratio_for_stack(&self, stacks: u8) -> u32 {
        if stacks == 0 {
            return 0;
        }
        let idx = (stacks as usize - 1).min(self.ratios.len() - 1);
        self.ratios[idx] as u32
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DamageProbe {
    info_30: u32,
    info_1e0: i32,
    info_1e4: i32,
    info_231: u8,
    d8: u8,
    d9: u8,
    da: u8,
    ed: u8,
    f4: u8,
    info_164: u8,
    info_1c4: u8,
    info_1c5: u8,
    info_1c6: u8,
    info_1ca: u8,
    info_1cb: u8,
    info_1cd: u8,
    info_1cf: u8,
    chr_228: u8,
    chr_25c: i32,
}

#[derive(Clone, Copy, Debug)]
struct DamageEvent {
    damage_type: u32,
    damage_level: u32,
    hp_drop: i32,
    posture_drop: i32,
    probe: DamageProbe,
}

static DAMAGE_EVENTS: OnceLock<Mutex<Vec<DamageEvent>>> = OnceLock::new();
static DAMAGE_HOOK_ACTIVE: AtomicBool = AtomicBool::new(false);
static FLOW_DEBUG: AtomicBool = AtomicBool::new(false);

fn damage_events() -> &'static Mutex<Vec<DamageEvent>> {
    DAMAGE_EVENTS.get_or_init(|| Mutex::new(Vec::with_capacity(8)))
}

fn push_damage_event(event: DamageEvent) {
    if let Ok(mut events) = damage_events().lock() {
        events.push(event);
    }
}

fn take_damage_events() -> Vec<DamageEvent> {
    damage_events()
        .lock()
        .map(|mut events| mem::take(&mut *events))
        .unwrap_or_default()
}

type DamageHookFn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u8, u8) -> usize;

static DAMAGE_HOOK_ORIG: OnceLock<DamageHookFn> = OnceLock::new();

/// Install the direct perfect-deflect hook.
///
/// The third argument is `SprjEventDamageInfo`; `+0x24` is DAMAGE_TYPE and
/// `+0x28` is damage level. Perfect deflect is `DAMAGE_TYPE_PARRY == 5`.
pub fn install_damage_hook() -> anyhow::Result<()> {
    unsafe {
        let target = game::PLAYER_DAMAGE_REACTION as *mut c_void;
        let detour = damage_reaction_detour as *mut c_void;
        let original = MinHook::create_hook(target, detour)
            .map_err(|status| anyhow!("MinHook failed for player damage reaction: {status:?}"))?;
        let original: DamageHookFn = mem::transmute(original);
        DAMAGE_HOOK_ORIG
            .set(original)
            .map_err(|_| anyhow!("player damage reaction hook was already installed"))?;
    }

    DAMAGE_HOOK_ACTIVE.store(true, Ordering::Release);
    log::warn!("flow: player damage reaction hook enabled (direct DAMAGE_TYPE detection).");
    Ok(())
}

unsafe extern "system" fn damage_reaction_detour(
    owner: *mut c_void,
    arg1: *mut c_void,
    damage_info: *mut c_void,
    arg3: u8,
    arg4: u8,
) -> usize {
    let Some(original) = DAMAGE_HOOK_ORIG.get().copied() else {
        return 0;
    };

    let (damage_type, damage_level, module, hp_before, posture_before) = unsafe {
        let (damage_type, damage_level) = if damage_info.is_null() {
            (u32::MAX, u32::MAX)
        } else {
            let info = damage_info.cast::<u8>();
            (read_u32(info, DAMAGE_TYPE_OFFSET), read_u32(info, DAMAGE_LEVEL_OFFSET))
        };
        let module = player_chr_data_module();
        let hp_before = module.map(|data| read_i32(data, CURRENT_HP_OFFSET)).unwrap_or(0);
        let posture_before = module.map(|data| read_i32(data, CURRENT_STAMINA_OFFSET)).unwrap_or(0);
        (damage_type, damage_level, module, hp_before, posture_before)
    };

    let result = unsafe { original(owner, arg1, damage_info, arg3, arg4) };

    let (hp_after, posture_after, probe) = unsafe {
        let hp_after = module
            .map(|data| read_i32(data, CURRENT_HP_OFFSET))
            .unwrap_or(hp_before);
        let posture_after = module
            .map(|data| read_i32(data, CURRENT_STAMINA_OFFSET))
            .unwrap_or(posture_before);
        let probe = if damage_info.is_null() {
            DamageProbe::default()
        } else {
            let info = damage_info.cast::<u8>();
            // These two flags are required for gameplay; keep them cheap and
            // read them even when diagnostic logging is disabled.
            let mut probe = DamageProbe {
                info_231: read_u8(info, 0x231),
                info_164: read_u8(info, 0x164),
                ..DamageProbe::default()
            };

            if FLOW_DEBUG.load(Ordering::Relaxed) {
                probe.info_30 = read_u32(info, 0x30);
                probe.info_1e0 = read_i32(info, 0x1e0);
                probe.info_1e4 = read_i32(info, 0x1e4);
                probe.d8 = read_u8(info, 0xd8);
                probe.d9 = read_u8(info, 0xd9);
                probe.da = read_u8(info, 0xda);
                probe.ed = read_u8(info, 0xed);
                probe.f4 = read_u8(info, 0xf4);
                probe.info_1c4 = read_u8(info, 0x1c4);
                probe.info_1c5 = read_u8(info, 0x1c5);
                probe.info_1c6 = read_u8(info, 0x1c6);
                probe.info_1ca = read_u8(info, 0x1ca);
                probe.info_1cb = read_u8(info, 0x1cb);
                probe.info_1cd = read_u8(info, 0x1cd);
                probe.info_1cf = read_u8(info, 0x1cf);
                probe.chr_228 = module.map(|data| read_u8(data, 0x228)).unwrap_or_default();
                probe.chr_25c = module.map(|data| read_i32(data, 0x25c)).unwrap_or_default();
            }
            probe
        };
        (hp_after, posture_after, probe)
    };

    let hp_drop = (hp_before - hp_after).max(0);
    let posture_drop = (posture_before - posture_after).max(0);

    if damage_type == DAMAGE_TYPE_PARRY || hp_drop > 0 || posture_drop > 0 {
        push_damage_event(DamageEvent {
            damage_type,
            damage_level,
            hp_drop,
            posture_drop,
            probe,
        });
    }

    result
}

#[derive(Clone, Copy, Debug, Default)]
struct FlowState {
    stacks: u8,
    timeout_left: u32,
}

impl FlowState {
    fn reset(&mut self) {
        self.stacks = 0;
        self.timeout_left = 0;
    }

    fn on_perfect_deflect(&mut self, config: &FlowConfig, posture_drop: i32) -> (u8, u8, i32) {
        let before = self.stacks;
        if self.stacks < config.max_stacks {
            self.stacks += 1;
        }
        self.timeout_left = config.duration_frames;

        let percent = config.ratio_for_stack(self.stacks);
        let refund = if posture_drop > 0 {
            ((posture_drop as i64 * percent as i64 + 50) / 100) as i32
        } else {
            0
        };
        (before, self.stacks, refund)
    }

    fn on_non_perfect_hit(&mut self) -> u8 {
        let before = self.stacks;
        self.reset();
        before
    }

    fn tick_timeout(&mut self) -> Option<u8> {
        if self.timeout_left == 0 {
            return None;
        }
        self.timeout_left -= 1;
        if self.timeout_left == 0 && self.stacks > 0 {
            let before = self.stacks;
            self.stacks = 0;
            return Some(before);
        }
        None
    }
}

pub struct Flow {
    config: FlowConfig,
    state: FlowState,
    warned_hook_unavailable: bool,
    /// Guard state sampled once per input tick.
    blocking: bool,
    /// Frames since guard was pressed. `u32::MAX` while guard is not held.
    guard_frames: u32,
}

impl Flow {
    pub fn new(path: impl AsRef<Path>) -> Flow {
        let config = FlowConfig::load(path.as_ref());
        FLOW_DEBUG.store(config.debug, Ordering::Relaxed);
        log::warn!(
            "flow settings: enabled={} max={} duration_frames={} ratios={:?} debug={}",
            config.enabled,
            config.max_stacks,
            config.duration_frames,
            config.ratios,
            config.debug
        );
        Flow {
            config,
            state: FlowState::default(),
            warned_hook_unavailable: false,
            blocking: false,
            guard_frames: u32::MAX,
        }
    }

    fn update_guard_state(&mut self, blocking: bool) {
        if blocking {
            if self.blocking {
                self.guard_frames = self.guard_frames.saturating_add(1);
            } else {
                self.guard_frames = 0;
            }
        } else {
            self.guard_frames = u32::MAX;
        }
        self.blocking = blocking;
    }

    pub fn tick(&mut self, blocking: bool) {
        self.update_guard_state(blocking);

        if !self.config.enabled {
            let _ = take_damage_events();
            return;
        }

        if !DAMAGE_HOOK_ACTIVE.load(Ordering::Acquire) {
            if !self.warned_hook_unavailable {
                self.warned_hook_unavailable = true;
                log::error!("flow: direct damage hook is not active");
            }
            let _ = take_damage_events();
            return;
        }

        let Some(player) = player_ins() else {
            let _ = take_damage_events();
            return;
        };
        let Some(chr_data) = chr_data_module(player) else {
            let _ = take_damage_events();
            return;
        };

        let max_stamina = unsafe { read_i32(chr_data, MAX_STAMINA_OFFSET) };

        if let Some(old) = self.state.tick_timeout() {
            log::info!("FLOW_TIMEOUT stack={}->0", old);
        }

        for event in take_damage_events() {
            if self.config.debug {
                log::info!(
                    "FLOW_EVENT type={} level={} hp_drop={} posture_drop={} blocking={} guard_frames={} probe={:?}",
                    event.damage_type,
                    event.damage_level,
                    event.hp_drop,
                    event.posture_drop,
                    self.blocking,
                    self.guard_frames,
                    event.probe
                );
            }

            // Controlled in-game tests showed the game's own flags are:
            //   info_164  != 0 -> this event was a guard/deflect reaction
            //   info_231 bit 6 -> the guard reaction was a perfect deflect
            // The input-timing window is intentionally NOT used: it produced
            // both false positives (ordinary blocks) and false negatives.
            let perfect = event.hp_drop == 0 && event.probe.info_164 != 0 && event.probe.info_231 & 0x40 != 0;

            if perfect {
                let (before, after, refund) = self.state.on_perfect_deflect(&self.config, event.posture_drop);
                if refund > 0 {
                    unsafe {
                        add_stamina(chr_data, refund, max_stamina);
                    }
                }
                if self.config.debug {
                    log::info!(
                        "FLOW_PERFECT type={} level={} posture_drop={} stack={}->{} refund={} hp_drop={} guard_frames={} source={}",
                        event.damage_type,
                        event.damage_level,
                        event.posture_drop,
                        before,
                        after,
                        refund,
                        event.hp_drop,
                        self.guard_frames,
                        "game_flags"
                    );
                }
            } else if event.hp_drop > 0 || event.posture_drop > 0 {
                let before = self.state.on_non_perfect_hit();
                if self.config.debug {
                    log::info!(
                        "FLOW_HIT type={} level={} hp_drop={} posture_drop={} stack={}->0 guard_frames={}",
                        event.damage_type,
                        event.damage_level,
                        event.hp_drop,
                        event.posture_drop,
                        before,
                        self.guard_frames
                    );
                }
            }
        }

        if self.config.debug {
            log::debug!(
                "FLOW_STATE stack={} timeout={} stamina={}/{}",
                self.state.stacks,
                self.state.timeout_left,
                unsafe { read_i32(chr_data, CURRENT_STAMINA_OFFSET) },
                max_stamina
            );
        }
    }
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let body = line.trim().strip_prefix('#')?.trim();
    for sep in [':', '：'] {
        if let Some((key, value)) = body.split_once(sep) {
            return Some((key, value));
        }
    }
    None
}

fn parse_bool(raw: &str) -> bool {
    let value = raw.trim().to_ascii_lowercase();
    matches!(value.as_str(), "1" | "开" | "on" | "true" | "yes" | "是") || value.starts_with("开")
}

fn parse_u8(raw: &str) -> Option<u8> {
    raw.trim().split_whitespace().next()?.parse::<u8>().ok()
}

fn parse_seconds(raw: &str) -> Option<f32> {
    let text = raw
        .trim()
        .split_whitespace()
        .next()?
        .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '秒');
    text.parse::<f32>().ok().map(|value| value.max(0.0))
}

fn seconds_to_frames(seconds: f32) -> u32 {
    let standard = (seconds * 60.0).round().clamp(1.0, u16::MAX as f32) as u16;
    Frames::standard(standard).as_actual() as u32
}

fn parse_seconds_frames(raw: &str, fallback: u32) -> u32 {
    parse_seconds(raw)
        .map(seconds_to_frames)
        .filter(|frames| *frames > 0)
        .unwrap_or(fallback)
}

fn parse_ratios(raw: &str) -> Option<[u8; 10]> {
    let values: Vec<u8> = raw
        .split([',', '，', '/', ' '])
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() { None } else { part.parse::<u8>().ok() }
        })
        .collect();
    if values.is_empty() {
        return None;
    }

    let mut ratios = DEFAULT_RATIOS;
    for (slot, value) in ratios.iter_mut().zip(values) {
        *slot = value;
    }
    Some(ratios)
}

fn player_ins() -> Option<*mut u8> {
    unsafe {
        let world_data = *(game::WORLD_DATA as *const *const u8);
        if world_data.is_null() {
            return None;
        }
        let player = *(world_data.add(0x88) as *const *mut u8);
        if player.is_null() {
            return None;
        }
        Some(player)
    }
}

fn chr_data_module(player: *mut u8) -> Option<*mut u8> {
    unsafe {
        let registry = *(player.add(PLAYER_MODULE_REGISTRY_OFFSET) as *const *mut u8);
        if registry.is_null() {
            return None;
        }
        let chr_data = *(registry.add(CHR_DATA_MODULE_OFFSET) as *const *mut u8);
        if chr_data.is_null() {
            return None;
        }
        Some(chr_data)
    }
}

fn player_chr_data_module() -> Option<*mut u8> {
    player_ins().and_then(chr_data_module)
}

unsafe fn add_stamina(chr_data: *mut u8, amount: i32, max_stamina: i32) {
    if amount <= 0 || max_stamina <= 0 {
        return;
    }
    unsafe {
        let current = read_i32(chr_data, CURRENT_STAMINA_OFFSET);
        let next = current.saturating_add(amount).min(max_stamina);
        write_i32(chr_data, CURRENT_STAMINA_OFFSET, next);
    }
}

unsafe fn read_u8(base: *const u8, offset: usize) -> u8 {
    unsafe { (base.add(offset) as *const u8).read_unaligned() }
}

unsafe fn read_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { (base.add(offset) as *const u32).read_unaligned() }
}

unsafe fn read_i32(base: *const u8, offset: usize) -> i32 {
    unsafe { (base.add(offset) as *const i32).read_unaligned() }
}

unsafe fn write_i32(base: *mut u8, offset: usize, value: i32) {
    unsafe {
        (base.add(offset) as *mut i32).write_unaligned(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FlowConfig {
        FlowConfig {
            enabled: true,
            max_stacks: 10,
            duration_frames: 360,
            ratios: DEFAULT_RATIOS,
            debug: false,
        }
    }

    #[test]
    fn guard_window_starts_on_press_and_resets_on_release() {
        let mut flow = Flow {
            config: config(),
            state: FlowState::default(),
            warned_hook_unavailable: false,
            blocking: false,
            guard_frames: u32::MAX,
        };

        flow.update_guard_state(false);
        assert_eq!(flow.guard_frames, u32::MAX);
        flow.update_guard_state(true);
        assert_eq!(flow.guard_frames, 0);
        flow.update_guard_state(true);
        assert_eq!(flow.guard_frames, 1);
        flow.update_guard_state(false);
        assert_eq!(flow.guard_frames, u32::MAX);
    }

    #[test]
    fn perfect_deflect_stacks_and_refunds() {
        let cfg = config();
        let mut state = FlowState::default();

        let (before, after, refund) = state.on_perfect_deflect(&cfg, 10);
        assert_eq!((before, after, refund), (0, 1, 2));
        for expected_stack in 2..=10u8 {
            let (_, after, _) = state.on_perfect_deflect(&cfg, 10);
            assert_eq!(after, expected_stack);
        }
        let (_, after, refund) = state.on_perfect_deflect(&cfg, 10);
        assert_eq!((after, refund), (10, 11));
    }

    #[test]
    fn non_perfect_hit_clears_stacks() {
        let cfg = config();
        let mut state = FlowState::default();
        state.on_perfect_deflect(&cfg, 10);
        state.on_perfect_deflect(&cfg, 10);
        assert_eq!(state.on_non_perfect_hit(), 2);
        assert_eq!(state.stacks, 0);
        assert_eq!(state.timeout_left, 0);
    }

    #[test]
    fn timeout_clears_stacks() {
        let cfg = config();
        let mut state = FlowState::default();
        state.on_perfect_deflect(&cfg, 10);
        for _ in 0..cfg.duration_frames - 1 {
            assert_eq!(state.tick_timeout(), None);
        }
        assert_eq!(state.tick_timeout(), Some(1));
        assert_eq!(state.stacks, 0);
    }
}
