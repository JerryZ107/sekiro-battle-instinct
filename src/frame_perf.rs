//! Frame pacing diagnostics.
//!
//! The game-facing API (record_input_hook / record_present / mark) is lock-free: each call
//! performs a bounded number of atomic stores into a pre-allocated ring buffer. All statistics, formatting and disk I/O happen
//! on dedicated threads. On the DXGI flip path (exclusive fullscreen) even a
//! nanosecond-scale lock can serialize a Present; keeping the game threads
//! allocation- and lock-free is the whole point of this module.

use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(1);
const LONG_FRAME_MS: f32 = 25.0;
const AGG_INTERVAL: Duration = Duration::from_millis(250);

/// Slots in the timing rings. 256 entries cover ~4.2 s at 60 fps, far beyond
/// the 100 ms aggregation interval, so samples are only lost if the collector
/// thread is starved for seconds.
const RING_LEN: usize = 256;
const RING_MASK: u64 = (RING_LEN as u64) - 1;
const EVENT_RING_LEN: usize = 64;
const EVENT_RING_MASK: u64 = (EVENT_RING_LEN as u64) - 1;

/// Ring slot layout: `now_us` (50 bits, ~35 years) + ui/menu flags.
/// Slot value 0 is the reserved "empty" marker; samples with time 0 are
/// dropped before they can reach a ring.
const TIME_MASK: u64 = (1 << 50) - 1;
const UI_BIT: u64 = 1 << 50;
const MENU_BIT: u64 = 1 << 51;
/// Present-ring flag: `Present(syncInterval = 0)` (vsync off / loading).
const PRESENT_SYNC0_BIT: u64 = 1 << 52;
/// Event slots shift the id above the menu bit.
const EVENT_SHIFT: u32 = 52;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventId {
    MenuOpen,
    MenuClose,
    AvailabilityRefresh,
    ConfigReload,
    UiOpen,
    UiClose,
    DamageHookDisabled,
    GameplayLogicDisabled,
    PaperInventoryRescan,
    EmblemScan,
}

impl EventId {
    const fn index(self) -> u64 {
        match self {
            EventId::MenuOpen => 0,
            EventId::MenuClose => 1,
            EventId::AvailabilityRefresh => 2,
            EventId::ConfigReload => 3,
            EventId::UiOpen => 4,
            EventId::UiClose => 5,
            EventId::DamageHookDisabled => 6,
            EventId::GameplayLogicDisabled => 7,
            EventId::PaperInventoryRescan => 8,
            EventId::EmblemScan => 9,
        }
    }

    fn label(index: u64) -> &'static str {
        match index {
            0 => "MENU_OPEN",
            1 => "MENU_CLOSE",
            2 => "AVAILABILITY_REFRESH",
            3 => "CONFIG_RELOAD",
            4 => "UI_OPEN",
            5 => "UI_CLOSE",
            6 => "DAMAGE_HOOK_DISABLED",
            7 => "GAMEPLAY_LOGIC_DISABLED",
            8 => "PAPER_INVENTORY_RESCAN",
            9 => "EMBLEM_SCAN",
            _ => "UNKNOWN",
        }
    }
}

static START: OnceLock<Instant> = OnceLock::new();

#[inline(always)]
fn now_us() -> u64 {
    match START.get() {
        Some(start) => (start.elapsed().as_micros() as u64).clamp(1, TIME_MASK),
        None => 0,
    }
}

//----------------------------------------------------------------------------
//  Game-thread side: atomic stores only
//----------------------------------------------------------------------------

/// One timestamp per game `process_input` tick. The tick stream is the frame
/// clock when the Present hook is parked (the usual gameplay case).
static TICK_RING: [AtomicU64; RING_LEN] = [const { AtomicU64::new(0) }; RING_LEN];
static TICK_POS: AtomicU64 = AtomicU64::new(0);
static LAST_TICK_US: AtomicU64 = AtomicU64::new(0);
static TICK_GENERATION: AtomicU64 = AtomicU64::new(0);

/// One timestamp per DXGI Present call. The tick stream can run at a different
/// rate when the game decouples logic and rendering, so Present is the real
/// render clock and is tracked separately.
static PRESENT_RING: [AtomicU64; RING_LEN] = [const { AtomicU64::new(0) }; RING_LEN];
static PRESENT_POS: AtomicU64 = AtomicU64::new(0);
static LAST_PRESENT_US: AtomicU64 = AtomicU64::new(0);
static PRESENT_GENERATION: AtomicU64 = AtomicU64::new(0);

static EVENT_RING: [AtomicU64; EVENT_RING_LEN] = [const { AtomicU64::new(0) }; EVENT_RING_LEN];
static EVENT_POS: AtomicU64 = AtomicU64::new(0);

/// input-hook timing accumulators; taken with `swap(0)` by the collector.
static INP_SAMPLES: AtomicU64 = AtomicU64::new(0);
static INP_MOD_SUM: AtomicU64 = AtomicU64::new(0);
static INP_MOD_MAX: AtomicU64 = AtomicU64::new(0);
static INP_ORIG_SUM: AtomicU64 = AtomicU64::new(0);
static INP_ORIG_MAX: AtomicU64 = AtomicU64::new(0);
static INP_GAP_MAX_US: AtomicU64 = AtomicU64::new(0);

// Overlay draw / paint-worker costs. These are recorded only while the editor
// is open (draw) or when the worker actually paints/uploads, so they do not add
// work to the closed-editor gameplay path.
static OVL_SAMPLES: AtomicU64 = AtomicU64::new(0);
static OVL_SUM_US: AtomicU64 = AtomicU64::new(0);
static OVL_MAX_US: AtomicU64 = AtomicU64::new(0);

#[inline(always)]
fn ring_push(ring: &[AtomicU64; RING_LEN], pos: &AtomicU64, value: u64) {
    let seq = pos.fetch_add(1, Ordering::Relaxed);
    ring[(seq & RING_MASK) as usize].store(value, Ordering::Relaxed);
}

#[inline]
fn sample_flags() -> u64 {
    let mut flags = 0u64;
    if crate::ui::overlay_open() {
        flags |= UI_BIT;
    }
    if crate::menu_probe::menu_open_cached() {
        flags |= MENU_BIT;
    }
    flags
}

/// Called once per `process_input` detour. Times are microseconds.
/// Lock-free: a few relaxed atomic adds, one swap and one ring store.
pub fn record_input_hook(mod_us: u64, orig_us: u64) {
    let now = now_us();
    if now == 0 {
        return;
    }
    INP_SAMPLES.fetch_add(1, Ordering::Relaxed);
    INP_MOD_SUM.fetch_add(mod_us, Ordering::Relaxed);
    INP_MOD_MAX.fetch_max(mod_us, Ordering::Relaxed);
    INP_ORIG_SUM.fetch_add(orig_us, Ordering::Relaxed);
    INP_ORIG_MAX.fetch_max(orig_us, Ordering::Relaxed);

    let prev = LAST_TICK_US.swap(now, Ordering::Relaxed);
    if prev != 0 {
        ring_push(&TICK_RING, &TICK_POS, now | sample_flags());
    }
}

/// Called once per DXGI `Present` / `Present1` call. Lock-free: one relaxed
/// swap plus one ring store. `sync == 0` marks a vsync-off present, which is
/// useful to correlate frame pacing with the game's own vsync choice.
pub fn record_present(sync: u32) {
    let now = now_us();
    if now == 0 {
        return;
    }
    let mut flags = 0u64;
    if crate::ui::overlay_open() {
        flags |= UI_BIT;
    }
    if sync == 0 {
        flags |= PRESENT_SYNC0_BIT;
    }
    let prev = LAST_PRESENT_US.swap(now, Ordering::Relaxed);
    if prev != 0 {
        ring_push(&PRESENT_RING, &PRESENT_POS, now | flags);
    }
}

/// Drop the previous Present timestamp after a swap-chain recreate. The next
/// Present starts a fresh interval instead of reporting the resize gap as one
/// multi-second frame.
pub fn reset_present_clock() {
    LAST_PRESENT_US.store(0, Ordering::Release);
    PRESENT_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// A menu transition means `process_input` may stop for the entire pause.
/// Clear the tick clock so the first post-menu sample is not reported as one
/// multi-second LONG frame.
pub fn reset_tick_clock() {
    LAST_TICK_US.store(0, Ordering::Release);
    TICK_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Record the Present-thread overlay Draw cost. Lock-free.
pub fn record_overlay_draw(us: u64) {
    if us == 0 {
        return;
    }
    OVL_SAMPLES.fetch_add(1, Ordering::Relaxed);
    OVL_SUM_US.fetch_add(us, Ordering::Relaxed);
    OVL_MAX_US.fetch_max(us, Ordering::Relaxed);
}

/// Event-level marker (menu open/close, refreshes, hook park/unpark).
/// Never called on the per-frame hot path except through these edges.
pub fn mark(event: EventId) {
    let now = now_us();
    if now == 0 {
        return;
    }
    let seq = EVENT_POS.fetch_add(1, Ordering::Relaxed);
    let value = now | (event.index() << EVENT_SHIFT);
    EVENT_RING[(seq & EVENT_RING_MASK) as usize].store(value, Ordering::Relaxed);
}

//----------------------------------------------------------------------------
//  Collector + writer threads
//----------------------------------------------------------------------------

/// True only when the diagnostic log has been explicitly enabled.
#[inline(always)]
pub fn enabled() -> bool {
    START.get().is_some()
}

pub fn init(game_dir: &Path) {
    if START.get().is_some() {
        return;
    }
    let path = game_dir.join("frame_timing.log");
    let file = match OpenOptions::new().create(true).write(true).truncate(true).open(&path) {
        Ok(file) => file,
        Err(error) => {
            log::error!("frame perf: cannot create {}: {error}", path.display());
            return;
        }
    };
    let _ = START.set(Instant::now());
    let (sender, receiver) = sync_channel::<String>(4096);

    let writer = thread::Builder::new()
        .name("paper-jutsu-frame-log".into())
        .spawn(move || {
            let mut file = BufWriter::new(file);
            let _ = writeln!(
                file,
                "# Paper Jutsu frame timing: one summary per second (src=tick); LONG lines are individual frames >= {LONG_FRAME_MS} ms"
            );
            let _ = writeln!(
                file,
                "# time_s fps avg_ms p99_ms max_ms frames ui_frames menu_frames gt16.7 gt25 gt33 src hook_samples mod_avg_us mod_max_us orig_avg_us orig_max_us update_gap_max_ms ovl_avg_us ovl_max_us present_fps present_avg_ms present_p99_ms present_max_ms present_frames present_gt16.7 present_gt33 present_ui present_sync0"
            );
            let _ = file.flush();
            while let Ok(line) = receiver.recv() {
                let _ = writeln!(file, "{line}");
                // The writer thread, not the render thread, performs disk I/O.
                let _ = file.flush();
            }
        });
    if writer.is_err() {
        log::error!("frame perf: cannot start writer thread");
        return;
    }

    let aggregator = thread::Builder::new()
        .name("paper-jutsu-frame-agg".into())
        .spawn(move || aggregator_loop(sender));
    if aggregator.is_err() {
        log::error!("frame perf: cannot start aggregator thread");
    }
    log::warn!("frame perf log: {}", path.display());
}

#[inline]
fn emit(sender: &SyncSender<String>, line: String) {
    let _ = sender.try_send(line);
}

struct RingReader {
    last_seq: u64,
    prev_us: u64,
    generation: u64,
}

impl RingReader {
    const fn new() -> Self {
        Self {
            last_seq: 0,
            prev_us: 0,
            generation: 0,
        }
    }
}

fn drain_events(sender: &SyncSender<String>, reader: &mut RingReader) {
    let end = EVENT_POS.load(Ordering::Acquire);
    while reader.last_seq < end {
        let index = (reader.last_seq & EVENT_RING_MASK) as usize;
        let value = EVENT_RING[index].swap(0, Ordering::Relaxed);
        reader.last_seq += 1;
        if value == 0 {
            continue;
        }
        let secs = (value & TIME_MASK) as f64 / 1_000_000.0;
        let label = EventId::label(value >> EVENT_SHIFT);
        emit(sender, format!("{secs:9.3}s EVENT {label}"));
    }
}

fn drain_stream(
    ring: &[AtomicU64; RING_LEN],
    pos: &AtomicU64,
    reader: &mut RingReader,
    out: &mut Vec<(f32, u64)>,
    flags: &mut Vec<u64>,
) {
    let end = pos.load(Ordering::Acquire);
    if end - reader.last_seq > RING_LEN as u64 {
        reader.last_seq = end - RING_LEN as u64;
    }
    while reader.last_seq < end {
        let index = (reader.last_seq & RING_MASK) as usize;
        let value = ring[index].swap(0, Ordering::Relaxed);
        if value == 0 {
            break;
        }
        let now = value & TIME_MASK;
        if reader.prev_us != 0 && now > reader.prev_us {
            out.push(((now - reader.prev_us) as f32 / 1000.0, now));
            flags.push(value & (UI_BIT | MENU_BIT | PRESENT_SYNC0_BIT));
        }
        reader.prev_us = now;
        reader.last_seq += 1;
    }
}

fn flush_long(sender: &SyncSender<String>, source: &str, deltas: &[(f32, u64)]) {
    for &(ms, time_us) in deltas.iter().filter(|&&(ms, _)| ms >= LONG_FRAME_MS) {
        let secs = time_us as f64 / 1_000_000.0;
        emit(sender, format!("{secs:9.3}s LONG delta_ms={ms:.3} src={source}"));
    }
}

#[derive(Default)]
struct WindowStats {
    count: usize,
    avg_ms: f32,
    p99_ms: f32,
    max_ms: f32,
    over_16_7: usize,
    over_25: usize,
    over_33: usize,
    ui_frames: usize,
    menu_frames: usize,
    sync0_frames: usize,
}

fn summarize(deltas: &[(f32, u64)], flags: &[u64], sorted: &mut Vec<f32>) -> WindowStats {
    if deltas.is_empty() {
        return WindowStats::default();
    }
    let count = deltas.len();
    let sum: f32 = deltas.iter().map(|&(ms, _)| ms).sum();
    sorted.clear();
    sorted.extend(deltas.iter().map(|&(ms, _)| ms));
    sorted.sort_by(|a, b| a.total_cmp(b));
    let p99_index = ((count as f32 * 0.99).ceil() as usize).saturating_sub(1).min(count - 1);
    WindowStats {
        count,
        avg_ms: sum / count as f32,
        p99_ms: sorted[p99_index],
        max_ms: sorted.last().copied().unwrap_or(0.0),
        over_16_7: deltas.iter().filter(|&&(ms, _)| ms > 16.7).count(),
        over_25: deltas.iter().filter(|&&(ms, _)| ms > 25.0).count(),
        over_33: deltas.iter().filter(|&&(ms, _)| ms > 33.0).count(),
        ui_frames: flags.iter().filter(|&&f| f & UI_BIT != 0).count(),
        menu_frames: flags.iter().filter(|&&f| f & MENU_BIT != 0).count(),
        sync0_frames: flags.iter().filter(|&&f| f & PRESENT_SYNC0_BIT != 0).count(),
    }
}

fn aggregator_loop(sender: SyncSender<String>) {
    let mut tick = RingReader::new();
    let mut present = RingReader::new();
    let mut events = RingReader::new();
    let mut tick_ms: Vec<(f32, u64)> = Vec::with_capacity(1024);
    let mut tick_flags: Vec<u64> = Vec::with_capacity(1024);
    let mut present_ms: Vec<(f32, u64)> = Vec::with_capacity(1024);
    let mut present_flags: Vec<u64> = Vec::with_capacity(1024);
    let mut sorted: Vec<f32> = Vec::with_capacity(1024);
    let mut window_started = Instant::now();

    loop {
        thread::sleep(AGG_INTERVAL);
        drain_events(&sender, &mut events);

        // Keep every 100 ms slice until the one-second window closes. Clearing
        // here would throw away 90% of the samples and make p99/max describe
        // only the last slice instead of the reported second.
        let tick_generation = TICK_GENERATION.load(Ordering::Acquire);
        if tick.generation != tick_generation {
            tick.generation = tick_generation;
            tick.prev_us = 0;
            tick_ms.clear();
            tick_flags.clear();
        }
        let present_generation = PRESENT_GENERATION.load(Ordering::Acquire);
        if present.generation != present_generation {
            present.generation = present_generation;
            present.prev_us = 0;
            present_ms.clear();
            present_flags.clear();
        }
        let tick_start = tick_ms.len();
        drain_stream(&TICK_RING, &TICK_POS, &mut tick, &mut tick_ms, &mut tick_flags);
        // LONG events are edge reports, so only inspect the slice drained now
        // rather than re-reporting the whole accumulated window every 250 ms.
        flush_long(&sender, "tick", &tick_ms[tick_start..]);

        let present_start = present_ms.len();
        drain_stream(
            &PRESENT_RING,
            &PRESENT_POS,
            &mut present,
            &mut present_ms,
            &mut present_flags,
        );
        flush_long(&sender, "present", &present_ms[present_start..]);

        if window_started.elapsed() < WINDOW {
            continue;
        }
        let elapsed = window_started.elapsed().as_secs_f32().max(0.001);
        window_started = Instant::now();

        if tick_ms.is_empty() && present_ms.is_empty() {
            continue;
        }

        let tick_stats = summarize(&tick_ms, &tick_flags, &mut sorted);
        let present_stats = summarize(&present_ms, &present_flags, &mut sorted);
        let fps = tick_stats.count as f32 / elapsed;
        let present_fps = if present_stats.count == 0 {
            0.0
        } else {
            present_stats.count as f32 / elapsed
        };

        let samples = INP_SAMPLES.swap(0, Ordering::Relaxed);
        let mod_sum = INP_MOD_SUM.swap(0, Ordering::Relaxed);
        let mod_max = INP_MOD_MAX.swap(0, Ordering::Relaxed);
        let orig_sum = INP_ORIG_SUM.swap(0, Ordering::Relaxed);
        let orig_max = INP_ORIG_MAX.swap(0, Ordering::Relaxed);
        let gap_max = INP_GAP_MAX_US.swap(0, Ordering::Relaxed);
        let ovl_samples = OVL_SAMPLES.swap(0, Ordering::Relaxed);
        let ovl_sum = OVL_SUM_US.swap(0, Ordering::Relaxed);
        let ovl_max = OVL_MAX_US.swap(0, Ordering::Relaxed);
        let ovl_avg = if ovl_samples == 0 {
            0.0
        } else {
            ovl_sum as f64 / ovl_samples as f64
        };
        let mod_avg = if samples == 0 {
            0.0
        } else {
            mod_sum as f64 / samples as f64
        };
        let orig_avg = if samples == 0 {
            0.0
        } else {
            orig_sum as f64 / samples as f64
        };

        let secs = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        emit(
            &sender,
            format!(
                "{secs:9.3}s fps={fps:6.1} avg_ms={:7.3} p99_ms={:7.3} max_ms={:7.3} frames={:4} ui_frames={} menu_frames={} gt16.7={} gt25={} gt33={} src=tick hook_samples={samples} mod_avg_us={mod_avg:.1} mod_max_us={mod_max} orig_avg_us={orig_avg:.1} orig_max_us={orig_max} update_gap_max_ms={:.2} ovl_avg_us={ovl_avg:.1} ovl_max_us={ovl_max} present_fps={present_fps:6.1} present_avg_ms={:7.3} present_p99_ms={:7.3} present_max_ms={:7.3} present_frames={:4} present_gt16.7={} present_gt33={} present_ui={} present_sync0={}",
                tick_stats.avg_ms,
                tick_stats.p99_ms,
                tick_stats.max_ms,
                tick_stats.count,
                tick_stats.ui_frames,
                tick_stats.menu_frames,
                tick_stats.over_16_7,
                tick_stats.over_25,
                tick_stats.over_33,
                gap_max as f64 / 1000.0,
                present_stats.avg_ms,
                present_stats.p99_ms,
                present_stats.max_ms,
                present_stats.count,
                present_stats.over_16_7,
                present_stats.over_33,
                present_stats.ui_frames,
                present_stats.sync0_frames,
            ),
        );
        tick_ms.clear();
        tick_flags.clear();
        present_ms.clear();
        present_flags.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_reports_p99_max_and_flags() {
        let deltas = [(10.0, 1u64), (20.0, 2), (30.0, 3), (40.0, 4)];
        let flags = [UI_BIT, 0, PRESENT_SYNC0_BIT, UI_BIT | PRESENT_SYNC0_BIT];
        let mut sorted = Vec::new();
        let stats = summarize(&deltas, &flags, &mut sorted);
        assert_eq!(stats.count, 4);
        assert_eq!(stats.max_ms, 40.0);
        assert_eq!(stats.over_16_7, 3);
        assert_eq!(stats.ui_frames, 2);
        assert_eq!(stats.sync0_frames, 2);
        assert_eq!(stats.p99_ms, 40.0);
    }

    #[test]
    fn empty_window_is_zeroed() {
        let mut sorted = Vec::new();
        let stats = summarize(&[], &[], &mut sorted);
        assert_eq!(stats.count, 0);
        assert_eq!(stats.max_ms, 0.0);
        assert_eq!(stats.sync0_frames, 0);
    }
}
