use std::{
    ffi::c_void,
    mem,
    sync::{
        LazyLock, Mutex, OnceLock, TryLockError,
        atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};

use anyhow::{Result, anyhow};
use minhook::MinHook;
use windows::{
    Win32::{
        Foundation::{BOOL, HWND},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDeviceAndSwapChain, ID3D11Device,
                ID3D11DeviceContext,
            },
            Dxgi::{
                Common::DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD,
                DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGISwapChain, IDXGISwapChain1,
            },
        },
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE},
    },
    core::{HRESULT, Interface, w},
};

use super::render::Renderer;
use crate::hook_util;

type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT;
type Present1Fn = unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void) -> HRESULT;

static PRESENT_ORIG: OnceLock<PresentFn> = OnceLock::new();
static PRESENT1_ORIG: OnceLock<Present1Fn> = OnceLock::new();
static RENDERER: LazyLock<Mutex<Option<Renderer>>> = LazyLock::new(|| Mutex::new(None));
static RENDER_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);
static SLOW_OVERLAY_DRAWS: AtomicU64 = AtomicU64::new(0);
const OVERLAY_DRAW_BUDGET_US: u128 = 200;

/// Raw vtable targets of the two present slots. Stored at install time so the
/// UI thread can enable/disable each detour without re-deriving the vtable.
static PRESENT_TARGET: AtomicUsize = AtomicUsize::new(0);
static PRESENT1_TARGET: AtomicUsize = AtomicUsize::new(0);
/// True while at least one present detour is patched into the game's call
/// path. Owned by the UI thread; read-only elsewhere.
static HOOKS_ENABLED: AtomicBool = AtomicBool::new(false);
/// Set when install() succeeded and create_hook landed, so the hook exists but
/// is parked (disabled). A failed create disables dynamic mount entirely.
static HOOKS_PARKED: AtomicBool = AtomicBool::new(false);
/// Raw pointer of the swap chain whose device currently owns `RENDERER`.
/// Present compares this cheap atomic before taking the renderer mutex.
static RENDERER_SWAPCHAIN: AtomicUsize = AtomicUsize::new(0);
/// One-shot resource prewarm while the pause menu is already open. This keeps
/// the first editor open from paying shader/resource initialization on a frame
/// the player can see.
static PREWARM_REQUESTED: AtomicBool = AtomicBool::new(false);
static PREWARM_DONE: AtomicBool = AtomicBool::new(false);

/// How the present hooks are mounted for the whole session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MountPolicy {
    /// Hooks are parked; the UI thread enables them while the editor is open.
    Dynamic,
    /// Diagnostics: hooks stay enabled permanently (old behavior).
    ForcedOn,
    /// Diagnostics: hooks stay disabled permanently; the overlay cannot mount.
    ForcedOff,
}

const POLICY_DYNAMIC: u8 = 0;
const POLICY_FORCED_ON: u8 = 1;
const POLICY_FORCED_OFF: u8 = 2;
static MOUNT_POLICY: AtomicU8 = AtomicU8::new(POLICY_DYNAMIC);

pub fn set_mount_policy(policy: MountPolicy) {
    let value = match policy {
        MountPolicy::Dynamic => POLICY_DYNAMIC,
        MountPolicy::ForcedOn => POLICY_FORCED_ON,
        MountPolicy::ForcedOff => POLICY_FORCED_OFF,
    };
    MOUNT_POLICY.store(value, Ordering::Release);
}

pub fn mount_policy() -> MountPolicy {
    match MOUNT_POLICY.load(Ordering::Acquire) {
        POLICY_FORCED_ON => MountPolicy::ForcedOn,
        POLICY_FORCED_OFF => MountPolicy::ForcedOff,
        _ => MountPolicy::Dynamic,
    }
}

/// Whether an overlay can ever appear (ForcedOff permanently parks the hook).
pub fn can_mount() -> bool {
    HOOKS_PARKED.load(Ordering::Acquire) && mount_policy() != MountPolicy::ForcedOff
}

/// Force the two detours to `enabled` regardless of the cached state flag.
/// Used once at startup: `enable_all_hooks()` re-armed the parked hooks behind
/// our back, and the in-memory flag alone would then skip the re-park.
///
/// # Safety
/// Patches live process code; UI/init thread only.
pub unsafe fn force_set_hooks_enabled(enabled: bool) {
    if !HOOKS_PARKED.load(Ordering::Acquire) {
        return;
    }
    for (target, created) in [
        (PRESENT_TARGET.load(Ordering::Acquire), PRESENT_ORIG.get().is_some()),
        (PRESENT1_TARGET.load(Ordering::Acquire), PRESENT1_ORIG.get().is_some()),
    ] {
        if target == 0 || !created {
            continue;
        }
        let result = unsafe {
            if enabled {
                MinHook::enable_hook(target as *mut c_void)
            } else {
                MinHook::disable_hook(target as *mut c_void)
            }
        };
        match result {
            Ok(()) => {}
            // Idempotent states are expected at startup.
            Err(minhook::MH_STATUS::MH_ERROR_ENABLED) if enabled => {}
            Err(minhook::MH_STATUS::MH_ERROR_DISABLED) => {}
            Err(error) => {
                let op = if enabled { "enable" } else { "disable" };
                log::error!("ui: present hook startup mount ({op}) failed: {error}");
            }
        }
    }
    HOOKS_ENABLED.store(enabled, Ordering::Release);
    crate::frame_perf::set_present_stream_active(enabled);
}

/// Apply the current policy right after startup `enable_all_hooks()`, which
/// would otherwise leave both detours permanently mounted.
///
/// # Safety
/// Patches live process code; call from the init worker thread only, before
/// any gameplay state depends on the hooks.
pub unsafe fn apply_mount_policy() {
    let desired = mount_policy() == MountPolicy::ForcedOn;
    unsafe { force_set_hooks_enabled(desired) };
    crate::frame_perf::mark(if desired {
        crate::frame_perf::EventId::PresentHookEnable
    } else {
        crate::frame_perf::EventId::PresentHookDisable
    });
}

/// Install the DXGI Present detour. The hooks are created **disabled** here:
/// during normal gameplay the game's Present call path stays 100% untouched
/// (no detour jump at all), which is what protects frame pacing in exclusive
/// fullscreen. The UI thread enables them only while the key editor is open.
/// This must run before `MinHook::enable_all_hooks()` so trampoline patching
/// happens together with the rest of the startup hooks.
pub fn install() -> Result<()> {
    let swapchain = unsafe { create_probe_swapchain() }?;
    let raw = Interface::as_raw(&swapchain);
    let vtable = unsafe { *(raw as *const *const usize) };
    let present_addr = unsafe { *vtable.add(8) };
    if present_addr == 0 {
        return Err(anyhow!("DXGI Present vtable entry is null"));
    }

    let trampoline = hook_util::create_hook_with_retry(
        present_addr as *mut c_void,
        present_detour as *mut c_void,
        "IDXGISwapChain::Present",
        true,
    )?;
    let original: PresentFn = unsafe { mem::transmute(trampoline) };
    PRESENT_ORIG
        .set(original)
        .map_err(|_| anyhow!("DXGI Present hook was already installed"))?;
    PRESENT_TARGET.store(present_addr, Ordering::Release);
    // A freshly created hook is already disabled; MH_ERROR_DISABLED is the
    // idempotent no-op case, anything else leaves the hook permanently mounted.
    match unsafe { MinHook::disable_hook(present_addr as *mut c_void) } {
        Ok(()) | Err(minhook::MH_STATUS::MH_ERROR_DISABLED) => {}
        Err(error) => return Err(anyhow!("parking IDXGISwapChain::Present hook failed: {error}")),
    }

    // A few DXGI 1.2+ games use Present1 instead of Present. Hook it too when
    // the probe swap chain exposes the derived interface.
    if let Ok(swapchain1) = swapchain.cast::<IDXGISwapChain1>() {
        let raw1 = Interface::as_raw(&swapchain1);
        let vtable1 = unsafe { *(raw1 as *const *const usize) };
        let present1_addr = unsafe { *vtable1.add(22) };
        if present1_addr != 0 {
            match hook_util::create_hook_with_retry(
                present1_addr as *mut c_void,
                present1_detour as *mut c_void,
                "IDXGISwapChain::Present1",
                false,
            ) {
                Ok(trampoline) => {
                    let original1: Present1Fn = unsafe { mem::transmute(trampoline) };
                    if PRESENT1_ORIG.set(original1).is_ok() {
                        PRESENT1_TARGET.store(present1_addr, Ordering::Release);
                        let _ = unsafe { MinHook::disable_hook(present1_addr as *mut c_void) };
                        log::warn!("ui: DXGI Present1 hook created (parked) for in-game overlay");
                    }
                }
                Err(status) => log::warn!("ui: Present1 hook skipped: {status:?}"),
            }
        }
    }

    drop(swapchain);
    HOOKS_PARKED.store(true, Ordering::Release);
    log::warn!("ui: DXGI Present hook installed and parked; mounted only while the editor is open");
    Ok(())
}

/// Enable both present detours. Called by the UI thread just before the
/// overlay becomes visible (WM_APP_OPEN), never from a game thread: patching
/// executable pages from the Present thread would serialize a Present and
/// create a visible hitch.
///
/// # Safety
/// Patches live process code. Single-caller contract: UI thread only.
pub unsafe fn set_hooks_enabled(enabled: bool) -> Result<()> {
    if !HOOKS_PARKED.load(Ordering::Acquire) {
        return Err(anyhow!("present hooks were never created"));
    }
    if enabled == HOOKS_ENABLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut first_error = None;
    for (target, created) in [
        (PRESENT_TARGET.load(Ordering::Acquire), PRESENT_ORIG.get().is_some()),
        (PRESENT1_TARGET.load(Ordering::Acquire), PRESENT1_ORIG.get().is_some()),
    ] {
        if target == 0 || !created {
            continue;
        }
        let result = unsafe {
            if enabled {
                MinHook::enable_hook(target as *mut c_void)
            } else {
                MinHook::disable_hook(target as *mut c_void)
            }
        };
        if let Err(error) = result {
            let op = if enabled { "enable" } else { "disable" };
            log::error!("ui: present hook {op} failed: {error}");
            if first_error.is_none() {
                first_error = Some(error.to_string());
            }
        }
    }
    // Record the desired state even on partial failure so repeat calls do not
    // thrash; a failed enable leaves the editor overlay invisible but never
    // corrupts the game's present path.
    HOOKS_ENABLED.store(enabled, Ordering::Release);
    crate::frame_perf::set_present_stream_active(enabled);
    crate::frame_perf::mark(if enabled {
        crate::frame_perf::EventId::PresentHookEnable
    } else {
        crate::frame_perf::EventId::PresentHookDisable
    });
    match first_error {
        Some(error) => Err(anyhow!("present hook toggle failed: {error}")),
        None => Ok(()),
    }
}

unsafe extern "system" fn present_detour(swapchain: *mut c_void, sync_interval: u32, flags: u32) -> HRESULT {
    crate::frame_perf::before_present();
    render_before_present(swapchain);

    match PRESENT_ORIG.get() {
        Some(original) => unsafe { original(swapchain, sync_interval, flags) },
        None => HRESULT(0),
    }
}

unsafe extern "system" fn present1_detour(
    swapchain: *mut c_void,
    sync_interval: u32,
    flags: u32,
    present_parameters: *const c_void,
) -> HRESULT {
    crate::frame_perf::before_present();
    render_before_present(swapchain);

    match PRESENT1_ORIG.get() {
        Some(original) => unsafe { original(swapchain, sync_interval, flags, present_parameters) },
        None => HRESULT(0),
    }
}

fn render_before_present(swapchain: *mut c_void) {
    // Only reached while the detour is enabled, i.e. the editor was opened.
    // The closed state never enters this function at all.
    if !super::overlay_open() && !PREWARM_REQUESTED.load(Ordering::Acquire) {
        return;
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        unsafe { render_present(swapchain) };
    }));
    if result.is_err() {
        log::error!("ui: panic while rendering in-game overlay");
    }
}

/// Request one prewarm pass while the pause menu is open. Returns true only
/// for the first request; the UI timer mounts the parked Present hook so the
/// next Present can create the D3D resources invisibly behind the menu.
pub(crate) fn request_prewarm() -> bool {
    if PREWARM_DONE.load(Ordering::Acquire) || PREWARM_REQUESTED.swap(true, Ordering::AcqRel) {
        return false;
    }
    true
}

pub(crate) fn prewarm_requested() -> bool {
    PREWARM_REQUESTED.load(Ordering::Acquire)
}

pub(crate) fn prewarm_done() -> bool {
    PREWARM_DONE.load(Ordering::Acquire)
}

pub(crate) fn cancel_prewarm() {
    PREWARM_REQUESTED.store(false, Ordering::Release);
}

pub(crate) fn mark_prewarm_done() {
    let was_requested = PREWARM_REQUESTED.swap(false, Ordering::AcqRel);
    if was_requested && !PREWARM_DONE.swap(true, Ordering::AcqRel) {
        log::warn!("ui: overlay resource prewarm complete");
    }
    PREWARM_DONE.store(true, Ordering::Release);
}

pub(crate) fn abandon_prewarm() {
    PREWARM_REQUESTED.store(false, Ordering::Release);
    PREWARM_DONE.store(true, Ordering::Release);
}

/// Toggle D3D multithread protection for the editor session. Called from the
/// UI thread on open/close; resource creation also applies it when the first
/// Present frame occurs after opening.
pub(crate) fn set_overlay_multithread_protected(enabled: bool) {
    let mut renderer = match RENDERER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(renderer) = renderer.as_mut() {
        renderer.set_multithread_protected(enabled);
    }
}

/// Upload a finished CPU DIB to the dynamic overlay texture. Called from the
/// paint worker; this is the only path that touches the immediate context from
/// a non-Present thread, and `ID3D11Multithread` serializes it internally.
pub(crate) fn upload_overlay_texture(dib: &super::render::DibSurface, size: (u32, u32)) -> Result<()> {
    let mut renderer = match RENDERER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::error!("ui: renderer mutex poisoned in paint worker; recovering");
            poisoned.into_inner()
        }
    };
    // Re-check inside the renderer lock: close_overlay sets UI_OPEN=false
    // before disabling multithread protection, so this ordering guarantees the
    // worker can never upload after protection has been turned off.
    if !super::overlay_open() {
        return Err(anyhow!("overlay closed before texture upload"));
    }
    renderer
        .as_mut()
        .ok_or_else(|| anyhow!("D3D11 overlay resources are not initialized yet"))?
        .update_texture(dib, size)
}

fn ensure_renderer_for_swapchain(swapchain: &IDXGISwapChain) -> bool {
    let raw = Interface::as_raw(swapchain) as usize;
    if RENDERER_SWAPCHAIN.load(Ordering::Acquire) == raw {
        return true;
    }

    let mut renderer = match RENDERER.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => {
            log::error!("ui: renderer mutex poisoned; recovering");
            poisoned.into_inner()
        }
        // The paint worker is currently uploading. Retry on the next frame;
        // never block the game's Present path.
        Err(TryLockError::WouldBlock) => return false,
    };

    if renderer.is_none() {
        match Renderer::new() {
            Ok(new_renderer) => *renderer = Some(new_renderer),
            Err(error) => {
                log_render_error_once(&format!("cannot initialize overlay renderer: {error:#}"));
                return false;
            }
        }
    }

    let Some(active_renderer) = renderer.as_mut() else {
        return false;
    };
    if let Err(error) = active_renderer.ensure_resources(swapchain) {
        log_render_error_once(&format!("cannot initialize overlay D3D resources: {error:#}"));
        return false;
    }

    if super::overlay_open() {
        active_renderer.set_multithread_protected(true);
    }
    RENDERER_SWAPCHAIN.store(raw, Ordering::Release);
    // A new swap chain means the old texture cannot be drawn. Invalidate it
    // and let the worker upload into the new device resources.
    super::paint_worker::reset_draw_ready();
    super::paint_worker::request_repaint();
    true
}

unsafe fn render_present(swapchain: *mut c_void) {
    let Some(swapchain) = (unsafe { IDXGISwapChain::from_raw_borrowed(&swapchain) }) else {
        return;
    };

    // The first Present after opening (or after a swap-chain recreation)
    // binds the renderer to the game device. This is a one-off path; once the
    // raw pointer is cached, it collapses to one atomic load.
    if !ensure_renderer_for_swapchain(&swapchain) {
        return;
    }
    mark_prewarm_done();

    if !super::overlay_open() {
        // Prewarm-only frame: resources now exist and the UI timer will
        // unmount the diagnostic Present hook. No texture or draw is needed.
        return;
    }

    // No uploaded texture yet: keep the original Present path completely
    // free of COM queries, locks, cursor work and GDI.
    let Some(plan) = super::paint_worker::draw_plan() else {
        return;
    };

    let desc = match unsafe { swapchain.GetDesc() } {
        Ok(desc) => desc,
        Err(error) => {
            log_render_error_once(&format!("IDXGISwapChain::GetDesc failed: {error}"));
            return;
        }
    };
    let frame_w = desc.BufferDesc.Width.max(1);
    let frame_h = desc.BufferDesc.Height.max(1);
    super::paint_worker::note_frame_size(frame_w, frame_h);

    let mut renderer = match RENDERER.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => {
            log::error!("ui: renderer mutex poisoned; recovering");
            poisoned.into_inner()
        }
        // Paint worker is uploading. Skip one overlay frame instead of
        // stalling the game's Present call.
        Err(TryLockError::WouldBlock) => return,
    };
    let Some(active_renderer) = renderer.as_mut() else {
        return;
    };

    let draw_started = Instant::now();
    let result = active_renderer.draw(
        &swapchain,
        plan.x,
        plan.y,
        plan.w,
        plan.h,
        frame_w as f32,
        frame_h as f32,
    );
    let draw_us = draw_started.elapsed().as_micros();
    crate::frame_perf::record_overlay_draw(draw_us as u64);
    if draw_us >= OVERLAY_DRAW_BUDGET_US {
        let count = SLOW_OVERLAY_DRAWS.fetch_add(1, Ordering::Relaxed);
        if count % 120 == 0 {
            log::warn!(
                "ui perf: overlay_draw_us={draw_us} budget_us={OVERLAY_DRAW_BUDGET_US} panel={}x{}",
                plan.w,
                plan.h
            );
        }
    }
    match result {
        Ok(()) => RENDER_ERROR_LOGGED.store(false, Ordering::Release),
        Err(error) => {
            log_render_error_once(&format!("cannot draw in-game overlay: {error:#}"));
            super::paint_worker::reset_draw_ready();
            super::paint_worker::request_repaint();
        }
    }
}

fn log_render_error_once(message: &str) {
    if !RENDER_ERROR_LOGGED.swap(true, Ordering::AcqRel) {
        log::error!("ui: {message}");
    }
}

/// Create a tiny hidden swap chain only to obtain the real `Present` vtable
/// address. This is the standard way to install a DXGI overlay hook without
/// depending on a particular D3D device creation path in the game.
unsafe fn create_probe_swapchain() -> Result<IDXGISwapChain> {
    let hwnd: HWND = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("BattleInstinctProbe"),
            WINDOW_STYLE(0),
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
    }
    .map_err(|error| anyhow!("CreateWindowExW probe failed: {error}"))?;

    let desc = DXGI_SWAP_CHAIN_DESC {
        BufferDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_DESC {
            Width: 1,
            Height: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            ..Default::default()
        },
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 1,
        OutputWindow: hwnd,
        Windowed: BOOL(1),
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        Flags: 0,
    };

    let mut swapchain: Option<IDXGISwapChain> = None;
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    let creation = unsafe {
        D3D11CreateDeviceAndSwapChain(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&desc),
            Some(&mut swapchain),
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };
    let _ = unsafe { DestroyWindow(hwnd) };
    creation.map_err(|error| anyhow!("D3D11CreateDeviceAndSwapChain probe failed: {error}"))?;
    drop(context);
    drop(device);
    swapchain.ok_or_else(|| anyhow!("D3D11CreateDeviceAndSwapChain returned no swap chain"))
}
