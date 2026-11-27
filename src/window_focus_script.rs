//! Launch an external window-focus helper script once at mod startup.
//!
//! Keeping the focus workaround outside the DLL avoids doing focus-sensitive
//! Win32 calls from a background thread inside the game process. The script is
//! optional: if `auto_alt_tab.ps1` is missing, nothing is launched.

use std::{os::windows::process::CommandExt, path::Path, process::Command, thread, time::Duration};

use crate::game;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start `auto_alt_tab.ps1` from the game directory unless explicitly disabled.
/// The script receives the current process id and waits for the game window.
pub fn launch_if_available(game_dir: &Path) {
    if game_dir.join("disable_auto_alt_tab.enable").is_file() {
        log::warn!("window focus script: disabled by disable_auto_alt_tab.enable");
        return;
    }

    let script = game_dir.join("auto_alt_tab.ps1");
    if !script.is_file() {
        log::warn!("window focus script: auto_alt_tab.ps1 not found; skip");
        return;
    }

    let game_dir = game_dir.to_path_buf();
    let _ = thread::Builder::new()
        .name("paper-jutsu-focus-script".into())
        .spawn(move || {
            log::warn!("window focus script: waiting for gameplay");
            let mut gameplay = false;
            for _ in 0..120 {
                if game::player_chr_data_module().is_some() {
                    gameplay = true;
                    break;
                }
                thread::sleep(Duration::from_millis(500));
            }
            if !gameplay {
                log::warn!("window focus script: gameplay not detected after 60s; launching fallback anyway");
            }
            thread::sleep(Duration::from_millis(1500));
            launch_script(&game_dir, &script);
        });
}

fn launch_script(game_dir: &Path, script: &Path) {
    let target_pid = std::process::id();
    log::warn!(
        "window focus script: launching {} target_pid={target_pid}",
        script.display()
    );
    match Command::new("powershell.exe")
        .current_dir(game_dir)
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(script)
        .arg("-TargetPid")
        .arg(target_pid.to_string())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(child) => log::warn!("window focus script: child_pid={}", child.id()),
        Err(error) => log::error!("window focus script: launch failed: {error:#}"),
    }
}
