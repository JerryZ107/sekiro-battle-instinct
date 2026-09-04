//! Early-load proxy for `version.dll` — opens the boot console at process start.

#![allow(non_snake_case)]

#[path = "../../src/boot.rs"]
mod boot;
#[path = "../../src/load_console.rs"]
mod load_console;

use std::{
    ffi::c_void,
    mem,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::OnceLock,
};

use windows::{
    Win32::{
        Foundation::{BOOL, HMODULE},
        System::{
            LibraryLoader::{
                DisableThreadLibraryCalls, GetModuleFileNameW, GetProcAddress, LoadLibraryW,
            },
            SystemServices::DLL_PROCESS_ATTACH,
        },
    },
    core::PCWSTR,
};

type DWORD = u32;
type LPCWSTR = PCWSTR;
type LPVOID = *mut c_void;
type LPBOOL = *mut BOOL;
type PUINT = *mut u32;

static SYSTEM_VERSION: OnceLock<isize> = OnceLock::new();

fn system_version() -> HMODULE {
    let raw = *SYSTEM_VERSION.get_or_init(|| unsafe {
        LoadLibraryW(windows::core::w!("C:\\Windows\\System32\\version.dll"))
            .expect("system version.dll")
            .0 as isize
    });
    HMODULE(raw as *mut c_void)
}

fn proc_named<T>(name: &str) -> T {
    let cname = std::ffi::CString::new(name).expect("export name");
    unsafe {
        let addr = GetProcAddress(
            system_version(),
            windows::core::PCSTR(cname.as_ptr() as *const u8),
        )
        .unwrap_or_else(|| panic!("missing version export: {name}"));
        mem::transmute_copy(&addr)
    }
}

macro_rules! forward {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $name($($arg: $ty),*) -> $ret {
            type FnType = unsafe extern "system" fn($($ty),*) -> $ret;
            let f: FnType = proc_named(stringify!($name));
            unsafe { f($($arg),*) }
        }
    };
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(hmodule: HMODULE, call_reason: u32, _reserved: *mut c_void) -> bool {
    if call_reason == DLL_PROCESS_ATTACH {
        unsafe {
            let _ = DisableThreadLibraryCalls(hmodule);
        }
        let mut buf = vec![0u16; 260];
        let len = unsafe { GetModuleFileNameW(hmodule, buf.as_mut_slice()) } as usize;
        let dll_path = PathBuf::from(std::ffi::OsString::from_wide(&buf[..len]));
        if let Some(dir) = dll_path.parent() {
            boot::on_early_attach(dir.to_path_buf());
        }
    }
    true
}

forward!(GetFileVersionInfoA(lpszFilename: *const u8, dwHandle: DWORD, dwLen: DWORD, lpData: LPVOID) -> BOOL);
forward!(GetFileVersionInfoW(lpszFilename: LPCWSTR, dwHandle: DWORD, dwLen: DWORD, lpData: LPVOID) -> BOOL);
forward!(GetFileVersionInfoSizeA(lpszFilename: *const u8, lpdwHandle: *mut DWORD) -> DWORD);
forward!(GetFileVersionInfoSizeW(lpszFilename: LPCWSTR, lpdwHandle: *mut DWORD) -> DWORD);
forward!(GetFileVersionInfoExA(dwFlags: DWORD, lpwstrFilename: *const u8, dwHandle: DWORD, dwLen: DWORD, lpData: LPVOID) -> BOOL);
forward!(GetFileVersionInfoExW(dwFlags: DWORD, lpwstrFilename: LPCWSTR, dwHandle: DWORD, dwLen: DWORD, lpData: LPVOID) -> BOOL);
forward!(GetFileVersionInfoSizeExA(dwFlags: DWORD, lpwstrFilename: *const u8, lpdwHandle: *mut DWORD) -> DWORD);
forward!(GetFileVersionInfoSizeExW(dwFlags: DWORD, lpwstrFilename: LPCWSTR, lpdwHandle: *mut DWORD) -> DWORD);
forward!(VerQueryValueA(pBlock: LPVOID, lpSubBlock: *const u8, lplpBuffer: *mut LPVOID, puLen: PUINT) -> BOOL);
forward!(VerQueryValueW(pBlock: LPVOID, lpSubBlock: LPCWSTR, lplpBuffer: *mut LPVOID, puLen: PUINT) -> BOOL);
forward!(VerLanguageNameA(wLang: DWORD, szLang: *mut u8, cchLang: DWORD) -> DWORD);
forward!(VerLanguageNameW(wLang: DWORD, szLang: LPCWSTR, cchLang: DWORD) -> DWORD);
forward!(VerFindFileA(uFlags: DWORD, szFileName: *const u8, szWinDir: *const u8, szAppDir: *const u8, szCurDir: *mut u8, puCurDirLen: PUINT, szDestDir: *mut u8, puDestDirLen: PUINT) -> DWORD);
forward!(VerFindFileW(uFlags: DWORD, szFileName: LPCWSTR, szWinDir: LPCWSTR, szAppDir: LPCWSTR, szCurDir: LPCWSTR, puCurDirLen: PUINT, szDestDir: LPCWSTR, puDestDirLen: PUINT) -> DWORD);
forward!(VerInstallFileA(uFlags: DWORD, szSrcFileName: *const u8, szDestFileName: *const u8, szSrcDir: *const u8, szDestDir: *const u8, szCurDir: *const u8, szTmpFile: *mut u8, puTmpFileLen: PUINT) -> DWORD);
forward!(VerInstallFileW(uFlags: DWORD, szSrcFileName: LPCWSTR, szDestFileName: LPCWSTR, szSrcDir: LPCWSTR, szDestDir: LPCWSTR, szCurDir: LPCWSTR, szTmpFile: LPCWSTR, puTmpFileLen: PUINT) -> DWORD);
