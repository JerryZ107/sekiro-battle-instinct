use std::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU16, Ordering},
    time::Instant,
};

pub const DEFAULT_FRAMERATE: u16 = 60;

/// Frame count under the standard FPS as a time unit, namely, 1/60s.
/// It can be adjusted to the current framerate
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Frames(u16);
impl Frames {
    #[inline(always)]
    pub const fn standard(value: u16) -> Frames {
        Frames(value)
    }
    #[inline(always)]
    pub fn as_actual(self) -> u16 {
        self.0 * FRAMERATE.cur() / DEFAULT_FRAMERATE
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
        if self.is_freezed() {
            return;
        }

        let since = *self.since.get_or_insert_with(Instant::now);
        self.frames += 1;
        if self.frames < Self::SAMPLE_SIZE {
            return;
        }

        let now = Instant::now();
        let elapsed = now - since;
        let measured = (self.frames as u128 * 1000 / elapsed.as_millis()) as u16;
        cur.store(measured, Ordering::Relaxed);
        self.frames = 0;
        self.since = Some(now);
        self.samples = self.samples.saturating_add(1);

        if measured > DEFAULT_FRAMERATE + 5 {
            self.unlocked = true;
        }

        if self.samples == Self::SAMPLE_COUNT {
            if self.unlocked {
                log::debug!("Framerate is unlocked.");
            } else {
                cur.store(DEFAULT_FRAMERATE, Ordering::Relaxed);
                log::debug!("Framerate is freezed.");
            }
        } else if self.samples < Self::SAMPLE_COUNT {
            log::trace!("Framerate: {}", measured);
            return;
        }
    }

    fn is_freezed(&self) -> bool {
        self.samples >= Self::SAMPLE_COUNT && !self.unlocked
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
}
