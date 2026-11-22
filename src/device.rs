use std::{error, fmt};

use gilrs::{Axis, EventType, Gilrs};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState, VIRTUAL_KEY};

pub fn is_key_down(keycode: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(keycode.0.into()) as u16 & 0x8000 != 0 }
}

fn is_key_async_down(keycode: u16) -> bool {
    unsafe { GetAsyncKeyState(keycode as i32) as u16 & 0x8000 != 0 }
}

/// Some game action bits (notably jump/crouch) are short press pulses rather
/// than held states. Learn which physical key produced each action edge so the
/// mod can keep using that action as "held" without hard-coding Space/Ctrl.
/// Triggering still comes from the game's logical action bit.
pub struct KeyHoldTracker {
    down: [bool; 256],
    mapped: [[u64; 4]; 3],
}

impl KeyHoldTracker {
    pub fn new() -> Self {
        Self {
            down: [false; 256],
            mapped: [[0; 4]; 3],
        }
    }

    /// Refresh physical key state and return the keys that became down this frame.
    pub fn update(&mut self) -> [u64; 4] {
        let mut newly_down = [0u64; 4];
        for vk in 8u16..=254 {
            let down = is_key_async_down(vk);
            if down && !self.down[vk as usize] {
                newly_down[(vk >> 6) as usize] |= 1u64 << (vk & 63);
            }
            self.down[vk as usize] = down;
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
        (8usize..=254).any(|vk| {
            let bit = 1u64 << (vk & 63);
            mapped[vk >> 6] & bit != 0 && self.down[vk]
        })
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
