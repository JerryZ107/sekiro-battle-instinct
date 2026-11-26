//! Optional game-side FPS unlock / frame pacing.
//!
//! Completely inert unless `<game dir>\fps_unlock.cfg` exists and contains
//! `enabled = true`. Every patch verifies the current game value first and
//! refuses to write when the expected Sekiro 1.6.0.0 default is not present
//! (for example, another unlocker is already active).
//!
//! Offsets are the same ones used by rootBrz/SekiroImGui and
//! kargoshakar/SekiroImGuiCustom. This module is experimental: >60 fps changes
//! Sekiro's gameplay timing, and the speed-fix table is only an approximation.

use std::{
    ffi::c_void,
    fs, mem,
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering},
    },
};

use anyhow::{Result, anyhow, bail};
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            CDS_TYPE, DEVMODEW, DISP_CHANGE, DM_DISPLAYFREQUENCY, GetDC, GetDeviceCaps, ReleaseDC, VREFRESH,
        },
        System::{
            Diagnostics::Debug::{FlushInstructionCache, ReadProcessMemory, WriteProcessMemory},
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Memory::{
                MEM_COMMIT, MEM_FREE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE,
                PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualAlloc, VirtualProtect, VirtualQuery,
            },
            Threading::GetCurrentProcess,
        },
    },
    core::{PCWSTR, s, w},
};

use crate::{game, hook_util};

// Sekiro 1.6.0.0 RVAs (upstream SekiroImGui / SekiroImGuiCustom).
const FPS_DELTA_RVA: usize = 0x11AB550;
const SPEED_FIX_DISP_RVA: usize = 0x7DE8AD + 15;
const LOADING_RVA: usize = 0x3D7ACB3;

const DEFAULT_DELTA: f32 = 1.0 / 60.0;
const LOADING_TARGET_FPS: u16 = 300;

// Values the game's speed constant is allowed to have. The high-fps fix picks
// the value nearest to `limit / 2`, matching upstream `speedFixMatrix`.
const SPEED_FIX_MATRIX: [f32; 35] = [
    15.0, 16.0, 16.6667, 18.0, 18.6875, 18.8516, 20.0, 24.0, 25.0, 27.5, 30.0, 32.0, 38.5, 40.0, 48.0, 49.5, 50.0,
    57.2958, 60.0, 64.0, 66.75, 67.0, 78.8438, 80.0, 84.0, 90.0, 93.8, 100.0, 120.0, 127.0, 128.0, 130.0, 140.0, 144.0,
    150.0,
];

static ACTIVE: AtomicBool = AtomicBool::new(false);
static LIMIT_FPS: AtomicU16 = AtomicU16::new(60);
/// 0 = auto (pass through), 1 = force on, 2 = force off.
static VSYNC_MODE: AtomicU32 = AtomicU32::new(0);
static LOADING_BOOST: AtomicBool = AtomicBool::new(false);
static LOADING_ADDR: AtomicUsize = AtomicUsize::new(0);
static FPS_DELTA_ADDR: AtomicUsize = AtomicUsize::new(0);
static CURRENT_TARGET_FPS: AtomicU16 = AtomicU16::new(0);
static ORIGINAL_DELTA_BITS: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VsyncMode {
    Auto,
    On,
    Off,
}

impl VsyncMode {
    const fn code(self) -> u32 {
        match self {
            VsyncMode::Auto => 0,
            VsyncMode::On => 1,
            VsyncMode::Off => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Config {
    enabled: bool,
    limit: u16,
    loading_boost: bool,
    vsync: VsyncMode,
    speed_fix: bool,
    fullscreen_refresh: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            // 60 fps divides both 60 Hz and 180 Hz panels, so the default
            // profile stays vanilla-smooth even when v-sync is on.
            limit: 60,
            loading_boost: false,
            vsync: VsyncMode::Auto,
            speed_fix: false,
            fullscreen_refresh: true,
        }
    }
}

impl Config {
    fn parse(text: &str) -> Config {
        let mut cfg = Config::default();
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase().replace('-', "_");
            let value = value.trim();
            match key.as_str() {
                "enabled" | "unlock" => cfg.enabled = parse_bool(value).unwrap_or(cfg.enabled),
                "limit" | "fps_limit" | "fps" => {
                    cfg.limit = value.parse::<u16>().unwrap_or(cfg.limit).clamp(30, 300);
                }
                "loading_boost" => cfg.loading_boost = parse_bool(value).unwrap_or(cfg.loading_boost),
                "vsync" => {
                    cfg.vsync = match value.to_ascii_lowercase().as_str() {
                        "on" | "1" | "true" | "开" => VsyncMode::On,
                        "off" | "0" | "false" | "关" => VsyncMode::Off,
                        _ => VsyncMode::Auto,
                    };
                }
                "speed_fix" => cfg.speed_fix = parse_bool(value).unwrap_or(cfg.speed_fix),
                "fullscreen_refresh" => {
                    cfg.fullscreen_refresh = parse_bool(value).unwrap_or(cfg.fullscreen_refresh);
                }
                _ => {}
            }
        }
        cfg
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "开" | "是" => Some(true),
        "0" | "false" | "no" | "off" | "关" | "否" => Some(false),
        _ => None,
    }
}

//----------------------------------------------------------------------------
//  Public API
//----------------------------------------------------------------------------

/// Read `fps_unlock.cfg` and activate the feature. No effect when the file is
/// missing or `enabled = false`.
pub fn init(game_dir: &Path) {
    let path = game_dir.join("fps_unlock.cfg");
    let Ok(text) = fs::read_to_string(&path) else {
        log::debug!("fps unlock: {} not found; disabled", path.display());
        return;
    };
    let config = Config::parse(&text);
    if !config.enabled {
        log::warn!("fps unlock: disabled by {}", path.display());
        return;
    }
    match activate(&config) {
        Ok(()) => {
            ACTIVE.store(true, Ordering::Release);
            log::warn!(
                "fps unlock: active limit={} loading_boost={} vsync={:?} speed_fix={} fullscreen_refresh={}",
                config.limit,
                config.loading_boost,
                config.vsync,
                config.speed_fix,
                config.fullscreen_refresh
            );
        }
        Err(error) => {
            log::error!("fps unlock: refused; no further game memory was changed: {error:#}");
        }
    }
}

/// Called from the Present detours. Returns the effective `syncInterval`.
/// When the feature is disabled this is one atomic load and a pass-through.
#[inline]
pub fn on_present(sync: u32) -> u32 {
    if !ACTIVE.load(Ordering::Relaxed) {
        return sync;
    }

    if LOADING_BOOST.load(Ordering::Relaxed) {
        let loading_addr = LOADING_ADDR.load(Ordering::Relaxed);
        if loading_addr != 0 {
            // Safe: the byte was readability-probed at init and is static data.
            let loading = unsafe { (loading_addr as *const u8).read_volatile() } != 0;
            let target = if loading {
                LOADING_TARGET_FPS
            } else {
                LIMIT_FPS.load(Ordering::Relaxed)
            };
            let previous = CURRENT_TARGET_FPS.swap(target, Ordering::Relaxed);
            if previous != target {
                let fps_addr = FPS_DELTA_ADDR.load(Ordering::Relaxed);
                if fps_addr == 0 || !write_f32(fps_addr, 1.0 / target as f32) {
                    CURRENT_TARGET_FPS.store(previous, Ordering::Relaxed);
                    log::error!("fps unlock: cannot update fps delta to {target}");
                }
            }
        }
    }

    match VSYNC_MODE.load(Ordering::Relaxed) {
        1 => 1,
        2 => 0,
        _ => sync,
    }
}

/// Restore the original 1/60 delta. Kept for completeness; the DLL normally
/// lives until the process exits.
#[allow(dead_code)]
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::AcqRel) {
        return;
    }
    let addr = FPS_DELTA_ADDR.load(Ordering::Acquire);
    let original = f32::from_bits(ORIGINAL_DELTA_BITS.load(Ordering::Acquire));
    if addr != 0 {
        let _ = write_f32(addr, original);
    }
    log::warn!("fps unlock: restored original frame delta");
}

//----------------------------------------------------------------------------
//  Activation / patching
//----------------------------------------------------------------------------

fn activate(config: &Config) -> Result<()> {
    let base = game::module_base();
    if base == 0 {
        bail!("sekiro.exe module base is null");
    }

    let fps_addr = base + FPS_DELTA_RVA;
    let current = read_f32(fps_addr).ok_or_else(|| anyhow!("cannot read fps delta at {fps_addr:#x}"))?;
    if !is_default_delta(current) {
        bail!("fps delta at {fps_addr:#x} is {current:.6}, not 1/60; another FPS unlocker is probably active");
    }

    FPS_DELTA_ADDR.store(fps_addr, Ordering::Release);
    ORIGINAL_DELTA_BITS.store(current.to_bits(), Ordering::Release);
    LIMIT_FPS.store(config.limit, Ordering::Release);
    VSYNC_MODE.store(config.vsync.code(), Ordering::Release);
    LOADING_BOOST.store(config.loading_boost, Ordering::Release);

    if let Some(refresh) = display_refresh_hz() {
        if limit_divides_refresh(config.limit, refresh) {
            log::warn!(
                "fps unlock: limit={} divides display {}Hz (even vblank pacing)",
                config.limit,
                refresh
            );
        } else {
            log::warn!(
                "fps unlock: limit={} does NOT divide display {}Hz; v-sync judder is likely - use {} or {}",
                config.limit,
                refresh,
                refresh,
                refresh / 2
            );
        }
    }

    let delta = 1.0 / config.limit as f32;
    if !write_f32(fps_addr, delta) {
        bail!("cannot write fps delta at {fps_addr:#x}");
    }
    CURRENT_TARGET_FPS.store(config.limit, Ordering::Release);
    log::warn!(
        "fps unlock: frame delta {current:.6} -> {delta:.6} (limit={})",
        config.limit
    );

    if config.loading_boost {
        let loading_addr = base + LOADING_RVA;
        let mut probe = 0u8;
        if read_bytes(loading_addr, std::slice::from_mut(&mut probe)) {
            LOADING_ADDR.store(loading_addr, Ordering::Release);
            log::warn!("fps unlock: loading boost enabled (300 fps while loading)");
        } else {
            log::warn!("fps unlock: loading-state byte at {loading_addr:#x} is unreadable; loading boost disabled");
        }
    }

    if config.speed_fix && config.limit != 60 {
        if let Err(error) = apply_speed_fix(base, config.limit) {
            log::error!("fps unlock: speed fix skipped: {error:#}");
        }
    }

    if config.fullscreen_refresh {
        // A failed optional hook must not roll back a working fps patch.
        if let Err(error) = install_fullscreen_refresh_hook() {
            log::error!("fps unlock: fullscreen refresh hook skipped: {error:#}");
        }
    }

    Ok(())
}

fn is_default_delta(value: f32) -> bool {
    (value - DEFAULT_DELTA).abs() < 0.002
}

/// True when `limit` can be displayed with an integer number of vblanks at
/// `refresh` Hz. 180 Hz accepts 60/90/180; 120/144 produce 3:2 or 5:4 judder.
fn limit_divides_refresh(limit: u16, refresh: u16) -> bool {
    limit > 0 && refresh > 0 && (limit == refresh || refresh % limit == 0)
}

fn display_refresh_hz() -> Option<u16> {
    unsafe {
        let hdc = GetDC(None);
        if hdc.0.is_null() {
            return None;
        }
        let hz = GetDeviceCaps(hdc, VREFRESH);
        let _ = ReleaseDC(None, hdc);
        u16::try_from(hz).ok().filter(|hz| *hz > 0)
    }
}

fn nearest_speed_fix(limit: u16) -> f32 {
    let ideal = limit as f32 / 2.0;
    SPEED_FIX_MATRIX
        .iter()
        .copied()
        .min_by(|a, b| (a - ideal).abs().total_cmp(&(b - ideal).abs()))
        .unwrap_or(30.0)
}

fn apply_speed_fix(base: usize, limit: u16) -> Result<()> {
    let disp_addr = base + SPEED_FIX_DISP_RVA;
    let displacement =
        read_i32(disp_addr).ok_or_else(|| anyhow!("cannot read speed-fix displacement at {disp_addr:#x}"))?;
    // RIP-relative operand: target = next_instruction + disp32.
    let target = (disp_addr as isize + 4 + displacement as isize) as usize;
    let current = read_f32(target).ok_or_else(|| anyhow!("cannot read current speed constant at {target:#x}"))?;

    let known = SPEED_FIX_MATRIX.iter().any(|value| (current - value).abs() < 0.01);
    if !known {
        bail!("unexpected speed constant {current:.3} at {target:#x}; refusing to patch");
    }

    let desired = nearest_speed_fix(limit);
    let allocated =
        unsafe { alloc_near(disp_addr, 4) }.ok_or_else(|| anyhow!("no free page within ±2GB of {disp_addr:#x}"))?;
    let allocated_addr = allocated as usize;
    if !write_f32(allocated_addr, desired) {
        bail!("cannot write speed-fix float to {allocated_addr:#x}");
    }

    let relative = allocated_addr as isize - (disp_addr as isize + 4);
    let relative = i32::try_from(relative).map_err(|_| anyhow!("speed-fix displacement out of range"))?;
    if !write_protected(disp_addr, &relative.to_ne_bytes()) {
        bail!("cannot patch speed-fix displacement at {disp_addr:#x}");
    }

    log::warn!("fps unlock: speed fix {current:.3} -> {desired:.3} (constant {target:#x} -> {allocated_addr:#x})");
    Ok(())
}

//----------------------------------------------------------------------------
//  Fullscreen refresh-rate hook
//----------------------------------------------------------------------------

type ChangeDisplaySettingsExWFn =
    unsafe extern "system" fn(PCWSTR, *const DEVMODEW, HWND, CDS_TYPE, *const c_void) -> DISP_CHANGE;

static CHANGE_DISPLAY_ORIG: OnceLock<ChangeDisplaySettingsExWFn> = OnceLock::new();

unsafe extern "system" fn change_display_settings_ex_w_detour(
    device: PCWSTR,
    dev_mode: *const DEVMODEW,
    hwnd: HWND,
    flags: CDS_TYPE,
    param: *const c_void,
) -> DISP_CHANGE {
    if !dev_mode.is_null() {
        // Clear DM_DISPLAYFREQUENCY so exclusive fullscreen is not forced to 60Hz.
        unsafe { (*(dev_mode as *mut DEVMODEW)).dmFields &= !DM_DISPLAYFREQUENCY };
    }
    match CHANGE_DISPLAY_ORIG.get() {
        Some(original) => unsafe { original(device, dev_mode, hwnd, flags, param) },
        None => DISP_CHANGE(0),
    }
}

fn install_fullscreen_refresh_hook() -> Result<()> {
    if CHANGE_DISPLAY_ORIG.get().is_some() {
        return Ok(());
    }
    unsafe {
        let user32 = GetModuleHandleW(w!("user32.dll")).map_err(|error| anyhow!("user32.dll not found: {error}"))?;
        let Some(proc) = GetProcAddress(user32, s!("ChangeDisplaySettingsExW")) else {
            bail!("ChangeDisplaySettingsExW is not exported");
        };
        let target = proc as *const () as *mut c_void;
        let trampoline = hook_util::create_hook_with_retry(
            target,
            change_display_settings_ex_w_detour as *mut c_void,
            "ChangeDisplaySettingsExW",
            false,
        )?;
        let original: ChangeDisplaySettingsExWFn = mem::transmute(trampoline);
        let _ = CHANGE_DISPLAY_ORIG.set(original);
    }
    log::warn!("fps unlock: fullscreen refresh-rate hook installed");
    Ok(())
}

//----------------------------------------------------------------------------
//  Memory helpers
//----------------------------------------------------------------------------

fn read_bytes(addr: usize, out: &mut [u8]) -> bool {
    if addr == 0 || out.is_empty() {
        return false;
    }
    unsafe {
        let process = GetCurrentProcess();
        let mut read = 0usize;
        ReadProcessMemory(
            process,
            addr as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            out.len(),
            Some(&mut read),
        )
        .is_ok()
            && read == out.len()
    }
}

fn write_bytes(addr: usize, bytes: &[u8]) -> bool {
    if addr == 0 || bytes.is_empty() {
        return false;
    }
    unsafe {
        let process = GetCurrentProcess();
        let mut written = 0usize;
        WriteProcessMemory(
            process,
            addr as *const c_void,
            bytes.as_ptr() as *const c_void,
            bytes.len(),
            Some(&mut written),
        )
        .is_ok()
            && written == bytes.len()
    }
}

fn read_f32(addr: usize) -> Option<f32> {
    let mut bytes = [0u8; 4];
    read_bytes(addr, &mut bytes).then(|| f32::from_ne_bytes(bytes))
}

fn write_f32(addr: usize, value: f32) -> bool {
    write_bytes(addr, &value.to_ne_bytes())
}

fn read_i32(addr: usize) -> Option<i32> {
    let mut bytes = [0u8; 4];
    read_bytes(addr, &mut bytes).then(|| i32::from_ne_bytes(bytes))
}

fn write_protected(addr: usize, bytes: &[u8]) -> bool {
    let mut old = PAGE_PROTECTION_FLAGS(0);
    unsafe {
        if VirtualProtect(addr as *const c_void, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old).is_err() {
            return false;
        }
        let ok = write_bytes(addr, bytes);
        let mut restored = PAGE_PROTECTION_FLAGS(0);
        let _ = VirtualProtect(addr as *const c_void, bytes.len(), old, &mut restored);
        if ok {
            let _ = FlushInstructionCache(GetCurrentProcess(), Some(addr as *const c_void), bytes.len());
        }
        ok
    }
}

/// Find a free page within ±2GB so the RIP-relative displacement still fits
/// in an `i32`.
unsafe fn alloc_near(target: usize, size: usize) -> Option<*mut c_void> {
    const RADIUS: usize = 0x7000_0000;
    const GRANULARITY: usize = 0x1_0000;
    let low = target.saturating_sub(RADIUS).max(GRANULARITY);
    let high = target.saturating_add(RADIUS);
    let mut addr = low;

    while addr < high {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let written = unsafe {
            VirtualQuery(
                Some(addr as *const c_void),
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if written == 0 {
            break;
        }
        let base = info.BaseAddress as usize;
        let region = info.RegionSize.max(GRANULARITY);
        if info.State == MEM_FREE && region >= size {
            let candidate = (base + GRANULARITY - 1) & !(GRANULARITY - 1);
            if candidate >= low && candidate < high {
                let allocated = unsafe {
                    VirtualAlloc(
                        Some(candidate as *const c_void),
                        size,
                        MEM_COMMIT | MEM_RESERVE,
                        PAGE_READWRITE,
                    )
                };
                if !allocated.is_null() {
                    return Some(allocated);
                }
            }
        }
        let next = base.saturating_add(region);
        if next <= addr {
            break;
        }
        addr = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fps_config() {
        let cfg = Config::parse("enabled = true\nlimit = 144\nloading_boost = off\nvsync = off\nspeed_fix = false\n");
        assert!(cfg.enabled);
        assert_eq!(cfg.limit, 144);
        assert!(!cfg.loading_boost);
        assert_eq!(cfg.vsync, VsyncMode::Off);
        assert!(!cfg.speed_fix);
    }

    #[test]
    fn clamps_and_defaults() {
        let cfg = Config::parse("enabled=1\nfps=999\nvsync=weird\n");
        assert!(cfg.enabled);
        assert_eq!(cfg.limit, 300);
        assert_eq!(cfg.vsync, VsyncMode::Auto);

        let cfg = Config::parse("enabled=0\nlimit=1\n");
        assert!(!cfg.enabled);
        assert_eq!(cfg.limit, 30);
    }

    #[test]
    fn defaults_are_vanilla_safe() {
        let cfg = Config::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.limit, 60);
        assert!(!cfg.loading_boost);
        assert!(!cfg.speed_fix);
        assert!(cfg.fullscreen_refresh);
    }

    #[test]
    fn only_integer_vblank_limits_are_smooth() {
        assert!(limit_divides_refresh(60, 180));
        assert!(limit_divides_refresh(90, 180));
        assert!(limit_divides_refresh(180, 180));
        assert!(!limit_divides_refresh(120, 180));
        assert!(!limit_divides_refresh(144, 180));
        assert!(limit_divides_refresh(60, 60));
        assert!(!limit_divides_refresh(120, 60));
    }

    #[test]
    fn speed_fix_uses_nearest_matrix_value() {
        assert_eq!(nearest_speed_fix(60), 30.0);
        assert_eq!(nearest_speed_fix(120), 60.0);
        assert_eq!(nearest_speed_fix(144), 67.0);
    }

    #[test]
    fn default_delta_detection_is_strict() {
        assert!(is_default_delta(1.0 / 60.0));
        assert!(is_default_delta(0.0166));
        assert!(!is_default_delta(1.0 / 120.0));
        assert!(!is_default_delta(0.02));
    }
}
