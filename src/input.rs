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
    third: Option<ArtToken>,
}

impl ArtCombo {
    pub const fn empty() -> ArtCombo {
        ArtCombo {
            first: None,
            second: None,
            third: None,
        }
    }

    pub const fn pair(a: ArtToken, b: ArtToken) -> ArtCombo {
        ArtCombo {
            first: Some(a),
            second: Some(b),
            third: None,
        }
    }

    pub const fn triple(a: ArtToken, b: ArtToken, c: ArtToken) -> ArtCombo {
        ArtCombo {
            first: Some(a),
            second: Some(b),
            third: Some(c),
        }
    }

    /// Build a two- or three-token combo from an ordered key sequence.
    pub fn from_tokens(tokens: &[ArtToken]) -> Option<ArtCombo> {
        match tokens {
            [first, second] => Some(ArtCombo::pair(*first, *second)),
            [first, second, third] => Some(ArtCombo::triple(*first, *second, *third)),
            _ => None,
        }
    }

    pub fn first(self) -> Option<ArtToken> {
        self.first
    }

    pub fn second(self) -> Option<ArtToken> {
        self.second
    }

    pub fn third(self) -> Option<ArtToken> {
        self.third
    }

    /// Last configured token; this is the key held for sustained art fire.
    pub fn last(self) -> Option<ArtToken> {
        self.third.or(self.second).or(self.first)
    }

    #[allow(dead_code)]
    pub fn len(self) -> usize {
        usize::from(self.first.is_some()) + usize::from(self.second.is_some()) + usize::from(self.third.is_some())
    }

    pub fn is_rl(self) -> bool {
        matches!(
            (self.first, self.second, self.third),
            (Some(ArtToken::Block), Some(ArtToken::Attack), None)
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
    pub fn update(&mut self, input: &InputHandler, up: bool, right: bool, down: bool, left: bool, crouch_held: bool) {
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
        if action & CROUCH != 0 || crouch_held {
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
    pub(crate) fn from_held_for_test(held: u16, last_held: u16) -> Self {
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

/// Fixed-size, allocation-free map for two- or three-token combos. This is
/// the fork's equivalent of dec32/sekiro-battle-instinct's `InputsTrie<T>`:
/// the key is a compact sequence of token indices and lookup is a direct array
/// access. Three nibbles (first/second/third) need 4096 slots.
pub const COMBO_TABLE_SIZE: usize = 4096;

#[derive(Clone, Debug)]
pub struct ComboTable<K: ComboKey, T> {
    entries: [Option<(K, T)>; COMBO_TABLE_SIZE],
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
    type IntoIter = std::iter::Flatten<std::array::IntoIter<Option<(K, T)>, COMBO_TABLE_SIZE>>;

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

/// ArtCombo uses token index 0 for `None`, 1..=11 for real tokens. The
/// three nibbles keep pair and triple lookups in the same direct-access table.
impl ComboKey for ArtCombo {
    fn index(self) -> usize {
        let first = self.first.map_or(0, ArtToken::index);
        let second = self.second.map_or(0, ArtToken::index);
        let third = self.third.map_or(0, ArtToken::index);
        ((first as usize) << 8) | ((second as usize) << 4) | third as usize
    }
}

/// Last two or three combat-art key edges within their perception windows.
pub struct ArtComboWindow {
    first: Option<ArtToken>,
    second: Option<ArtToken>,
    third: Option<ArtToken>,
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
    /// ~300ms at 60fps for most combo stages (~0.3s).
    const MAX_AGE: Frames = Frames::standard(18);
    /// Same window for `ee` / e-then-X and `ff` / f-then-X (~0.3s).
    const MAX_AGE_FF: Frames = Frames::standard(18);
    /// Longer window when first key is `l` (attack), ~0.7s @60fps.
    const MAX_AGE_L: Frames = Frames::standard(42);
    /// First -> middle stage of a three-key combo (~0.3s).
    const TRIPLE_STAGE: Frames = Frames::standard(18);
    /// Middle -> tail stage of a three-key combo (1s). Both upstream keys
    /// must still be held when the tail is pressed, so the longer window does
    /// not make accidental triggers more likely.
    const TRIPLE_TAIL_STAGE: Frames = Frames::standard(60);

    pub fn new(max_age_rl: Frames) -> ArtComboWindow {
        ArtComboWindow {
            first: None,
            second: None,
            third: None,
            age: 0,
            completed_this_frame: None,
            late_rl_rejected: false,
            held: 0,
            max_age_rl,
            #[cfg(test)]
            test_last_held: 0,
        }
    }

    /// Consume the shared per-frame action edges. A two-token prefix always
    /// completes here; production code uses [`Self::tick_frame_with`] so a
    /// configured pair wins before the third key is inspected.
    #[allow(dead_code)]
    pub fn tick_frame(&mut self, frame: &ActionFrame) {
        self.tick_frame_with(frame, |_| true);
    }

    /// Consume action edges, using `combo_is_bound` to decide whether a pair
    /// is a complete trigger or merely the prefix of a three-key trigger.
    /// A bound pair fires immediately; remaining edges in the same frame are
    /// deliberately ignored so the third key cannot preempt it.
    pub fn tick_frame_with<F>(&mut self, frame: &ActionFrame, combo_is_bound: F)
    where
        F: Fn(ArtCombo) -> bool,
    {
        self.completed_this_frame = None;
        let mut updated = false;
        let mut fired = false;

        // The current held mask must be visible to push for same-frame
        // BLOCK+ATTACK validation, so update it before processing edges.
        self.held = frame.held_mask();

        // Order matters for same-frame sequences: directions, block, interact,
        // grapple, jump, sprint, crouch, attack.
        for token in ArtToken::ALL {
            if frame.pressed(token) {
                let (did_update, did_fire) = self.push(token, &combo_is_bound);
                updated |= did_update;
                if did_fire {
                    fired = true;
                    break;
                }
            }
        }

        if fired || updated {
            self.age = 0;
        } else {
            self.age = self.age.saturating_add(1);
            let block_held = self.held & ArtToken::Block.bit() != 0;
            let max_age = match (self.first, self.second, self.third) {
                (Some(ArtToken::Attack), None, None) => Self::MAX_AGE_L.as_actual(),
                (Some(ArtToken::Interact), None, None) | (Some(ArtToken::Grapple), None, None) => {
                    Self::MAX_AGE_FF.as_actual()
                }
                // After `r`, `rl` window while block held; +1 frame so late `l` can set `late_rl_rejected`.
                (Some(ArtToken::Block), None, None) if block_held => {
                    let rl_age = self.max_age_rl.as_actual().saturating_add(1);
                    rl_age.max(Self::TRIPLE_STAGE.as_actual())
                }
                (Some(_), Some(_), None) => Self::TRIPLE_TAIL_STAGE.as_actual(),
                _ => Self::MAX_AGE.as_actual(),
            };
            if self.age >= max_age {
                self.clear();
            }
        }
    }

    /// Test-only compatibility wrapper. Production code calls
    /// `tick_frame_with`.
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
        self.tick_with(
            up,
            right,
            down,
            left,
            block,
            attack,
            interact,
            grapple,
            jump,
            sprint,
            crouch,
            |_| true,
        );
    }

    /// Test wrapper that lets a test model which pair/triple prefixes are
    /// configured, matching the real config lookup used by `core.rs`.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn tick_with<F>(
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
        combo_is_bound: F,
    ) where
        F: Fn(ArtCombo) -> bool,
    {
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
        self.tick_frame_with(&frame, combo_is_bound);
    }

    /// Returns `(state_changed, completed_trigger)`.
    fn push<F>(&mut self, token: ArtToken, combo_is_bound: &F) -> (bool, bool)
    where
        F: Fn(ArtCombo) -> bool,
    {
        match (self.first, self.second, self.third) {
            (None, _, _) => {
                self.first = Some(token);
                self.second = None;
                self.third = None;
                (true, false)
            }
            (Some(a), None, _) => {
                let pair = ArtCombo::pair(a, token);
                let pair_bound = combo_is_bound(pair);

                // A configured pair only fires when the first key is still
                // held as the tail is pressed, matching the hold-chain rule
                // used by three-key combos. Same-key pairs such as `ee` still
                // work because the new press puts that key into `held`.
                if pair_bound && !self.token_held(a) {
                    if a == ArtToken::Block && token == ArtToken::Attack && self.age >= self.max_age_rl.as_actual() {
                        self.late_rl_rejected = true;
                    }
                    self.clear();
                    return (false, false);
                }

                // The special `r` -> `l` rule additionally requires the attack
                // edge to be inside the configured rl window.
                if pair_bound && a == ArtToken::Block && token == ArtToken::Attack {
                    if self.age >= self.max_age_rl.as_actual() {
                        self.late_rl_rejected = true;
                        self.clear();
                        return (false, false);
                    }
                }

                if pair_bound {
                    self.second = Some(token);
                    self.third = None;
                    self.completed_this_frame = Some(pair);
                    (true, true)
                } else if self.token_held(a) && self.age < Self::TRIPLE_STAGE.as_actual() {
                    // No double match and the first key is still held: this
                    // edge is the middle key of a possible triple.
                    self.second = Some(token);
                    self.third = None;
                    self.age = 0;
                    (true, false)
                } else {
                    // A triple must keep every upstream key held. Releasing
                    // the first key before the middle edge invalidates it.
                    if a == ArtToken::Block && token == ArtToken::Attack && self.age >= self.max_age_rl.as_actual() {
                        self.late_rl_rejected = true;
                    }
                    self.clear();
                    (false, false)
                }
            }
            (Some(a), Some(b), None) => {
                let triple = ArtCombo::triple(a, b, token);
                let chain_held = self.token_held(a) && self.token_held(b);
                if combo_is_bound(triple) && chain_held && self.age < Self::TRIPLE_TAIL_STAGE.as_actual() {
                    self.third = Some(token);
                    self.completed_this_frame = Some(triple);
                    (true, true)
                } else {
                    // Triple failed: do not keep a stale first/middle chain.
                    self.clear();
                    (false, false)
                }
            }
            (Some(_), Some(_), Some(_)) => {
                self.first = Some(token);
                self.second = None;
                self.third = None;
                (true, false)
            }
        }
    }

    /// Two- or three-token combo completed on this frame, if any.
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

    /// True when a two-token prefix did not match and we are waiting for the
    /// third key of a three-token combo.
    #[allow(dead_code)]
    pub fn awaiting_third(&self) -> bool {
        self.first.is_some() && self.second.is_some() && self.third.is_none()
    }

    /// Block first, waiting for the second or third key.
    pub fn awaiting_block_combo(&self) -> bool {
        matches!(self.first, Some(ArtToken::Block)) && self.third.is_none()
    }

    /// Block first, waiting for second (e.g. `rl`, `r↑`).
    #[allow(dead_code)]
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
        self.third = None;
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
    fn triple_combo_fires_after_unbound_pair_prefix() {
        let triple = ArtCombo::triple(ArtToken::Up, ArtToken::Right, ArtToken::Attack);
        let mut w = ArtComboWindow::new(Frames::standard(18));

        w.tick_with(
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.take_completed().is_none());

        // Middle key arrives while the first key is still held.
        w.tick_with(
            true,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.take_completed().is_none());
        assert!(w.awaiting_third());

        // Tail arrives while both upstream keys are still held.
        w.tick_with(
            true,
            true,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert_eq!(w.take_completed(), Some(triple));
    }

    #[test]
    fn triple_requires_every_upstream_key_to_stay_held() {
        let triple = ArtCombo::triple(ArtToken::Up, ArtToken::Right, ArtToken::Attack);

        // Releasing the first key before the middle edge invalidates it.
        let mut w = ArtComboWindow::new(Frames::standard(18));
        w.tick_with(
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        w.tick_with(
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.take_completed().is_none());
        assert!(!w.awaiting_third());

        // Releasing the middle key before the tail edge also invalidates it.
        let mut w = ArtComboWindow::new(Frames::standard(18));
        w.tick_with(
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        w.tick_with(
            true,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        w.tick_with(
            true,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.take_completed().is_none());
    }

    #[test]
    fn unbound_block_attack_prefix_uses_three_key_window() {
        let triple = ArtCombo::triple(ArtToken::Block, ArtToken::Attack, ArtToken::Interact);
        // The normal rl double window is only 0.1s, but the triple stage is 0.3s.
        let mut w = ArtComboWindow::new(Frames::standard(6));
        w.tick_with(
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        for _ in 0..10 {
            w.tick_with(
                false,
                false,
                false,
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                false,
                |combo| combo == triple,
            );
        }
        w.tick_with(
            false,
            false,
            false,
            false,
            true,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.awaiting_third());

        w.tick_with(
            false,
            false,
            false,
            false,
            true,
            true,
            true,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert_eq!(w.take_completed(), Some(triple));
    }

    #[test]
    fn triple_first_to_middle_expires_after_three_tenths_of_a_second() {
        let triple = ArtCombo::triple(ArtToken::Attack, ArtToken::Right, ArtToken::Jump);
        let mut w = ArtComboWindow::new(Frames::standard(18));

        // Attack normally has a longer double-key window, but the triple
        // first -> middle stage is capped at 0.3s.
        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        for _ in 0..18 {
            w.tick_with(
                false,
                false,
                false,
                false,
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                |combo| combo == triple,
            );
        }
        w.tick_with(
            false,
            true,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.take_completed().is_none());
        assert!(!w.awaiting_third());
    }

    #[test]
    fn triple_middle_to_tail_allows_one_second() {
        let triple = ArtCombo::triple(ArtToken::Attack, ArtToken::Right, ArtToken::Jump);
        let mut w = ArtComboWindow::new(Frames::standard(18));

        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        w.tick_with(
            false,
            true,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.awaiting_third());

        // A 0.5s pause between the middle and tail key still fires.
        for _ in 0..30 {
            w.tick_with(
                false,
                true,
                false,
                false,
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                |combo| combo == triple,
            );
        }
        w.tick_with(
            false,
            true,
            false,
            false,
            false,
            true,
            false,
            false,
            true,
            false,
            false,
            |combo| combo == triple,
        );
        assert_eq!(w.take_completed(), Some(triple));

        // Exactly one second (60 frames) after the middle key expires.
        let mut w = ArtComboWindow::new(Frames::standard(18));
        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        w.tick_with(
            false,
            true,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.awaiting_third());
        for _ in 0..60 {
            w.tick_with(
                false,
                true,
                false,
                false,
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                |combo| combo == triple,
            );
        }
        w.tick_with(
            false,
            true,
            false,
            false,
            false,
            true,
            false,
            false,
            true,
            false,
            false,
            |combo| combo == triple,
        );
        assert!(w.take_completed().is_none());
        assert!(!w.awaiting_third());
    }

    #[test]
    fn pair_requires_first_key_held_when_tail_is_pressed() {
        let pair = ArtCombo::pair(ArtToken::Up, ArtToken::Attack);
        let mut w = ArtComboWindow::new(Frames::standard(18));

        w.tick_with(
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == pair,
        );
        // Releasing the first key before the tail edge invalidates the pair.
        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == pair,
        );
        assert!(w.take_completed().is_none());

        // With the first key still held, the same pair fires.
        w.tick_with(
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == pair,
        );
        w.tick_with(
            true,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == pair,
        );
        assert_eq!(w.take_completed(), Some(pair));
    }

    #[test]
    fn bound_pair_takes_priority_and_skips_third_key_same_frame() {
        let pair = ArtCombo::pair(ArtToken::Interact, ArtToken::Jump);
        let triple = ArtCombo::triple(ArtToken::Interact, ArtToken::Jump, ArtToken::Attack);
        let mut w = ArtComboWindow::new(Frames::standard(18));

        // interact, jump and attack all get pressed on the same frame. The
        // interact+jump pair is configured, so the attack edge must not be
        // judged as the third token at all.
        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            true,
            true,
            false,
            true,
            false,
            false,
            |combo| combo == pair || combo == triple,
        );
        assert_eq!(w.take_completed(), Some(pair));
    }

    #[test]
    fn triple_is_not_judged_after_pair_already_fired() {
        let pair = ArtCombo::pair(ArtToken::Interact, ArtToken::Interact);
        let triple = ArtCombo::triple(ArtToken::Interact, ArtToken::Interact, ArtToken::Attack);
        let mut w = ArtComboWindow::new(Frames::standard(18));

        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            |combo| combo == pair || combo == triple,
        );
        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            |combo| combo == pair || combo == triple,
        );
        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            false,
            false,
            false,
            |combo| combo == pair || combo == triple,
        );
        assert_eq!(w.take_completed(), Some(pair));
        w.clear();

        // After the pair fired, a later attack edge starts a new sequence and
        // cannot retroactively turn the already-consumed pair into a triple.
        w.tick_with(
            false,
            false,
            false,
            false,
            false,
            true,
            true,
            false,
            false,
            false,
            false,
            |combo| combo == pair || combo == triple,
        );
        assert!(w.take_completed().is_none());
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
