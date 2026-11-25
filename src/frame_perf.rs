//! Frame pacing diagnostics.
//!
//! The game-facing API (record_input_hook / before_present / mark) is
//! lock-free: each call performs a bounded number of atomic stores into a
//! pre-allocated ring buffer. All statistics, formatting and disk I/O happen
//! on dedicated threads. On the DXGI flip path (exclusive fullscreen) even a
//! nanosecond-scale lock can serialize a Present; keeping the game threads
//! allocation- and lock-free is the whole point of this module.

use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
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
    PresentHookEnable,
    PresentHookDisable,
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
            EventId::PresentHookEnable => 10,
            EventId::PresentHookDisable => 11,
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
            10 => "PRESENT_HOOK_ENABLE",
            11 => "PRESENT_HOOK_DISABLE",
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

/// One timestamp per hooked Present. Only written while the Present hook is
/// enabled (key editor open).
static PRESENT_RING: [AtomicU64; RING_LEN] = [const { AtomicU64::new(0) }; RING_LEN];
static PRESENT_POS: AtomicU64 = AtomicU64::new(0);
static LAST_PRESENT_US: AtomicU64 = AtomicU64::new(0);
/// True only while the Present detour is mounted. Lets the aggregator ignore
/// stale present samples after the overlay is closed and fall back to ticks.
static PRESENT_STREAM_ACTIVE: AtomicBool = AtomicBool::new(false);
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
static PAINT_SAMPLES: AtomicU64 = AtomicU64::new(0);
static PAINT_SUM_US: AtomicU64 = AtomicU64::new(0);
static PAINT_MAX_US: AtomicU64 = AtomicU64::new(0);
static UPLOAD_SUM_US: AtomicU64 = AtomicU64::new(0);
static UPLOAD_MAX_US: AtomicU64 = AtomicU64::new(0);

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

/// Record GDI paint and texture-upload costs from the paint worker.
pub fn record_paint_worker(paint_us: u64, upload_us: u64) {
    PAINT_SAMPLES.fetch_add(1, Ordering::Relaxed);
    PAINT_SUM_US.fetch_add(paint_us, Ordering::Relaxed);
    PAINT_MAX_US.fetch_max(paint_us, Ordering::Relaxed);
    UPLOAD_SUM_US.fetch_add(upload_us, Ordering::Relaxed);
    UPLOAD_MAX_US.fetch_max(upload_us, Ordering::Relaxed);
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

/// Track whether Present samples are currently meaningful. Mounting also
/// clears the inter-frame clock: otherwise the first Present after a long
/// parked interval would report the whole closed-editor period as one giant
/// LONG frame.
pub fn set_present_stream_active(active: bool) {
    PRESENT_STREAM_ACTIVE.store(active, Ordering::Release);
    LAST_PRESENT_US.store(0, Ordering::Release);
    PRESENT_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Called from the Present detour while the hook is enabled.
pub fn before_present() {
    let now = now_us();
    if now == 0 {
        return;
    }
    let prev = LAST_PRESENT_US.swap(now, Ordering::Relaxed);
    if prev != 0 {
        ring_push(&PRESENT_RING, &PRESENT_POS, now | sample_flags());
    }
}

//----------------------------------------------------------------------------
//  Collector + writer threads
//----------------------------------------------------------------------------

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
                "# Paper Jutsu frame timing: one summary per second (src=present|tick); LONG lines are individual frames >= {LONG_FRAME_MS} ms"
            );
            let _ = writeln!(
                file,
                "# time_s fps avg_ms p99_ms max_ms frames ui_frames menu_frames gt16.7 gt25 gt33 src hook_samples mod_avg_us mod_max_us orig_avg_us orig_max_us update_gap_max_ms ovl_avg_us ovl_max_us paint_avg_us paint_max_us upload_avg_us upload_max_us"
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
            flags.push(value & (UI_BIT | MENU_BIT));
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

fn aggregator_loop(sender: SyncSender<String>) {
    let mut tick = RingReader::new();
    let mut present = RingReader::new();
    let mut events = RingReader::new();
    let mut tick_ms: Vec<(f32, u64)> = Vec::with_capacity(1024);
    let mut present_ms: Vec<(f32, u64)> = Vec::with_capacity(1024);
    let mut tick_flags: Vec<u64> = Vec::with_capacity(1024);
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
        let present_start = present_ms.len();
        drain_stream(&TICK_RING, &TICK_POS, &mut tick, &mut tick_ms, &mut tick_flags);
        drain_stream(
            &PRESENT_RING,
            &PRESENT_POS,
            &mut present,
            &mut present_ms,
            &mut present_flags,
        );
        // LONG events are edge reports, so only inspect the slices drained now
        // rather than re-reporting the whole accumulated window every 100 ms.
        flush_long(&sender, "tick", &tick_ms[tick_start..]);
        flush_long(&sender, "present", &present_ms[present_start..]);

        let present_active = PRESENT_STREAM_ACTIVE.load(Ordering::Acquire);
        if !present_active {
            // Drop samples left over from the editor session so the next
            // one-second summary is based on the tick clock, not a partial
            // present window.
            present_ms.clear();
            present_flags.clear();
        }

        if window_started.elapsed() < WINDOW {
            continue;
        }
        let elapsed = window_started.elapsed().as_secs_f32().max(0.001);
        window_started = Instant::now();

        // Prefer the Present stream while the hook is mounted; otherwise the
        // tick stream is the authoritative pacing clock (the game main thread
        // ticks once per presented frame under vsync).
        let (deltas, flags, source) =
            if present_active && !present_ms.is_empty() && present_ms.len() >= tick_ms.len() / 2 {
                (present_ms.as_slice(), present_flags.as_slice(), "present")
            } else {
                (tick_ms.as_slice(), tick_flags.as_slice(), "tick")
            };
        if deltas.is_empty() {
            tick_ms.clear();
            present_ms.clear();
            tick_flags.clear();
            present_flags.clear();
            continue;
        }

        let count = deltas.len();
        let sum: f32 = deltas.iter().map(|&(ms, _)| ms).sum();
        let avg = sum / count as f32;
        let fps = count as f32 / elapsed;
        let max = deltas.iter().map(|&(ms, _)| ms).fold(0.0f32, f32::max);
        let over_16_7 = deltas.iter().filter(|&&(ms, _)| ms > 16.7).count();
        let over_25 = deltas.iter().filter(|&&(ms, _)| ms > 25.0).count();
        let over_33 = deltas.iter().filter(|&&(ms, _)| ms > 33.0).count();
        let ui_frames = flags.iter().filter(|&&f| f & UI_BIT != 0).count();
        let menu_frames = flags.iter().filter(|&&f| f & MENU_BIT != 0).count();

        sorted.clear();
        sorted.extend(deltas.iter().map(|&(ms, _)| ms));
        sorted.sort_by(|a, b| a.total_cmp(b));
        let p99_index = ((count as f32 * 0.99).ceil() as usize).saturating_sub(1).min(count - 1);
        let p99 = sorted[p99_index];

        let samples = INP_SAMPLES.swap(0, Ordering::Relaxed);
        let mod_sum = INP_MOD_SUM.swap(0, Ordering::Relaxed);
        let mod_max = INP_MOD_MAX.swap(0, Ordering::Relaxed);
        let orig_sum = INP_ORIG_SUM.swap(0, Ordering::Relaxed);
        let orig_max = INP_ORIG_MAX.swap(0, Ordering::Relaxed);
        let gap_max = INP_GAP_MAX_US.swap(0, Ordering::Relaxed);
        let ovl_samples = OVL_SAMPLES.swap(0, Ordering::Relaxed);
        let ovl_sum = OVL_SUM_US.swap(0, Ordering::Relaxed);
        let ovl_max = OVL_MAX_US.swap(0, Ordering::Relaxed);
        let paint_samples = PAINT_SAMPLES.swap(0, Ordering::Relaxed);
        let paint_sum = PAINT_SUM_US.swap(0, Ordering::Relaxed);
        let paint_max = PAINT_MAX_US.swap(0, Ordering::Relaxed);
        let upload_sum = UPLOAD_SUM_US.swap(0, Ordering::Relaxed);
        let upload_max = UPLOAD_MAX_US.swap(0, Ordering::Relaxed);
        let ovl_avg = if ovl_samples == 0 {
            0.0
        } else {
            ovl_sum as f64 / ovl_samples as f64
        };
        let paint_avg = if paint_samples == 0 {
            0.0
        } else {
            paint_sum as f64 / paint_samples as f64
        };
        let upload_avg = if paint_samples == 0 {
            0.0
        } else {
            upload_sum as f64 / paint_samples as f64
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
                "{secs:9.3}s fps={fps:6.1} avg_ms={avg:7.3} p99_ms={p99:7.3} max_ms={max:7.3} frames={count:4} ui_frames={ui_frames} menu_frames={menu_frames} gt16.7={over_16_7} gt25={over_25} gt33={over_33} src={source} hook_samples={samples} mod_avg_us={mod_avg:.1} mod_max_us={mod_max} orig_avg_us={orig_avg:.1} orig_max_us={orig_max} update_gap_max_ms={:.2} ovl_avg_us={ovl_avg:.1} ovl_max_us={ovl_max} paint_avg_us={paint_avg:.1} paint_max_us={paint_max} upload_avg_us={upload_avg:.1} upload_max_us={upload_max}",
                gap_max as f64 / 1000.0
            ),
        );
        tick_ms.clear();
        present_ms.clear();
        tick_flags.clear();
        present_flags.clear();
    }
}
