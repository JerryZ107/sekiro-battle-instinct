use std::{
    error, fmt,
    time::{Duration, Instant},
};

use gilrs::{Axis, EventType, Gilrs};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY};

pub fn is_key_down(keycode: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(keycode.0.into()) as u16 & 0x8000 != 0 }
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
