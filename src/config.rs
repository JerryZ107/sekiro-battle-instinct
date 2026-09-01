use std::{collections::HashMap, fs, io, path::Path};

use widestring::U16CStr;

use crate::{
    core::UID,
    game,
    input::{ArtCombo, ArtToken},
};

const COMBART_ART_UID_MIN: UID = 5000;
const COMBART_ART_UID_MAX: UID = 10000;
const PROSTHETIC_TOOL_UID_MIN: UID = 70000;
const PROSTHETIC_TOOL_UID_MAX: UID = 100000;

/// Tail key of a prosthetic combo: `q` = switch, `t` = use.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ToolTail {
    /// In-game「切换忍具」(action 0x400).
    Switch,
    /// In-game「使用忍具」(USE_PROSTHETIC).
    Use,
}

/// First key of a prosthetic combo. `t` cannot be first; `q` can.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ToolFirst {
    Up,
    Right,
    Down,
    Left,
    Block,
    Attack,
    Interact,
    /// `q` as first key (e.g. `qt`).
    Switch,
}

impl ToolFirst {
    pub fn from_art(token: ArtToken) -> ToolFirst {
        match token {
            ArtToken::Up => ToolFirst::Up,
            ArtToken::Right => ToolFirst::Right,
            ArtToken::Down => ToolFirst::Down,
            ArtToken::Left => ToolFirst::Left,
            ArtToken::Block => ToolFirst::Block,
            ArtToken::Attack => ToolFirst::Attack,
            ArtToken::Interact => ToolFirst::Interact,
        }
    }
}

/// First key (dir / r / l / f / q) + tail (`q` / `t`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ToolCombo {
    pub first: ToolFirst,
    pub tail: ToolTail,
}

#[derive(Debug)]
pub struct Config {
    /// Combat arts keyed by r/l/f + direction pairs (or empty for ∅).
    pub arts: HashMap<ArtCombo, UID>,
    /// Two-key prosthetic binds: (↑↓←→|r|l|f|q) then (q|t). `t` cannot be first.
    pub tools: HashMap<ToolCombo, UID>,
    /// Unique bare-`t` prosthetic (default after other tools are released).
    pub tool_on_t: Option<UID>,
}

impl Config {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Config> {
        // Do not uppercase the whole file: art tokens use lowercase r/l/f/q/t.
        Ok(fs::read_to_string(path)?.into())
    }

    pub fn art(&self, combo: ArtCombo) -> Option<UID> {
        self.arts.get(&combo).copied()
    }

    pub fn tool(&self, combo: ToolCombo) -> Option<UID> {
        self.tools.get(&combo).copied()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            arts: HashMap::new(),
            tools: HashMap::new(),
            tool_on_t: None,
        }
    }
}

impl<S: AsRef<str>> From<S> for Config {
    fn from(value: S) -> Config {
        let mut config = Config::default();
        for line in value.as_ref().lines() {
            let mut items = line.split_whitespace().take_while(|item| !item.starts_with("#"));
            let Some(id) = items.next().and_then(|id| id.parse::<UID>().ok()) else {
                continue;
            };
            let tool = match id {
                PROSTHETIC_TOOL_UID_MIN..=PROSTHETIC_TOOL_UID_MAX => true,
                COMBART_ART_UID_MIN..=COMBART_ART_UID_MAX => false,
                _ => {
                    log::warn!("Illegal ID {id} is ignored.");
                    continue;
                }
            };

            let Some(motion) = items.last() else {
                continue;
            };

            if tool {
                match parse_tool_motion(motion) {
                    Some(ToolMotion::TAlone) => {
                        if config.tool_on_t.is_some() {
                            log::warn!("Multiple bare-t tools; keeping first, ignoring {id}");
                        } else {
                            config.tool_on_t = Some(id);
                        }
                    }
                    Some(ToolMotion::Combo(combo)) => {
                        if let Some(prev) = config.tools.get(&combo) {
                            log::warn!(
                                "Duplicate tool bind {:?}: keeping UID {prev}, ignoring {id}",
                                combo
                            );
                        } else {
                            config.tools.insert(combo, id);
                        }
                    }
                    None => {
                        // Unbound prosthetic line (name only) — ignore.
                    }
                }
                continue;
            }

            if let Some(combo) = parse_art_combo(motion) {
                config.arts.insert(combo, id);
            }
        }
        config
    }
}

enum ToolMotion {
    TAlone,
    Combo(ToolCombo),
}

/// `t` alone, or first∈{↑↓←→,r,l,f,q} then tail∈{q,t}. `t` cannot be the first key.
fn parse_tool_motion(motion: &str) -> Option<ToolMotion> {
    let motion = motion.trim();
    if motion == "t" || motion == "T" {
        return Some(ToolMotion::TAlone);
    }

    let mut chars = motion.chars();
    let first_ch = chars.next()?;
    let first = match first_ch {
        '↑' => ToolFirst::Up,
        '→' => ToolFirst::Right,
        '↓' => ToolFirst::Down,
        '←' => ToolFirst::Left,
        'r' | 'R' => ToolFirst::Block,
        'l' | 'L' => ToolFirst::Attack,
        'f' | 'F' => ToolFirst::Interact,
        'q' | 'Q' => ToolFirst::Switch,
        't' | 'T' => return None, // t cannot be first
        _ => return None,
    };
    let tail_ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let tail = match tail_ch {
        'q' | 'Q' => ToolTail::Switch,
        't' | 'T' => ToolTail::Use,
        _ => return None,
    };
    Some(ToolMotion::Combo(ToolCombo { first, tail }))
}

/// Combat-art motions: `∅`, or exactly two of {↑↓←→, r, l, f} (e.g. `r↑`, `ff`, `rl`).
fn parse_art_combo(motion: &str) -> Option<ArtCombo> {
    let motion = motion.trim();
    if matches!(motion, "∅" | "NONE" | "none") {
        return Some(ArtCombo::empty());
    }

    let mut tokens = Vec::new();
    for ch in motion.chars() {
        let token = match ch {
            '↑' => ArtToken::Up,
            '→' => ArtToken::Right,
            '↓' => ArtToken::Down,
            '←' => ArtToken::Left,
            'r' | 'R' => ArtToken::Block,
            'l' | 'L' => ArtToken::Attack,
            'f' | 'F' => ArtToken::Interact,
            _ => return None,
        };
        tokens.push(token);
    }
    if tokens.len() == 2 {
        Some(ArtCombo::pair(tokens[0], tokens[1]))
    } else {
        None
    }
}

#[allow(unused)]
fn get_item_name(uid: UID) -> Option<String> {
    let p = game::get_item_name(game::msg_repo(), uid);
    if p.is_null() {
        return None;
    } else {
        let name = unsafe { U16CStr::from_ptr_str(p) };
        Some(name.to_string_lossy())
    }
}

#[cfg(test)]
mod test {
    use crate::{
        config::{Config, ToolCombo, ToolFirst, ToolTail},
        input::{ArtCombo, ArtToken},
    };

    #[test]
    fn test_load_tools_and_arts() {
        let raw = "
            7100  Ichimonji: Double           rl
            5500  Ashina Cross                r↓
            7400  High Monk                   fl
            7700  Sakura Dance                ff
            5400  Dragon Flash                rf
            6100  One Mind                    fr
            70500 Lazulite Shuriken           t
            74100 Aged Feather Mist Raven     ↑q
            75300 Lazulite Sabimaru           ↑t
            72200 Long Spark                  ↓q
            73200 Sparking Axe                qt
            ";
        let config = Config::from(raw);
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Block, ArtToken::Attack)),
            Some(7100)
        );
        assert_eq!(config.tool_on_t, Some(70500));
        assert_eq!(
            config.tool(ToolCombo {
                first: ToolFirst::Up,
                tail: ToolTail::Switch
            }),
            Some(74100)
        );
        assert_eq!(
            config.tool(ToolCombo {
                first: ToolFirst::Up,
                tail: ToolTail::Use
            }),
            Some(75300)
        );
        assert_eq!(
            config.tool(ToolCombo {
                first: ToolFirst::Down,
                tail: ToolTail::Switch
            }),
            Some(72200)
        );
        assert_eq!(
            config.tool(ToolCombo {
                first: ToolFirst::Switch,
                tail: ToolTail::Use
            }),
            Some(73200)
        );
    }
}
