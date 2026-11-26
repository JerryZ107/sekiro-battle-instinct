//! Startup-only window focus workaround.
//!
//! Sekiro's exclusive-fullscreen path can stutter until the window loses and
//! regains foreground (the same thing a manual Alt+Tab does). This module waits
//! until gameplay data exists, then sends a real Alt+Tab out/back sequence.
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
    UI::{
        Input::KeyboardAndMouse::{KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU, VK_TAB, keybd_event},
        WindowsAndMessaging::{
            BringWindowToTop, EnumWindows, GW_OWNER, GetClassNameW, GetForegroundWindow, GetWindow,
            GetWindowThreadProcessId, IsWindowVisible, SW_MINIMIZE, SW_RESTORE, SetForegroundWindow, ShowWindow,
        },
    },
};

use crate::game;

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

fn send_alt_tab() {
    unsafe {
        keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_TAB.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
        keybd_event(VK_TAB.0 as u8, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

/// Wait until the player object exists, then Alt+Tab out and back once.
pub fn schedule_after_startup() {
    let _ = thread::Builder::new()
        .name("paper-jutsu-window-focus".into())
        .spawn(|| {
            log::warn!("window focus: waiting for gameplay before auto alt-tab");
            let mut gameplay = false;
            for _ in 0..120 {
                if game::player_chr_data_module().is_some() {
                    gameplay = true;
                    break;
                }
                thread::sleep(Duration::from_millis(500));
            }
            if !gameplay {
                log::warn!("window focus: gameplay not detected after 60s; running fallback anyway");
            }
            // Give the first gameplay frames / fullscreen transition time to settle.
            thread::sleep(Duration::from_millis(1500));

            let Some(hwnd) = find_game_window() else {
                log::warn!("window focus: CSclass window not found; skip auto alt-tab");
                return;
            };
            log::warn!("window focus: sending Alt+Tab out/back hwnd={:?}", hwnd.0);
            send_alt_tab();
            thread::sleep(Duration::from_millis(450));
            send_alt_tab();
            thread::sleep(Duration::from_millis(450));

            if unsafe { GetForegroundWindow() } != hwnd {
                log::warn!("window focus: Alt+Tab did not restore foreground; using minimize/restore fallback");
                let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
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
            }
            log::warn!("window focus: auto Alt+Tab sequence done (foreground={})", unsafe {
                GetForegroundWindow() == hwnd
            });
        });
}
