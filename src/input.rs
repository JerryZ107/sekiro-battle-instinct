use crate::{
    action::{ATTACK, BLOCK, CROUCH, DODGE, GRAPPLE, INTERACT, JUMP, SPRINT, SWITCH_PROSTHETIC, USE_PROSTHETIC},
    frame::Frames,
    game::InputHandler,
};

//----------------------------------------------------------------------------
//
//  Combat-art tokens: directions + mouse r/l + interact e + grapple f
//
//----------------------------------------------------------------------------

/// `r` = mouse right = block, `l` = mouse left = attack, `e` = interact, `f` = grapple/hook.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArtToken {
    Up,
    Right,
    Down,
    Left,
    Block,
    Attack,
    Interact,
    Grapple,
    Jump,
    Sprint,
    Crouch,
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

    pub fn second(self) -> Option<ArtToken> {
        self.second
    }

    pub fn first(self) -> Option<ArtToken> {
        self.first
    }

    pub fn is_rl(self) -> bool {
        matches!(
            (self.first, self.second),
            (Some(ArtToken::Block), Some(ArtToken::Attack))
        )
    }
}

impl ArtToken {
    pub const ALL: [ArtToken; 11] = [
        ArtToken::Up,
        ArtToken::Right,
        ArtToken::Down,
        ArtToken::Left,
        ArtToken::Block,
        ArtToken::Interact,
        ArtToken::Grapple,
        ArtToken::Jump,
        ArtToken::Sprint,
        ArtToken::Crouch,
        ArtToken::Attack,
    ];

    #[inline(always)]
    pub const fn index(self) -> u8 {
        match self {
            ArtToken::Up => 1,
            ArtToken::Right => 2,
            ArtToken::Down => 3,
            ArtToken::Left => 4,
            ArtToken::Block => 5,
            ArtToken::Interact => 6,
            ArtToken::Grapple => 7,
            ArtToken::Jump => 8,
            ArtToken::Sprint => 9,
            ArtToken::Crouch => 10,
            ArtToken::Attack => 11,
        }
    }

    #[inline(always)]
    pub const fn bit(self) -> u16 {
        match self {
            ArtToken::Up => 1 << 0,
            ArtToken::Right => 1 << 1,
            ArtToken::Down => 1 << 2,
            ArtToken::Left => 1 << 3,
            ArtToken::Block => 1 << 4,
            ArtToken::Interact => 1 << 5,
            ArtToken::Grapple => 1 << 6,
            ArtToken::Jump => 1 << 7,
            ArtToken::Sprint => 1 << 8,
            ArtToken::Crouch => 1 << 9,
            ArtToken::Attack => 1 << 10,
        }
    }
}

/// One frame of action state, derived exactly once from the physical directions
/// and the game's action bitfield. Combat-art and prosthetic state machines both
/// consume this object, so edge tracking is no longer duplicated.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActionFrame {
    held: u16,
    last_held: u16,
    pressed: u16,
    released: u16,
    use_prosthetic: bool,
    use_prosthetic_pressed: bool,
    last_use_prosthetic: bool,
    switch_prosthetic: bool,
    switch_prosthetic_pressed: bool,
    last_switch_prosthetic: bool,
    dodging: bool,
}

impl ActionFrame {
    pub fn update(&mut self, input: &InputHandler, up: bool, right: bool, down: bool, left: bool) {
        let action = input.action;
        let mut held = 0u16;
        if up {
            held |= ArtToken::Up.bit();
        }
        if right {
            held |= ArtToken::Right.bit();
        }
        if down {
            held |= ArtToken::Down.bit();
        }
        if left {
            held |= ArtToken::Left.bit();
        }
        if action & BLOCK != 0 {
            held |= ArtToken::Block.bit();
        }
        if action & ATTACK != 0 {
            held |= ArtToken::Attack.bit();
        }
        if action & INTERACT != 0 {
            held |= ArtToken::Interact.bit();
        }
        if action & GRAPPLE != 0 {
            held |= ArtToken::Grapple.bit();
        }
        if action & JUMP != 0 {
            held |= ArtToken::Jump.bit();
        }
        if action & SPRINT != 0 {
            held |= ArtToken::Sprint.bit();
        }
        if action & CROUCH != 0 {
            held |= ArtToken::Crouch.bit();
        }

        self.last_held = self.held;
        self.held = held;
        self.pressed = held & !self.last_held;
        self.released = self.last_held & !held;

        self.last_use_prosthetic = self.use_prosthetic;
        self.use_prosthetic = action & USE_PROSTHETIC != 0;
        self.use_prosthetic_pressed = self.use_prosthetic && !self.last_use_prosthetic;

        self.last_switch_prosthetic = self.switch_prosthetic;
        self.switch_prosthetic = action & SWITCH_PROSTHETIC != 0;
        self.switch_prosthetic_pressed = self.switch_prosthetic && !self.last_switch_prosthetic;

        self.dodging = action & DODGE != 0;
    }

    #[inline(always)]
    pub fn held(self, token: ArtToken) -> bool {
        self.held & token.bit() != 0
    }

    #[inline(always)]
    pub fn was_held(self, token: ArtToken) -> bool {
        self.last_held & token.bit() != 0
    }

    #[inline(always)]
    pub fn pressed(self, token: ArtToken) -> bool {
        self.pressed & token.bit() != 0
    }

    #[inline(always)]
    pub fn use_prosthetic(self) -> bool {
        self.use_prosthetic
    }

    #[inline(always)]
    pub fn use_prosthetic_pressed(self) -> bool {
        self.use_prosthetic_pressed
    }

    #[inline(always)]
    pub fn switch_prosthetic(self) -> bool {
        self.switch_prosthetic
    }

    #[inline(always)]
    pub fn switch_prosthetic_pressed(self) -> bool {
        self.switch_prosthetic_pressed
    }

    #[inline(always)]
    pub fn dodging(self) -> bool {
        self.dodging
    }

    #[inline(always)]
    pub fn held_mask(self) -> u16 {
        self.held
    }

    #[inline(always)]
    pub fn pressed_direction(self) -> bool {
        self.pressed & (ArtToken::Up.bit() | ArtToken::Right.bit() | ArtToken::Down.bit() | ArtToken::Left.bit()) != 0
    }

    #[cfg(test)]
    fn from_held_for_test(held: u16, last_held: u16) -> Self {
        Self {
            held,
            last_held,
            pressed: held & !last_held,
            released: last_held & !held,
            ..Self::default()
        }
    }
}

// Joystick ergonomics, borrowed from dec32/sekiro-battle-instinct's InputBuffer:
// square neutral zone, easier opposite-direction bounces, harder rotations.
const COMMON_THRESHOLD: f32 = 0.85;
const ROTATE_THRESHOLD: f32 = 0.9;
const BOUNCE_THRESHOLD: f32 = 0.4;

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectionBuffer {
    last: Option<ArtToken>,
    neutral: bool,
}

impl DirectionBuffer {
    pub const fn new() -> Self {
        Self {
            last: None,
            neutral: true,
        }
    }

    /// Convert the raw left stick into at most one directional token, using
    /// hysteresis so small stick drift does not create a new edge every frame.
    /// Keyboard directions are used when the stick is inside the neutral zone.
    pub fn update(
        &mut self,
        stick: Option<(f32, f32)>,
        up: bool,
        right: bool,
        down: bool,
        left: bool,
    ) -> (bool, bool, bool, bool) {
        if let Some((x, y)) = stick {
            let abs_x = x.abs();
            let abs_y = y.abs();
            let distance = abs_x.max(abs_y);

            let direction = if abs_y >= abs_x {
                if y > 0.0 { ArtToken::Up } else { ArtToken::Down }
            } else if x > 0.0 {
                ArtToken::Right
            } else {
                ArtToken::Left
            };

            let threshold = match self.last {
                Some(last) if direction == last => COMMON_THRESHOLD,
                Some(last) if Some(direction) == opposite_direction(last) => BOUNCE_THRESHOLD,
                Some(_) => ROTATE_THRESHOLD,
                None => COMMON_THRESHOLD,
            };

            if distance < threshold {
                self.neutral = true;
                return (false, false, false, false);
            }

            if self.neutral || self.last != Some(direction) {
                self.last = Some(direction);
            }
            self.neutral = false;
            return match direction {
                ArtToken::Up => (true, false, false, false),
                ArtToken::Right => (false, true, false, false),
                ArtToken::Down => (false, false, true, false),
                ArtToken::Left => (false, false, false, true),
                _ => (false, false, false, false),
            };
        }

        self.neutral = !(up || right || down || left);
        (up, right, down, left)
    }
}

fn opposite_direction(token: ArtToken) -> Option<ArtToken> {
    Some(match token {
        ArtToken::Up => ArtToken::Down,
        ArtToken::Right => ArtToken::Left,
        ArtToken::Down => ArtToken::Up,
        ArtToken::Left => ArtToken::Right,
        _ => return None,
    })
}

/// Key type for `ComboTable`. Index 0 is reserved for "no token".
pub trait ComboKey: Copy + Eq {
    fn index(self) -> usize;
}

/// Fixed-size, allocation-free map for two-token combos. This is the fork's
/// equivalent of dec32/sekiro-battle-instinct's `InputsTrie<T>`: the key is a
/// compact pair of token indices and lookup is a direct array access.
#[derive(Clone, Debug)]
pub struct ComboTable<K: ComboKey, T> {
    entries: [Option<(K, T)>; 256],
}

impl<K: ComboKey, T> Default for ComboTable<K, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: ComboKey, T> ComboTable<K, T> {
    pub fn new() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
        }
    }

    #[inline]
    pub fn get(&self, key: &K) -> Option<&T> {
        self.entries[key.index()].as_ref().map(|(_, value)| value)
    }

    #[inline]
    pub fn contains_key(&self, key: &K) -> bool {
        self.entries[key.index()].is_some()
    }

    pub fn insert(&mut self, key: K, value: T) -> Option<T> {
        self.entries[key.index()].replace((key, value)).map(|(_, old)| old)
    }

    pub fn remove(&mut self, key: &K) -> Option<T> {
        self.entries[key.index()].take().map(|(_, value)| value)
    }

    pub fn len(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(Option::is_none)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &T)> {
        self.entries
            .iter()
            .filter_map(|entry| entry.as_ref().map(|(key, value)| (key, value)))
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries
            .iter()
            .filter_map(|entry| entry.as_ref().map(|(key, _)| key))
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.entries
            .iter()
            .filter_map(|entry| entry.as_ref().map(|(_, value)| value))
    }

    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&K, &T) -> bool,
    {
        for entry in &mut self.entries {
            let remove = entry.as_ref().is_some_and(|(key, value)| !keep(key, value));
            if remove {
                *entry = None;
            }
        }
    }
}

impl<K: ComboKey, T> IntoIterator for ComboTable<K, T> {
    type Item = (K, T);
    type IntoIter = std::iter::Flatten<std::array::IntoIter<Option<(K, T)>, 256>>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter().flatten()
    }
}

impl<'a, K: ComboKey, T> IntoIterator for &'a ComboTable<K, T> {
    type Item = (&'a K, &'a T);
    type IntoIter =
        std::iter::FilterMap<std::slice::Iter<'a, Option<(K, T)>>, fn(&'a Option<(K, T)>) -> Option<(&'a K, &'a T)>>;

    fn into_iter(self) -> Self::IntoIter {
        fn project<'a, K, T>(entry: &'a Option<(K, T)>) -> Option<(&'a K, &'a T)> {
            entry.as_ref().map(|(key, value)| (key, value))
        }
        self.entries.iter().filter_map(project::<K, T>)
    }
}

impl<K: ComboKey, T> FromIterator<(K, T)> for ComboTable<K, T> {
    fn from_iter<I: IntoIterator<Item = (K, T)>>(iter: I) -> Self {
        let mut table = Self::new();
        for (key, value) in iter {
            table.insert(key, value);
        }
        table
    }
}

/// ArtCombo uses token index 0 for `None`, 1..=11 for real tokens.
impl ComboKey for ArtCombo {
    fn index(self) -> usize {
        let first = self.first.map_or(0, ArtToken::index);
        let second = self.second.map_or(0, ArtToken::index);
        ((first as usize) << 4) | second as usize
    }
}

/// Last two combat-art key edges within a short window.
pub struct ArtComboWindow {
    first: Option<ArtToken>,
    second: Option<ArtToken>,
    age: u16,
    completed_this_frame: Option<ArtCombo>,
    /// `l` after `r` outside the configured rl window.
    late_rl_rejected: bool,
    /// Current physical token bitmask, updated once per frame by `tick_frame`.
    held: u16,
    max_age_rl: Frames,
    #[cfg(test)]
    test_last_held: u16,
}

impl ArtComboWindow {
    /// ~300ms at 60fps for most two-token arts (~0.3s).
    const MAX_AGE: Frames = Frames::standard(18);
    /// Same window for `ee` / e-then-X and `ff` / f-then-X (~0.3s).
    const MAX_AGE_FF: Frames = Frames::standard(18);
    /// Longer window when first key is `l` (attack), ~0.7s @60fps.
    const MAX_AGE_L: Frames = Frames::standard(42);

    pub fn new(max_age_rl: Frames) -> ArtComboWindow {
        ArtComboWindow {
            first: None,
            second: None,
            age: 0,
            completed_this_frame: None,
            late_rl_rejected: false,
            held: 0,
            max_age_rl,
            #[cfg(test)]
            test_last_held: 0,
        }
    }

    /// Consume the shared per-frame action edges.
    pub fn tick_frame(&mut self, frame: &ActionFrame) {
        self.completed_this_frame = None;
        let mut updated = false;

        // The current held mask must be visible to push for same-frame
        // BLOCK+ATTACK validation, so update it before processing edges.
        self.held = frame.held_mask();

        // Order matters for same-frame pairs: directions, block, interact,
        // grapple, jump, sprint, crouch, attack.
        for token in ArtToken::ALL {
            if frame.pressed(token) {
                updated |= self.push(token);
            }
        }

        if updated {
            self.age = 0;
        } else {
            self.age = self.age.saturating_add(1);
            let block_held = self.held & ArtToken::Block.bit() != 0;
            let max_age = match (self.first, self.second) {
                (Some(ArtToken::Attack), None) => Self::MAX_AGE_L.as_actual(),
                (Some(ArtToken::Interact), None) | (Some(ArtToken::Grapple), None) => Self::MAX_AGE_FF.as_actual(),
                // After `r`, `rl` window while block held; +1 frame so late `l` can set `late_rl_rejected`.
                (Some(ArtToken::Block), None) if block_held => self.max_age_rl.as_actual().saturating_add(1),
                _ => Self::MAX_AGE.as_actual(),
            };
            if self.age >= max_age {
                self.clear();
            }
        }
    }

    /// Test-only compatibility wrapper. Production code calls `tick_frame`.
    #[cfg(test)]
    pub fn tick(
        &mut self,
        up: bool,
        right: bool,
        down: bool,
        left: bool,
        block: bool,
        attack: bool,
        interact: bool,
        grapple: bool,
        jump: bool,
        sprint: bool,
        crouch: bool,
    ) {
        let mut held = 0u16;
        if up {
            held |= ArtToken::Up.bit();
        }
        if right {
            held |= ArtToken::Right.bit();
        }
        if down {
            held |= ArtToken::Down.bit();
        }
        if left {
            held |= ArtToken::Left.bit();
        }
        if block {
            held |= ArtToken::Block.bit();
        }
        if attack {
            held |= ArtToken::Attack.bit();
        }
        if interact {
            held |= ArtToken::Interact.bit();
        }
        if grapple {
            held |= ArtToken::Grapple.bit();
        }
        if jump {
            held |= ArtToken::Jump.bit();
        }
        if sprint {
            held |= ArtToken::Sprint.bit();
        }
        if crouch {
            held |= ArtToken::Crouch.bit();
        }

        let frame = ActionFrame::from_held_for_test(held, self.test_last_held);
        self.test_last_held = held;
        self.tick_frame(&frame);
    }

    fn push(&mut self, token: ArtToken) -> bool {
        match (self.first, self.second) {
            (None, _) => {
                self.first = Some(token);
                self.second = None;
                true
            }
            (Some(a), None) => {
                if a == ArtToken::Block && token == ArtToken::Attack {
                    // Within cfg window, and `r`+`l` both held on the attack edge.
                    if self.age >= self.max_age_rl.as_actual() {
                        self.late_rl_rejected = true;
                        self.clear();
                        return false;
                    }
                    if self.held & ArtToken::Block.bit() == 0 {
                        self.clear();
                        return false;
                    }
                }
                self.second = Some(token);
                self.completed_this_frame = Some(ArtCombo::pair(a, token));
                true
            }
            (Some(_), Some(_)) => {
                self.first = Some(token);
                self.second = None;
                true
            }
        }
    }

    /// Two-token combo completed on this frame, if any.
    pub fn take_completed(&mut self) -> Option<ArtCombo> {
        self.completed_this_frame.take()
    }

    pub fn take_late_rl_rejected(&mut self) -> bool {
        let v = self.late_rl_rejected;
        self.late_rl_rejected = false;
        v
    }

    /// True when the first token is held and we are waiting for the second.
    pub fn awaiting_second(&self) -> bool {
        self.first.is_some() && self.second.is_none()
    }

    /// Block first, waiting for second (e.g. `rl`, `r↑`).
    pub fn awaiting_block_second(&self) -> bool {
        matches!(self.first, Some(ArtToken::Block)) && self.second.is_none()
    }

    /// Whether the physical key/direction for `token` is currently held.
    pub fn token_held(&self, token: ArtToken) -> bool {
        self.held & token.bit() != 0
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
    use super::{ArtCombo, ArtComboWindow, ArtToken, DirectionBuffer};
    use crate::frame::Frames;

    #[test]
    fn joystick_uses_deadzone_and_direction_hysteresis() {
        let mut buffer = DirectionBuffer::new();
        assert_eq!(
            buffer.update(Some((0.5, 0.0)), false, false, false, false),
            (false, false, false, false)
        );
        assert_eq!(
            buffer.update(Some((0.9, 0.0)), false, false, false, false),
            (false, true, false, false)
        );
        // Opposite direction uses the easier bounce threshold.
        assert_eq!(
            buffer.update(Some((-0.5, 0.0)), false, false, false, false),
            (false, false, false, true)
        );
        // Rotation needs the stricter threshold.
        assert_eq!(
            buffer.update(Some((0.0, 0.85)), false, false, false, false),
            (false, false, false, false)
        );
        assert_eq!(
            buffer.update(Some((0.0, 0.95)), false, false, false, false),
            (true, false, false, false)
        );
    }

    #[test]
    fn keyboard_directions_are_used_when_stick_is_absent() {
        let mut buffer = DirectionBuffer::new();
        assert_eq!(
            buffer.update(None, false, true, false, false),
            (false, true, false, false)
        );
    }

    #[test]
    fn test_art_button_pairs() {
        // same-frame rl
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, true, true, false, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Attack))
        );

        // same-frame r+e
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, true, false, true, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Interact))
        );

        // same-frame e+l
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, false, true, true, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Interact, ArtToken::Attack))
        );

        // sequential e+r
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, false, false, true, false, false, false, false,
        );
        assert!(w.take_completed().is_none());
        w.tick(
            false, false, false, false, true, false, true, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Interact, ArtToken::Block))
        );

        // sequential rl with block held until attack
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, true, false, false, false, false, false, false,
        );
        for _ in 0..4 {
            w.tick(
                false, false, false, false, true, false, false, false, false, false, false,
            );
        }
        w.tick(
            false, false, false, false, true, true, false, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Attack))
        );

        // block released before attack: no rl
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, true, false, false, false, false, false, false,
        );
        for _ in 0..6 {
            w.tick(
                false, false, false, false, false, false, false, false, false, false, false,
            );
        }
        w.tick(
            false, false, false, false, false, true, false, false, false, false, false,
        );
        assert!(w.take_completed().is_none());

        // hold block past rl window then attack: no rl
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, true, false, false, false, false, false, false,
        );
        for _ in 0..6 {
            w.tick(
                false, false, false, false, true, false, false, false, false, false, false,
            );
        }
        w.tick(
            false, false, false, false, true, true, false, false, false, false, false,
        );
        assert!(w.take_completed().is_none());
        assert!(w.take_late_rl_rejected());

        // within rl window while both held
        let mut w = ArtComboWindow::new(Frames::standard(18));
        w.tick(
            false, false, false, false, true, false, false, false, false, false, false,
        );
        for _ in 0..10 {
            w.tick(
                false, false, false, false, true, false, false, false, false, false, false,
            );
        }
        w.tick(
            false, false, false, false, true, true, false, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Attack))
        );

        // The grapple key is an independent token and can form the same pairs as interact.
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, false, false, false, true, false, false, false,
        );
        assert!(w.take_completed().is_none());
        w.tick(
            false, false, false, false, false, true, false, true, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Grapple, ArtToken::Attack))
        );

        // f then r
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick(
            false, false, false, false, false, false, false, true, false, false, false,
        );
        w.tick(
            false, false, false, false, true, false, false, true, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Grapple, ArtToken::Block))
        );
        assert!(w.token_held(ArtToken::Grapple));
    }

    #[test]
    fn test_jump_sprint_crouch_tokens() {
        let mut w = ArtComboWindow::new(Frames::standard(18));
        w.tick(
            false, false, false, false, false, false, false, false, true, false, false,
        );
        assert!(w.token_held(ArtToken::Jump));
        assert!(w.take_completed().is_none());
        w.tick(
            false, false, false, false, false, true, false, false, true, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Jump, ArtToken::Attack))
        );

        let mut w = ArtComboWindow::new(Frames::standard(18));
        w.tick(
            false, false, false, false, false, false, false, false, false, true, false,
        );
        assert!(w.token_held(ArtToken::Sprint));
        w.tick(
            false, false, false, false, false, true, false, false, false, true, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Sprint, ArtToken::Attack))
        );

        let mut w = ArtComboWindow::new(Frames::standard(18));
        w.tick(
            false, false, false, false, false, false, false, false, false, false, true,
        );
        assert!(w.token_held(ArtToken::Crouch));
        w.tick(
            false, false, false, false, false, true, false, false, false, false, true,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Crouch, ArtToken::Attack))
        );
    }
}
