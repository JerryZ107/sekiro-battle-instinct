//! CPU-only overlay painter.
//!
//! The worker never touches D3D11. It paints the panel into a GDI DIB, copies
//! it into a packed BGRA buffer, and publishes the newest generation. The
//! Present thread uploads that buffer only when the generation changes.

use std::{
    sync::{
        LazyLock, Mutex, TryLockError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::Result;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};

use super::{
    overlay_render::PaintFrame,
    render::{UiPainter, panel_geometry},
    state,
};

static STARTED: AtomicBool = AtomicBool::new(false);
static PAINT_DIRTY: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static SESSION: AtomicU64 = AtomicU64::new(0);
static LATEST: LazyLock<Mutex<Option<PaintFrame>>> = LazyLock::new(|| Mutex::new(None));
static WAKE: LazyLock<(Mutex<()>, std::sync::Condvar)> = LazyLock::new(|| (Mutex::new(()), std::sync::Condvar::new()));

pub(crate) fn start() -> Result<()> {
    if STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let result = thread::Builder::new()
        .name("paper-jutsu-paint".into())
        .spawn(paint_loop);
    if result.is_err() {
        STARTED.store(false, Ordering::Release);
    } else {
        log::warn!("ui: CPU overlay painter started");
    }
    result.map(|_| ()).map_err(Into::into)
}

pub(crate) fn request_repaint() {
    let (lock, cv) = &*WAKE;
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    PAINT_DIRTY.store(true, Ordering::Release);
    cv.notify_one();
}

pub(crate) fn clear() {
    SESSION.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut latest) = LATEST.lock() {
        *latest = None;
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

pub(crate) fn latest_generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub(crate) fn take_latest() -> Option<PaintFrame> {
    LATEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
}

fn snapshot() -> Option<(super::editor::Editor, i32, i32)> {
    let state = match state().try_lock() {
        Ok(state) => state,
        Err(TryLockError::WouldBlock) => return None,
        Err(TryLockError::Poisoned(poisoned)) => {
            log::error!("ui: UI state mutex poisoned in painter; recovering");
            poisoned.into_inner()
        }
    };
    let editor = state.editor.clone()?;
    let (frame_w, frame_h) = super::frame_size();
    let (frame_w, frame_h) = if frame_w > 1 && frame_h > 1 {
        (frame_w, frame_h)
    } else {
        (state.frame_width.max(1), state.frame_height.max(1))
    };
    Some((editor, frame_w, frame_h))
}

fn paint_loop() {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut painter = UiPainter::new();
    loop {
        {
            let (lock, cv) = &*WAKE;
            let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            while !PAINT_DIRTY.swap(false, Ordering::AcqRel) {
                guard = cv.wait(guard).unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }

        let session = SESSION.load(Ordering::Acquire);
        let Some((editor, frame_w, frame_h)) = snapshot() else {
            if super::overlay_open() {
                PAINT_DIRTY.store(true, Ordering::Release);
                thread::sleep(Duration::from_millis(1));
            }
            continue;
        };

        let (panel_w, panel_h, panel_x, panel_y) = panel_geometry(&editor, frame_w, frame_h);
        if panel_w <= 0 || panel_h <= 0 {
            continue;
        }
        if let Err(error) = painter.paint(&editor, panel_w, panel_h) {
            log::error!("ui: CPU painter failed: {error:#}");
            continue;
        }
        let Some((pixels, width, height)) = painter.copy_pixels() else {
            continue;
        };
        if SESSION.load(Ordering::Acquire) != session || !super::overlay_open() {
            continue;
        }
        let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
        let frame = PaintFrame {
            generation,
            width: width.max(1) as u32,
            height: height.max(1) as u32,
            panel_x,
            panel_y,
            panel_w: panel_w as f32,
            panel_h: panel_h as f32,
            pixels,
        };
        *LATEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(frame);
    }
}
