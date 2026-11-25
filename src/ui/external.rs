//! External, independent key-config window.
//!
//! This mode deliberately does not touch the game's DXGI swap chain or the
//! immediate D3D context. The editor is a normal top-level Win32 window painted
//! from the same GDI panel renderer used by the in-game overlay, so gameplay
//! frame pacing is completely independent of editor UI work.

use std::{cell::RefCell, sync::Once};

use anyhow::{Result, anyhow};
use windows::{
    Win32::{
        Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT, UpdateWindow},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_F},
            WindowsAndMessaging::{
                AdjustWindowRect, CreateWindowExW, DefWindowProcW, GetSystemMetrics, GetWindowRect, HMENU, HTCLIENT,
                HWND_TOPMOST, IDC_ARROW, IsIconic, IsWindowVisible, LoadCursorW, RegisterClassW, SM_CXSCREEN,
                SM_CYSCREEN, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_SHOWWINDOW, SetCursor, SetForegroundWindow,
                SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_ERASEBKGND, WM_KEYDOWN,
                WM_LBUTTONDOWN, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOPMOST,
                WS_SYSMENU,
            },
        },
    },
    core::{PCWSTR, w},
};

use super::render::UiPainter;

const EXTERNAL_CLASS: PCWSTR = w!("BattleInstinctExternalUi");
const WINDOW_STYLE_VALUE: WINDOW_STYLE = WINDOW_STYLE(WS_CAPTION.0 | WS_SYSMENU.0);
const WINDOW_EX_STYLE_VALUE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(WS_EX_TOPMOST.0 | WS_EX_APPWINDOW.0);
const WINDOW_TITLE: PCWSTR = w!("Paper Jutsu - Key Config");

thread_local! {
    static WINDOW: RefCell<Option<ExternalWindow>> = const { RefCell::new(None) };
}

struct ExternalWindow {
    hwnd: HWND,
    game_hwnd: HWND,
    painter: UiPainter,
    width: i32,
    height: i32,
    visible: bool,
}

unsafe impl Send for ExternalWindow {}

impl ExternalWindow {
    fn create(width: i32, height: i32, game_hwnd: HWND) -> Result<Self> {
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(external_wndproc),
                hInstance: HINSTANCE(instance.0),
                lpszClassName: EXTERNAL_CLASS,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);
        });

        let instance = unsafe { GetModuleHandleW(None)? };
        let (outer_w, outer_h) = outer_size(width, height)?;
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE_VALUE,
                EXTERNAL_CLASS,
                WINDOW_TITLE,
                WINDOW_STYLE_VALUE,
                0,
                0,
                outer_w,
                outer_h,
                None,
                HMENU::default(),
                instance,
                None,
            )
        }
        .map_err(|error| anyhow!("CreateWindowExW external UI failed: {error}"))?;

        Ok(Self {
            hwnd,
            game_hwnd,
            painter: UiPainter::new(),
            width,
            height,
            visible: false,
        })
    }
}

fn outer_size(width: i32, height: i32) -> Result<(i32, i32)> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width.max(1),
        bottom: height.max(1),
    };
    unsafe { AdjustWindowRect(&mut rect, WINDOW_STYLE_VALUE, BOOL(0)) }
        .map_err(|error| anyhow!("AdjustWindowRect external UI failed: {error}"))?;
    Ok(((rect.right - rect.left).max(1), (rect.bottom - rect.top).max(1)))
}

fn centered_origin(game_hwnd: HWND, width: i32, height: i32) -> (i32, i32) {
    let mut game_rect = RECT::default();
    let (left, top, right, bottom) = unsafe {
        if !game_hwnd.0.is_null() && GetWindowRect(game_hwnd, &mut game_rect).is_ok() {
            (game_rect.left, game_rect.top, game_rect.right, game_rect.bottom)
        } else {
            (0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
        }
    };
    (
        left + ((right - left) - width).max(0) / 2,
        top + ((bottom - top) - height).max(0) / 2,
    )
}

pub(crate) fn show(width: i32, height: i32, game_hwnd: HWND) -> Result<()> {
    let width = width.max(320);
    let height = height.max(320);

    // Create/update the window without holding the thread-local borrow across
    // SetWindowPos/ShowWindow, because those calls can synchronously dispatch
    // WM_PAINT back into this module.
    let hwnd = WINDOW.with(|slot| -> Result<HWND> {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(ExternalWindow::create(width, height, game_hwnd)?);
        }
        let window = slot.as_mut().ok_or_else(|| anyhow!("external UI state disappeared"))?;
        window.width = width;
        window.height = height;
        window.game_hwnd = game_hwnd;
        Ok(window.hwnd)
    })?;

    // Repaint the CPU panel before making the window visible.
    {
        let state = super::lock_state();
        let editor = state
            .editor
            .as_ref()
            .ok_or_else(|| anyhow!("external UI opened without an editor state"))?;
        WINDOW.with(|slot| -> Result<()> {
            let mut slot = slot.borrow_mut();
            let window = slot.as_mut().ok_or_else(|| anyhow!("external UI state disappeared"))?;
            window.painter.paint(editor, width, height)
        })?;
    }

    let (outer_w, outer_h) = outer_size(width, height)?;
    let (x, y) = centered_origin(game_hwnd, outer_w, outer_h);
    let (foreground, visible) = unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, outer_w, outer_h, SWP_SHOWWINDOW)
            .map_err(|error| anyhow!("SetWindowPos external UI failed: {error}"))?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        let foreground = SetForegroundWindow(hwnd).as_bool();
        let _ = SetFocus(hwnd);
        let visible = IsWindowVisible(hwnd).as_bool();
        let _ = InvalidateRect(hwnd, None, BOOL(0));
        let _ = UpdateWindow(hwnd);
        (foreground, visible)
    };
    WINDOW.with(|slot| {
        if let Some(window) = slot.borrow_mut().as_mut() {
            window.visible = true;
        }
    });
    log::warn!("ui: external key-config window shown ({width}x{height}) visible={visible} foreground={foreground}");
    Ok(())
}

pub(crate) fn hide() {
    let (hwnd, game_hwnd) = WINDOW.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(window) = slot.as_mut() else {
            return (HWND::default(), HWND::default());
        };
        window.visible = false;
        (window.hwnd, window.game_hwnd)
    });
    if hwnd.0.is_null() {
        return;
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        if !game_hwnd.0.is_null() {
            if IsIconic(game_hwnd).as_bool() {
                let _ = ShowWindow(game_hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(game_hwnd);
            let _ = SetFocus(game_hwnd);
        }
    }
}

pub(crate) fn is_active() -> bool {
    WINDOW.with(|slot| slot.borrow().as_ref().is_some_and(|window| window.visible))
}

pub(crate) fn invalidate() {
    let hwnd = WINDOW.with(|slot| slot.borrow().as_ref().map(|window| window.hwnd));
    if let Some(hwnd) = hwnd {
        if !hwnd.0.is_null() {
            unsafe {
                let _ = InvalidateRect(hwnd, None, BOOL(0));
            }
        }
    }
}

unsafe extern "system" fn external_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            WINDOW.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(window) = slot.as_mut() else {
                    return;
                };
                // Never block a paint on a user interaction that is currently
                // holding the UI-state lock. In that case the previous DIB is
                // still valid and will be replaced by the next invalidate.
                match super::state().try_lock() {
                    Ok(state) => {
                        if let Some(editor) = state.editor.as_ref() {
                            let _ = window.painter.paint(editor, window.width, window.height);
                        }
                    }
                    Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                        let state = poisoned.into_inner();
                        if let Some(editor) = state.editor.as_ref() {
                            let _ = window.painter.paint(editor, window.width, window.height);
                        }
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {}
                }
                let _ = window.painter.blit_to(hdc);
            });
            let _ = unsafe { EndPaint(hwnd, &paint) };
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as u16 as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
            let size = WINDOW.with(|slot| slot.borrow().as_ref().map(|window| (window.width, window.height)));
            if let Some((width, height)) = size {
                super::handle_external_click(x, y, width, height);
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lparam.0 as u32 & 0xffff) == HTCLIENT {
                if let Ok(cursor) = unsafe { LoadCursorW(None, IDC_ARROW) } {
                    unsafe {
                        let _ = SetCursor(cursor);
                    }
                    return LRESULT(1);
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u32;
            if key == VK_ESCAPE.0 as u32 || key == VK_F.0 as u32 {
                super::request_close();
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_CLOSE => {
            super::request_close();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
