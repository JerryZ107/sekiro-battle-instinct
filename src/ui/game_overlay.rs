//! Lightweight in-game overlay hooks.
//!
//! Rendering is intentionally confined to the Present thread. A CPU worker
//! paints the editor into a DIB and publishes a generation; the Present detour
//! only uploads when that generation changes, then draws one textured quad and
//! a software cursor. No worker thread ever touches D3D11.

use std::{
    ffi::c_void,
    mem,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        LazyLock, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicUsize, Ordering},
    },
};

use anyhow::{Result, anyhow};
use windows::{
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM},
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
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Threading::GetCurrentProcessId,
        },
        UI::WindowsAndMessaging::{
            CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GWLP_WNDPROC, GetClassNameW,
            GetWindowThreadProcessId, IsWindowVisible, SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CHAR,
            WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
            WM_RBUTTONUP, WM_SETCURSOR, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDPROC,
        },
    },
    core::{HRESULT, Interface, s, w},
};

use super::{overlay_render::OverlayRenderer, paint_worker};
use crate::hook_util;

type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT;
type Present1Fn = unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void) -> HRESULT;
type ResizeBuffersFn = unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, u32) -> HRESULT;
type GetRawInputDataFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut u32, u32) -> u32;
type SetCursorPosFn = unsafe extern "system" fn(i32, i32) -> BOOL;
type ClipCursorFn = unsafe extern "system" fn(*const RECT) -> BOOL;

static PRESENT_ORIG: OnceLock<PresentFn> = OnceLock::new();
static PRESENT1_ORIG: OnceLock<Present1Fn> = OnceLock::new();
static RESIZE_ORIG: OnceLock<ResizeBuffersFn> = OnceLock::new();
static RAW_INPUT_ORIG: OnceLock<GetRawInputDataFn> = OnceLock::new();
static SET_CURSOR_ORIG: OnceLock<SetCursorPosFn> = OnceLock::new();
static CLIP_CURSOR_ORIG: OnceLock<ClipCursorFn> = OnceLock::new();

static RENDERER: LazyLock<Mutex<Option<OverlayRenderer>>> = LazyLock::new(|| Mutex::new(None));
static LAST_UPLOADED_GENERATION: AtomicU64 = AtomicU64::new(0);
static HAS_TEXTURE: AtomicBool = AtomicBool::new(false);
static RENDERER_READY: AtomicBool = AtomicBool::new(false);
/// Set after a failed renderer creation so Present does not retry D3DCompile
/// every frame. Reset on ResizeBuffers or when the editor is opened.
static RENDERER_FAILED: AtomicBool = AtomicBool::new(false);

pub(crate) fn reset_surface() {
    RENDERER_READY.store(false, Ordering::Release);
    RENDERER_FAILED.store(false, Ordering::Release);
    LAST_UPLOADED_GENERATION.store(0, Ordering::Release);
    HAS_TEXTURE.store(false, Ordering::Release);
    DRAW_PLAN.store(0, Ordering::Release);
}
static DRAW_PLAN: AtomicU64 = AtomicU64::new(0);
static RENDER_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);

static HOOKED_HWND: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_WNDPROC: AtomicIsize = AtomicIsize::new(0);

const PLAN_MASK: u64 = 0xffff;

fn pack_plan(x: f32, y: f32, w: f32, h: f32) -> u64 {
    let x = x.round().clamp(0.0, PLAN_MASK as f32) as u64;
    let y = y.round().clamp(0.0, PLAN_MASK as f32) as u64;
    let w = w.round().clamp(0.0, PLAN_MASK as f32) as u64;
    let h = h.round().clamp(0.0, PLAN_MASK as f32) as u64;
    x | (y << 16) | (w << 32) | (h << 48)
}

fn unpack_plan(value: u64) -> (f32, f32, f32, f32) {
    (
        (value & PLAN_MASK) as f32,
        ((value >> 16) & PLAN_MASK) as f32,
        ((value >> 32) & PLAN_MASK) as f32,
        ((value >> 48) & PLAN_MASK) as f32,
    )
}

/// Install all hooks. Called before `MinHook::enable_all_hooks()`.
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
    PRESENT_ORIG
        .set(unsafe { mem::transmute(trampoline) })
        .map_err(|_| anyhow!("DXGI Present hook was already installed"))?;

    let resize_addr = unsafe { *vtable.add(13) };
    if resize_addr != 0 {
        if let Ok(trampoline) = hook_util::create_hook_with_retry(
            resize_addr as *mut c_void,
            resize_buffers_detour as *mut c_void,
            "IDXGISwapChain::ResizeBuffers",
            false,
        ) {
            let _ = RESIZE_ORIG.set(unsafe { mem::transmute(trampoline) });
        }
    }

    if let Ok(swapchain1) = swapchain.cast::<IDXGISwapChain1>() {
        let raw1 = Interface::as_raw(&swapchain1);
        let vtable1 = unsafe { *(raw1 as *const *const usize) };
        let present1_addr = unsafe { *vtable1.add(22) };
        if present1_addr != 0 {
            if let Ok(trampoline) = hook_util::create_hook_with_retry(
                present1_addr as *mut c_void,
                present1_detour as *mut c_void,
                "IDXGISwapChain::Present1",
                false,
            ) {
                let _ = PRESENT1_ORIG.set(unsafe { mem::transmute(trampoline) });
            }
        }
    }

    install_user32_hooks();

    let hwnd = find_game_window();
    if !hwnd.0.is_null() {
        install_wndproc(hwnd);
    } else {
        log::warn!("ui: game window not found yet; WndProc will be installed on first Present");
    }

    drop(swapchain);
    log::warn!("ui: lightweight in-game overlay hooks created (Present/ResizeBuffers/RawInput)");
    Ok(())
}

fn install_user32_hooks() {
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else {
            log::error!("ui: user32.dll not found");
            return;
        };
        if let Some(proc) = GetProcAddress(user32, s!("GetRawInputData")) {
            let addr = proc as *const () as *mut c_void;
            if let Ok(trampoline) =
                hook_util::create_hook_with_retry(addr, get_raw_input_detour as *mut c_void, "GetRawInputData", false)
            {
                let _ = RAW_INPUT_ORIG.set(mem::transmute(trampoline));
            }
        }
        if let Some(proc) = GetProcAddress(user32, s!("SetCursorPos")) {
            let addr = proc as *const () as *mut c_void;
            if let Ok(trampoline) =
                hook_util::create_hook_with_retry(addr, set_cursor_pos_detour as *mut c_void, "SetCursorPos", false)
            {
                let _ = SET_CURSOR_ORIG.set(mem::transmute(trampoline));
            }
        }
        if let Some(proc) = GetProcAddress(user32, s!("ClipCursor")) {
            let addr = proc as *const () as *mut c_void;
            if let Ok(trampoline) =
                hook_util::create_hook_with_retry(addr, clip_cursor_detour as *mut c_void, "ClipCursor", false)
            {
                let _ = CLIP_CURSOR_ORIG.set(mem::transmute(trampoline));
            }
        }
    }
}

unsafe extern "system" fn get_raw_input_detour(
    raw_input: *mut c_void,
    command: u32,
    data: *mut c_void,
    size: *mut u32,
    header_size: u32,
) -> u32 {
    if super::overlay_open() {
        if !size.is_null() {
            unsafe { *size = 0 };
        }
        return 0;
    }
    match RAW_INPUT_ORIG.get() {
        Some(original) => unsafe { original(raw_input, command, data, size, header_size) },
        None => 0,
    }
}

unsafe extern "system" fn set_cursor_pos_detour(x: i32, y: i32) -> BOOL {
    if super::overlay_open() {
        return BOOL(1);
    }
    match SET_CURSOR_ORIG.get() {
        Some(original) => unsafe { original(x, y) },
        None => BOOL(0),
    }
}

unsafe extern "system" fn clip_cursor_detour(rect: *const RECT) -> BOOL {
    if super::overlay_open() {
        return match CLIP_CURSOR_ORIG.get() {
            Some(original) => unsafe { original(std::ptr::null()) },
            None => BOOL(1),
        };
    }
    match CLIP_CURSOR_ORIG.get() {
        Some(original) => unsafe { original(rect) },
        None => BOOL(0),
    }
}

unsafe extern "system" fn present_detour(swapchain: *mut c_void, sync: u32, flags: u32) -> HRESULT {
    unsafe { prepare_window_hook(swapchain) };
    if !RENDERER_READY.load(Ordering::Acquire) && !RENDERER_FAILED.load(Ordering::Acquire) {
        unsafe { ensure_renderer(swapchain) };
    }
    if super::overlay_open() {
        let result = catch_unwind(AssertUnwindSafe(|| unsafe { render_overlay(swapchain) }));
        if result.is_err() {
            log::error!("ui: panic while rendering lightweight overlay");
        }
    }
    match PRESENT_ORIG.get() {
        Some(original) => unsafe { original(swapchain, sync, flags) },
        None => HRESULT(0),
    }
}

unsafe extern "system" fn present1_detour(
    swapchain: *mut c_void,
    sync: u32,
    flags: u32,
    parameters: *const c_void,
) -> HRESULT {
    unsafe { prepare_window_hook(swapchain) };
    if !RENDERER_READY.load(Ordering::Acquire) && !RENDERER_FAILED.load(Ordering::Acquire) {
        unsafe { ensure_renderer(swapchain) };
    }
    if super::overlay_open() {
        let result = catch_unwind(AssertUnwindSafe(|| unsafe { render_overlay(swapchain) }));
        if result.is_err() {
            log::error!("ui: panic while rendering lightweight overlay");
        }
    }
    match PRESENT1_ORIG.get() {
        Some(original) => unsafe { original(swapchain, sync, flags, parameters) },
        None => HRESULT(0),
    }
}

unsafe extern "system" fn resize_buffers_detour(
    swapchain: *mut c_void,
    buffer_count: u32,
    width: u32,
    height: u32,
    format: u32,
    flags: u32,
) -> HRESULT {
    if let Ok(mut renderer) = RENDERER.lock() {
        *renderer = None;
    }
    RENDERER_READY.store(false, Ordering::Release);
    RENDERER_FAILED.store(false, Ordering::Release);
    LAST_UPLOADED_GENERATION.store(0, Ordering::Release);
    HAS_TEXTURE.store(false, Ordering::Release);
    DRAW_PLAN.store(0, Ordering::Release);
    RENDER_ERROR_LOGGED.store(false, Ordering::Release);
    match RESIZE_ORIG.get() {
        Some(original) => unsafe { original(swapchain, buffer_count, width, height, format, flags) },
        None => HRESULT(0),
    }
}

unsafe fn prepare_window_hook(swapchain: *mut c_void) {
    if HOOKED_HWND.load(Ordering::Acquire) != 0 {
        return;
    }
    let Some(swapchain) = (unsafe { IDXGISwapChain::from_raw_borrowed(&swapchain) }) else {
        return;
    };
    if let Ok(desc) = unsafe { swapchain.GetDesc() } {
        install_wndproc(desc.OutputWindow);
    }
}

unsafe fn ensure_renderer(swapchain: *mut c_void) {
    let Some(swapchain) = (unsafe { IDXGISwapChain::from_raw_borrowed(&swapchain) }) else {
        return;
    };
    let mut renderer = match RENDERER.lock() {
        Ok(renderer) => renderer,
        Err(poisoned) => poisoned.into_inner(),
    };
    if renderer.is_none() {
        match OverlayRenderer::new(&swapchain) {
            Ok(new_renderer) => {
                *renderer = Some(new_renderer);
                RENDERER_READY.store(true, Ordering::Release);
                RENDERER_FAILED.store(false, Ordering::Release);
                log::warn!("ui: lightweight overlay renderer preinitialized");
            }
            Err(error) => {
                RENDERER_FAILED.store(true, Ordering::Release);
                log_render_error_once(&format!("cannot initialize overlay renderer: {error:#}"));
            }
        }
    } else {
        RENDERER_READY.store(true, Ordering::Release);
        RENDERER_FAILED.store(false, Ordering::Release);
    }
}

/// Allow one retry after a previous failure; called when the user explicitly
/// opens the editor.
pub(crate) fn retry_renderer_failure() {
    RENDERER_FAILED.store(false, Ordering::Release);
}

pub(crate) fn release_cursor_clip() {
    if let Some(original) = CLIP_CURSOR_ORIG.get() {
        unsafe {
            let _ = original(std::ptr::null());
        }
    }
}

unsafe fn render_overlay(swapchain: *mut c_void) {
    let Some(swapchain) = (unsafe { IDXGISwapChain::from_raw_borrowed(&swapchain) }) else {
        return;
    };
    let mut renderer = match RENDERER.lock() {
        Ok(renderer) => renderer,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(active) = renderer.as_mut() else {
        return;
    };
    let (screen_w, screen_h) = active.screen_size();
    super::set_frame_size(screen_w as i32, screen_h as i32);

    let generation = paint_worker::latest_generation();
    if generation != LAST_UPLOADED_GENERATION.load(Ordering::Acquire) {
        if let Some(frame) = paint_worker::take_latest() {
            if let Err(error) = active.update_texture(&frame) {
                log_render_error_once(&format!("cannot upload overlay texture: {error:#}"));
                return;
            }
            DRAW_PLAN.store(
                pack_plan(frame.panel_x, frame.panel_y, frame.panel_w, frame.panel_h),
                Ordering::Release,
            );
            LAST_UPLOADED_GENERATION.store(frame.generation, Ordering::Release);
            HAS_TEXTURE.store(true, Ordering::Release);
        }
    }

    if !HAS_TEXTURE.load(Ordering::Acquire) {
        return;
    }
    let (x, y, w, h) = unpack_plan(DRAW_PLAN.load(Ordering::Acquire));
    let cursor = super::cursor_position();
    let draw_result = active.draw(&swapchain, x, y, w, h, screen_w, screen_h, cursor);
    if let Err(error) = draw_result {
        log_render_error_once(&format!("overlay draw failed: {error:#}"));
        HAS_TEXTURE.store(false, Ordering::Release);
        LAST_UPLOADED_GENERATION.store(0, Ordering::Release);
        paint_worker::request_repaint();
    } else {
        RENDER_ERROR_LOGGED.store(false, Ordering::Release);
    }
}

fn log_render_error_once(message: &str) {
    if !RENDER_ERROR_LOGGED.swap(true, Ordering::AcqRel) {
        log::error!("ui: {message}");
    }
}

unsafe extern "system" fn enum_windows(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid != unsafe { GetCurrentProcessId() } || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return BOOL(1);
    }
    let mut class = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    if len > 0 && String::from_utf16_lossy(&class[..len as usize]) == "CSclass" {
        unsafe { *(lparam.0 as *mut HWND) = hwnd };
        return BOOL(0);
    }
    BOOL(1)
}

fn find_game_window() -> HWND {
    let mut hwnd = HWND::default();
    let _ = unsafe { EnumWindows(Some(enum_windows), LPARAM(&mut hwnd as *mut HWND as isize)) };
    hwnd
}

fn install_wndproc(hwnd: HWND) {
    if hwnd.0.is_null() || HOOKED_HWND.load(Ordering::Acquire) == hwnd.0 as usize {
        return;
    }
    let old = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, game_wndproc as *const () as isize) };
    if old == 0 {
        log::error!("ui: SetWindowLongPtrW(GWLP_WNDPROC) failed");
        return;
    }
    ORIGINAL_WNDPROC.store(old, Ordering::Release);
    HOOKED_HWND.store(hwnd.0 as usize, Ordering::Release);
    log::warn!("ui: game WndProc hooked for overlay input");
}

unsafe extern "system" fn game_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let open = super::overlay_open();
    match msg {
        WM_KEYDOWN => {
            let vk = wparam.0 as u32;
            if super::on_key(hwnd, vk) {
                return LRESULT(0);
            }
        }
        WM_KEYUP | WM_CHAR | WM_SYSKEYDOWN | WM_SYSKEYUP | WM_MOUSEWHEEL if open => return LRESULT(0),
        WM_MOUSEMOVE if open => {
            let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
            super::on_mouse_move(hwnd, x, y);
            return LRESULT(0);
        }
        WM_LBUTTONDOWN if open => {
            let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
            super::on_mouse_move(hwnd, x, y);
            super::on_mouse_click(hwnd);
            return LRESULT(0);
        }
        WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP if open => return LRESULT(0),
        WM_SETCURSOR if open => return LRESULT(1),
        _ => {}
    }

    let old = ORIGINAL_WNDPROC.load(Ordering::Acquire);
    if old == 0 {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let proc: WNDPROC = unsafe { mem::transmute(old) };
    unsafe { CallWindowProcW(proc, hwnd, msg, wparam, lparam) }
}

unsafe fn create_probe_swapchain() -> Result<IDXGISwapChain> {
    let hwnd: HWND = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("PaperJutsuProbe"),
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
