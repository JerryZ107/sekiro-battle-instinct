use std::{
    ffi::c_void,
    mem,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
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

type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT;
type Present1Fn = unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void) -> HRESULT;

static PRESENT_ORIG: OnceLock<PresentFn> = OnceLock::new();
static PRESENT1_ORIG: OnceLock<Present1Fn> = OnceLock::new();
static RENDERER: OnceLock<Mutex<Option<Renderer>>> = OnceLock::new();
static RENDER_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);

/// Install the DXGI Present detour. This must be called before
/// `MinHook::enable_all_hooks()` so the game's first presented frame is covered.
pub fn install() -> Result<()> {
    let swapchain = unsafe { create_probe_swapchain() }?;
    let raw = Interface::as_raw(&swapchain);
    let vtable = unsafe { *(raw as *const *const usize) };
    let present_addr = unsafe { *vtable.add(8) };
    if present_addr == 0 {
        return Err(anyhow!("DXGI Present vtable entry is null"));
    }

    let trampoline = unsafe { MinHook::create_hook(present_addr as *mut c_void, present_detour as *mut c_void) }
        .map_err(|status| anyhow!("MinHook create_hook(IDXGISwapChain::Present) failed: {status:?}"))?;
    let original: PresentFn = unsafe { mem::transmute(trampoline) };
    PRESENT_ORIG
        .set(original)
        .map_err(|_| anyhow!("DXGI Present hook was already installed"))?;

    // A few DXGI 1.2+ games use Present1 instead of Present. Hook it too when
    // the probe swap chain exposes the derived interface.
    if let Ok(swapchain1) = swapchain.cast::<IDXGISwapChain1>() {
        let raw1 = Interface::as_raw(&swapchain1);
        let vtable1 = unsafe { *(raw1 as *const *const usize) };
        let present1_addr = unsafe { *vtable1.add(22) };
        if present1_addr != 0 {
            match unsafe { MinHook::create_hook(present1_addr as *mut c_void, present1_detour as *mut c_void) } {
                Ok(trampoline) => {
                    let original1: Present1Fn = unsafe { mem::transmute(trampoline) };
                    if PRESENT1_ORIG.set(original1).is_ok() {
                        log::warn!("ui: DXGI Present1 hook installed for in-game overlay");
                    }
                }
                Err(status) => log::warn!("ui: Present1 hook skipped: {status:?}"),
            }
        }
    }

    drop(swapchain);
    log::warn!("ui: DXGI Present hook installed for in-game overlay");
    Ok(())
}

unsafe extern "system" fn present_detour(swapchain: *mut c_void, sync_interval: u32, flags: u32) -> HRESULT {
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
    render_before_present(swapchain);

    match PRESENT1_ORIG.get() {
        Some(original) => unsafe { original(swapchain, sync_interval, flags, present_parameters) },
        None => HRESULT(0),
    }
}

fn render_before_present(swapchain: *mut c_void) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        unsafe { render_present(swapchain) };
    }));
    if result.is_err() {
        log::error!("ui: panic while rendering in-game overlay");
    }
}

unsafe fn render_present(swapchain: *mut c_void) {
    let Some(swapchain) = (unsafe { IDXGISwapChain::from_raw_borrowed(&swapchain) }) else {
        return;
    };
    let desc = match unsafe { swapchain.GetDesc() } {
        Ok(desc) => desc,
        Err(error) => {
            log_render_error_once(&format!("IDXGISwapChain::GetDesc failed: {error}"));
            return;
        }
    };

    let renderer = RENDERER.get_or_init(|| Mutex::new(None));
    let mut renderer = match renderer.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::error!("ui: renderer mutex poisoned; recovering");
            poisoned.into_inner()
        }
    };

    let state = super::state();
    let mut state = match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::error!("ui: UI state mutex poisoned; recovering");
            poisoned.into_inner()
        }
    };

    state.frame_width = desc.BufferDesc.Width.max(1) as i32;
    state.frame_height = desc.BufferDesc.Height.max(1) as i32;

    if state.editor.is_none() {
        return;
    }
    if !crate::menu_probe::menu_open() {
        super::close_overlay_for_gameplay(&mut state);
        return;
    }
    super::sync_overlay_cursor(&mut state);
    let Some(editor) = state.editor.as_ref() else {
        return;
    };

    if renderer.is_none() {
        match Renderer::new() {
            Ok(new_renderer) => {
                *renderer = Some(new_renderer);
                log::warn!("ui: D3D11 software-texture renderer initialized");
            }
            Err(error) => {
                log_render_error_once(&format!("cannot initialize overlay renderer: {error:#}"));
                return;
            }
        }
    }

    let Some(active_renderer) = renderer.as_mut() else {
        return;
    };
    let redraw_ui = state.ui_dirty;
    let result = active_renderer.paint(
        swapchain,
        editor,
        desc.BufferDesc.Width,
        desc.BufferDesc.Height,
        redraw_ui,
    );
    match result {
        Ok(()) => {
            state.ui_dirty = false;
            RENDER_ERROR_LOGGED.store(false, Ordering::Release);
        }
        Err(error) => log_render_error_once(&format!("cannot draw in-game overlay: {error:#}")),
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
