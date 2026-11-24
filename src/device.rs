use std::{
    error, fmt,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use gilrs::{Axis, EventType, Gilrs};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY};

/// Low-level keyboard events are captured by the UI thread's WH_KEYBOARD_LL
/// hook and published here. Reading this bitmap is much cheaper than polling
/// ~247 virtual-key states on every game tick.
static PHYSICAL_KEY_BITS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static KEYBOARD_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn record_key_event(vk: u16, down: bool) {
    if !(8..=254).contains(&vk) {
        return;
    }
    let word = &PHYSICAL_KEY_BITS[(vk >> 6) as usize];
    let bit = 1u64 << (vk & 63);
    if down {
        word.fetch_or(bit, Ordering::Relaxed);
    } else {
        word.fetch_and(!bit, Ordering::Relaxed);
    }
}

pub fn set_keyboard_capture_active(active: bool) {
    KEYBOARD_CAPTURE_ACTIVE.store(active, Ordering::Release);
}

fn keyboard_capture_active() -> bool {
    KEYBOARD_CAPTURE_ACTIVE.load(Ordering::Acquire)
}

pub fn is_key_down(keycode: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(keycode.0.into()) as u16 & 0x8000 != 0 }
}

/// Some game action bits (notably jump/crouch) are short press pulses rather
/// than held states. Learn which physical key produced each action edge so the
/// mod can keep using that action as "held" without hard-coding Space/Ctrl.
/// Triggering still comes from the game's logical action bit.
pub struct KeyHoldTracker {
    down: [u64; 4],
    mapped: [[u64; 4]; 3],
}

impl KeyHoldTracker {
    pub fn new() -> Self {
        Self {
            down: [0; 4],
            mapped: [[0; 4]; 3],
        }
    }

    /// Refresh physical key state and return the keys that became down this frame.
    pub fn update(&mut self) -> [u64; 4] {
        if !keyboard_capture_active() {
            // No polling fallback: the UI layer reports the failed hook once
            // and leaves the key-hold extension disabled.
            return self.update_from_snapshot([0; 4]);
        }
        let snapshot = [
            PHYSICAL_KEY_BITS[0].load(Ordering::Relaxed),
            PHYSICAL_KEY_BITS[1].load(Ordering::Relaxed),
            PHYSICAL_KEY_BITS[2].load(Ordering::Relaxed),
            PHYSICAL_KEY_BITS[3].load(Ordering::Relaxed),
        ];
        self.update_from_snapshot(snapshot)
    }

    fn update_from_snapshot(&mut self, snapshot: [u64; 4]) -> [u64; 4] {
        let mut newly_down = [0u64; 4];
        for index in 0..4 {
            newly_down[index] = snapshot[index] & !self.down[index];
            self.down[index] = snapshot[index];
        }
        newly_down
    }

    /// Remember the key(s) that caused an action edge. Empty edge sets (for
    /// example a gamepad button) leave the previous keyboard mapping intact.
    pub fn learn(&mut self, slot: usize, newly_down: [u64; 4]) {
        if newly_down.iter().any(|bits| *bits != 0) {
            self.mapped[slot] = newly_down;
        }
    }

    pub fn held(&self, slot: usize) -> bool {
        let mapped = &self.mapped[slot];
        (0..4).any(|word| mapped[word] & self.down[word] != 0)
    }
}

pub struct Gamepad {
    girls: Gilrs,
    connected: bool,
    left_pos: (f32, f32),
}

impl Gamepad {
    pub fn new() -> Result<Self, Error> {
        let girls = Gilrs::new()?;
        let connected = girls.gamepads().next().is_some();
        let gamepad = Self {
            girls,
            connected,
            left_pos: (0.0, 0.0),
        };
        Ok(gamepad)
    }

    pub fn get_left_pos(&mut self) -> Option<(f32, f32)> {
        while let Some(event) = self.girls.next_event() {
            match event.event {
                EventType::Connected => self.connected = true,
                EventType::Disconnected => self.connected = self.girls.gamepads().next().is_some(),
                EventType::AxisChanged(Axis::LeftStickX, value, _code) => self.left_pos.0 = value,
                EventType::AxisChanged(Axis::LeftStickY, value, _code) => self.left_pos.1 = value,
                _ => (),
            }
        }
        if self.connected { Some(self.left_pos) } else { None }
    }
}

#[derive(Debug)]
pub enum Error {
    NotImplemented,
    InvalidAxisToBtn,
    Other(#[allow(unused)] Box<dyn error::Error + Send + Sync + 'static>),
}

impl error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotImplemented => f.write_str("Gilrs does not support current platform."),
            Error::InvalidAxisToBtn => {
                f.write_str("Either `pressed ≤ released` or one of values is outside [0.0, 1.0] range.")
            }
            Error::Other(e) => e.fmt(f),
        }
    }
}

impl From<gilrs::Error> for Error {
    fn from(value: gilrs::Error) -> Self {
        match value {
            gilrs::Error::NotImplemented(_dummy) => Self::NotImplemented,
            gilrs::Error::InvalidAxisToBtn => Self::InvalidAxisToBtn,
            gilrs::Error::Other(error) => Self::Other(error),
            _ => todo!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_events_drive_key_hold_tracking_without_async_polling() {
        set_keyboard_capture_active(true);
        let mut tracker = KeyHoldTracker::new();
        let vk = 0x41u16; // A

        record_key_event(vk, true);
        let newly_down = tracker.update();
        assert_ne!(newly_down[(vk >> 6) as usize] & (1u64 << (vk & 63)), 0);
        tracker.learn(0, newly_down);
        assert!(tracker.held(0));

        record_key_event(vk, false);
        tracker.update();
        assert!(!tracker.held(0));

        // With the hook unavailable the feature is intentionally disabled;
        // there must be no hidden GetAsyncKeyState polling fallback.
        set_keyboard_capture_active(false);
        record_key_event(vk, true);
        assert_eq!(tracker.update(), [0; 4]);
        assert!(!tracker.held(0));
    }
}
