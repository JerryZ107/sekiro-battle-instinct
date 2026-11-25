//! Off-Present overlay painter.
//!
//! The Present detour must stay as close as possible to a single atomic load +
//! Draw. This worker owns the GDI DIB, takes a cheap snapshot of `UiState` when
//! the UI changes, paints into CPU memory, and uploads the result through the
//! dynamic overlay texture. The Present thread only reads the published draw
//! plan and submits one textured quad.

use std::{
    sync::{
        LazyLock, Mutex, TryLockError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};

use super::{
    render::{UiPainter, panel_geometry},
    state,
};

const PLAN_COMPONENT_MASK: u64 = 0xffff;
const RETRY_DELAY: Duration = Duration::from_millis(8);

static STARTED: AtomicBool = AtomicBool::new(false);
static PAINT_DIRTY: AtomicBool = AtomicBool::new(false);
static DRAW_PENDING: AtomicBool = AtomicBool::new(false);
static DRAW_PLAN: AtomicU64 = AtomicU64::new(0);
static FRAME_SIZE: AtomicU64 = AtomicU64::new(0);
static SESSION: AtomicU64 = AtomicU64::new(0);
static FIRST_UPLOAD_LOGGED: AtomicBool = AtomicBool::new(false);
static WAKE: LazyLock<(Mutex<()>, std::sync::Condvar)> = LazyLock::new(|| (Mutex::new(()), std::sync::Condvar::new()));

/// Published panel rectangle. Screen dimensions come from the current swap
/// chain description on the Present side.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DrawPlan {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

impl DrawPlan {
    fn pack(self) -> u64 {
        let x = self.x.round().clamp(0.0, PLAN_COMPONENT_MASK as f32) as u64;
        let y = self.y.round().clamp(0.0, PLAN_COMPONENT_MASK as f32) as u64;
        let w = self.w.round().clamp(0.0, PLAN_COMPONENT_MASK as f32) as u64;
        let h = self.h.round().clamp(0.0, PLAN_COMPONENT_MASK as f32) as u64;
        x | (y << 16) | (w << 32) | (h << 48)
    }

    fn unpack(value: u64) -> Self {
        Self {
            x: (value & PLAN_COMPONENT_MASK) as f32,
            y: ((value >> 16) & PLAN_COMPONENT_MASK) as f32,
            w: ((value >> 32) & PLAN_COMPONENT_MASK) as f32,
            h: ((value >> 48) & PLAN_COMPONENT_MASK) as f32,
        }
    }
}

fn pack_size(width: u32, height: u32) -> u64 {
    (width as u64) << 32 | height as u64
}

fn unpack_size(value: u64) -> Option<(i32, i32)> {
    if value == 0 {
        return None;
    }
    Some(((value >> 32) as i32, (value & 0xffff_ffff) as i32))
}

pub(crate) fn start() -> Result<()> {
    if STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let result = thread::Builder::new()
        .name("battle-instinct-paint".into())
        .spawn(paint_loop);
    if result.is_err() {
        STARTED.store(false, Ordering::Release);
    } else {
        log::warn!("ui: overlay paint worker started");
    }
    result.map(|_| ()).map_err(Into::into)
}

/// Called by the UI thread when the editor opens. The stale texture from a
/// previous session must never be presented before the new paint is uploaded.
pub(crate) fn begin_session() {
    SESSION.fetch_add(1, Ordering::AcqRel);
    DRAW_PENDING.store(false, Ordering::Release);
    FRAME_SIZE.store(0, Ordering::Release);
    request_repaint();
}

/// Called by the UI thread when the editor closes.
pub(crate) fn end_session() {
    SESSION.fetch_add(1, Ordering::AcqRel);
    DRAW_PENDING.store(false, Ordering::Release);
    PAINT_DIRTY.store(false, Ordering::Release);
}

/// Invalidate and wake the paint worker.
pub(crate) fn request_repaint() {
    // Update the flag and notify while holding the condvar mutex. Without
    // this handshake a request can slip between the worker's flag check and
    // its wait, leaving the overlay permanently stale.
    let (lock, cv) = &*WAKE;
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    PAINT_DIRTY.store(true, Ordering::Release);
    cv.notify_one();
}

/// Drop a previously uploaded texture because the swap chain/device changed.
pub(crate) fn reset_draw_ready() {
    DRAW_PENDING.store(false, Ordering::Release);
    DRAW_PLAN.store(0, Ordering::Release);
}

/// The Present thread reads this without taking any lock. `None` means there
/// is no completely uploaded texture to draw yet.
pub(crate) fn draw_plan() -> Option<DrawPlan> {
    if !DRAW_PENDING.load(Ordering::Acquire) {
        return None;
    }
    Some(DrawPlan::unpack(DRAW_PLAN.load(Ordering::Acquire)))
}

/// Present-side frame size hint, used when the game resizes or switches
/// display mode while the editor is open.
pub(crate) fn note_frame_size(width: u32, height: u32) -> bool {
    let packed = pack_size(width.max(1), height.max(1));
    let changed = FRAME_SIZE.swap(packed, Ordering::AcqRel) != packed;
    if changed {
        request_repaint();
    }
    changed
}

pub(crate) fn frame_size() -> Option<(i32, i32)> {
    unpack_size(FRAME_SIZE.load(Ordering::Acquire))
}

fn lock_state_for_snapshot() -> Option<(super::editor::Editor, i32, i32)> {
    let state = match state().try_lock() {
        Ok(state) => state,
        Err(TryLockError::WouldBlock) => return None,
        Err(TryLockError::Poisoned(poisoned)) => {
            log::error!("ui: UI state mutex poisoned in paint worker; recovering");
            poisoned.into_inner()
        }
    };
    let editor = state.editor.clone()?;
    let (frame_w, frame_h) = frame_size().unwrap_or((state.frame_width.max(1), state.frame_height.max(1)));
    Some((editor, frame_w, frame_h))
}

fn paint_loop() {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut painter = UiPainter::new();
    let mut last_error = Instant::now() - Duration::from_secs(10);

    loop {
        {
            let (lock, cv) = &*WAKE;
            let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            while !PAINT_DIRTY.swap(false, Ordering::AcqRel) {
                guard = cv.wait(guard).unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }

        let session = SESSION.load(Ordering::Acquire);
        let Some((editor, frame_w, frame_h)) = lock_state_for_snapshot() else {
            if super::overlay_open() {
                // The UI thread briefly holds the state lock; try again shortly.
                PAINT_DIRTY.store(true, Ordering::Release);
                thread::sleep(Duration::from_millis(1));
            } else {
                DRAW_PENDING.store(false, Ordering::Release);
            }
            continue;
        };

        let (panel_w, panel_h, panel_x, panel_y) = panel_geometry(&editor, frame_w, frame_h);
        if panel_w <= 0 || panel_h <= 0 {
            DRAW_PENDING.store(false, Ordering::Release);
            continue;
        }

        let paint_started = Instant::now();
        let paint_result = painter.paint(&editor, panel_w, panel_h);
        let paint_us = paint_started.elapsed().as_micros() as u64;
        let (result, upload_us) = match paint_result {
            Ok(()) => {
                let dib = match painter.dib() {
                    Some(dib) => dib,
                    None => {
                        crate::frame_perf::record_paint_worker(paint_us, 0);
                        log::error!("ui: paint worker produced no DIB");
                        continue;
                    }
                };
                let upload_started = Instant::now();
                let result = super::game_overlay::upload_overlay_texture(dib, (panel_w as u32, panel_h as u32));
                (result, upload_started.elapsed().as_micros() as u64)
            }
            Err(error) => (Err(error), 0),
        };
        crate::frame_perf::record_paint_worker(paint_us, upload_us);

        match result {
            Ok(()) => {
                if !FIRST_UPLOAD_LOGGED.swap(true, Ordering::AcqRel) {
                    log::warn!("ui: overlay texture uploaded ({}x{})", panel_w, panel_h);
                }
                // The editor may have closed while the worker was painting.
                // Never publish a stale texture into a newer/closed session.
                if SESSION.load(Ordering::Acquire) == session && super::overlay_open() {
                    DRAW_PLAN.store(
                        DrawPlan {
                            x: panel_x,
                            y: panel_y,
                            w: panel_w as f32,
                            h: panel_h as f32,
                        }
                        .pack(),
                        Ordering::Release,
                    );
                    DRAW_PENDING.store(true, Ordering::Release);
                }
            }
            Err(error) => {
                // Resources may not exist until the first Present frame binds
                // the current swap chain. Retry shortly without spamming logs.
                if last_error.elapsed() >= Duration::from_secs(1) {
                    log::warn!("ui: paint worker upload deferred: {error:#}");
                    last_error = Instant::now();
                }
                PAINT_DIRTY.store(true, Ordering::Release);
                thread::sleep(RETRY_DELAY);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_plan_round_trips_panel_geometry() {
        let plan = DrawPlan {
            x: 1920.0,
            y: 180.0,
            w: 820.0,
            h: 900.0,
        };
        let decoded = DrawPlan::unpack(plan.pack());
        assert_eq!(decoded.x, plan.x);
        assert_eq!(decoded.y, plan.y);
        assert_eq!(decoded.w, plan.w);
        assert_eq!(decoded.h, plan.h);
    }

    #[test]
    fn frame_size_zero_means_unset() {
        assert_eq!(unpack_size(0), None);
        assert_eq!(unpack_size(pack_size(2560, 1440)), Some((2560, 1440)));
    }
}
