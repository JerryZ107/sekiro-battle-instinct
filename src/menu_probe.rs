use std::{
    ffi::c_void,
    sync::{
        Once, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use windows::Win32::System::{
    LibraryLoader::GetModuleHandleW,
    Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
        PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    },
};

// RVA from fromsoftware-rs / Sekiro 1.6.0.0. This is only used to gate the
// F-key UI request; the selected skill/tool is read from the game menu
// structures by `game_menu`.
const MENU_MAN_PTR: usize = 0x3d67408;
const MENU_FLAGS_OFFSET: usize = 0x1c;
const MENU_MODE_FLAG: usize = 9;
const MENU_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MENU_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(100);

static MENU_MONITOR_STARTED: Once = Once::new();
static MENU_OPEN_CACHED: AtomicBool = AtomicBool::new(false);
static MENU_CACHE_READY: AtomicBool = AtomicBool::new(false);
static MENU_WAS_OPEN: AtomicBool = AtomicBool::new(false);
static MENU_CLOSE_GENERATION: AtomicU64 = AtomicU64::new(0);

fn game_base() -> usize {
    static BASE: OnceLock<usize> = OnceLock::new();
    *BASE.get_or_init(|| unsafe { GetModuleHandleW(None).map(|h| h.0 as usize).unwrap_or(0) })
}

fn read_i32(addr: usize) -> Option<i32> {
    if addr == 0 || !range_readable(addr, 4) {
        return None;
    }
    Some(unsafe { (addr as *const i32).read_unaligned() })
}

/// Start a lightweight monitor so frame paths can read the menu state without
/// issuing `VirtualQuery` on every game tick. The monitor only observes state;
/// it never blocks or mutates game data.
pub fn start_menu_monitor() {
    MENU_MONITOR_STARTED.call_once(|| {
        let result = thread::Builder::new()
            .name("battle-instinct-menu-probe".into())
            .spawn(|| {
                loop {
                    let open = menu_open();
                    let was_open = MENU_WAS_OPEN.swap(open, Ordering::AcqRel);
                    MENU_OPEN_CACHED.store(open, Ordering::Release);
                    if was_open != open {
                        crate::frame_perf::reset_tick_clock();
                        crate::frame_perf::mark(if open {
                            crate::frame_perf::EventId::MenuOpen
                        } else {
                            crate::frame_perf::EventId::MenuClose
                        });
                    }
                    if was_open && !open {
                        // Consumers that need to rescan inventory only need an
                        // edge, not a per-frame menu probe. Keeping this as a
                        // generation counter lets several consumers observe the
                        // same close event without racing each other.
                        MENU_CLOSE_GENERATION.fetch_add(1, Ordering::AcqRel);
                    }
                    MENU_CACHE_READY.store(true, Ordering::Release);
                    // Poll slowly during gameplay and faster only while the
                    // pause menu is actually open.
                    thread::sleep(if open {
                        MENU_POLL_INTERVAL
                    } else {
                        MENU_IDLE_POLL_INTERVAL
                    });
                }
            });
        if let Err(error) = result {
            log::error!("menu probe monitor could not start: {error}");
        }
    });
}

/// Cached menu state for per-frame consumers. Falls back to a direct probe only
/// until the monitor publishes its first sample (or if the thread failed).
#[inline]
pub fn menu_open_cached() -> bool {
    if MENU_CACHE_READY.load(Ordering::Acquire) {
        MENU_OPEN_CACHED.load(Ordering::Relaxed)
    } else {
        menu_open()
    }
}

/// Monotonically increasing counter for open -> closed transitions. The
/// gameplay thread can poll one atomic instead of repeatedly querying the
/// pause-menu memory and calling `VirtualQuery`.
#[inline]
pub fn menu_close_generation() -> u64 {
    MENU_CLOSE_GENERATION.load(Ordering::Acquire)
}

pub fn menu_open() -> bool {
    let base = game_base();
    if base == 0 {
        return false;
    }
    let menu_man = unsafe { ((base + MENU_MAN_PTR) as *const usize).read_unaligned() };
    if menu_man == 0 {
        return false;
    }
    read_i32(menu_man + MENU_FLAGS_OFFSET + MENU_MODE_FLAG * 4)
        .map(|value| value > 0)
        .unwrap_or(false)
}

/// Returns true only when the range is committed and executable. MinHook (and
/// the game loader) requires target pages to have reached their final
/// executable protection; readable is not enough because protected game code
/// can still be unpacking while the first pages are readable.
pub(crate) fn range_executable(addr: usize, len: usize) -> bool {
    let mut off = 0usize;
    while off < len {
        let mut mbi = MEMORY_BASIC_INFORMATION::default();
        let written = unsafe {
            VirtualQuery(
                Some((addr + off) as *const c_void),
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if written == 0 || mbi.State != MEM_COMMIT {
            return false;
        }
        let protect = mbi.Protect.0;
        if protect & PAGE_GUARD.0 != 0 || protect & PAGE_NOACCESS.0 != 0 {
            return false;
        }
        let executable =
            (PAGE_EXECUTE.0 | PAGE_EXECUTE_READ.0 | PAGE_EXECUTE_READWRITE.0 | PAGE_EXECUTE_WRITECOPY.0) & protect != 0;
        if !executable {
            return false;
        }
        off = off.saturating_add(mbi.RegionSize.max(1));
    }
    true
}

pub(crate) fn has_expected_prefix(addr: usize, expected: &[u8]) -> bool {
    if expected.is_empty() || !range_readable(addr, expected.len()) {
        return false;
    }
    let bytes = unsafe { std::slice::from_raw_parts(addr as *const u8, expected.len()) };
    bytes == expected
}

pub(crate) fn range_readable(addr: usize, len: usize) -> bool {
    let mut off = 0usize;
    while off < len {
        let mut mbi = MEMORY_BASIC_INFORMATION::default();
        let written = unsafe {
            VirtualQuery(
                Some((addr + off) as *const c_void),
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if written == 0 || mbi.State != MEM_COMMIT {
            return false;
        }
        let protect = mbi.Protect.0;
        if protect & PAGE_GUARD.0 != 0 || protect & PAGE_NOACCESS.0 != 0 {
            return false;
        }
        off = off.saturating_add(mbi.RegionSize.max(1));
    }
    true
}
