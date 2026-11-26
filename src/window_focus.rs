//! Startup-only window focus workaround.
//!
//! Sekiro's exclusive-fullscreen path can stutter until the window loses and
//! regains foreground (the same thing a manual Alt+Tab does). This module
//! performs one minimize/restore cycle shortly after the mod starts.
//!
//! Disable with `disable_auto_alt_tab.enable` in the game directory.

use std::{
    ffi::c_void,
    path::Path,
    sync::atomic::{AtomicIsize, Ordering},
    thread,
    time::Duration,
};

use windows::Win32::{
    Foundation::{BOOL, HWND, LPARAM},
    System::Threading::GetCurrentProcessId,
    UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GW_OWNER, GetClassNameW, GetShellWindow, GetWindow, GetWindowThreadProcessId,
        IsWindowVisible, SW_MINIMIZE, SW_RESTORE, SetForegroundWindow, ShowWindow,
    },
};

static FOUND: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn enum_proc(hwnd: HWND, _: LPARAM) -> BOOL {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid != unsafe { GetCurrentProcessId() } {
        return BOOL(1);
    }
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return BOOL(1);
    }
    if unsafe { GetWindow(hwnd, GW_OWNER) }
        .map(|owner| !owner.0.is_null())
        .unwrap_or(false)
    {
        return BOOL(1);
    }
    let mut class = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    if len <= 0 {
        return BOOL(1);
    }
    if String::from_utf16_lossy(&class[..len as usize]) == "CSclass" {
        FOUND.store(hwnd.0 as isize, Ordering::Release);
        return BOOL(0);
    }
    BOOL(1)
}

fn find_game_window() -> Option<HWND> {
    FOUND.store(0, Ordering::Release);
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(0));
    }
    let raw = FOUND.load(Ordering::Acquire);
    (raw != 0).then(|| HWND(raw as *mut c_void))
}

pub fn enabled(game_dir: &Path) -> bool {
    !game_dir.join("disable_auto_alt_tab.enable").is_file()
}

/// Run one minimize/restore cycle after the game has had time to settle into
/// its fullscreen mode. This is the programmatic equivalent of Alt+Tab out and
/// back, without injecting keyboard input.
pub fn schedule_after_startup() {
    let _ = thread::Builder::new()
        .name("paper-jutsu-window-focus".into())
        .spawn(|| {
            thread::sleep(Duration::from_secs(6));
            let Some(hwnd) = find_game_window() else {
                log::warn!("window focus: CSclass window not found; skip auto alt-tab");
                return;
            };
            log::warn!("window focus: auto alt-tab minimize/restore hwnd={:?}", hwnd.0);
            let minimized = unsafe { ShowWindow(hwnd, SW_MINIMIZE) }.as_bool();
            if !minimized {
                let shell = unsafe { GetShellWindow() };
                if !shell.0.is_null() {
                    unsafe {
                        let _ = SetForegroundWindow(shell);
                    }
                }
            }
            thread::sleep(Duration::from_millis(300));
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
            }
            thread::sleep(Duration::from_millis(150));
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            log::warn!("window focus: auto alt-tab sequence done");
        });
}
