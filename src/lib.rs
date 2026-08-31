mod config;
mod core;
mod device;
mod frame;
mod game;
mod input;
mod logger;

use core::Mod;
use std::{
    ffi::{OsStr, OsString, c_void},
    fs, mem,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread::{self},
    time::Duration,
};

use anyhow::anyhow;
use frame::FRAMERATE;
use minhook::MinHook;
use windows::{
    Win32::{
        Foundation::{GetLastError, HINSTANCE, HMODULE},
        System::{
            LibraryLoader::{
                DisableThreadLibraryCalls, GetModuleFileNameW, GetProcAddress, LoadLibraryW,
            },
            SystemInformation::GetSystemDirectoryW,
            SystemServices::DLL_PROCESS_ATTACH,
        },
    },
    core::{GUID, HRESULT, PCWSTR, s},
};

//----------------------------------------------------------------------------
//
//  Entry for the DLL
//
//----------------------------------------------------------------------------

#[unsafe(no_mangle)]
extern "system" fn DllMain(hmodule: HMODULE, call_reason: u32, _reserved: *mut c_void) -> bool {
    if call_reason == DLL_PROCESS_ATTACH {
        unsafe {
            let _ = DisableThreadLibraryCalls(hmodule);
        }

        let mut buf: Vec<u16> = vec![0; 260];
        let len = unsafe { GetModuleFileNameW(hmodule, buf.as_mut_slice()) } as usize;
        let dll_path = PathBuf::from(OsString::from_wide(&buf[..len]));
        let Some(dir_path) = dll_path.parent().map(|p| p.to_path_buf()) else {
            return true;
        };

        logger::init(&dir_path);

        // Never call LoadLibrary from DllMain: it can deadlock the Windows loader and
        // leave Steam stuck on "正在启动". Defer chainload + hook setup to a worker thread.
        thread::spawn(move || {
            chainload(&dir_path);
            modify(&dir_path);
        });
    }
    true
}

//----------------------------------------------------------------------------
//
//  Redirect DirectInput8Create to the original dinput8.dll
//
//----------------------------------------------------------------------------

static DIRECT_INPUT8_CREATE: OnceLock<DirectInput8CreateFn> = OnceLock::new();

type DirectInput8CreateFn =
    fn(HINSTANCE, u32, *const GUID, *mut *mut c_void, HINSTANCE) -> HRESULT;

#[unsafe(no_mangle)]
extern "system" fn DirectInput8Create(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: HINSTANCE,
) -> HRESULT {
    match load_dll() {
        Ok(proc) => proc(hinst, dwversion, riidltf, ppvout, punkouter),
        Err(e) => e.into(),
    }
}

fn load_dll() -> windows::core::Result<DirectInput8CreateFn> {
    if let Some(proc) = DIRECT_INPUT8_CREATE.get().copied() {
        return Ok(proc);
    }

    unsafe {
        let mut path = vec![0u16; 260];
        let len = GetSystemDirectoryW(Some(&mut path));
        path.truncate(len as usize);
        path.extend(OsStr::new("\\dinput8.dll").encode_wide());
        path.push(0);

        let hmodule = LoadLibraryW(PCWSTR::from_raw(path.as_ptr()))?;
        let Some(address) = GetProcAddress(hmodule, s!("DirectInput8Create")) else {
            return Err(GetLastError().into());
        };

        let proc: DirectInput8CreateFn = mem::transmute(address);
        let _ = DIRECT_INPUT8_CREATE.set(proc);
        log::debug!("Located DirectInput8Create at {address:p}.");
        Ok(proc)
    }
}

//----------------------------------------------------------------------------
//
//  Chainload other dinput8.dll files used by other MODs
//
//----------------------------------------------------------------------------

fn chainload(path: &Path) {
    let res: anyhow::Result<()> = (|| {
        let mut names = Vec::new();
        for entry in fs::read_dir(path)?.filter_map(Result::ok) {
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if !name_lossy.starts_with("dinput8_") {
                continue;
            }
            if !name_lossy.ends_with(".dll") {
                continue;
            }
            names.push(name);
        }
        names.sort();
        for name in names {
            let path = path.join(&name);
            let path = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
            let loaded = unsafe { LoadLibraryW(PCWSTR::from_raw(path.as_ptr())) };
            match loaded {
                Ok(_) => log::debug!("Chainloaded dll: {name:?}"),
                Err(e) => log::error!("Failed to chainload {name:?}: {e:?}"),
            }
        }
        Ok(())
    })();

    if let Err(e) = res {
        log::error!("Error occured when chainloading. {e:?}")
    }
}

//----------------------------------------------------------------------------
//
//  Initialize the MOD
//
//----------------------------------------------------------------------------

const HOOK_DELAY: Duration = Duration::from_secs(10);

static PROCESS_INPUT_ORIG: OnceLock<fn(*mut game::InputHandler, usize) -> usize> = OnceLock::new();

static STATE: OnceLock<State> = OnceLock::new();

struct State {
    modification: Mutex<Mod>,
}

fn modify(path: &Path) {
    let path = path.join("battle_instinct.cfg");
    thread::sleep(HOOK_DELAY);
    let result = (|| unsafe {
        let modification = Mod::new(path)?;

        let target = game::PROCESS_INPUT as *mut c_void;
        let detour = process_input as *mut c_void;
        let process_input_orig = MinHook::create_hook(target, detour)?;
        let process_input_orig = mem::transmute(process_input_orig);
        PROCESS_INPUT_ORIG
            .set(process_input_orig)
            .map_err(|_| anyhow!("Failed to set PROCESS_INPUT_ORIG"))?;

        let state = State {
            modification: Mutex::new(modification),
        };

        STATE.set(state).map_err(|_| anyhow!("Failed to set STATE"))?;
        MinHook::enable_all_hooks()?;
        log::warn!("Battle Instinct hook enabled.");
        Ok::<_, anyhow::Error>(())
    })();

    if let Err(e) = result {
        log::error!("Errored occured when modifying the game. {e:?}")
    }
}

fn process_input(input_handler: *mut game::InputHandler, arg: usize) -> usize {
    let input_handler = unsafe {
        FRAMERATE.tick();
        input_handler.as_mut().expect("input_handler is null")
    };

    if let Some(State { modification }) = STATE.get() {
        modification.lock().unwrap().process_input(input_handler);
    }

    if let Some(process_input_orig) = PROCESS_INPUT_ORIG.get() {
        return process_input_orig(input_handler, arg);
    }

    // Hook not fully ready: fall back to the raw game function.
    let process_input_orig: fn(*mut game::InputHandler, usize) -> usize =
        unsafe { mem::transmute(game::PROCESS_INPUT) };
    process_input_orig(input_handler, arg)
}
