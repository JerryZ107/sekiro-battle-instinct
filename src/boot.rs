use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::load_console;

static EARLY_SCAN_DONE: AtomicBool = AtomicBool::new(false);

/// Called from `version.dll` as early as the game process starts.
pub fn on_early_attach(dir: PathBuf) {
    load_console::open();
    if EARLY_SCAN_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    load_console::print_line("Sekiro 进程已启动");
    load_console::print_line("正在扫描游戏目录 MOD…");
    list_detected_mod_dlls(&dir);
    load_console::print_line("Mod Engine 正在后台加载（其调试输出不会显示在此窗口）");
    load_console::print_line("等待 dinput8.dll 注入 Battle Instinct…");
}

/// Called from `dinput8.dll` when DirectInput is first loaded.
pub fn on_dinput_attach(dir: &Path) {
    load_console::open();
    if !EARLY_SCAN_DONE.swap(true, Ordering::SeqCst) {
        load_console::print_line("Sekiro 进程已启动");
        load_console::print_line("正在扫描游戏目录 MOD…");
        list_detected_mod_dlls(dir);
    }
    load_console::print_line("Battle Instinct (dinput8.dll) 已注入，开始初始化…");
}

pub fn list_detected_mod_dlls(dir: &Path) {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        load_console::print_line("  （无法读取游戏目录）");
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let lossy = name.to_string_lossy();
        let lower = lossy.to_ascii_lowercase();
        if !lower.ends_with(".dll") {
            continue;
        }
        let interesting = lower == "dinput8.dll"
            || lower.starts_with("dinput8_")
            || lower.contains("modengine")
            || lower == "version.dll";
        if interesting {
            names.push(lossy.into_owned());
        }
    }
    names.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
    if names.is_empty() {
        load_console::print_line("  （未检测到 dinput8 / Mod Engine 相关 DLL）");
        return;
    }
    load_console::print_line("检测到以下 MOD DLL：");
    for name in names {
        let note = mod_dll_note(&name);
        if note.is_empty() {
            load_console::print_line(&format!("  · {name}"));
        } else {
            load_console::print_line(&format!("  · {name}  ({note})"));
        }
    }
}

fn mod_dll_note(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower == "dinput8.dll" {
        "Battle Instinct 入口"
    } else if lower.contains("modengine") {
        "Mod Engine"
    } else if lower == "version.dll" {
        "Battle Instinct 预加载"
    } else if lower.starts_with("dinput8_") {
        "链式兼容 MOD"
    } else {
        ""
    }
}
