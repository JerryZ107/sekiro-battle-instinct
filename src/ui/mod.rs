mod editor;
mod render;

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicIsize, Ordering},
    thread,
};

use anyhow::{Context, Result, anyhow};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{ClientToScreen, InvalidateRect, UpdateWindow},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetThreadDpiAwarenessContext},
            Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_F},
            WindowsAndMessaging::{
                CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetClientRect,
                GetForegroundWindow, GetMessageW, GetWindowLongPtrW, HWND_TOPMOST, IsWindowVisible, KBDLLHOOKSTRUCT,
                MB_ICONWARNING, MB_OK, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, SW_HIDE, SW_SHOW,
                SWP_SHOWWINDOW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow,
                TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
                WM_LBUTTONDOWN, WM_PAINT, WM_SYSKEYDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
    core::HSTRING,
};

use crate::{
    bindings::{self, BindingSnapshot},
    game_menu, menu_probe,
};

use self::editor::{Editor, ModalAction, layout};

const WM_APP_OPEN: u32 = WM_APP + 1;
const WM_APP_CLOSE: u32 = WM_APP + 2;
const WINDOW_CLASS: windows::core::PCWSTR = windows::core::w!("BattleInstinctKeyConfigOverlay");

static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);
static OPEN_PENDING: AtomicBool = AtomicBool::new(false);

struct Overlay {
    cfg_path: PathBuf,
    game_hwnd: HWND,
    editor: Option<Editor>,
}

pub fn start(cfg_path: PathBuf) {
    let _ = thread::Builder::new().name("battle-instinct-ui".into()).spawn(move || {
        if let Err(error) = run_ui_thread(cfg_path) {
            log::error!("ui: thread stopped: {error:#}");
        }
    });
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let keyboard = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if keyboard.vkCode == VK_F.0 as u32 {
            let hwnd = HWND(OVERLAY_HWND.load(Ordering::Acquire) as *mut core::ffi::c_void);
            if !hwnd.0.is_null() {
                if unsafe { IsWindowVisible(hwnd).as_bool() } {
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_APP_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    return LRESULT(1);
                }
                if menu_probe::menu_open() {
                    if OPEN_PENDING.swap(true, Ordering::AcqRel) {
                        return LRESULT(1);
                    }
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_APP_OPEN, WPARAM(0), LPARAM(0));
                    }
                    return LRESULT(1);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn run_ui_thread(cfg_path: PathBuf) -> Result<()> {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            style: windows::Win32::UI::WindowsAndMessaging::CS_HREDRAW
                | windows::Win32::UI::WindowsAndMessaging::CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(anyhow!("RegisterClassW failed"));
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            WINDOW_CLASS,
            windows::core::w!("Battle Instinct Key Config"),
            WS_POPUP,
            0,
            0,
            760,
            760,
            None,
            None,
            HINSTANCE(instance.0),
            None,
        )?;
        if hwnd.0.is_null() {
            return Err(anyhow!("CreateWindowExW failed"));
        }
        let overlay = Box::new(Overlay {
            cfg_path,
            game_hwnd: HWND::default(),
            editor: None,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(overlay) as isize);
        OVERLAY_HWND.store(hwnd.0 as isize, Ordering::Release);
        let hook =
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), None, 0).context("install keyboard event hook")?;

        let mut msg = Default::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
        OVERLAY_HWND.store(0, Ordering::Release);
        Ok(())
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            if let Some(overlay) = overlay_mut(hwnd) {
                if let Some(editor) = &overlay.editor {
                    render::paint(hwnd, editor);
                    return LRESULT(0);
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_APP_OPEN => {
            OPEN_PENDING.store(false, Ordering::Release);
            open_overlay(hwnd);
            LRESULT(0)
        }
        WM_APP_CLOSE => {
            request_close(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            handle_click(hwnd, x, y);
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = wparam.0 as u32;
            if vk == VK_F.0 as u32 || vk == VK_ESCAPE.0 as u32 {
                request_close(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn overlay_mut(hwnd: HWND) -> Option<&'static mut Overlay> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Overlay;
    unsafe { ptr.as_mut() }
}

fn open_overlay(hwnd: HWND) {
    let Some(overlay) = overlay_mut(hwnd) else {
        return;
    };
    if overlay.editor.is_some() {
        return;
    }
    let game_hwnd = unsafe { GetForegroundWindow() };
    match open_editor(&overlay.cfg_path) {
        Ok(editor) => {
            log::warn!(
                "ui: opened uid={} name={} from game menu uid={} category={}",
                editor.uid,
                editor.name,
                editor.menu_uid,
                editor.menu_category
            );
            overlay.game_hwnd = game_hwnd;
            overlay.editor = Some(editor);
            position_overlay(hwnd, game_hwnd);
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                        | SWP_SHOWWINDOW,
                );
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(hwnd);
                let _ = UpdateWindow(hwnd);
                let _ = InvalidateRect(hwnd, None, true);
            }
        }
        Err(error) => {
            log::warn!("ui: cannot open key config: {error:#}");
            unsafe {
                let text = HSTRING::from(format!(
                    "未能从游戏菜单读取当前技能/忍具。\n请重新选择条目后按 F 重试。\n\n{error:#}"
                ));
                let caption = HSTRING::from("只狼 · 键位配置");
                MessageBoxW(None, &text, &caption, MB_OK | MB_ICONWARNING);
            }
        }
    }
}

fn open_editor(cfg_path: &std::path::Path) -> Result<Editor> {
    let snapshot = BindingSnapshot::load(cfg_path)?;
    let selection = game_menu::current_selection()
        .ok_or_else(|| anyhow!("尚未从当前菜单分类表定位到选中条目，请重新高亮条目后按 F"))?;
    // The menu resolved an exact SkillParam `acquireWeaponId` anchor. It does
    // not need to already exist in cfg: the editor can create a UI override for
    // any valid combat-art/prosthetic anchor, and the existing combo reload path
    // will consume it unchanged.
    let uid = selection.uid;
    let mut editor = Editor::new(uid, snapshot)
        .ok_or_else(|| anyhow!("invalid anchor UID {uid} category={}", selection.category))?;
    editor.menu_uid = selection.uid;
    editor.menu_category = selection.category;
    Ok(editor)
}

fn position_overlay(hwnd: HWND, game_hwnd: HWND) {
    unsafe {
        let mut client = windows::Win32::Foundation::RECT::default();
        if GetClientRect(game_hwnd, &mut client).is_err() {
            return;
        }
        let mut origin = windows::Win32::Foundation::POINT::default();
        let _ = ClientToScreen(game_hwnd, &mut origin);
        let cw = client.right - client.left;
        let ch = client.bottom - client.top;
        let scale = (GetDpiForSystem() as f32 / 96.0).max(1.0);
        let visible_w = cw as f32 / scale;
        let visible_h = ch as f32 / scale;
        let w = ((visible_w * 0.43).round() as i32).max(560);
        let h = ((visible_h * 0.74).round() as i32).max(620);
        let x = origin.x + (visible_w - w as f32 - visible_w * 0.015).round() as i32;
        let y = origin.y + (visible_h * 0.145).round() as i32;
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
        log::debug!(
            "ui: position client={}x{} origin={},{} desired={},{} {}x{} scale={}",
            cw,
            ch,
            origin.x,
            origin.y,
            x,
            y,
            w,
            h,
            scale
        );
    }
}

fn request_close(hwnd: HWND) {
    let Some(overlay) = overlay_mut(hwnd) else {
        return;
    };
    let Some(editor) = overlay.editor.as_mut() else {
        return;
    };
    editor.request_close();
    if editor.modal.is_some() {
        invalidate(hwnd);
    } else {
        close_overlay(hwnd);
    }
}

fn close_overlay(hwnd: HWND) {
    let Some(overlay) = overlay_mut(hwnd) else {
        return;
    };
    let game_hwnd = overlay.game_hwnd;
    overlay.editor = None;
    // The next F press re-reads the highlighted row from the live menu page.
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        if !game_hwnd.0.is_null() {
            let _ = SetForegroundWindow(game_hwnd);
        }
    }
}

fn handle_click(hwnd: HWND, x: i32, y: i32) {
    let Some(overlay) = overlay_mut(hwnd) else {
        return;
    };
    let Some(editor) = overlay.editor.as_mut() else {
        return;
    };
    let mut client = windows::Win32::Foundation::RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let l = layout(editor, client.right, client.bottom);

    if let Some(panel) = l.modal_panel {
        if panel.contains(x, y) {
            if let Some(confirm) = l.modal_confirm {
                if confirm.contains(x, y) {
                    match editor.modal_confirm() {
                        Some(ModalAction::Dismiss) => {}
                        Some(ModalAction::DiscardAndClose) => {
                            close_overlay(hwnd);
                            return;
                        }
                        Some(ModalAction::ApplyMode(mode)) => editor.apply_mode(mode),
                        None => {}
                    }
                    invalidate(hwnd);
                    return;
                }
            }
            if let Some(cancel) = l.modal_cancel {
                if cancel.contains(x, y) {
                    editor.modal_cancel();
                    invalidate(hwnd);
                    return;
                }
            }
            return;
        }
    }
    if editor.modal.is_some() {
        return;
    }

    if editor.mode_is_switchable() && l.mode_left.contains(x, y) {
        editor.mode_prev();
        invalidate(hwnd);
        return;
    }
    if editor.mode_is_switchable() && l.mode_right.contains(x, y) {
        editor.mode_next();
        invalidate(hwnd);
        return;
    }
    for (index, slot) in l.slots.iter().enumerate() {
        if slot.contains(x, y) {
            editor.click_slot(index);
            invalidate(hwnd);
            return;
        }
    }
    for item in &l.grid {
        if item.rect.contains(x, y) {
            if item.disabled {
                editor.show_message("当前版本暂不支持该键位");
            } else if let Some(token) = item.token {
                editor.click_key(token);
            }
            invalidate(hwnd);
            return;
        }
    }
    if l.save.contains(x, y) {
        save_editor(hwnd);
    } else if l.cancel.contains(x, y) {
        request_close(hwnd);
    }
}

fn save_editor(hwnd: HWND) {
    let Some(overlay) = overlay_mut(hwnd) else {
        return;
    };
    let Some(editor) = overlay.editor.as_mut() else {
        return;
    };
    let keys = match editor.validate_for_save() {
        Ok(keys) => keys,
        Err(message) => {
            editor.show_message(&message);
            invalidate(hwnd);
            return;
        }
    };
    match bindings::save_ui_binding(&overlay.cfg_path, &editor.snapshot, editor.uid, keys) {
        Ok(()) => {
            bindings::request_reload();
            log::warn!(
                "ui: saved uid={} name={} mode={:?}",
                editor.uid,
                editor.name,
                editor.mode
            );
            close_overlay(hwnd);
        }
        Err(error) => {
            editor.show_message(&format!("保存失败：{error:#}"));
            invalidate(hwnd);
        }
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(hwnd, None, true);
        let _ = UpdateWindow(hwnd);
    }
}
