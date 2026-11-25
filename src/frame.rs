use std::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU16, Ordering},
    time::Instant,
};

pub const DEFAULT_FRAMERATE: u16 = 60;

/// Frame count under the standard FPS as a time unit, namely, 1/60s.
/// It can be adjusted to the current framerate.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frames(u16);

impl Frames {
    #[inline(always)]
    pub const fn standard(value: u16) -> Frames {
        Frames(value)
    }

    /// Convert a 60 fps frame budget to `framerate` fps.
    ///
    /// The arithmetic is done in `u32` and saturated: a long budget such as
    /// `240` frames at a 300 fps loading-screen rate would otherwise overflow
    /// `u16` and silently wrap in release builds.
    #[inline(always)]
    pub const fn actual_at(self, framerate: u16) -> u16 {
        let framerate = if framerate < DEFAULT_FRAMERATE {
            DEFAULT_FRAMERATE
        } else {
            framerate
        } as u32;
        let scaled = (self.0 as u32 * framerate + DEFAULT_FRAMERATE as u32 / 2) / DEFAULT_FRAMERATE as u32;
        if scaled > u16::MAX as u32 {
            u16::MAX
        } else {
            scaled as u16
        }
    }

    #[inline(always)]
    pub fn as_actual(self) -> u16 {
        self.actual_at(FRAMERATE.cur())
    }

    #[allow(unused)]
    #[inline(always)]
    pub const fn as_standard(self) -> u16 {
        self.0
    }
}

/// A tracker that tracks the in-game framerate.
pub struct Framerate {
    /// Mutable sampling state. Only the game's main tick may access this.
    inner: UnsafeCell<FramerateInner>,
    /// Readable from any thread without creating an aliasing `&mut`.
    cur: AtomicU16,
}

// The mutable inner state is only touched from the game's main tick. `cur` is
// atomic so safe readers never alias the mutable sampling state.
unsafe impl Sync for Framerate {}

impl Framerate {
    const fn new() -> Framerate {
        Framerate {
            inner: UnsafeCell::new(FramerateInner::new()),
            cur: AtomicU16::new(DEFAULT_FRAMERATE),
        }
    }

    #[inline(always)]
    pub fn cur(&self) -> u16 {
        self.cur.load(Ordering::Relaxed).max(DEFAULT_FRAMERATE)
    }

    /// Safety: this function should only be hooked into the main tick of the game, but never
    /// called directly from arbitrary threads
    #[inline(always)]
    pub unsafe fn tick(&self) {
        unsafe { (*self.inner.get()).tick(&self.cur) }
    }
}

struct FramerateInner {
    // passing frames since `since`
    frames: u16,
    since: Option<Instant>,
    // stores how many framerate value is sampled
    samples: u16,
    unlocked: bool,
}

impl FramerateInner {
    const SAMPLE_SIZE: u16 = 60;
    const SAMPLE_COUNT: u16 = 30;

    const fn new() -> FramerateInner {
        FramerateInner {
            frames: 0,
            samples: 0,
            unlocked: false,
            since: None,
        }
    }

    fn tick(&mut self, cur: &AtomicU16) {
        let since = *self.since.get_or_insert_with(Instant::now);
        self.frames += 1;
        if self.frames < Self::SAMPLE_SIZE {
            return;
        }

        let now = Instant::now();
        let elapsed = now.saturating_duration_since(since);
        self.frames = 0;
        self.since = Some(now);
        self.samples = self.samples.saturating_add(1);

        // Ignore windows that clearly contain a pause/loading gap. Without
        // this, a menu pause would be measured as a handful of fps and could
        // briefly shrink every frame-based window.
        let expected = Self::SAMPLE_SIZE as f64 / cur.load(Ordering::Relaxed).max(1) as f64;
        // The 2 s floor is important: after measuring 120/300 fps, a normal
        // 60 fps window is ~1 s and must still be allowed to downgrade `cur`.
        if elapsed.as_secs_f64() > (expected * 3.0).max(2.0) {
            log::trace!("Framerate: skipped {:.3}s sampling gap", elapsed.as_secs_f64());
            return;
        }

        // `as_secs_f64` keeps sub-millisecond precision; the old
        // `as_millis()` path truncated to 0 and could divide by zero.
        let measured = if elapsed.as_secs_f64() <= f64::EPSILON {
            DEFAULT_FRAMERATE
        } else {
            ((Self::SAMPLE_SIZE as f64 / elapsed.as_secs_f64()).round() as u16).max(1)
        };
        // Treat 1..=65 fps as the normal 60 fps cap. Only a genuinely higher
        // rate changes frame-based timings.
        let effective = if measured > DEFAULT_FRAMERATE + 5 {
            measured
        } else {
            DEFAULT_FRAMERATE
        };
        cur.store(effective, Ordering::Relaxed);

        let was_unlocked = self.unlocked;
        if measured > DEFAULT_FRAMERATE + 5 {
            self.unlocked = true;
        }

        if self.samples == Self::SAMPLE_COUNT {
            if self.unlocked {
                log::debug!("Framerate is unlocked.");
            } else {
                // Normal 60 fps play. Keep sampling instead of freezing
                // forever: an unlocker can be toggled mid-session.
                log::debug!("Framerate is freezed at 60.");
            }
        } else if self.samples < Self::SAMPLE_COUNT {
            log::trace!("Framerate: {measured}");
        }

        if !was_unlocked && self.unlocked {
            log::debug!("Framerate became unlocked at {measured} fps.");
        }
    }
}

/// The global framerate tracker
pub static FRAMERATE: Framerate = Framerate::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_framerate_is_safe_to_read_from_multiple_threads() {
        let framerate = Framerate::new();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| assert_eq!(framerate.cur(), DEFAULT_FRAMERATE));
            }
        });
    }

    #[test]
    fn frames_scale_to_actual_framerate() {
        assert_eq!(Frames::standard(18).actual_at(60), 18);
        assert_eq!(Frames::standard(18).actual_at(120), 36);
        assert_eq!(Frames::standard(18).actual_at(144), 43);
        assert_eq!(Frames::standard(18).actual_at(300), 90);
        assert_eq!(Frames::standard(42).actual_at(144), 101);
        assert_eq!(Frames::standard(240).actual_at(300), 1200);
    }

    #[test]
    fn frame_scaling_saturates_instead_of_overflowing() {
        assert_eq!(Frames::standard(u16::MAX).actual_at(300), u16::MAX);
        assert_eq!(Frames::standard(0).actual_at(300), 0);
        // Below 60 fps the mod keeps the 60 fps budget instead of shrinking it.
        assert_eq!(Frames::standard(18).actual_at(30), 18);
    }
}
