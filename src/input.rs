use std::fmt::Debug;

use Input::*;

use crate::frame::Frames;

// buffer behavior
const MAX_INTERVAL: Frames = Frames::standard(10);
const MAX_DELAY: Frames = Frames::standard(10);
const MAX_DELAY_FOR_SINGLE_INPUT: Frames = Frames::standard(2);
// joystick ergonomics
const COMMON_THRESHOLD: f32 = 0.85;
const ROTATE_THRESHOLD: f32 = 0.9;
const BOUNCE_THRESHOLD: f32 = 0.4;

//----------------------------------------------------------------------------
//
//  An input buffer that remembers the most recent 3 motion inputs
//  The buffer expires after several frames unless new inputs are pushed into it and reset its age
//
//----------------------------------------------------------------------------
pub struct InputBuffer {
    inputs: Inputs,
    age: u16,
    neutral: bool,
    keys_down: [bool; 4],
}

impl InputBuffer {
    pub const fn new() -> InputBuffer {
        InputBuffer {
            inputs: Inputs::new(),
            age: 0,
            neutral: true,
            keys_down: [false; 4],
        }
    }

    pub fn update_keys(&mut self, up: bool, right: bool, down: bool, left: bool) -> Inputs {
        let mut updated = false;
        for (i, (down, input)) in [(up, Up), (right, Right), (down, Down), (left, Left)]
            .into_iter()
            .enumerate()
        {
            if !self.keys_down[i] && down {
                // newly pressed key
                self.push(input);
                updated = true;
            }
            self.keys_down[i] = down;
        }
        self.age(updated);
        self.inputs
    }

    pub fn update_joystick(&mut self, x: f32, y: f32) -> Inputs {
        let mut updated = false;
        let x_abs = x.abs();
        let y_abs = y.abs();

        let input = if y_abs >= x_abs {
            if y > 0.0 { Up } else { Down }
        } else {
            if x > 0.0 { Right } else { Left }
        };

        // using chebyshev distance means we have a square-shaped neutral zone
        let distance = f32::max(x_abs, y_abs);
        let threshold = if let Some(last) = self.inputs.last() {
            if input == last {
                COMMON_THRESHOLD
            } else if input == last.opposite() {
                // makes bouncing inputs (↑↓, ↓↑, ←→, →←) easier by using a smaller threshold
                BOUNCE_THRESHOLD
            } else {
                // makes rotating inputs (↑→, →↓, ↓←, ←↑) HARDER by using a bigger threshold
                ROTATE_THRESHOLD
            }
        } else {
            COMMON_THRESHOLD
        };

        if distance < threshold {
            self.neutral = true;
        } else {
            if self.neutral || self.inputs.last().into_iter().any(|last| input != last) {
                self.push(input);
                updated = true;
            }
            self.neutral = false;
        }

        self.age(updated);
        self.inputs.clone()
    }

    fn push(&mut self, input: Input) {
        if self.inputs.len() >= Inputs::CAP || self.age > MAX_INTERVAL.as_actual() {
            self.inputs.clear();
        }
        self.inputs.push(input);
    }

    fn age(&mut self, updated: bool) {
        if updated {
            self.age = 0;
        } else {
            self.age = self.age.saturating_add(1);
        }
    }

    pub fn expired(&self) -> bool {
        if self.inputs.len() == 1 {
            self.age >= MAX_DELAY_FOR_SINGLE_INPUT.as_actual() && self.released()
        } else {
            self.age >= MAX_DELAY.as_actual()
        }
    }

    fn released(&self) -> bool {
        self.neutral && self.keys_down == [false, false, false, false]
    }

    pub fn clear(&mut self) {
        self.inputs.clear();
        self.age = 0;
    }
}

//----------------------------------------------------------------------------
//
//  The input enum.
//
//----------------------------------------------------------------------------
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum Input {
    Up = 0,
    Right = 1,
    Down = 2,
    Left = 3,
}

impl Input {
    #[inline(always)]
    pub fn opposite(self) -> Input {
        Input::from_repr((self.as_repr() + 2) % 4)
    }

    #[inline(always)]
    pub fn rotate(self) -> Input {
        Input::from_repr((self.as_repr() + 1) % 4)
    }

    #[inline(always)]
    fn from_repr(repr: u8) -> Input {
        match repr {
            0 => Up,
            1 => Right,
            2 => Down,
            3 => Left,
            _ => panic!("Illegal representation {repr}."),
        }
    }

    #[inline(always)]
    fn as_repr(self) -> u8 {
        self as u8
    }
}

impl TryFrom<char> for Input {
    type Error = ();
    fn try_from(value: char) -> Result<Self, Self::Error> {
        match value.to_ascii_uppercase() {
            '↑' | 'U' => Ok(Up),
            '→' | 'R' => Ok(Right),
            '↓' | 'D' => Ok(Down),
            '←' | 'L' => Ok(Left),
            _ => Err(()),
        }
    }
}

impl Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Up => write!(f, "↑"),
            Self::Right => write!(f, "→"),
            Self::Down => write!(f, "↓"),
            Self::Left => write!(f, "←"),
        }
    }
}

//----------------------------------------------------------------------------
//
//  A pseudo-vec that can store a sequence of inputs in the form of the perfect
//  hash of itself.
//
//----------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Inputs {
    // the bit-wise content of `value` follows the pattern
    // [inputs[0], inputs[1], inputs[2], len]
    // the possible values for inputs[n] and len are both 0, 1, 2, 3
    // thus each of the values takes exactly 2 bits of space
    // making 4 of them to fit into an 8-bit integer
    value: u8,
}

impl Inputs {
    const CAP: u8 = 3;
    const MAX_HASHCODE: usize = 0b11111111;

    #[inline(always)]
    pub const fn new() -> Inputs {
        Inputs { value: 0 }
    }

    #[inline(always)]
    pub fn from_perfect_hash(perfect_hash: usize) -> Inputs {
        Inputs {
            value: perfect_hash as u8,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, input: Input) -> bool {
        let len = self.len();
        if len == Inputs::CAP {
            false
        } else {
            self.value += input.as_repr() << ((Inputs::CAP - len) * 2);
            self.value += 1;
            true
        }
    }

    #[inline(always)]
    pub fn pop(&mut self) -> Option<Input> {
        let len = self.len();
        if len == 0 {
            None
        } else {
            let shift = (Inputs::CAP + 1 - len) * 2;
            let last = self.value >> shift & 0b11;
            self.value &= !(0b11 << shift);
            self.value -= 1;
            Some(Input::from_repr(last))
        }
    }

    #[inline(always)]
    pub fn last(self) -> Option<Input> {
        let len = self.len();
        if len == 0 {
            None
        } else {
            Some(Input::from_repr(self.value >> ((Inputs::CAP + 1 - len) * 2) & 0b11))
        }
    }

    #[inline(always)]
    pub fn rev(mut self) -> Inputs {
        let mut rev = Inputs::new();
        while let Some(input) = self.pop() {
            rev.push(input);
        }
        rev
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.value = 0;
    }

    #[inline(always)]
    pub fn len(self) -> u8 {
        self.value & 0b11
    }

    #[inline(always)]
    pub fn perfect_hash(self) -> usize {
        self.value as usize
    }

    #[inline(always)]
    pub fn meant_for_art(self) -> bool {
        self.len() >= 2
    }
}

impl FromIterator<Input> for Inputs {
    #[inline(always)]
    fn from_iter<T: IntoIterator<Item = Input>>(iter: T) -> Self {
        let mut inputs = Inputs::new();
        let mut iter = iter.into_iter();
        while let Some(input) = iter.next() {
            if !inputs.push(input) {
                panic!("Number of inputs exceeds capacity.")
            }
        }
        inputs
    }
}

impl From<&[Input]> for Inputs {
    #[inline(always)]
    fn from(array: &[Input]) -> Self {
        Inputs::from_iter(array.iter().copied())
    }
}

impl<const N: usize> From<[Input; N]> for Inputs {
    #[inline(always)]
    fn from(array: [Input; N]) -> Self {
        Inputs::from_iter(array.into_iter())
    }
}

impl Debug for Inputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut inputs = [Input::Up; Self::CAP as usize];
        let mut len = 0;

        let mut cloned = self.clone();
        while let Some(input) = cloned.pop() {
            inputs[len] = input;
            len += 1;
        }
        let inputs = &mut inputs[..len];
        inputs.reverse();
        f.debug_list().entries(inputs).finish()
    }
}

//----------------------------------------------------------------------------
//
//  An array-based trie that uses input sequence as keys
//
//----------------------------------------------------------------------------
pub struct InputsTrie<T> {
    array: [Option<T>; Inputs::MAX_HASHCODE + 1],
}

impl<T: Copy> InputsTrie<T> {
    pub fn new() -> InputsTrie<T> {
        InputsTrie {
            array: [None; Inputs::MAX_HASHCODE + 1],
        }
    }

    pub fn get(&self, inputs: impl Into<Inputs>) -> Option<T> {
        self.array[inputs.into().perfect_hash()]
    }

    pub fn insert(&mut self, inputs: impl Into<Inputs>, value: T) {
        self.array[inputs.into().perfect_hash()] = Some(value);
    }

    pub fn try_insert(&mut self, inputs: impl Into<Inputs>, value: T) {
        self.array[inputs.into().perfect_hash()].get_or_insert(value);
    }

    pub fn iter(&self) -> impl Iterator<Item = (Inputs, T)> {
        self.array
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(hash, value)| Some((Inputs::from_perfect_hash(hash), value?)))
    }
}

impl<T: Default + Copy> InputsTrie<T> {
    pub fn get_or_default(&self, inputs: impl Into<Inputs>) -> T {
        self.array[inputs.into().perfect_hash()].unwrap_or_default()
    }
}

impl<T> Debug for InputsTrie<T>
where
    T: Debug + Copy,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

//----------------------------------------------------------------------------
//
//  Combat-art tokens: directions + mouse r/l + interact f
//  (Separate from Input/Inputs so prosthetic tools stay unchanged.)
//
//----------------------------------------------------------------------------

/// `r` = mouse right = block, `l` = mouse left = attack, `f` = interact.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArtToken {
    Up,
    Right,
    Down,
    Left,
    Block,
    Attack,
    Interact,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ArtCombo {
    first: Option<ArtToken>,
    second: Option<ArtToken>,
}

impl ArtCombo {
    pub const fn empty() -> ArtCombo {
        ArtCombo {
            first: None,
            second: None,
        }
    }

    pub const fn pair(a: ArtToken, b: ArtToken) -> ArtCombo {
        ArtCombo {
            first: Some(a),
            second: Some(b),
        }
    }

    pub fn is_empty(self) -> bool {
        self.first.is_none() && self.second.is_none()
    }

    pub fn second(self) -> Option<ArtToken> {
        self.second
    }
}

/// Last two combat-art key edges within a short window.
pub struct ArtComboWindow {
    first: Option<ArtToken>,
    second: Option<ArtToken>,
    age: u16,
    completed_this_frame: Option<ArtCombo>,
    keys_down: [bool; 4],
    block_down: bool,
    attack_down: bool,
    interact_down: bool,
}

impl ArtComboWindow {
    /// ~133ms at 60fps for most two-token arts.
    const MAX_AGE: u16 = 8;
    /// Slightly looser window so `ff` (and other f-then-X) is easier to land.
    const MAX_AGE_FF: u16 = 9;

    pub const fn new() -> ArtComboWindow {
        ArtComboWindow {
            first: None,
            second: None,
            age: 0,
            completed_this_frame: None,
            keys_down: [false; 4],
            block_down: false,
            attack_down: false,
            interact_down: false,
        }
    }

    pub fn tick(
        &mut self,
        up: bool,
        right: bool,
        down: bool,
        left: bool,
        block: bool,
        attack: bool,
        interact: bool,
    ) {
        self.completed_this_frame = None;
        let mut updated = false;

        for (i, (held, token)) in [
            (up, ArtToken::Up),
            (right, ArtToken::Right),
            (down, ArtToken::Down),
            (left, ArtToken::Left),
        ]
        .into_iter()
        .enumerate()
        {
            if held && !self.keys_down[i] {
                self.push(token);
                updated = true;
            }
            self.keys_down[i] = held;
        }

        // Button edges: block → interact → attack so same-frame rf/fl resolve correctly;
        // order-sensitive pairs (fr, lf) still need sequential presses.
        if block && !self.block_down {
            self.push(ArtToken::Block);
            updated = true;
        }
        self.block_down = block;

        if interact && !self.interact_down {
            self.push(ArtToken::Interact);
            updated = true;
        }
        self.interact_down = interact;

        if attack && !self.attack_down {
            self.push(ArtToken::Attack);
            updated = true;
        }
        self.attack_down = attack;

        if updated {
            self.age = 0;
        } else {
            self.age = self.age.saturating_add(1);
            let max_age = if self.first == Some(ArtToken::Interact) && self.second.is_none() {
                Self::MAX_AGE_FF
            } else {
                Self::MAX_AGE
            };
            if self.age >= max_age {
                self.clear();
            }
        }
    }

    fn push(&mut self, token: ArtToken) {
        match (self.first, self.second) {
            (None, _) => {
                self.first = Some(token);
                self.second = None;
            }
            (Some(a), None) => {
                self.second = Some(token);
                self.completed_this_frame = Some(ArtCombo::pair(a, token));
            }
            (Some(_), Some(_)) => {
                self.first = Some(token);
                self.second = None;
            }
        }
    }

    /// Two-token combo completed on this frame, if any.
    pub fn take_completed(&mut self) -> Option<ArtCombo> {
        self.completed_this_frame.take()
    }

    /// True when the first token is held and we are waiting for the second.
    pub fn awaiting_second(&self) -> bool {
        self.first.is_some() && self.second.is_none()
    }

    /// Whether the physical key/direction for `token` is currently held.
    pub fn token_held(&self, token: ArtToken) -> bool {
        match token {
            ArtToken::Up => self.keys_down[0],
            ArtToken::Right => self.keys_down[1],
            ArtToken::Down => self.keys_down[2],
            ArtToken::Left => self.keys_down[3],
            ArtToken::Block => self.block_down,
            ArtToken::Attack => self.attack_down,
            ArtToken::Interact => self.interact_down,
        }
    }

    pub fn clear(&mut self) {
        self.first = None;
        self.second = None;
        self.age = 0;
        self.completed_this_frame = None;
    }
}

#[cfg(test)]
mod test {
    use crate::input::{Input::*, Inputs};
    #[test]
    fn test_inputs() {
        macro_rules! assert_len {
            ($inputs:expr, $len:expr) => {
                assert_eq!(Inputs::from($inputs).len(), $len);
            };
        }

        macro_rules! assert_value {
            ($inputs:expr, $value:expr) => {
                assert_eq!(
                    Inputs::from($inputs).value,
                    u8::from_str_radix($value, 4).unwrap()
                );
            };
        }

        // len
        assert_len!([], 0);
        assert_len!([Up], 1);
        assert_len!([Up, Right], 2);
        assert_len!([Up, Right, Down], 3);

        // hash
        assert_value!([], "0000");
        assert_value!([Up], "0001");
        assert_value!([Right], "1001");
        assert_value!([Down], "2001");
        assert_value!([Left], "3001");
        assert_value!([Up, Up], "0002");
        assert_value!([Up, Right], "0102");
        assert_value!([Up, Right, Down], "0123");
        assert_value!([Left, Left, Left], "3333");

        // push and pop
        let src = [Up, Right, Down];
        let rev = [Down, Right, Up];

        let mut inputs = Inputs::new();
        for (i, input) in src.iter().copied().enumerate() {
            assert!(inputs.push(input));
            assert_eq!(inputs, Inputs::from(&src[..i + 1]));
        }
        assert_eq!(inputs.push(Left), false);

        for last in rev {
            assert_eq!(inputs.last(), Some(last));
            assert_eq!(inputs.pop(), Some(last));
        }
        assert_eq!(inputs.last(), None);
        assert_eq!(inputs.pop(), None);

        // rev
        assert_eq!(Inputs::from([Up]).rev(), Inputs::from([Up]));
        assert_eq!(Inputs::from([Up, Right]).rev(), Inputs::from([Right, Up]));
        assert_eq!(Inputs::from([Up, Right, Down]).rev(), Inputs::from([Down, Right, Up]));
    }

    #[test]
    fn bench_inputs() {
        const ROUNDS: usize = 1_000_000;
        macro_rules! push_and_pop {
            ($target:ident) => {{
                let start = std::time::Instant::now();
                let src = [Up, Right, Down, Left];
                for _ in 0..ROUNDS {
                    for input in src {
                        $target.push(input);
                    }
                    for _ in src {
                        $target.pop();
                    }
                }
                start.elapsed()
            }};
        }

        let mut inputs = Inputs::new();
        let mut vec = Vec::with_capacity(Inputs::CAP as usize);

        println!("Inputs:     {:?}", push_and_pop!(inputs));
        println!("Vec<Input>: {:?}", push_and_pop!(vec));
    }

    #[test]
    fn test_art_button_pairs() {
        use super::{ArtCombo, ArtComboWindow, ArtToken};

        // same-frame rl
        let mut w = ArtComboWindow::new();
        w.tick(false, false, false, false, true, true, false);
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Attack))
        );

        // same-frame rf
        let mut w = ArtComboWindow::new();
        w.tick(false, false, false, false, true, false, true);
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Interact))
        );

        // same-frame fl
        let mut w = ArtComboWindow::new();
        w.tick(false, false, false, false, false, true, true);
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Interact, ArtToken::Attack))
        );

        // sequential fr
        let mut w = ArtComboWindow::new();
        w.tick(false, false, false, false, false, false, true);
        assert!(w.take_completed().is_none());
        w.tick(false, false, false, false, true, false, true);
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Interact, ArtToken::Block))
        );
    }
}
