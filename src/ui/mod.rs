mod editor;
mod game_overlay;
mod render;

use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicIsize, Ordering},
    },
    thread,
};

use anyhow::{Context, Result, anyhow};
use windows::{
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ClientToScreen,
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
            Input::KeyboardAndMouse::{VK_ESCAPE, VK_F},
            WindowsAndMessaging::{
                CallNextHookEx, ClipCursor, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
                GetClientRect, GetClipCursor, GetCursor, GetCursorPos, GetForegroundWindow, GetMessageW,
                GetWindowThreadProcessId, HCURSOR, IDC_ARROW, KBDLLHOOKSTRUCT, LoadCursorW, PostMessageW,
                PostQuitMessage, RegisterClassW, SetCursor, SetCursorPos, SetWindowsHookExW, ShowCursor,
                TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_DESTROY, WM_KEYDOWN,
                WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
                WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_POPUP,
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
const WINDOW_CLASS: PCWSTR = w!("BattleInstinctUiController");

static UI_STATE: OnceLock<Mutex<UiState>> = OnceLock::new();
static CONTROLLER_HWND: AtomicIsize = AtomicIsize::new(0);
static OPEN_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) struct UiState {
    pub(crate) editor: Option<Editor>,
    pub(crate) frame_width: i32,
    pub(crate) frame_height: i32,
    pub(crate) ui_dirty: bool,
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
    UI_STATE
        .set(Mutex::new(UiState {
            editor: None,
            frame_width: 0,
            frame_height: 0,
            ui_dirty: false,
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

    // Must happen before MinHook::enable_all_hooks() in lib.rs. A graphics
    // hook failure disables only this optional UI, never the combat/gameplay mod.
    if let Err(error) = game_overlay::install() {
        log::error!("ui: in-game overlay unavailable: {error:#}");
        return Ok(());
    }

    thread::Builder::new()
        .name("battle-instinct-ui".into())
        .spawn(|| {
            if let Err(error) = run_ui_thread() {
                log::error!("ui: thread stopped: {error:#}");
            }
        })
        .context("spawn battle-instinct-ui thread")?;
    Ok(())
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        // The overlay hotkey is only valid while the game owns the foreground
        // window. A background F press must never read stale menu memory.
        let foreground = unsafe { GetForegroundWindow() };
        let mut foreground_pid = 0u32;
        unsafe {
            let _ = GetWindowThreadProcessId(foreground, Some(&mut foreground_pid));
        }
        if foreground_pid != unsafe { GetCurrentProcessId() } {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }

        let keyboard = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let vk = keyboard.vkCode;
        let overlay_open = lock_state().editor.is_some();
        let hwnd = HWND(CONTROLLER_HWND.load(Ordering::Acquire) as *mut core::ffi::c_void);

        if vk == VK_F.0 as u32 {
            if overlay_open {
                if !hwnd.0.is_null() {
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_APP_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
                return LRESULT(1);
            }
            if menu_probe::menu_open() && current_row_is_bind_target() {
                if !hwnd.0.is_null() && !OPEN_PENDING.swap(true, Ordering::AcqRel) {
                    let posted = unsafe { PostMessageW(hwnd, WM_APP_OPEN, WPARAM(0), LPARAM(0)) };
                    if posted.is_err() {
                        OPEN_PENDING.store(false, Ordering::Release);
                    }
                }
                return LRESULT(1);
            }
        } else if vk == VK_ESCAPE.0 as u32 && overlay_open {
            if !hwnd.0.is_null() {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_APP_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lock_state().editor.is_some() {
        // Never swallow clicks while another application owns the foreground.
        let foreground = unsafe { GetForegroundWindow() };
        let mut foreground_pid = 0u32;
        unsafe {
            let _ = GetWindowThreadProcessId(foreground, Some(&mut foreground_pid));
        }
        if foreground_pid != unsafe { GetCurrentProcessId() } {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }

        let message = wparam.0 as u32;
        match message {
            WM_LBUTTONDOWN => {
                let hwnd = HWND(CONTROLLER_HWND.load(Ordering::Acquire) as *mut core::ffi::c_void);
                if !hwnd.0.is_null() {
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_APP_CLICK, WPARAM(0), LPARAM(0));
                    }
                }
                return LRESULT(1);
            }
            // Never swallow WM_MOUSEMOVE. The overlay polls the OS cursor
            // position; blocking the move message can stop the OS cursor from
            // moving at all. The game's menu is keyboard-driven, so letting the
            // move message through while the editor is open is harmless.
            WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_XBUTTONDOWN
            | WM_XBUTTONUP | WM_MOUSEWHEEL => return LRESULT(1),
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
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

        // Hidden top-level tool window used only for the low-level hooks and
        // their message pump. It is never shown or activated.
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

        let keyboard =
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0).context("install keyboard hook")?;
        let mouse = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) {
            Ok(hook) => hook,
            Err(error) => {
                let _ = UnhookWindowsHookEx(keyboard);
                return Err(anyhow!("install mouse hook: {error}"));
            }
        };

        let mut msg = Default::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnhookWindowsHookEx(mouse);
        let _ = UnhookWindowsHookEx(keyboard);
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
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
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

    let mut state = lock_state();
    state.game_hwnd = game_hwnd.0 as isize;
    state.client_width = client_width;
    state.client_height = client_height;
    if state.frame_width <= 0 {
        state.frame_width = client_width;
    }
    if state.frame_height <= 0 {
        state.frame_height = client_height;
    }
    let (panel_w, panel_h, panel_x, panel_y) = render::panel_geometry(client_width, client_height);
    state.cursor_x = panel_x + panel_w as f32 * 0.5;
    state.cursor_y = panel_y + panel_h as f32 * 0.5;
    state.editor = Some(editor);
    state.ui_dirty = true;
    state.last_os_cursor = None;
    state.os_cursor_seen = false;
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
        if let Ok(cursor) = LoadCursorW(None, IDC_ARROW) {
            SetCursor(cursor);
        }
    }
}

fn sync_system_cursor(state: &mut UiState) {
    if state.editor.is_none() || state.game_hwnd == 0 {
        return;
    }
    let mut foreground_pid = 0u32;
    unsafe {
        let _ = GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground_pid));
    }
    if foreground_pid != unsafe { GetCurrentProcessId() } {
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
    if unsafe { GetCursorPos(&mut physical) }.is_ok() {
        if let Some(last) = state.last_os_cursor {
            let dx = physical.x - last.x;
            let dy = physical.y - last.y;
            if dx != 0 || dy != 0 {
                state.cursor_x = (state.cursor_x + dx as f32 * scale_x).clamp(0.0, (frame_width - 1) as f32);
                state.cursor_y = (state.cursor_y + dy as f32 * scale_y).clamp(0.0, (frame_height - 1) as f32);
                if !state.os_cursor_seen {
                    state.os_cursor_seen = true;
                    log::warn!("ui: OS cursor movement observed while key editor is open");
                }
            }
        }
    }

    ensure_cursor_visible(state);
    let mut point = POINT {
        x: (state.cursor_x * client_width as f32 / frame_width as f32).round() as i32,
        y: (state.cursor_y * client_height as f32 / frame_height as f32).round() as i32,
    };
    let game_hwnd = HWND(state.game_hwnd as *mut core::ffi::c_void);
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
        editor.request_close();
        editor.modal.is_none()
    } else {
        false
    };
    if close {
        close_overlay(state);
    }
}

fn close_overlay(state: &mut UiState) {
    state.editor = None;
    state.game_hwnd = 0;
    state.last_os_cursor = None;
    state.os_cursor_seen = false;
    restore_game_cursor(state);
}

fn handle_click_cursor() {
    let mut state = lock_state();
    if state.editor.is_none() || state.game_hwnd == 0 {
        return;
    }
    let (panel_w, panel_h, panel_x, panel_y) =
        render::panel_geometry(state.frame_width.max(1), state.frame_height.max(1));
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
    SaveKeys(Vec<KeyToken>, bool),
    ResetNow,
    Cancel,
}

fn click_action(editor: &mut Editor, w: i32, h: i32, x: i32, y: i32) -> ClickAction {
    let l = layout(editor, w, h);

    if let Some(panel) = l.modal_panel {
        if panel.contains(x, y) {
            if let Some(confirm) = l.modal_confirm {
                if confirm.contains(x, y) {
                    return match editor.modal_confirm() {
                        Some(ModalAction::Dismiss) | None => ClickAction::None,
                        Some(ModalAction::DiscardAndClose) => ClickAction::Close,
                        Some(ModalAction::ApplyMode(mode)) => {
                            editor.apply_mode(mode);
                            ClickAction::None
                        }
                        Some(ModalAction::OverwriteAndSave(keys)) => ClickAction::SaveKeys(keys, true),
                        Some(ModalAction::ResetAndClose) => ClickAction::ResetNow,
                    };
                }
            }
            if let Some(cancel) = l.modal_cancel {
                if cancel.contains(x, y) {
                    editor.modal_cancel();
                }
            }
        }
        return ClickAction::None;
    }
    if editor.modal.is_some() {
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
                editor.show_message("当前版本暂不支持该键位");
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
    state.ui_dirty = true;
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
        ClickAction::SaveKeys(keys, overwrite) => save_editor_keys(state, keys, overwrite),
        ClickAction::ResetNow => reset_editor_binding(state),
        ClickAction::Cancel => request_close_state(state),
    }
}

fn save_editor(state: &mut UiState) {
    let keys = {
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        if !editor.complete() {
            editor.show_message("请先完成键位配置");
            return;
        }

        let keys = editor.keys();
        match editor.snapshot.validate_candidate(editor.uid, &keys) {
            Ok(()) => keys,
            Err(BindingValidationError::Occupied) => {
                let conflict_name = editor
                    .snapshot
                    .conflicting_entry(editor.uid, &keys)
                    .map(|entry| {
                        editor
                            .snapshot
                            .name(entry.uid)
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("UID {}", entry.uid))
                    })
                    .unwrap_or_else(|| "其他技能/忍具".to_string());
                editor.show_overwrite_confirm(&conflict_name, keys);
                return;
            }
            Err(error) => {
                editor.show_message(&error.to_string());
                return;
            }
        }
    };

    save_editor_keys(state, keys, false);
}

fn save_editor_keys(state: &mut UiState, keys: Vec<KeyToken>, overwrite: bool) {
    let result = {
        let Some(editor) = state.editor.as_ref() else {
            return;
        };
        if overwrite {
            bindings::save_ui_binding_overwrite(&state.cfg_path, &editor.snapshot, editor.uid, keys)
        } else {
            bindings::save_ui_binding(&state.cfg_path, &editor.snapshot, editor.uid, keys)
        }
    };

    match result {
        Ok(()) => {
            let (uid, name, mode) = state
                .editor
                .as_ref()
                .map(|editor| (editor.uid, editor.name.clone(), editor.mode))
                .unwrap_or((0, String::new(), editor::BindingMode::Two));
            bindings::request_reload();
            log::warn!("ui: saved uid={uid} name={name} mode={mode:?} overwrite={overwrite}");
            close_overlay(state);
        }
        Err(error) => {
            if let Some(editor) = state.editor.as_mut() {
                editor.show_message(&format!("保存失败：{error:#}"));
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
                editor.show_message(&format!("重置失败：{error:#}"));
            }
        }
    }
}
