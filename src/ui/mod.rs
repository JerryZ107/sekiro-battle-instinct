mod editor;
mod external;
mod game_overlay;
mod i18n;
mod paint_worker;
mod render;

use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
    },
    thread,
};

use anyhow::{Result, anyhow};
use windows::{
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ClientToScreen,
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_F, VK_LBUTTON},
            WindowsAndMessaging::{
                ClipCursor, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
                GetClipCursor, GetCursor, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
                HCURSOR, IDC_ARROW, KillTimer, LoadCursorW, MB_ICONWARNING, MB_OK, MessageBoxW, PostMessageW,
                PostQuitMessage, RegisterClassW, SetCursor, SetCursorPos, SetTimer, ShowCursor, TranslateMessage,
                WM_APP, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
            },
        },
    },
    core::{PCWSTR, w},
};

use crate::{
    binding_validation::BindingValidationError,
    bindings::{self, BindingSnapshot, KeyToken},
    game_menu, menu_probe,
};

use self::editor::{Editor, ModalAction, layout};

const WM_APP_OPEN: u32 = WM_APP + 1;
const WM_APP_CLOSE: u32 = WM_APP + 2;
const WM_APP_CLICK: u32 = WM_APP + 3;
const HOTKEY_TIMER_ID: usize = 1;
const HOTKEY_POLL_MS_ACTIVE: u32 = 10;
const HOTKEY_POLL_MS_IDLE: u32 = 50;
const WINDOW_CLASS: PCWSTR = w!("BattleInstinctUiController");

static UI_STATE: OnceLock<Mutex<UiState>> = OnceLock::new();
static CONTROLLER_HWND: AtomicIsize = AtomicIsize::new(0);
static HOTKEY_TIMER_MS: AtomicU32 = AtomicU32::new(0);
static OPEN_PENDING: AtomicBool = AtomicBool::new(false);
static F_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static ESC_KEY_DOWN: AtomicBool = AtomicBool::new(false);
static LEFT_BUTTON_DOWN: AtomicBool = AtomicBool::new(false);
/// Fast path for the Present/input hooks: no UI state lock is needed while the
/// key editor is closed.
static UI_OPEN: AtomicBool = AtomicBool::new(false);
/// Default: independent external window. Create `use_ingame_overlay.enable`
/// in the game directory to switch back to the DXGI in-game overlay.
static EXTERNAL_UI_MODE: AtomicBool = AtomicBool::new(true);
/// Set by close_overlay (either thread); the UI timer performs the actual
/// hook unmount. A Present-thread close must never patch the detour it is
/// currently executing inside.
static PENDING_UNMOUNT: AtomicBool = AtomicBool::new(false);
/// Rendering hook availability is independent from keyboard capture: gameplay
/// key tracking still needs the UI message pump if DXGI hooking fails.
static OVERLAY_AVAILABLE: AtomicBool = AtomicBool::new(false);
static HOOK_FAILURE_NOTIFIED: AtomicBool = AtomicBool::new(false);

/// Log the failure and show at most one modal warning per game process. Other
/// combat/gameplay hooks keep running; only the input/UI features are affected.
fn notify_hook_failure(detail: &str) {
    log::error!("ui: {detail}");
    if HOOK_FAILURE_NOTIFIED.swap(true, Ordering::AcqRel) {
        return;
    }
    let text = format!(
        "键位编辑或部分组合键功能不可用。\n\n{detail}\n\n其他战斗功能仍会继续运行，详情见 battle_instinct.log。"
    );
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Paper Jutsu".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR::from_raw(text.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            MB_OK | MB_ICONWARNING,
        );
    }
}

pub(crate) struct UiState {
    pub(crate) editor: Option<Editor>,
    pub(crate) frame_width: i32,
    pub(crate) frame_height: i32,
    cursor_x: f32,
    cursor_y: f32,
    cfg_path: PathBuf,
    game_hwnd: isize,
    client_width: i32,
    client_height: i32,
    saved_clip: Option<RECT>,
    saved_cursor: isize,
    cursor_show_calls: i32,
    last_os_cursor: Option<POINT>,
    os_cursor_seen: bool,
}

pub(crate) fn state() -> &'static Mutex<UiState> {
    UI_STATE.get().expect("UI state must be initialized before hooks")
}

#[inline]
pub(crate) fn overlay_open() -> bool {
    UI_OPEN.load(Ordering::Acquire)
}

fn game_is_foreground() -> bool {
    let mut foreground_pid = 0u32;
    unsafe {
        let _ = GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground_pid));
    }
    foreground_pid == unsafe { GetCurrentProcessId() }
}

fn key_is_down(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

/// Edge detector for polled hotkeys. The release half is mandatory: without
/// clearing the stored state a single press would permanently consume the
/// edge and the key could never trigger again.
fn key_edge(down: bool, was_down: &AtomicBool) -> bool {
    if down {
        !was_down.swap(true, Ordering::AcqRel)
    } else {
        was_down.store(false, Ordering::Release);
        false
    }
}

/// Poll overlay hotkeys and mouse clicks from a lightweight UI timer. Keeping
/// this out of `process_input` avoids adding work to the gameplay input hook.
fn retune_hotkey_timer() {
    let desired = if external::is_active() || (!overlay_open() && !menu_probe::menu_open_cached()) {
        HOTKEY_POLL_MS_IDLE
    } else {
        HOTKEY_POLL_MS_ACTIVE
    };
    if HOTKEY_TIMER_MS.swap(desired, Ordering::AcqRel) == desired {
        return;
    }
    let hwnd = HWND(CONTROLLER_HWND.load(Ordering::Acquire) as *mut core::ffi::c_void);
    if !hwnd.0.is_null() {
        unsafe {
            let _ = SetTimer(hwnd, HOTKEY_TIMER_ID, desired, None);
        }
    }
}

fn poll_async_hotkeys() {
    if UI_STATE.get().is_none() {
        return;
    }
    retune_hotkey_timer();
    if PENDING_UNMOUNT.swap(false, Ordering::AcqRel) {
        unmount_overlay_hooks();
    }
    let overlay_open = overlay_open();
    let menu_open = menu_probe::menu_open_cached();

    // Warm the D3D overlay resources while the pause menu is already up, so
    // the player's first F press does not pay shader/resource initialization.
    if !EXTERNAL_UI_MODE.load(Ordering::Acquire)
        && !overlay_open
        && menu_open
        && OVERLAY_AVAILABLE.load(Ordering::Acquire)
        && !game_overlay::prewarm_done()
        && game_overlay::request_prewarm()
    {
        log::warn!("ui: prewarming overlay resources while the pause menu is open");
        if !mount_overlay_hooks() {
            log::warn!("ui: overlay prewarm skipped because the Present hook could not mount");
            game_overlay::abandon_prewarm();
        }
    }
    if !overlay_open && game_overlay::prewarm_done() {
        unmount_overlay_hooks();
    }
    if !overlay_open && !menu_open && game_overlay::prewarm_requested() {
        game_overlay::cancel_prewarm();
        unmount_overlay_hooks();
    }

    if !EXTERNAL_UI_MODE.load(Ordering::Acquire) && overlay_open && !menu_open {
        // The Present detour no longer takes the UI-state lock; keep the
        // menu-closed transition on the UI timer instead. External windows are
        // independent and must stay open even if losing focus closes the game's
        // pause menu.
        let mut state = lock_state();
        close_overlay_for_gameplay(&mut state);
        return;
    }
    if EXTERNAL_UI_MODE.load(Ordering::Acquire) && external::is_active() {
        // The external window owns mouse/keyboard input; do not run the
        // in-game cursor/click polling against it.
        return;
    }
    // F/Esc only matter while the pause menu or the editor is visible. Do not
    // poll keyboard state at 100 Hz during normal gameplay.
    if !overlay_open && !menu_open {
        F_KEY_DOWN.store(false, Ordering::Release);
        ESC_KEY_DOWN.store(false, Ordering::Release);
        LEFT_BUTTON_DOWN.store(false, Ordering::Release);
        return;
    }

    let f_edge = key_edge(key_is_down(VK_F.0), &F_KEY_DOWN);
    let esc_edge = key_edge(key_is_down(VK_ESCAPE.0), &ESC_KEY_DOWN);
    let hwnd = HWND(CONTROLLER_HWND.load(Ordering::Acquire) as *mut core::ffi::c_void);

    // Mouse clicks must be polled on EVERY timer tick while the editor is
    // visible. The early return for "no F/Esc edge" belongs after this block;
    // otherwise clicks are only sampled on the exact frame F or Esc changes.
    if overlay_open {
        if game_is_foreground() {
            let click_edge = key_edge(key_is_down(VK_LBUTTON.0), &LEFT_BUTTON_DOWN);
            if click_edge && !hwnd.0.is_null() {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_APP_CLICK, WPARAM(0), LPARAM(0));
                }
            }
        } else {
            LEFT_BUTTON_DOWN.store(false, Ordering::Release);
        }
    } else {
        LEFT_BUTTON_DOWN.store(false, Ordering::Release);
    }

    if overlay_open {
        if game_is_foreground() {
            let mut state = lock_state();
            if let Some((width, height)) = paint_worker::frame_size() {
                state.frame_width = width.max(1);
                state.frame_height = height.max(1);
            }
            sync_overlay_cursor(&mut state);
        } else {
            LEFT_BUTTON_DOWN.store(false, Ordering::Release);
        }
    }

    if !f_edge && !esc_edge {
        return;
    }
    if !game_is_foreground() {
        return;
    }

    if f_edge {
        if overlay_open {
            if !hwnd.0.is_null() {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_APP_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            return;
        }
        if OVERLAY_AVAILABLE.load(Ordering::Acquire) && menu_probe::menu_open_cached() && current_row_is_bind_target() {
            if !hwnd.0.is_null() && !OPEN_PENDING.swap(true, Ordering::AcqRel) {
                let posted = unsafe { PostMessageW(hwnd, WM_APP_OPEN, WPARAM(0), LPARAM(0)) };
                if posted.is_err() {
                    OPEN_PENDING.store(false, Ordering::Release);
                }
            }
        }
    }
    if esc_edge && overlay_open && !hwnd.0.is_null() {
        unsafe {
            let _ = PostMessageW(hwnd, WM_APP_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

fn lock_state() -> std::sync::MutexGuard<'static, UiState> {
    match state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::error!("ui: state mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

pub fn start(cfg_path: PathBuf) -> Result<()> {
    i18n::set_lang(i18n::load_lang_from_cfg(&cfg_path));
    let external_ui = !cfg_path.with_file_name("use_ingame_overlay.enable").is_file();
    EXTERNAL_UI_MODE.store(external_ui, Ordering::Release);
    if external_ui {
        log::warn!("ui: external independent window mode enabled");
    } else {
        log::warn!("ui: DXGI in-game overlay mode enabled by use_ingame_overlay.enable");
    }
    if cfg_path.with_file_name("disable_context_state.enable").is_file() {
        render::set_context_state_fast_path_enabled(false);
        log::warn!("diagnostic: D3D11.1 context-state fast path disabled");
    }
    UI_STATE
        .set(Mutex::new(UiState {
            editor: None,
            frame_width: 0,
            frame_height: 0,
            cursor_x: 0.0,
            cursor_y: 0.0,
            cfg_path,
            game_hwnd: 0,
            client_width: 0,
            client_height: 0,
            saved_clip: None,
            saved_cursor: 0,
            cursor_show_calls: 0,
            last_os_cursor: None,
            os_cursor_seen: false,
        }))
        .map_err(|_| anyhow!("UI state was already initialized"))?;

    if !external_ui {
        if let Err(error) = paint_worker::start() {
            log::error!("ui: cannot start paint worker: {error:#}");
        }
    }

    // Must happen before MinHook::enable_all_hooks() in lib.rs. A graphics
    // hook failure disables only the optional overlay; the keyboard capture and
    // controller message pump must still start for gameplay key tracking.
    let overlay_available = if external_ui {
        // External window mode must not create the DXGI probe swap chain or
        // patch Present/Present1 at all.
        true
    } else {
        match game_overlay::install() {
            Ok(()) => true,
            Err(error) => {
                notify_hook_failure(&format!("DXGI overlay hook 安装失败: {error:#}"));
                false
            }
        }
    };
    OVERLAY_AVAILABLE.store(overlay_available, Ordering::Release);

    if let Err(error) = thread::Builder::new().name("battle-instinct-ui".into()).spawn(|| {
        if let Err(error) = run_ui_thread() {
            OVERLAY_AVAILABLE.store(false, Ordering::Release);
            notify_hook_failure(&format!("输入 Hook 线程停止: {error:#}"));
        }
    }) {
        notify_hook_failure(&format!("无法创建输入 Hook 线程: {error:#}"));
        return Err(error.into());
    }
    Ok(())
}

fn current_row_is_bind_target() -> bool {
    game_menu::current_selection().is_some_and(|selection| bindings::category_for_uid(selection.uid).is_some())
}

fn run_ui_thread() -> Result<()> {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(controller_wndproc),
            hInstance: instance.into(),
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(anyhow!("RegisterClassW failed"));
        }

        // Hidden top-level tool window used for the overlay hotkey timer and
        // the optional mouse hook. It is never shown or activated.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            WINDOW_CLASS,
            w!("Paper Jutsu UI Controller"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            instance,
            None,
        )?;
        if hwnd.0.is_null() {
            return Err(anyhow!("CreateWindowExW controller failed"));
        }
        CONTROLLER_HWND.store(hwnd.0 as isize, Ordering::Release);

        if SetTimer(hwnd, HOTKEY_TIMER_ID, HOTKEY_POLL_MS_IDLE, None) == 0 {
            return Err(anyhow!("SetTimer for overlay hotkeys failed"));
        }
        HOTKEY_TIMER_MS.store(HOTKEY_POLL_MS_IDLE, Ordering::Release);

        let mut msg = Default::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = KillTimer(hwnd, HOTKEY_TIMER_ID);
        CONTROLLER_HWND.store(0, Ordering::Release);
        let _ = DestroyWindow(hwnd);
        Ok(())
    }
}

unsafe extern "system" fn controller_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_OPEN => {
            OPEN_PENDING.store(false, Ordering::Release);
            if let Err(error) = open_overlay() {
                log::warn!("ui: cannot open key config: {error:#}");
            }
            LRESULT(0)
        }
        WM_APP_CLOSE => {
            request_close();
            LRESULT(0)
        }
        WM_APP_CLICK => {
            handle_click_cursor();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == HOTKEY_TIMER_ID => {
            poll_async_hotkeys();
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Re-apply the present hook mount policy after `MinHook::enable_all_hooks()`
/// re-enabled every created hook. Marker files:
/// - `disable_present_hook.enable` → permanent park (no overlay ever)
/// - `keep_present_hook.enable`    → permanent mount (pre-M1 behavior)
/// Neither marker present → dynamic mount tied to the editor lifecycle.
pub fn mount_present_hooks_after_startup(game_dir: Option<&std::path::Path>) -> Result<()> {
    let Some(dir) = game_dir else {
        return Err(anyhow!("cannot resolve the game directory for mount policy"));
    };
    if dir.join("disable_present_hook.enable").is_file() {
        game_overlay::set_mount_policy(game_overlay::MountPolicy::ForcedOff);
        log::warn!("diagnostic: present hook permanently parked (disable_present_hook.enable)");
    } else if dir.join("keep_present_hook.enable").is_file() {
        game_overlay::set_mount_policy(game_overlay::MountPolicy::ForcedOn);
        log::warn!("diagnostic: present hook permanently mounted (keep_present_hook.enable)");
    }
    unsafe { game_overlay::apply_mount_policy() };
    Ok(())
}

/// Enable the present detours just before the overlay becomes visible.
fn mount_overlay_hooks() -> bool {
    if !game_overlay::can_mount() {
        return false;
    }
    match unsafe { game_overlay::set_hooks_enabled(true) } {
        Ok(()) => true,
        Err(error) => {
            log::error!("ui: cannot mount present hooks: {error:#}");
            false
        }
    }
}

/// Unmount the detours once the overlay is gone (UI thread only).
fn unmount_overlay_hooks() {
    if game_overlay::mount_policy() != game_overlay::MountPolicy::Dynamic {
        return;
    }
    if let Err(error) = unsafe { game_overlay::set_hooks_enabled(false) } {
        log::error!("ui: cannot unmount present hooks: {error:#}");
    }
}

fn open_overlay() -> Result<()> {
    let cfg_path = {
        let state = lock_state();
        if state.editor.is_some() {
            return Ok(());
        }
        state.cfg_path.clone()
    };

    let Some(editor) = open_editor(&cfg_path)? else {
        log::debug!("ui: F ignored; current menu row is not a keybind target");
        return Ok(());
    };

    let game_hwnd = unsafe { GetForegroundWindow() };
    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(game_hwnd, &mut client) };
    let client_width = (client.right - client.left).max(1);
    let client_height = (client.bottom - client.top).max(1);

    let uid = editor.uid;
    let name = editor.name.clone();
    let menu_uid = editor.menu_uid;
    let menu_category = editor.menu_category;

    game_overlay::cancel_prewarm();
    let external_ui = EXTERNAL_UI_MODE.load(Ordering::Acquire);
    let (panel_w, panel_h, panel_x, panel_y) = render::panel_geometry(&editor, client_width, client_height);

    let mut state = lock_state();
    state.game_hwnd = game_hwnd.0 as isize;
    state.client_width = client_width;
    state.client_height = client_height;
    state.frame_width = client_width;
    state.frame_height = client_height;
    state.cursor_x = panel_x + panel_w as f32 * 0.5;
    state.cursor_y = panel_y + panel_h as f32 * 0.5;
    state.editor = Some(editor);
    state.last_os_cursor = None;
    state.os_cursor_seen = false;

    if external_ui {
        UI_OPEN.store(true, Ordering::Release);
        crate::frame_perf::mark(crate::frame_perf::EventId::UiOpen);
        drop(state);
        if let Err(error) = external::show(panel_w, panel_h, game_hwnd) {
            UI_OPEN.store(false, Ordering::Release);
            let mut state = lock_state();
            state.editor = None;
            state.game_hwnd = 0;
            return Err(error);
        }
        log::warn!(
            "ui: opened external window uid={} name={} from game menu uid={} category={}",
            uid,
            name,
            menu_uid,
            menu_category
        );
        return Ok(());
    }

    if game_overlay::mount_policy() != game_overlay::MountPolicy::ForcedOff && !mount_overlay_hooks() {
        log::warn!("ui: overlay may not appear because hook mount failed");
    }
    // Existing renderer resources: enable protection before the worker can
    // observe UI_OPEN and start an upload. If resources do not exist yet, the
    // first Present frame enables protection when it creates them.
    game_overlay::set_overlay_multithread_protected(true);
    UI_OPEN.store(true, Ordering::Release);
    crate::frame_perf::mark(crate::frame_perf::EventId::UiOpen);
    paint_worker::begin_session();
    show_game_cursor(&mut state);

    log::warn!(
        "ui: opened uid={} name={} from game menu uid={} category={}",
        uid,
        name,
        menu_uid,
        menu_category
    );
    Ok(())
}

fn open_editor(cfg_path: &std::path::Path) -> Result<Option<Editor>> {
    let snapshot = BindingSnapshot::load(cfg_path)?;
    let Some(selection) = game_menu::current_selection() else {
        return Ok(None);
    };
    if bindings::category_for_uid(selection.uid).is_none() {
        return Ok(None);
    }
    let uid = selection.uid;
    let Some(mut editor) = Editor::new(uid, snapshot) else {
        return Ok(None);
    };
    editor.menu_uid = selection.uid;
    editor.menu_category = selection.category;
    Ok(Some(editor))
}

fn show_game_cursor(state: &mut UiState) {
    unsafe {
        let mut clip = RECT::default();
        if GetClipCursor(&mut clip).is_ok() {
            state.saved_clip = Some(clip);
        }
        state.saved_cursor = GetCursor().0 as isize;
        if let Ok(cursor) = LoadCursorW(None, IDC_ARROW) {
            SetCursor(cursor);
        }
        sync_system_cursor(state);
    }
}

fn ensure_cursor_visible(state: &mut UiState) {
    unsafe {
        // Many games hide the OS cursor every frame. Show it just enough to
        // reach a non-negative display count without accumulating indefinitely.
        loop {
            let count = ShowCursor(BOOL(1));
            if count < 0 {
                state.cursor_show_calls += 1;
                continue;
            }
            if count > 0 {
                ShowCursor(BOOL(0));
            }
            break;
        }
    }
}

fn sync_system_cursor(state: &mut UiState) {
    if state.editor.is_none() || state.game_hwnd == 0 {
        return;
    }
    let game_hwnd = HWND(state.game_hwnd as *mut core::ffi::c_void);
    if unsafe { GetForegroundWindow() } != game_hwnd {
        return;
    }

    let client_width = state.client_width.max(1);
    let client_height = state.client_height.max(1);
    let frame_width = state.frame_width.max(1);
    let frame_height = state.frame_height.max(1);
    let scale_x = frame_width as f32 / client_width as f32;
    let scale_y = frame_height as f32 / client_height as f32;

    // Do not read the game's DirectInput camera stream here. Sekiro's pause
    // menu is not guaranteed to poll the DirectInput mouse, and the game may
    // keep the device unacquired while the menu is open. Poll the OS cursor
    // instead: that stream exists regardless of what the game is polling.
    let mut physical = POINT::default();
    let mut moved = false;
    let mut dx = 0;
    let mut dy = 0;
    if unsafe { GetCursorPos(&mut physical) }.is_ok() {
        if let Some(last) = state.last_os_cursor {
            dx = physical.x - last.x;
            dy = physical.y - last.y;
            moved = dx != 0 || dy != 0;
        } else {
            // First sample after opening the editor: establish the anchor and
            // place the cursor at the initial UI position.
            moved = true;
        }
    }
    if moved && state.last_os_cursor.is_some() {
        state.cursor_x = (state.cursor_x + dx as f32 * scale_x).clamp(0.0, (frame_width - 1) as f32);
        state.cursor_y = (state.cursor_y + dy as f32 * scale_y).clamp(0.0, (frame_height - 1) as f32);
        if !state.os_cursor_seen {
            state.os_cursor_seen = true;
            log::warn!("ui: OS cursor movement observed while key editor is open");
        }
    }

    // The game hides the OS cursor every frame in some states. Keep the arrow
    // alive, but do not warp the OS cursor unless the physical mouse actually
    // moved: repeated SetCursorPos calls inject mouse input every frame and
    // make the game/camera feel like it is stuttering.
    ensure_cursor_visible(state);
    if !moved {
        return;
    }

    let mut point = POINT {
        x: (state.cursor_x * client_width as f32 / frame_width as f32).round() as i32,
        y: (state.cursor_y * client_height as f32 / frame_height as f32).round() as i32,
    };
    unsafe {
        let _ = ClipCursor(None);
        if ClientToScreen(game_hwnd, &mut point).as_bool() {
            let _ = SetCursorPos(point.x, point.y);
        }
        // SetCursorPos may be DPI-virtualized. Store the real post-warp
        // position so the next frame compares against the same coordinate
        // space that GetCursorPos reports.
        let mut actual = POINT::default();
        state.last_os_cursor = GetCursorPos(&mut actual).ok().map(|_| actual);
    }
}

pub(crate) fn sync_overlay_cursor(state: &mut UiState) {
    sync_system_cursor(state);
}

pub(crate) fn close_overlay_for_gameplay(state: &mut UiState) {
    close_overlay(state);
}

fn restore_game_cursor(state: &mut UiState) {
    unsafe {
        for _ in 0..state.cursor_show_calls {
            ShowCursor(BOOL(0));
        }
        state.cursor_show_calls = 0;
        if state.saved_cursor != 0 {
            SetCursor(HCURSOR(state.saved_cursor as *mut core::ffi::c_void));
        }
        if let Some(clip) = state.saved_clip.take() {
            let _ = ClipCursor(Some(&clip));
        } else {
            let _ = ClipCursor(None);
        }
    }
}

fn request_close() {
    let mut state = lock_state();
    request_close_state(&mut state);
}

fn request_close_state(state: &mut UiState) {
    let close = if let Some(editor) = state.editor.as_mut() {
        if editor.modal.is_some() {
            // Esc / F while a dialog is open only dismisses that dialog.
            editor.modal_cancel();
            false
        } else {
            editor.request_close();
            editor.modal.is_none()
        }
    } else {
        false
    };
    if close {
        close_overlay(state);
    }
}

fn close_overlay(state: &mut UiState) {
    UI_OPEN.store(false, Ordering::Release);
    LEFT_BUTTON_DOWN.store(false, Ordering::Release);
    crate::frame_perf::mark(crate::frame_perf::EventId::UiClose);

    if EXTERNAL_UI_MODE.load(Ordering::Acquire) {
        external::hide();
        state.editor = None;
        state.game_hwnd = 0;
        state.last_os_cursor = None;
        state.os_cursor_seen = false;
        return;
    }

    PENDING_UNMOUNT.store(true, Ordering::Release);
    paint_worker::end_session();
    game_overlay::set_overlay_multithread_protected(false);
    state.editor = None;
    state.game_hwnd = 0;
    state.last_os_cursor = None;
    state.os_cursor_seen = false;
    restore_game_cursor(state);
}

pub(crate) fn handle_external_click(x: i32, y: i32, w: i32, h: i32) {
    let mut state = lock_state();
    if state.editor.is_none() {
        return;
    }
    handle_click(&mut state, x, y, w, h);
}

fn handle_click_cursor() {
    let mut state = lock_state();
    if state.editor.is_none() || state.game_hwnd == 0 {
        return;
    }
    let Some(editor) = state.editor.as_ref() else {
        return;
    };
    let (panel_w, panel_h, panel_x, panel_y) =
        render::panel_geometry(editor, state.frame_width.max(1), state.frame_height.max(1));
    let local_x = state.cursor_x.round() as i32 - panel_x as i32;
    let local_y = state.cursor_y.round() as i32 - panel_y as i32;
    if local_x < 0 || local_y < 0 || local_x >= panel_w || local_y >= panel_h {
        return;
    }
    handle_click(&mut state, local_x, local_y, panel_w, panel_h);
}

#[derive(Debug)]
enum ClickAction {
    None,
    Close,
    Save,
    SaveSchemes(Vec<Vec<KeyToken>>, bool),
    ResetNow,
    Cancel,
}

fn click_action(editor: &mut Editor, w: i32, h: i32, x: i32, y: i32) -> ClickAction {
    let l = layout(editor, w, h);

    if let Some(panel) = l.modal_panel {
        if panel.contains(x, y) {
            for (index, button) in l.modal_buttons.iter().copied().enumerate() {
                if button.contains(x, y) {
                    return match editor.take_modal_action(index) {
                        Some(ModalAction::Dismiss) | None => ClickAction::None,
                        Some(ModalAction::SaveAndClose) => ClickAction::Save,
                        Some(ModalAction::DiscardAndClose) => ClickAction::Close,
                        Some(ModalAction::ApplyMode(mode)) => {
                            editor.apply_mode(mode);
                            ClickAction::None
                        }
                        Some(ModalAction::OverwriteAndSave(schemes)) => ClickAction::SaveSchemes(schemes, true),
                        Some(ModalAction::ResetAndClose) => ClickAction::ResetNow,
                        Some(ModalAction::DeleteSelectedScheme) => {
                            editor.delete_selected_scheme();
                            ClickAction::None
                        }
                    };
                }
            }
        }
        return ClickAction::None;
    }
    if editor.modal.is_some() {
        return ClickAction::None;
    }

    for (index, rect) in l.scheme_buttons.iter().copied().enumerate() {
        if rect.contains(x, y) {
            editor.select_scheme(index);
            return ClickAction::None;
        }
    }
    if l.add_scheme.contains(x, y) {
        editor.add_scheme();
        return ClickAction::None;
    }
    if l.delete_scheme.contains(x, y) {
        editor.request_delete_scheme();
        return ClickAction::None;
    }

    if editor.mode_is_switchable() && l.mode_left.contains(x, y) {
        editor.mode_prev();
        return ClickAction::None;
    }
    if editor.mode_is_switchable() && l.mode_right.contains(x, y) {
        editor.mode_next();
        return ClickAction::None;
    }
    for (index, slot) in l.slots.iter().enumerate() {
        if slot.contains(x, y) {
            editor.click_slot(index);
            return ClickAction::None;
        }
    }
    for item in &l.grid {
        if item.rect.contains(x, y) {
            if item.disabled {
                editor.show_message(i18n::pick(
                    "当前版本暂不支持该键位",
                    "This key is not supported in this version.",
                ));
            } else if let Some(token) = item.token {
                editor.click_key(token);
            }
            return ClickAction::None;
        }
    }
    if l.save.contains(x, y) {
        ClickAction::Save
    } else if l.reset.contains(x, y) {
        editor.request_reset();
        ClickAction::None
    } else if l.cancel.contains(x, y) {
        ClickAction::Cancel
    } else {
        ClickAction::None
    }
}

fn handle_click(state: &mut UiState, x: i32, y: i32, w: i32, h: i32) {
    if EXTERNAL_UI_MODE.load(Ordering::Acquire) {
        external::invalidate();
    } else {
        paint_worker::request_repaint();
    }
    let action = {
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        click_action(editor, w, h, x, y)
    };

    match action {
        ClickAction::None => {}
        ClickAction::Close => close_overlay(state),
        ClickAction::Save => save_editor(state),
        ClickAction::SaveSchemes(schemes, overwrite) => save_editor_schemes(state, schemes, overwrite),
        ClickAction::ResetNow => reset_editor_binding(state),
        ClickAction::Cancel => request_close_state(state),
    }
}

fn save_editor(state: &mut UiState) {
    let schemes = {
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        if !editor.complete() {
            editor.show_message(i18n::pick(
                "请先完成所有方案的键位配置",
                "Complete every scheme before saving.",
            ));
            return;
        }

        let schemes = editor.all_keys();
        match editor.snapshot.validate_scheme_set(editor.uid, &schemes) {
            Ok(()) => schemes,
            Err(BindingValidationError::Occupied) => {
                let conflicts = editor.snapshot.conflicting_entries(editor.uid, &schemes);
                let mut names = Vec::new();
                for entry in conflicts {
                    let name = editor
                        .snapshot
                        .name(entry.uid)
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| format!("UID {}", entry.uid));
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                let conflict_names = if names.is_empty() {
                    i18n::pick("其他技能/忍具", "another combat art / prosthetic").to_string()
                } else {
                    names.join("、")
                };
                editor.show_overwrite_confirm(&conflict_names, schemes);
                return;
            }
            Err(error) => {
                editor.show_message(&i18n::validation_error(error));
                return;
            }
        }
    };

    save_editor_schemes(state, schemes, false);
}

fn save_editor_schemes(state: &mut UiState, schemes: Vec<Vec<KeyToken>>, overwrite: bool) {
    let result = {
        let Some(editor) = state.editor.as_ref() else {
            return;
        };
        if overwrite {
            bindings::save_ui_schemes_overwrite(&state.cfg_path, &editor.snapshot, editor.uid, schemes)
        } else {
            bindings::save_ui_schemes(&state.cfg_path, &editor.snapshot, editor.uid, schemes)
        }
    };

    match result {
        Ok(()) => {
            let (uid, name, mode, scheme_count) = state
                .editor
                .as_ref()
                .map(|editor| (editor.uid, editor.name.clone(), editor.mode(), editor.schemes.len()))
                .unwrap_or((0, String::new(), editor::BindingMode::Two, 0));
            bindings::request_reload();
            log::warn!("ui: saved uid={uid} name={name} mode={mode:?} schemes={scheme_count} overwrite={overwrite}");
            close_overlay(state);
        }
        Err(error) => {
            if let Some(editor) = state.editor.as_mut() {
                editor.show_message(&format!("{}: {error:#}", i18n::pick("保存失败", "Save failed"),));
            }
        }
    }
}

fn reset_editor_binding(state: &mut UiState) {
    let (uid, name) = match state.editor.as_ref() {
        Some(editor) => (editor.uid, editor.name.clone()),
        None => return,
    };
    match bindings::reset_ui_binding(&state.cfg_path, uid) {
        Ok(()) => {
            bindings::request_reload();
            log::warn!("ui: reset uid={uid} name={name}");
            close_overlay(state);
        }
        Err(error) => {
            if let Some(editor) = state.editor.as_mut() {
                editor.show_message(&format!("{}: {error:#}", i18n::pick("重置失败", "Reset failed"),));
            }
        }
    }
}

#[cfg(test)]
mod hotkey_tests {
    use super::*;

    #[test]
    fn polled_hotkey_resets_on_release() {
        let state = AtomicBool::new(false);
        assert!(key_edge(true, &state));
        assert!(!key_edge(true, &state));
        assert!(!key_edge(false, &state));
        assert!(key_edge(true, &state));
        assert!(!key_edge(false, &state));
        assert!(key_edge(true, &state));
    }
}
