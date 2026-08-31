use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
};

use widestring::U16CStr;

use crate::{
    core::UID,
    game,
    input::{
        ArtCombo, ArtToken, Inputs, InputsTrie,
        Input::*,
    },
};

const COMBART_ART_UID_MIN: UID = 5000;
const COMBART_ART_UID_MAX: UID = 10000;
const PROSTHETIC_TOOL_UID_MIN: UID = 70000;
const PROSTHETIC_TOOL_UID_MAX: UID = 100000;

#[derive(Debug)]
pub struct Config {
    /// Combat arts keyed by r/l/f + direction pairs (or empty for ∅).
    pub arts: HashMap<ArtCombo, UID>,
    pub tools: InputsTrie<&'static [UID]>,
    pub tools_for_block: &'static [UID],
    pub tools_on_x1: &'static [UID],
    pub tools_on_x2: &'static [UID],
}

impl Config {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Config> {
        // Do not uppercase the whole file: art tokens use lowercase r/l/f.
        Ok(fs::read_to_string(path)?.into())
    }

    pub fn art(&self, combo: ArtCombo) -> Option<UID> {
        self.arts.get(&combo).copied()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            arts: HashMap::new(),
            tools: InputsTrie::new(),
            tools_for_block: &[],
            tools_on_x1: &[],
            tools_on_x2: &[],
        }
    }
}

impl<S: AsRef<str>> From<S> for Config {
    fn from(value: S) -> Config {
        let mut config = Config::default();
        let mut tools = HashMap::<Inputs, Vec<UID>>::new();
        let mut tools_for_block = Vec::new();
        let mut tools_on_x1 = Vec::new();
        let mut tools_on_x2 = Vec::new();
        let mut used_inputs = HashSet::new();
        for line in value.as_ref().lines() {
            let mut items = line.split_whitespace().take_while(|item| !item.starts_with("#"));
            let Some(id) = items.next().and_then(|id| id.parse::<UID>().ok()) else {
                continue;
            };
            let Some(inputs) = items.last() else {
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

            if tool {
                let upper = inputs.to_ascii_uppercase();
                match upper.as_str() {
                    "X1" | "M4" => tools_on_x1.push(id),
                    "X2" | "M5" => tools_on_x2.push(id),
                    "BLOCK" => tools_for_block.push(id),
                    _ if inputs == "\u{26c9}" || inputs == "\u{26e8}" => {
                        tools_for_block.push(id);
                    }
                    other => {
                        if let Some(motion) = parse_motion(other) {
                            used_inputs.insert(motion);
                            tools.entry(motion).or_insert_with(Vec::new).push(id);
                        } else if let Some(motion) = parse_motion(inputs) {
                            used_inputs.insert(motion);
                            tools.entry(motion).or_insert_with(Vec::new).push(id);
                        }
                    }
                }
            } else if let Some(combo) = parse_art_combo(inputs) {
                config.arts.insert(combo, id);
            }
        }

        for (inputs, tools) in tools {
            config.tools.insert(inputs, tools.leak());
        }
        config.tools_for_block = tools_for_block.leak();
        config.tools_on_x1 = tools_on_x1.leak();
        config.tools_on_x2 = tools_on_x2.leak();

        for inputs in used_inputs {
            for alt_inputs in possible_altenrnatives(inputs) {
                if let Some(tools) = config.tools.get(inputs) {
                    config.tools.try_insert(alt_inputs, tools);
                }
            }
        }
        config
    }
}

/// Combat-art motions: `∅`, or exactly two of {↑↓←→, r, l, f} (e.g. `r↑`, `ff`, `rl`, `rf`, `fr`, `fl`).
/// `r` = mouse right (block), `l` = mouse left (attack), `f` = interact.
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

fn parse_motion(motion: &str) -> Option<Inputs> {
    if matches!(motion, "∅" | "NONE" | "none") {
        Some(Inputs::new())
    } else {
        let chars = motion.chars();
        let char_count = chars.count();
        let inputs = motion
            .trim()
            .chars()
            .filter_map(|ch| ch.try_into().ok())
            .collect::<Vec<_>>();
        if inputs.len() != char_count {
            return None;
        }
        Some(inputs.into_iter().take(3).collect::<Inputs>())
    }
}

#[allow(unused)]
fn possible_altenrnatives(mut inputs: Inputs) -> Vec<Inputs> {
    if inputs.len() == 2 {
        let mut possible_inputs = Vec::new();
        possible_inputs.push(inputs.rev());
        let tail = inputs.pop().unwrap();
        let head = inputs.pop().unwrap();
        if tail == head {
            possible_inputs.push(Inputs::from([tail, tail, tail]));
        } else if tail == head.opposite() {
            possible_inputs.push(Inputs::from([head, tail.rotate(), tail]));
            possible_inputs.push(Inputs::from([head, head.rotate(), tail]));
        }
        possible_inputs
    } else if inputs == [Left, Down, Right].into() {
        vec![
            Inputs::from([Left, Right, Down]),
            Inputs::from([Right, Left, Down]),
            Inputs::from([Right, Down, Left]),
            Inputs::from([Down, Left, Right]),
            Inputs::from([Down, Right, Left]),
        ]
    } else {
        Vec::new()
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
        config::Config,
        input::{ArtCombo, ArtToken},
        input::Input::*,
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
            70000 Loaded Shuriken             ∅
            74000 Mist Raven                 ←→
            ";
        let config = Config::from(raw);
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Block, ArtToken::Attack)),
            Some(7100)
        );
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Block, ArtToken::Down)),
            Some(5500)
        );
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Interact, ArtToken::Attack)),
            Some(7400)
        );
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Interact, ArtToken::Interact)),
            Some(7700)
        );
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Block, ArtToken::Interact)),
            Some(5400)
        );
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Interact, ArtToken::Block)),
            Some(6100)
        );
        assert_eq!(config.tools.get_or_default([]), [70000]);
        assert_eq!(config.tools.get_or_default([Left, Right]), &[74000]);
    }
}


