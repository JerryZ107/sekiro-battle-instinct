use std::{
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use anyhow::{Result, anyhow};
use minhook::MinHook;
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW},
    core::PCWSTR,
};

const MAX_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(250);
static FAILURE_NOTIFIED: AtomicBool = AtomicBool::new(false);

/// Create one MinHook detour, retrying transient "not executable / still
/// unpacking" failures. The caller decides whether the hook is optional.
pub fn create_hook_with_retry(
    target: *mut c_void,
    detour: *mut c_void,
    label: &str,
    notify_on_failure: bool,
) -> Result<*mut c_void> {
    let mut last_error = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        match unsafe { MinHook::create_hook(target, detour) } {
            Ok(original) => {
                log::warn!("{label}: hook created on attempt {attempt}/{MAX_ATTEMPTS}");
                return Ok(original);
            }
            Err(status) => {
                last_error = format!("{status:?}");
                log::error!("{label}: MinHook create_hook attempt {attempt}/{MAX_ATTEMPTS} failed: {last_error}");
                if attempt < MAX_ATTEMPTS {
                    thread::sleep(RETRY_DELAY);
                }
            }
        }
    }

    let detail = format!("{label} Hook 安装失败（已重试 {MAX_ATTEMPTS} 次）: {last_error}");
    log::error!("{detail}");
    if notify_on_failure {
        notify_once(&detail);
    }
    Err(anyhow!(detail))
}

fn notify_once(detail: &str) {
    if FAILURE_NOTIFIED.swap(true, Ordering::AcqRel) {
        return;
    }
    let text = format!("部分 MOD 功能不可用。\n\n{detail}\n\n其他战斗功能仍会继续运行，详情见 battle_instinct.log。");
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
