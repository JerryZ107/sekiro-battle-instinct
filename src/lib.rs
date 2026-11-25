mod binding_validation;
mod bindings;
mod boot;
mod cfg_meta;
mod config;
mod core;
mod device;
mod emblem;
mod flow;
mod frame;
mod frame_perf;
mod game;
mod game_menu;
mod hook_util;
mod input;
mod load_console;
mod logger;
mod menu_probe;
mod paper_guard;
mod ui;

use core::Mod;
use std::{
    cell::UnsafeCell,
    ffi::{OsStr, OsString, c_void},
    fs, mem,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self},
    time::{Duration, Instant},
};

use frame::FRAMERATE;
use minhook::MinHook;
use windows::{
    Win32::{
        Foundation::{GetLastError, HINSTANCE, HMODULE},
        System::{
            LibraryLoader::{DisableThreadLibraryCalls, GetModuleFileNameW, GetProcAddress, LoadLibraryW},
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
        boot::on_dinput_attach(&dir_path);

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

type DirectInput8CreateFn = fn(HINSTANCE, u32, *const GUID, *mut *mut c_void, HINSTANCE) -> HRESULT;

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
        if names.is_empty() {
            load_console::print_line("  （未发现 dinput8_*.dll，跳过链式加载）");
        }
        for name in names {
            let lossy = name.to_string_lossy();
            let label = if lossy.to_ascii_lowercase().contains("modengine") {
                "正在启动 Mod Engine"
            } else {
                "正在加载"
            };
            load_console::print_line(&format!("  {label}: {lossy}"));
            let path = path.join(&name);
            let path = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
            let loaded = unsafe { LoadLibraryW(PCWSTR::from_raw(path.as_ptr())) };
            match loaded {
                Ok(_) => {
                    log::debug!("Chainloaded dll: {name:?}");
                    load_console::print_line(&format!("  ✓ {lossy} 已加载"));
                }
                Err(e) => {
                    log::error!("Failed to chainload {name:?}: {e:?}");
                    load_console::print_line(&format!("  ✗ {lossy} 加载失败: {e}"));
                }
            }
        }
        Ok(())
    })();

    if let Err(e) = res {
        log::error!("Error occured when chainloading. {e:?}");
        load_console::print_line(&format!("链式加载出错: {e}"));
    }
}

//----------------------------------------------------------------------------
//
//  Initialize the MOD
//
//----------------------------------------------------------------------------

/// Hook targets are already mapped into the game process when dinput8.dll loads.
/// Poll their committed pages/signature instead of always sleeping for 10 seconds.
const HOOK_READY_TIMEOUT: Duration = Duration::from_secs(10);
const HOOK_READY_POLL: Duration = Duration::from_millis(20);

fn hook_target_ready(address: usize, signature: &[u8]) -> bool {
    menu_probe::range_executable(address, signature.len()) && menu_probe::has_expected_prefix(address, signature)
}

fn wait_for_game_code() -> bool {
    let deadline = Instant::now() + HOOK_READY_TIMEOUT;
    loop {
        // MinHook needs committed executable pages for its input/damage targets.
        // The menu signature also rejects unsupported Sekiro executable builds.
        if game_menu::code_ready()
            && hook_target_ready(game::PROCESS_INPUT, &game::PROCESS_INPUT_PREFIX)
            && hook_target_ready(game::PLAYER_DAMAGE_REACTION, &game::PLAYER_DAMAGE_REACTION_PREFIX)
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(HOOK_READY_POLL);
    }
}

static PROCESS_INPUT_ORIG: OnceLock<fn(*mut game::InputHandler, usize) -> usize> = OnceLock::new();

static STATE: OnceLock<State> = OnceLock::new();
static INPUT_STATE_BUSY: AtomicBool = AtomicBool::new(false);
static GAMEPLAY_LOGIC_ENABLED: AtomicBool = AtomicBool::new(true);

/// The process-input detour is only invoked by the game's main thread, and no
/// other thread ever touches the mod state. Avoid taking a mutex on every game
/// tick; the mutable reference is created only inside that detour.
struct State {
    modification: UnsafeCell<Mod>,
}

unsafe impl Sync for State {}

/// Re-entrancy guard for the lock-free `UnsafeCell` above. A second concurrent
/// or recursive call skips only the mod logic and still invokes the original
/// game function.
struct InputAccessGuard;

impl InputAccessGuard {
    fn acquire() -> Option<Self> {
        INPUT_STATE_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for InputAccessGuard {
    fn drop(&mut self) {
        INPUT_STATE_BUSY.store(false, Ordering::Release);
    }
}

fn modify(path: &Path) {
    frame_perf::init(path);
    let gameplay_disabled = path.join("disable_gameplay_logic.enable").is_file();
    GAMEPLAY_LOGIC_ENABLED.store(!gameplay_disabled, Ordering::Release);
    if gameplay_disabled {
        log::warn!("diagnostic: gameplay MOD logic disabled by marker file");
        frame_perf::mark(frame_perf::EventId::GameplayLogicDisabled);
    }
    let damage_hook_disabled = path.join("disable_damage_hook.enable").is_file();
    let path = path.join("battle_instinct.cfg");
    load_console::print_line("等待游戏代码就绪…");
    let ready_started = Instant::now();
    if wait_for_game_code() {
        load_console::print_line(&format!(
            "  游戏代码已就绪（{:.3} 秒）",
            ready_started.elapsed().as_secs_f32()
        ));
    } else {
        load_console::print_line("  ✗ 游戏代码就绪超时，继续尝试安装钩子");
        log::error!(
            "hook targets were not readable after {} s",
            HOOK_READY_TIMEOUT.as_secs()
        );
    }

    load_console::print_line("读取配置 battle_instinct.cfg …");
    let result = (|| unsafe {
        let ui_cfg_path = path.clone();
        menu_probe::start_menu_monitor();
        let modification = Mod::new(path)?;
        log::warn!(
            "perf: event-driven inventory refresh, no global keyboard hook, dynamic mouse hook, lock-free input state"
        );

        load_console::print_line("安装伤害事件钩子…");
        if damage_hook_disabled {
            log::warn!("diagnostic: player damage reaction hook disabled by marker file");
            frame_perf::mark(frame_perf::EventId::DamageHookDisabled);
            load_console::print_line("  伤害事件钩子已跳过（诊断模式）");
        } else if let Err(e) = flow::install_damage_hook() {
            log::error!("Failed to install damage reaction hook: {e:?}");
            load_console::print_line(&format!("  伤害事件钩子安装失败: {e}"));
        } else {
            load_console::print_line("  伤害事件钩子已安装");
        }

        load_console::print_line("安装输入钩子…");
        let target = game::PROCESS_INPUT as *mut c_void;
        let detour = process_input as *mut c_void;
        let mut process_input_hook_ready = false;
        match hook_util::create_hook_with_retry(target, detour, "process_input", true) {
            Ok(process_input_orig) => {
                let process_input_orig = mem::transmute(process_input_orig);
                match PROCESS_INPUT_ORIG.set(process_input_orig) {
                    Ok(()) => {
                        process_input_hook_ready = true;
                        load_console::print_line("  ✓ 输入 Hook 已安装");
                    }
                    Err(_) => {
                        log::error!("Failed to set PROCESS_INPUT_ORIG");
                        load_console::print_line("  ✗ 输入 Hook 原始函数保存失败");
                    }
                }
            }
            Err(error) => {
                log::error!("Failed to install process_input hook: {error:?}");
                load_console::print_line(&format!("  ✗ 输入 Hook 安装失败（UI 继续尝试启动）: {error:?}"));
            }
        }

        let state = State {
            modification: UnsafeCell::new(modification),
        };

        if STATE.set(state).is_err() {
            log::error!("Failed to set STATE");
            load_console::print_line("  ✗ 战斗状态初始化失败（UI 继续尝试启动）");
        }

        load_console::print_line("安装菜单选中项 Hook…");
        if let Err(error) = game_menu::install_hooks() {
            log::error!("menu hook install failed: {error:#}");
            load_console::print_line(&format!("  ✗ 菜单选中项 Hook 安装失败（战斗功能继续）: {error:#}"));
        } else {
            load_console::print_line("  ✔ 菜单选中项 Hook 已安装");
        }

        if let Err(error) = ui::start(ui_cfg_path) {
            log::error!("ui/input hook startup failed: {error:#}");
            load_console::print_line(&format!("  ✗ 输入/UI Hook 启动失败（战斗功能继续）: {error:#}"));
        }
        if let Err(error) = MinHook::enable_all_hooks() {
            log::error!("Failed to enable MinHook hooks: {error:?}");
            load_console::print_line(&format!("  ✗ 启用 Hook 失败: {error:?}"));
        } else {
            log::warn!("Paper Jutsu hook enabled (process_input={process_input_hook_ready}).");
        }
        Ok::<_, anyhow::Error>(())
    })();

    match result {
        Ok(()) => load_console::finish(true, None),
        Err(e) => {
            log::error!("Errored occured when modifying the game. {e:?}");
            load_console::finish(false, Some(&format!("{e:#}")));
        }
    }
}

fn process_input(input_handler: *mut game::InputHandler, arg: usize) -> usize {
    let input_handler = unsafe {
        FRAMERATE.tick();
        input_handler.as_mut().expect("input_handler is null")
    };

    let mod_started = Instant::now();
    if GAMEPLAY_LOGIC_ENABLED.load(Ordering::Acquire) && !crate::ui::overlay_open() {
        if let Some(state) = STATE.get() {
            // Safety: the guard prevents concurrent/re-entrant access. The UI and
            // background threads never read or mutate `Mod`.
            if let Some(_guard) = InputAccessGuard::acquire() {
                let modification = unsafe { &mut *state.modification.get() };
                modification.process_input(input_handler);
            }
        }
    }
    let mod_us = mod_started.elapsed().as_micros() as u64;

    let orig_started = Instant::now();
    let result = if let Some(process_input_orig) = PROCESS_INPUT_ORIG.get() {
        process_input_orig(input_handler, arg)
    } else {
        // Hook not fully ready: fall back to the raw game function.
        let process_input_orig: fn(*mut game::InputHandler, usize) -> usize =
            unsafe { mem::transmute(game::PROCESS_INPUT) };
        process_input_orig(input_handler, arg)
    };
    let orig_us = orig_started.elapsed().as_micros() as u64;
    frame_perf::record_input_hook(mod_us, orig_us);
    result
}
