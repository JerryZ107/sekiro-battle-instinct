use std::{
    sync::{Mutex, OnceLock},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::Console::{
            AllocConsole, FreeConsole, GetStdHandle, SetConsoleTitleW, SetStdHandle,
            WriteConsoleW, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        },
    },
    core::PCWSTR,
};

static OPEN: AtomicBool = AtomicBool::new(false);
/// Console screen buffer — kept before stdout/stderr are redirected to NUL.
static CONSOLE_OUT: OnceLock<isize> = OnceLock::new();
static PRINT_LOCK: Mutex<()> = Mutex::new(());

/// Pop up a console for startup progress (no-op if allocation fails).
pub fn open() {
    if OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        if AllocConsole().is_err() {
            OPEN.store(false, Ordering::SeqCst);
            return;
        }
        let Ok(console) = GetStdHandle(STD_OUTPUT_HANDLE) else {
            OPEN.store(false, Ordering::SeqCst);
            let _ = FreeConsole();
            return;
        };
        let _ = CONSOLE_OUT.set(console.0 as isize);

        // Other DLLs (e.g. Mod Engine) printf to stdout/stderr — hide that spam.
        if let Ok(nul) = CreateFileW(
            windows::core::w!("\\\\.\\NUL"),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, nul);
            let _ = SetStdHandle(STD_ERROR_HANDLE, nul);
        }

        let title: Vec<u16> = "Battle Instinct — 加载中"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let _ = SetConsoleTitleW(PCWSTR::from_raw(title.as_ptr()));
    }
    print_line("Battle Instinct 正在加载…");
}

pub fn print_line(msg: &str) {
    if !OPEN.load(Ordering::Relaxed) {
        return;
    }
    let _guard = PRINT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut line: Vec<u16> = msg.encode_utf16().collect();
    line.push('\n' as u16);
    let Some(&raw) = CONSOLE_OUT.get() else {
        return;
    };
    let handle = HANDLE(raw as *mut _);
    unsafe {
        let mut written = 0u32;
        let _ = WriteConsoleW(handle, &line, Some(&mut written), None);
    }
}

/// Print a final message; auto-close only on success.
pub fn finish(success: bool, detail: Option<&str>) {
    if !OPEN.load(Ordering::Relaxed) {
        return;
    }
    if success {
        print_line("加载完成，窗口即将关闭。");
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(1));
            unsafe {
                let _ = FreeConsole();
            }
            OPEN.store(false, Ordering::SeqCst);
        });
    } else {
        print_line("加载失败。");
        if let Some(d) = detail {
            print_line(d);
        }
        print_line("详见 battle_instinct.log");
        print_line("请保留此窗口以便查看错误信息。");
    }
}
