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

// PlayerIns -> module registry -> SprjChrDataModule.
const PLAYER_MODULE_REGISTRY_OFFSET: usize = 0x1ff8;
const CHR_DATA_MODULE_OFFSET: usize = 0x18;

// SprjChrDataModule fields.
const CURRENT_HP_OFFSET: usize = 0x130;
const CURRENT_STAMINA_OFFSET: usize = 0x148;
const MAX_STAMINA_OFFSET: usize = 0x14c;

// SprjEventDamageInfo gameplay flags (verified in-game).
const GUARD_REACTION_OFFSET: usize = 0x164;
const PERFECT_DEFLECT_FLAGS_OFFSET: usize = 0x231;
const PERFECT_DEFLECT_BIT: u8 = 0x40;

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

#[derive(Clone, Copy, Debug)]
struct DamageEvent {
    hp_drop: i32,
    posture_drop: i32,
    guard_reaction: bool,
    perfect_deflect: bool,
}

static DAMAGE_EVENTS: OnceLock<Mutex<Vec<DamageEvent>>> = OnceLock::new();
static DAMAGE_HOOK_ACTIVE: AtomicBool = AtomicBool::new(false);

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
    log::warn!("flow: player damage reaction hook enabled.");
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

    let (module, hp_before, posture_before) = unsafe {
        let module = player_chr_data_module();
        let hp_before = module.map(|data| read_i32(data, CURRENT_HP_OFFSET)).unwrap_or(0);
        let posture_before = module.map(|data| read_i32(data, CURRENT_STAMINA_OFFSET)).unwrap_or(0);
        (module, hp_before, posture_before)
    };

    let result = unsafe { original(owner, arg1, damage_info, arg3, arg4) };

    let (hp_after, posture_after, guard_reaction, perfect_deflect) = unsafe {
        let hp_after = module
            .map(|data| read_i32(data, CURRENT_HP_OFFSET))
            .unwrap_or(hp_before);
        let posture_after = module
            .map(|data| read_i32(data, CURRENT_STAMINA_OFFSET))
            .unwrap_or(posture_before);
        let (guard_reaction, perfect_deflect) = if damage_info.is_null() {
            (false, false)
        } else {
            let info = damage_info.cast::<u8>();
            (
                read_u8(info, GUARD_REACTION_OFFSET) != 0,
                read_u8(info, PERFECT_DEFLECT_FLAGS_OFFSET) & PERFECT_DEFLECT_BIT != 0,
            )
        };
        (hp_after, posture_after, guard_reaction, perfect_deflect)
    };

    let hp_drop = (hp_before - hp_after).max(0);
    let posture_drop = (posture_before - posture_after).max(0);

    if hp_drop > 0 || posture_drop > 0 || guard_reaction {
        push_damage_event(DamageEvent {
            hp_drop,
            posture_drop,
            guard_reaction,
            perfect_deflect,
        });
    }

    result
}

#[derive(Clone, Copy, Debug, Default)]
struct FlowState {
    stacks: u8,
    timeout_left: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct FlowRefund {
    /// Portion of the incoming posture increase that is cancelled.
    mitigated: i32,
    /// Portion above 100% that actually restores posture.
    restored: i32,
}

impl FlowState {
    fn reset(&mut self) {
        self.stacks = 0;
        self.timeout_left = 0;
    }

    fn on_perfect_deflect(&mut self, config: &FlowConfig, posture_drop: i32) -> (u8, u8, FlowRefund) {
        let before = self.stacks;
        if self.stacks < config.max_stacks {
            self.stacks += 1;
        }
        self.timeout_left = config.duration_frames;

        let percent = config.ratio_for_stack(self.stacks);
        let (mitigated, restored) = if posture_drop > 0 {
            // Up to 100%: cancel part (or all) of this deflect's posture
            // increase. Above 100%: only the excess actually restores posture.
            let mitigated_percent = percent.min(100);
            let mitigated = ((posture_drop as i64 * mitigated_percent as i64 + 50) / 100) as i32;
            let restored = if percent > 100 {
                ((posture_drop as i64 * (percent - 100) as i64 + 50) / 100) as i32
            } else {
                0
            };
            (mitigated, restored)
        } else {
            (0, 0)
        };
        (before, self.stacks, FlowRefund { mitigated, restored })
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
}

impl Flow {
    pub fn new(path: impl AsRef<Path>) -> Flow {
        let config = FlowConfig::load(path.as_ref());
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
        }
    }

    pub fn tick(&mut self) {
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

        let Some(chr_data) = player_chr_data_module() else {
            let _ = take_damage_events();
            return;
        };

        let max_stamina = unsafe { read_i32(chr_data, MAX_STAMINA_OFFSET) };

        if let Some(old) = self.state.tick_timeout() {
            if self.config.debug {
                log::debug!("FLOW_TIMEOUT stack={}->0", old);
            }
        }

        for event in take_damage_events() {
            if self.config.debug {
                log::debug!(
                    "FLOW_EVENT hp_drop={} posture_drop={} guard={} perfect_flag={}",
                    event.hp_drop,
                    event.posture_drop,
                    event.guard_reaction,
                    event.perfect_deflect
                );
            }

            let perfect = event.hp_drop == 0 && event.guard_reaction && event.perfect_deflect;

            if perfect {
                let (before, after, refund) = self.state.on_perfect_deflect(&self.config, event.posture_drop);
                // Apply the two parts separately so <100% is only damage
                // mitigation and only the >100% excess is a real heal.
                if refund.mitigated > 0 {
                    unsafe {
                        add_stamina(chr_data, refund.mitigated, max_stamina);
                    }
                }
                if refund.restored > 0 {
                    unsafe {
                        add_stamina(chr_data, refund.restored, max_stamina);
                    }
                }
                if self.config.debug {
                    log::debug!(
                        "FLOW_PERFECT posture_drop={} stack={}->{} mitigated={} restored={}",
                        event.posture_drop,
                        before,
                        after,
                        refund.mitigated,
                        refund.restored
                    );
                }
            } else if event.hp_drop > 0 || event.posture_drop > 0 {
                let before = self.state.on_non_perfect_hit();
                if self.config.debug {
                    log::debug!(
                        "FLOW_HIT hp_drop={} posture_drop={} stack={}->0",
                        event.hp_drop,
                        event.posture_drop,
                        before
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
    // Keys may be commented (`# flow: on`) or written bare (`flow: on`).
    let body = line.trim().trim_start_matches('#').trim();
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
    fn perfect_deflect_stacks_and_refunds() {
        let cfg = config();
        let mut state = FlowState::default();

        let (before, after, refund) = state.on_perfect_deflect(&cfg, 10);
        assert_eq!((before, after), (0, 1));
        assert_eq!((refund.mitigated, refund.restored), (2, 0));
        for expected_stack in 2..=10u8 {
            let (_, after, _) = state.on_perfect_deflect(&cfg, 10);
            assert_eq!(after, expected_stack);
        }
        let (_, after, refund) = state.on_perfect_deflect(&cfg, 10);
        assert_eq!(after, 10);
        assert_eq!((refund.mitigated, refund.restored), (10, 1));
    }

    #[test]
    fn flow_ratio_at_or_below_100_only_mitigates() {
        let mut cfg = config();
        cfg.ratios = [99; 10];
        let mut state = FlowState::default();
        let (_, _, refund) = state.on_perfect_deflect(&cfg, 100);
        assert_eq!((refund.mitigated, refund.restored), (99, 0));

        cfg.ratios = [100; 10];
        let mut state = FlowState::default();
        let (_, _, refund) = state.on_perfect_deflect(&cfg, 100);
        assert_eq!((refund.mitigated, refund.restored), (100, 0));
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

    /// The shipped cfgs write these keys *uncommented* (`flow: on`);
    /// they must parse exactly like the old `# flow: on` form.
    #[test]
    fn shipped_cfg_reads_bare_flow_keys() {
        for path in ["res/battle_instinct.cfg", "res/battle_instinct_zh.cfg"] {
            let cfg = FlowConfig::load(Path::new(path));
            assert!(cfg.enabled);
            assert_eq!(cfg.max_stacks, 10);
            assert_eq!(cfg.duration_frames, seconds_to_frames(6.0));
            assert_eq!(cfg.ratios, DEFAULT_RATIOS);
        }
    }
}
