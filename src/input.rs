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
    pub fn is_direction(self) -> bool {
        matches!(self, ArtToken::Up | ArtToken::Right | ArtToken::Down | ArtToken::Left)
    }
}

/// Last two combat-art key edges within a short window.
pub struct ArtComboWindow {
    first: Option<ArtToken>,
    second: Option<ArtToken>,
    age: u16,
    completed_this_frame: Option<ArtCombo>,
    /// `l` after `r` outside `# rl触发时限` (core strips vanilla B+A once).
    late_rl_rejected: bool,
    keys_down: [bool; 4],
    block_down: bool,
    attack_down: bool,
    interact_down: bool,
    grapple_down: bool,
    jump_down: bool,
    sprint_down: bool,
    crouch_down: bool,
    max_age_rl: u16,
}

impl ArtComboWindow {
    /// ~300ms at 60fps for most two-token arts (~0.3s).
    const MAX_AGE: u16 = 18;
    /// Same window for `ee` / e-then-X and `ff` / f-then-X (~0.3s).
    const MAX_AGE_FF: u16 = 18;
    /// Longer window when first key is `l` (attack), ~0.7s @60fps.
    const MAX_AGE_L: u16 = 42;

    pub fn new(max_age_rl: u16) -> ArtComboWindow {
        ArtComboWindow {
            first: None,
            second: None,
            age: 0,
            completed_this_frame: None,
            late_rl_rejected: false,
            keys_down: [false; 4],
            block_down: false,
            attack_down: false,
            interact_down: false,
            grapple_down: false,
            jump_down: false,
            sprint_down: false,
            crouch_down: false,
            max_age_rl,
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
        grapple: bool,
        jump: bool,
        sprint: bool,
        crouch: bool,
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
                updated |= self.push(token);
            }
            self.keys_down[i] = held;
        }

        // Button edges: block → interact → attack so same-frame rf/fl resolve correctly;
        // order-sensitive pairs (fr, lf) still need sequential presses.
        if block && !self.block_down {
            updated |= self.push(ArtToken::Block);
        }
        self.block_down = block;

        if interact && !self.interact_down {
            updated |= self.push(ArtToken::Interact);
        }
        self.interact_down = interact;

        if grapple && !self.grapple_down {
            updated |= self.push(ArtToken::Grapple);
        }
        self.grapple_down = grapple;

        if jump && !self.jump_down {
            updated |= self.push(ArtToken::Jump);
        }
        self.jump_down = jump;

        if sprint && !self.sprint_down {
            updated |= self.push(ArtToken::Sprint);
        }
        self.sprint_down = sprint;

        if crouch && !self.crouch_down {
            updated |= self.push(ArtToken::Crouch);
        }
        self.crouch_down = crouch;

        if attack && !self.attack_down {
            updated |= self.push(ArtToken::Attack);
        }
        self.attack_down = attack;

        if updated {
            self.age = 0;
        } else {
            self.age = self.age.saturating_add(1);
            let max_age = match (self.first, self.second) {
                (Some(ArtToken::Attack), None) => Self::MAX_AGE_L,
                (Some(ArtToken::Interact), None) | (Some(ArtToken::Grapple), None) => Self::MAX_AGE_FF,
                // After `r`, `rl` window while block held; +1 frame so late `l` can set `late_rl_rejected`.
                (Some(ArtToken::Block), None) if self.block_down => self.max_age_rl.saturating_add(1),
                _ => Self::MAX_AGE,
            };
            if self.age >= max_age {
                self.clear();
            }
        }
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
                    if self.age >= self.max_age_rl {
                        self.late_rl_rejected = true;
                        self.clear();
                        return false;
                    }
                    if !self.block_down {
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
        match token {
            ArtToken::Up => self.keys_down[0],
            ArtToken::Right => self.keys_down[1],
            ArtToken::Down => self.keys_down[2],
            ArtToken::Left => self.keys_down[3],
            ArtToken::Block => self.block_down,
            ArtToken::Attack => self.attack_down,
            ArtToken::Interact => self.interact_down,
            ArtToken::Grapple => self.grapple_down,
            ArtToken::Jump => self.jump_down,
            ArtToken::Sprint => self.sprint_down,
            ArtToken::Crouch => self.crouch_down,
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
    use super::{ArtCombo, ArtComboWindow, ArtToken};

    #[test]
    fn test_art_button_pairs() {
        // same-frame rl
        let mut w = ArtComboWindow::new(6);
        w.tick(
            false, false, false, false, true, true, false, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Attack))
        );

        // same-frame r+e
        let mut w = ArtComboWindow::new(6);
        w.tick(
            false, false, false, false, true, false, true, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Block, ArtToken::Interact))
        );

        // same-frame e+l
        let mut w = ArtComboWindow::new(6);
        w.tick(
            false, false, false, false, false, true, true, false, false, false, false,
        );
        assert_eq!(
            w.take_completed(),
            Some(ArtCombo::pair(ArtToken::Interact, ArtToken::Attack))
        );

        // sequential e+r
        let mut w = ArtComboWindow::new(6);
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
        let mut w = ArtComboWindow::new(6);
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
        let mut w = ArtComboWindow::new(6);
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
        let mut w = ArtComboWindow::new(6);
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
        let mut w = ArtComboWindow::new(18);
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
        let mut w = ArtComboWindow::new(6);
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
        let mut w = ArtComboWindow::new(6);
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
        let mut w = ArtComboWindow::new(18);
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

        let mut w = ArtComboWindow::new(18);
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

        let mut w = ArtComboWindow::new(18);
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
