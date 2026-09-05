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

    /// First key (dir / r / l / e / q) + tail (`q` / `t`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ToolCombo {
    pub first: ToolFirst,
    pub tail: ToolTail,
}

/// Default `rl` (Block→Attack) combo window in seconds @60fps.
pub const DEFAULT_RL_WINDOW_SECS: f32 = 0.1;
const RL_WINDOW_FPS: f32 = 60.0;

/// Convert cfg seconds to frame budget (same 60fps basis as other combo windows).
pub fn rl_window_secs_to_frames(secs: f32) -> u16 {
    (secs * RL_WINDOW_FPS).round().clamp(1.0, 120.0) as u16
}

/// Default prosthetic multi-hit lock after releasing the tail key (@60fps basis for secs).
pub const DEFAULT_TOOL_MULTI_LOCK_SECS: f32 = 1.0;

/// Prosthetic families for `# 多段触发时限` (UID decade ranges).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ToolMultiLockCategory {
    Shuriken,
    Umbrella,
    MistRaven,
    Firecracker,
    Sabimaru,
}

pub fn tool_uid_multi_lock_category(uid: UID) -> Option<ToolMultiLockCategory> {
    match uid {
        70000..=70999 => Some(ToolMultiLockCategory::Shuriken),
        71000..=71999 => Some(ToolMultiLockCategory::Firecracker),
        74000..=74999 => Some(ToolMultiLockCategory::MistRaven),
        75000..=75999 => Some(ToolMultiLockCategory::Sabimaru),
        76000..=76999 => Some(ToolMultiLockCategory::Umbrella),
        _ => None,
    }
}

pub fn tool_multi_lock_secs_to_frames(secs: f32) -> u16 {
    if secs <= 0.0 {
        0
    } else {
        (secs * RL_WINDOW_FPS).round().clamp(1.0, 300.0) as u16
    }
}

#[derive(Debug)]
pub struct Config {
    /// Combat arts keyed by r/l/e + direction pairs (or empty for ∅).
    pub arts: HashMap<ArtCombo, UID>,
    /// Two-key prosthetic binds: (↑↓←→|r|l|e|q) then (q|t).
    pub tools: HashMap<ToolCombo, UID>,
    /// Unique bare-`t` prosthetic (default after other tools are released).
    pub tool_on_t: Option<UID>,
    /// Unique bare-`q` prosthetic (same fire style as bare `t`; does not become return-default).
    pub tool_on_q: Option<UID>,
    /// `rl` only: max frames between `r` and `l` (@60fps).
    pub rl_combo_max_age: u16,
    /// Startup progress console (`# 启动信息print窗口`).
    pub boot_console: bool,
    /// Per-family multi-hit lock after tail release (`# 多段触发时限`).
    pub tool_multi_lock_secs: HashMap<ToolMultiLockCategory, f32>,
}

impl Config {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Config> {
        // Do not uppercase the whole file: art tokens use lowercase r/l/e/q/t.
        Ok(fs::read_to_string(path)?.into())
    }

    pub fn art(&self, combo: ArtCombo) -> Option<UID> {
        self.arts.get(&combo).copied()
    }

    pub fn tool(&self, combo: ToolCombo) -> Option<UID> {
        self.tools.get(&combo).copied()
    }

    /// Lock frames for re-triggering the same prosthetic after releasing the tail key.
    pub fn tool_lock_frames(&self, uid: Option<UID>) -> u16 {
        let secs = uid
            .and_then(tool_uid_multi_lock_category)
            .and_then(|cat| self.tool_multi_lock_secs.get(&cat).copied())
            .unwrap_or(DEFAULT_TOOL_MULTI_LOCK_SECS);
        tool_multi_lock_secs_to_frames(secs)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            arts: HashMap::new(),
            tools: HashMap::new(),
            tool_on_t: None,
            tool_on_q: None,
            rl_combo_max_age: rl_window_secs_to_frames(DEFAULT_RL_WINDOW_SECS),
            boot_console: false,
            tool_multi_lock_secs: HashMap::new(),
        }
    }
}

impl<S: AsRef<str>> From<S> for Config {
    fn from(value: S) -> Config {
        let mut config = Config::default();
        for line in value.as_ref().lines() {
            if let Some(secs) = parse_rl_window_comment(line) {
                config.rl_combo_max_age = rl_window_secs_to_frames(secs);
                continue;
            }
            if let Some(v) = crate::cfg_meta::parse_boot_console_comment(line) {
                config.boot_console = v;
                continue;
            }
            if let Some((cat, secs)) = parse_tool_multi_lock_entry(line) {
                config.tool_multi_lock_secs.insert(cat, secs);
                continue;
            }
            if let Some(entries) = parse_tool_multi_lock_bulk(line) {
                for (cat, secs) in entries {
                    config.tool_multi_lock_secs.insert(cat, secs);
                }
                continue;
            }

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
                for alt in split_motion_alternates(motion) {
                    match parse_tool_motion(alt) {
                        Some(ToolMotion::TAlone) => {
                            if config.tool_on_t.is_some() {
                                log::warn!("Multiple bare-t tools; keeping first, ignoring {id}");
                            } else {
                                config.tool_on_t = Some(id);
                            }
                        }
                        Some(ToolMotion::QAlone) => {
                            if config.tool_on_q.is_some() {
                                log::warn!("Multiple bare-q tools; keeping first, ignoring {id}");
                            } else {
                                config.tool_on_q = Some(id);
                            }
                        }
                        Some(ToolMotion::Combo(combo)) => {
                            insert_tool_bind(&mut config, combo, id);
                        }
                        None => {}
                    }
                }
                continue;
            }

            for alt in split_motion_alternates(motion) {
                if let Some(combo) = parse_art_combo(alt) {
                    insert_art_bind(&mut config, combo, id);
                }
            }
        }
        config
    }
}

fn split_motion_alternates(motion: &str) -> impl Iterator<Item = &str> {
    motion.split('/').map(str::trim).filter(|s| !s.is_empty())
}

fn insert_art_bind(config: &mut Config, combo: ArtCombo, id: UID) {
    if let Some(prev) = config.arts.get(&combo) {
        if *prev != id {
            log::warn!(
                "Duplicate art bind {:?}: keeping UID {prev}, ignoring {id}",
                combo
            );
        }
        return;
    }
    config.arts.insert(combo, id);
}

fn insert_tool_bind(config: &mut Config, combo: ToolCombo, id: UID) {
    if let Some(prev) = config.tools.get(&combo) {
        if *prev != id {
            log::warn!(
                "Duplicate tool bind {:?}: keeping UID {prev}, ignoring {id}",
                combo
            );
        }
        return;
    }
    config.tools.insert(combo, id);
}

/// `# rl触发时限: 0.1s` or `# rl window: 0.1s` (optional trailing `s`).
fn parse_rl_window_comment(line: &str) -> Option<f32> {
    let text = line.trim();
    if !text.starts_with('#') {
        return None;
    }
    let body = text.trim_start_matches('#').trim();
    let key = body.split([':', '：']).next()?.trim().to_ascii_lowercase();
    if key != "rl触发时限" && key != "rl window" {
        return None;
    }
    let rest = body.split([':', '：']).nth(1)?.trim();
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let secs = num.parse::<f32>().ok()?;
    (secs > 0.0).then_some(secs)
}

/// `# 多段触发时限: 手里剑 0s, 锈丸 0.5s` or `# multi-hit window: shuriken 0s, sabimaru 0.5s`
fn parse_tool_multi_lock_bulk(line: &str) -> Option<Vec<(ToolMultiLockCategory, f32)>> {
    let text = line.trim();
    if !text.starts_with('#') {
        return None;
    }
    let body = text.trim_start_matches('#').trim();
    let key = body.split([':', '：']).next()?.trim().to_ascii_lowercase();
    if key != "多段触发时限" && key != "multi-hit window" {
        return None;
    }
    let rest = body.split([':', '：']).nth(1)?.trim();
    let mut out = Vec::new();
    for part in rest.split(['，', ',']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, secs) = part.rsplit_once(' ').or_else(|| part.rsplit_once('\t'))?;
        let cat = parse_tool_multi_lock_category(name.trim())?;
        out.push((cat, parse_secs_value(secs.trim())?));
    }
    (!out.is_empty()).then_some(out)
}

/// `# 多段触发时限·锈丸: 0.5s`
fn parse_tool_multi_lock_entry(line: &str) -> Option<(ToolMultiLockCategory, f32)> {
    let text = line.trim();
    if !text.starts_with('#') {
        return None;
    }
    let body = text.trim_start_matches('#').trim();
    let (key, rest) = body.split_once([':', '：'])?;
    let key = key.trim();
    let prefix = if key.starts_with("多段触发时限·") {
        "多段触发时限·"
    } else if key.to_ascii_lowercase().starts_with("multi-hit window·") {
        "multi-hit window·"
    } else if key.to_ascii_lowercase().starts_with("multi-hit window.") {
        "multi-hit window."
    } else {
        return None;
    };
    let cat = parse_tool_multi_lock_category(key.trim_start_matches(prefix).trim())?;
    Some((cat, parse_secs_value(rest.trim())?))
}

fn parse_tool_multi_lock_category(name: &str) -> Option<ToolMultiLockCategory> {
    match name.trim().to_ascii_lowercase().as_str() {
        "手里剑" | "shuriken" => Some(ToolMultiLockCategory::Shuriken),
        "伞" | "umbrella" => Some(ToolMultiLockCategory::Umbrella),
        "雾鸦" | "mist raven" | "mistraven" | "mist" => Some(ToolMultiLockCategory::MistRaven),
        "爆竹" | "firecracker" => Some(ToolMultiLockCategory::Firecracker),
        "锈丸" | "sabimaru" => Some(ToolMultiLockCategory::Sabimaru),
        _ => None,
    }
}

fn parse_secs_value(raw: &str) -> Option<f32> {
    let num: String = raw
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f32>().ok().filter(|&s| s >= 0.0)
}

enum ToolMotion {
    TAlone,
    QAlone,
    Combo(ToolCombo),
}

/// `t` / `q` alone, or first∈{↑↓←→,r,l,e,q} then tail∈{q,t}. `f` still parses as interact (legacy).
/// Bare `q` is exclusive with using `q` as a two-key first (e.g. `qt`).
fn parse_tool_motion(motion: &str) -> Option<ToolMotion> {
    let motion = motion.trim();
    if motion == "t" || motion == "T" {
        return Some(ToolMotion::TAlone);
    }
    if motion == "q" || motion == "Q" {
        return Some(ToolMotion::QAlone);
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
        'f' | 'F' | 'e' | 'E' => ToolFirst::Interact,
        'q' | 'Q' => ToolFirst::Switch,
        't' | 'T' => return None, // t cannot be first of a two-key bind
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

/// Combat-art motions: `∅`, or exactly two of {↑↓←→, r, l, e} (e.g. `r↑`, `ee`, `rl`). `f` still parses as interact (legacy).
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
            'f' | 'F' | 'e' | 'E' => ArtToken::Interact,
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
        config::{rl_window_secs_to_frames, Config, ToolCombo, ToolFirst, ToolTail},
        input::{ArtCombo, ArtToken},
    };

    #[test]
    fn test_boot_console_comment() {
        use crate::cfg_meta::parse_boot_console_comment;

        assert_eq!(
            parse_boot_console_comment("# 启动信息print窗口: 关"),
            Some(false)
        );
        assert_eq!(parse_boot_console_comment("# 启动窗口: 关"), Some(false));
        assert_eq!(parse_boot_console_comment("# boot console: on"), Some(true));
        let config = Config::from("# 启动信息print窗口: 开\n7100 x ee");
        assert!(config.boot_console);
        assert!(!Config::default().boot_console);
    }

    #[test]
    fn test_rl_window_comment() {
        let config = Config::from(
            "# rl触发时限: 0.2s\n7100  x  ee",
        );
        assert_eq!(config.rl_combo_max_age, 12);

        let config = Config::from("# rl window: 0.15\n7700 Sakura rl");
        assert_eq!(config.rl_combo_max_age, 9);
        assert_eq!(config.rl_combo_max_age, rl_window_secs_to_frames(0.15));
    }

    #[test]
    fn test_tool_multi_lock_comment() {
        use crate::config::{
            tool_multi_lock_secs_to_frames, ToolMultiLockCategory, DEFAULT_TOOL_MULTI_LOCK_SECS,
        };

        let config = Config::from(
            "# 多段触发时限: 手里剑 0s, 锈丸 0.5s\n70500 x t",
        );
        assert_eq!(config.tool_lock_frames(Some(70500)), 0);
        assert_eq!(config.tool_lock_frames(Some(75300)), 30);
        assert_eq!(
            config.tool_lock_frames(Some(78400)),
            tool_multi_lock_secs_to_frames(DEFAULT_TOOL_MULTI_LOCK_SECS),
        );

        let config = Config::from("# 多段触发时限·雾鸦: 0s\n74100 x q");
        assert_eq!(config.tool_lock_frames(Some(74100)), 0);
        assert!(config
            .tool_multi_lock_secs
            .contains_key(&ToolMultiLockCategory::MistRaven));
    }

    #[test]
    fn test_art_multi_bind() {
        let config = Config::from("7000  Nightjar Reversal  ↓l/l↓");
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Down, ArtToken::Attack)),
            Some(7000)
        );
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Attack, ArtToken::Down)),
            Some(7000)
        );
    }

    #[test]
    fn test_load_tools_and_arts() {
        let raw = "
            7100  Ichimonji: Double           ee
            5500  Ashina Cross                r↓
            7400  High Monk                   el
            7700  Sakura Dance                rl
            5400  Dragon Flash                re
            6100  One Mind                    er
            70500 Lazulite Shuriken           t
            74100 Aged Feather Mist Raven     q
            75300 Lazulite Sabimaru           ↑t
            72200 Long Spark                  ↓q
            79200 Mountain Echo               →t
            ";
        let config = Config::from(raw);
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Interact, ArtToken::Interact)),
            Some(7100)
        );
        assert_eq!(config.tool_on_t, Some(70500));
        assert_eq!(config.tool_on_q, Some(74100));
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
                first: ToolFirst::Right,
                tail: ToolTail::Use
            }),
            Some(79200)
        );
    }
}
