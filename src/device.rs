use std::{
    error, fmt,
    time::{Duration, Instant},
};

use gilrs::{Axis, EventType, Gilrs};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState, VIRTUAL_KEY};

pub fn is_key_down(keycode: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(keycode.0.into()) as u16 & 0x8000 != 0 }
}

fn is_key_async_down(keycode: u16) -> bool {
    unsafe { GetAsyncKeyState(keycode as i32) as u16 & 0x8000 != 0 }
}

/// Some game action bits are short press pulses rather than held states.
/// Learn the physical key that produced an action edge so that a combo can
/// still verify that the first key remains held while the tail is pressed.
pub struct KeyHoldTracker {
    down: [u64; 4],
    mapped: [u64; 4],
}

impl KeyHoldTracker {
    pub const fn new() -> Self {
        Self {
            down: [0; 4],
            mapped: [0; 4],
        }
    }

    /// Refresh all physical key states and return keys that became down.
    pub fn update(&mut self) -> [u64; 4] {
        let mut snapshot = [0u64; 4];
        for vk in 8u16..=254 {
            if is_key_async_down(vk) {
                snapshot[(vk >> 6) as usize] |= 1u64 << (vk & 63);
            }
        }
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

    /// Remember the physical key(s) learned at a game-action edge. An empty
    /// edge set (for example a gamepad button) keeps the previous mapping.
    pub fn learn(&mut self, newly_down: [u64; 4]) {
        if newly_down.iter().any(|bits| *bits != 0) {
            self.mapped = newly_down;
        }
    }

    pub fn held(&self) -> bool {
        (0..4).any(|word| self.mapped[word] & self.down[word] != 0)
    }

    pub fn clear(&mut self) {
        self.down = [0; 4];
        self.mapped = [0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::KeyHoldTracker;

    fn key_bit(vk: u16) -> [u64; 4] {
        let mut bits = [0u64; 4];
        bits[(vk >> 6) as usize] = 1u64 << (vk & 63);
        bits
    }

    #[test]
    fn learned_key_tracks_physical_hold_and_release() {
        let mut tracker = KeyHoldTracker::new();
        let ctrl = key_bit(0x11);

        let newly_down = tracker.update_from_snapshot(ctrl);
        tracker.learn(newly_down);
        assert!(tracker.held());

        tracker.update_from_snapshot([0; 4]);
        assert!(!tracker.held());

        tracker.update_from_snapshot(ctrl);
        assert!(tracker.held());
    }

    #[test]
    fn gamepad_edges_keep_the_previous_keyboard_mapping() {
        let mut tracker = KeyHoldTracker::new();
        let ctrl = key_bit(0x11);
        let newly_down = tracker.update_from_snapshot(ctrl);
        tracker.learn(newly_down);
        tracker.learn([0; 4]);
        assert!(tracker.held());
    }
}

const RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(500);

pub struct Gamepad {
    girls: Gilrs,
    connected: bool,
    left_pos: (f32, f32),
    last_reconnect_poll: Instant,
}

impl Gamepad {
    pub fn new() -> Result<Self, Error> {
        let girls = Gilrs::new()?;
        let connected = girls.gamepads().next().is_some();
        let gamepad = Self {
            girls,
            connected,
            left_pos: (0.0, 0.0),
            last_reconnect_poll: Instant::now(),
        };
        Ok(gamepad)
    }

    pub fn get_left_pos(&mut self) -> Option<(f32, f32)> {
        // Keyboard-only players should not pay for a full HID event pump every
        // frame. When no pad is connected, poll for reconnects at a low rate;
        // once connected, keep draining events normally.
        if !self.connected {
            if self.last_reconnect_poll.elapsed() < RECONNECT_POLL_INTERVAL {
                return None;
            }
            self.last_reconnect_poll = Instant::now();
            while let Some(event) = self.girls.next_event() {
                if matches!(event.event, EventType::Connected) {
                    self.connected = true;
                }
            }
            if !self.connected {
                return None;
            }
        }

        while let Some(event) = self.girls.next_event() {
            match event.event {
                EventType::Connected => self.connected = true,
                EventType::Disconnected => {
                    self.connected = self.girls.gamepads().next().is_some();
                    self.last_reconnect_poll = Instant::now();
                }
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
