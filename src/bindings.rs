use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, anyhow};

use crate::binding_validation::{BindingValidationError, validate_scheme, validate_scheme_set};

use crate::{
    config::{Config, ToolCombo, ToolFirst, ToolTail},
    input::{ArtCombo, ArtToken},
};

pub const KEYBIND_FILE: &str = "battle_instinct_keybinds.json";
const KEYBIND_VERSION: u32 = 3;

pub const COMBAT_ART_UID_MIN: u32 = 5000;
pub const COMBAT_ART_UID_MAX: u32 = 10000;
pub const PROSTHETIC_UID_MIN: u32 = 70000;
pub const PROSTHETIC_UID_MAX: u32 = 100000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyToken {
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
    ProstheticUse,
    ProstheticSwitch,
}

impl KeyToken {
    pub const ALL_NORMAL: [KeyToken; 11] = [
        KeyToken::Up,
        KeyToken::Right,
        KeyToken::Down,
        KeyToken::Left,
        KeyToken::Block,
        KeyToken::Attack,
        KeyToken::Interact,
        KeyToken::Grapple,
        KeyToken::Jump,
        KeyToken::Sprint,
        KeyToken::Crouch,
    ];
    pub fn label(self) -> &'static str {
        match self {
            KeyToken::Up => "移动前",
            KeyToken::Right => "移动右",
            KeyToken::Down => "移动后",
            KeyToken::Left => "移动左",
            KeyToken::Block => "防御",
            KeyToken::Attack => "攻击",
            KeyToken::Interact => "交互",
            KeyToken::Grapple => "钩锁",
            KeyToken::Jump => "跳跃",
            KeyToken::Sprint => "冲刺",
            KeyToken::Crouch => "下蹲",
            KeyToken::ProstheticUse => "忍具键",
            KeyToken::ProstheticSwitch => "忍具切换键",
        }
    }

    pub fn is_prosthetic(self) -> bool {
        matches!(self, KeyToken::ProstheticUse | KeyToken::ProstheticSwitch)
    }

    pub fn from_art(token: ArtToken) -> KeyToken {
        match token {
            ArtToken::Up => KeyToken::Up,
            ArtToken::Right => KeyToken::Right,
            ArtToken::Down => KeyToken::Down,
            ArtToken::Left => KeyToken::Left,
            ArtToken::Block => KeyToken::Block,
            ArtToken::Attack => KeyToken::Attack,
            ArtToken::Interact => KeyToken::Interact,
            ArtToken::Grapple => KeyToken::Grapple,
            ArtToken::Jump => KeyToken::Jump,
            ArtToken::Sprint => KeyToken::Sprint,
            ArtToken::Crouch => KeyToken::Crouch,
        }
    }

    pub fn to_art(self) -> Option<ArtToken> {
        Some(match self {
            KeyToken::Up => ArtToken::Up,
            KeyToken::Right => ArtToken::Right,
            KeyToken::Down => ArtToken::Down,
            KeyToken::Left => ArtToken::Left,
            KeyToken::Block => ArtToken::Block,
            KeyToken::Attack => ArtToken::Attack,
            KeyToken::Interact => ArtToken::Interact,
            KeyToken::Grapple => ArtToken::Grapple,
            KeyToken::Jump => ArtToken::Jump,
            KeyToken::Sprint => ArtToken::Sprint,
            KeyToken::Crouch => ArtToken::Crouch,
            KeyToken::ProstheticUse | KeyToken::ProstheticSwitch => return None,
        })
    }

    pub fn from_tool_first(token: ToolFirst) -> KeyToken {
        match token {
            ToolFirst::Up => KeyToken::Up,
            ToolFirst::Right => KeyToken::Right,
            ToolFirst::Down => KeyToken::Down,
            ToolFirst::Left => KeyToken::Left,
            ToolFirst::Block => KeyToken::Block,
            ToolFirst::Attack => KeyToken::Attack,
            ToolFirst::Interact => KeyToken::Interact,
            ToolFirst::Grapple => KeyToken::Grapple,
            ToolFirst::Jump => KeyToken::Jump,
            ToolFirst::Sprint => KeyToken::Sprint,
            ToolFirst::Crouch => KeyToken::Crouch,
            ToolFirst::Switch => KeyToken::ProstheticSwitch,
        }
    }

    pub fn to_tool_first(self) -> Option<ToolFirst> {
        Some(match self {
            KeyToken::Up => ToolFirst::Up,
            KeyToken::Right => ToolFirst::Right,
            KeyToken::Down => ToolFirst::Down,
            KeyToken::Left => ToolFirst::Left,
            KeyToken::Block => ToolFirst::Block,
            KeyToken::Attack => ToolFirst::Attack,
            KeyToken::Interact => ToolFirst::Interact,
            KeyToken::Grapple => ToolFirst::Grapple,
            KeyToken::Jump => ToolFirst::Jump,
            KeyToken::Sprint => ToolFirst::Sprint,
            KeyToken::Crouch => ToolFirst::Crouch,
            KeyToken::ProstheticSwitch => ToolFirst::Switch,
            KeyToken::ProstheticUse => return None,
        })
    }

    pub fn from_tool_tail(token: ToolTail) -> KeyToken {
        match token {
            ToolTail::Switch => KeyToken::ProstheticSwitch,
            ToolTail::Use => KeyToken::ProstheticUse,
            ToolTail::Block => KeyToken::Block,
            ToolTail::Attack => KeyToken::Attack,
            ToolTail::Interact => KeyToken::Interact,
            ToolTail::Grapple => KeyToken::Grapple,
            ToolTail::Jump => KeyToken::Jump,
            ToolTail::Sprint => KeyToken::Sprint,
            ToolTail::Crouch => KeyToken::Crouch,
        }
    }

    pub fn to_tool_tail(self) -> Option<ToolTail> {
        Some(match self {
            KeyToken::ProstheticSwitch => ToolTail::Switch,
            KeyToken::ProstheticUse => ToolTail::Use,
            KeyToken::Block => ToolTail::Block,
            KeyToken::Attack => ToolTail::Attack,
            KeyToken::Interact => ToolTail::Interact,
            KeyToken::Grapple => ToolTail::Grapple,
            KeyToken::Jump => ToolTail::Jump,
            KeyToken::Sprint => ToolTail::Sprint,
            KeyToken::Crouch => ToolTail::Crouch,
            KeyToken::Up | KeyToken::Right | KeyToken::Down | KeyToken::Left => return None,
        })
    }

    pub fn json_name(self) -> &'static str {
        match self {
            KeyToken::Up => "Up",
            KeyToken::Right => "Right",
            KeyToken::Down => "Down",
            KeyToken::Left => "Left",
            KeyToken::Block => "Block",
            KeyToken::Attack => "Attack",
            KeyToken::Interact => "Interact",
            KeyToken::Grapple => "Grapple",
            KeyToken::Jump => "Jump",
            KeyToken::Sprint => "Sprint",
            KeyToken::Crouch => "Crouch",
            KeyToken::ProstheticUse => "ProstheticUse",
            KeyToken::ProstheticSwitch => "ProstheticSwitch",
        }
    }

    pub fn from_json_name(name: &str) -> Option<KeyToken> {
        Some(match name {
            "Up" => KeyToken::Up,
            "Right" => KeyToken::Right,
            "Down" => KeyToken::Down,
            "Left" => KeyToken::Left,
            "Block" => KeyToken::Block,
            "Attack" => KeyToken::Attack,
            "Interact" => KeyToken::Interact,
            "Grapple" => KeyToken::Grapple,
            "Jump" => KeyToken::Jump,
            "Sprint" => KeyToken::Sprint,
            "Crouch" => KeyToken::Crouch,
            "ProstheticUse" => KeyToken::ProstheticUse,
            "ProstheticSwitch" => KeyToken::ProstheticSwitch,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingCategory {
    CombatArt,
    Prosthetic,
}

pub fn category_for_uid(uid: u32) -> Option<BindingCategory> {
    if (COMBAT_ART_UID_MIN..=COMBAT_ART_UID_MAX).contains(&uid) {
        Some(BindingCategory::CombatArt)
    } else if (PROSTHETIC_UID_MIN..=PROSTHETIC_UID_MAX).contains(&uid) {
        Some(BindingCategory::Prosthetic)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingSource {
    Cfg,
    UiOverride,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingEntry {
    pub uid: u32,
    pub keys: Vec<KeyToken>,
    pub source: BindingSource,
}

#[derive(Clone, Debug, Default)]
pub struct UiBindings {
    /// Every UID may have one or more complete trigger schemes.
    pub entries: HashMap<u32, Vec<Vec<KeyToken>>>,
    pub disabled: HashSet<u32>,
}

impl UiBindings {
    pub fn path_for_cfg(cfg_path: &Path) -> PathBuf {
        cfg_path.with_file_name(KEYBIND_FILE)
    }

    pub fn load(cfg_path: &Path) -> Result<UiBindings> {
        let path = Self::path_for_cfg(cfg_path);
        if !path.exists() {
            return Ok(UiBindings::default());
        }
        let text = fs::read_to_string(&path).with_context(|| format!("read keybind JSON {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parse keybind JSON {}", path.display()))
    }

    fn parse(text: &str) -> Result<UiBindings> {
        // The file is generated by this MOD, but keep parsing tolerant of
        // whitespace and hand-editing. A repeated UID means multiple schemes:
        //   {"uid":1234,"keys":["Interact","Attack"]}
        // Older version-2 files contain exactly one object per UID and load
        // unchanged as a one-scheme list.
        let mut out = UiBindings::default();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else {
                return Err(anyhow!("unterminated binding object"));
            };
            let obj = &after[..end];
            if let (Some(uid), Some(keys)) = (parse_uid_field(obj), parse_keys_field(obj)?) {
                validate_scheme(uid, &keys).map_err(|error| anyhow!("UID {uid}: {error}"))?;
                let schemes = out.entries.entry(uid).or_default();
                if !schemes.contains(&keys) {
                    schemes.push(keys);
                }
            }
            rest = &after[end + 1..];
        }
        out.disabled = parse_disabled_field(text);
        out.entries.retain(|uid, _| !out.disabled.contains(uid));
        Ok(out)
    }

    pub fn upsert(&mut self, uid: u32, schemes: Vec<Vec<KeyToken>>) {
        self.entries.insert(uid, schemes);
    }

    pub fn save(&self, cfg_path: &Path) -> Result<()> {
        let path = Self::path_for_cfg(cfg_path);
        let mut uids: Vec<u32> = self
            .entries
            .keys()
            .copied()
            .filter(|uid| !self.disabled.contains(uid))
            .collect();
        uids.sort_unstable();

        let mut objects = Vec::new();
        for uid in uids {
            for keys in &self.entries[&uid] {
                let mut object = String::new();
                object.push_str("    {\"uid\":");
                object.push_str(&uid.to_string());
                object.push_str(",\"keys\":[");
                for (i, key) in keys.iter().enumerate() {
                    if i > 0 {
                        object.push(',');
                    }
                    object.push('"');
                    object.push_str(key.json_name());
                    object.push('"');
                }
                object.push_str("]}");
                objects.push(object);
            }
        }

        let mut text = String::new();
        text.push_str("{\n");
        text.push_str(&format!("  \"version\": {KEYBIND_VERSION},\n"));
        if !self.disabled.is_empty() {
            let mut disabled: Vec<u32> = self.disabled.iter().copied().collect();
            disabled.sort_unstable();
            text.push_str("  \"disabled\": [");
            for (index, uid) in disabled.iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                text.push_str(&uid.to_string());
            }
            text.push_str("],\n");
        }
        text.push_str("  \"bindings\": [\n");
        text.push_str(&objects.join(",\n"));
        if !objects.is_empty() {
            text.push('\n');
        }
        text.push_str("  ]\n}\n");

        let tmp = path.with_extension("json.tmp");
        let backup = path.with_extension("json.bak");
        fs::write(&tmp, text.as_bytes()).with_context(|| format!("write temp keybind JSON {}", tmp.display()))?;
        if path.exists() {
            let _ = fs::copy(&path, &backup);
            fs::remove_file(&path).with_context(|| format!("replace keybind JSON {}", path.display()))?;
        }
        fs::rename(&tmp, &path).with_context(|| format!("commit keybind JSON {}", path.display()))?;
        Ok(())
    }
}

fn parse_uid_field(obj: &str) -> Option<u32> {
    let pos = obj.find("\"uid\"")?;
    let after = &obj[pos + 5..];
    let colon = after.find(':')?;
    let digits: String = after[colon + 1..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn parse_disabled_field(text: &str) -> HashSet<u32> {
    let Some(pos) = text.find("\"disabled\"") else {
        return HashSet::new();
    };
    let after = &text[pos + 10..];
    let Some(open) = after.find('[') else {
        return HashSet::new();
    };
    let Some(close) = after[open + 1..].find(']') else {
        return HashSet::new();
    };
    let arr = &after[open + 1..open + 1 + close];
    let mut out = HashSet::new();
    let mut digits = String::new();
    for ch in arr.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            if let Ok(uid) = digits.parse() {
                out.insert(uid);
            }
            digits.clear();
        }
    }
    if !digits.is_empty() {
        if let Ok(uid) = digits.parse() {
            out.insert(uid);
        }
    }
    out
}

fn parse_keys_field(obj: &str) -> Result<Option<Vec<KeyToken>>> {
    let Some(pos) = obj.find("\"keys\"") else {
        return Ok(None);
    };
    let after = &obj[pos + 6..];
    let Some(open) = after.find('[') else {
        return Err(anyhow!("missing keys array"));
    };
    let Some(close) = after[open + 1..].find(']') else {
        return Err(anyhow!("unterminated keys array"));
    };
    let arr = &after[open + 1..open + 1 + close];
    let mut keys = Vec::new();
    let mut rest = arr;
    while let Some(q0) = rest.find('"') {
        let after_q = &rest[q0 + 1..];
        let Some(q1) = after_q.find('"') else {
            return Err(anyhow!("unterminated key name"));
        };
        let name = &after_q[..q1];
        keys.push(KeyToken::from_json_name(name).ok_or_else(|| anyhow!("unknown key {name}"))?);
        rest = &after_q[q1 + 1..];
    }
    Ok(Some(keys))
}

#[derive(Clone, Debug, Default)]
pub struct BindingSnapshot {
    /// One entry represents one complete trigger scheme. The same UID may
    /// appear multiple times, in the order it should be shown in the editor.
    pub entries: Vec<BindingEntry>,
    pub names: HashMap<u32, String>,
}

impl BindingSnapshot {
    pub fn load(cfg_path: &Path) -> Result<BindingSnapshot> {
        let config = Config::open(cfg_path)?;
        let ui = UiBindings::load(cfg_path)?;
        Ok(Self::from_config(cfg_path, &config, &ui))
    }

    fn from_config(cfg_path: &Path, config: &Config, ui: &UiBindings) -> BindingSnapshot {
        let mut entries = Vec::new();
        let mut seen = HashSet::<(u32, Vec<KeyToken>)>::new();
        let overridden: HashSet<u32> = ui.entries.keys().copied().collect();
        let disabled = ui.disabled.clone();
        let mut push = |uid: u32, keys: Vec<KeyToken>| {
            if overridden.contains(&uid) || disabled.contains(&uid) {
                return;
            }
            let key = (uid, keys.clone());
            if seen.insert(key) {
                entries.push(BindingEntry {
                    uid,
                    keys,
                    source: BindingSource::Cfg,
                });
            }
        };

        // Config maps are unordered. Sort only the cfg-derived sequences so
        // the editor has a deterministic "first scheme"; UI override order is
        // preserved from the JSON array below.
        let mut art_entries = Vec::<(u32, Vec<KeyToken>)>::new();
        for combo in config.arts.keys().chain(config.instant_arts.keys()) {
            let (Some(first), Some(second)) = (combo.first(), combo.second()) else {
                continue;
            };
            let uid = config
                .instant_arts
                .get(combo)
                .or_else(|| config.arts.get(combo))
                .copied();
            if let Some(uid) = uid {
                art_entries.push((uid, vec![KeyToken::from_art(first), KeyToken::from_art(second)]));
            }
        }
        art_entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_slice().cmp(b.1.as_slice())));
        for (uid, keys) in art_entries {
            push(uid, keys);
        }

        let mut tool_entries = Vec::<(u32, Vec<KeyToken>)>::new();
        for (combo, uid) in &config.tools {
            tool_entries.push((
                *uid,
                vec![
                    KeyToken::from_tool_first(combo.first),
                    KeyToken::from_tool_tail(combo.tail),
                ],
            ));
        }
        tool_entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_slice().cmp(b.1.as_slice())));
        for (uid, keys) in tool_entries {
            push(uid, keys);
        }
        if let Some(uid) = config.tool_on_t {
            push(uid, vec![KeyToken::ProstheticUse]);
        }
        if let Some(uid) = config.tool_on_q {
            push(uid, vec![KeyToken::ProstheticSwitch]);
        }

        for (uid, schemes) in &ui.entries {
            if disabled.contains(uid) {
                continue;
            }
            for keys in schemes {
                let key = (*uid, keys.clone());
                if seen.insert(key) {
                    entries.push(BindingEntry {
                        uid: *uid,
                        keys: keys.clone(),
                        source: BindingSource::UiOverride,
                    });
                }
            }
        }

        // Stable sort by UID: configured order above and JSON order for UI
        // overrides are both preserved within one UID.
        entries.sort_by_key(|e| e.uid);

        BindingSnapshot {
            entries,
            names: parse_names(cfg_path),
        }
    }

    #[allow(dead_code)]
    pub fn current_entry(&self, uid: u32) -> Option<&BindingEntry> {
        self.entries.iter().find(|entry| entry.uid == uid)
    }

    pub fn entries_for_uid(&self, uid: u32) -> Vec<&BindingEntry> {
        self.entries.iter().filter(|entry| entry.uid == uid).collect()
    }

    pub fn name(&self, uid: u32) -> Option<&str> {
        self.names.get(&uid).map(String::as_str)
    }

    pub fn validate_candidate(&self, uid: u32, keys: &[KeyToken]) -> Result<(), BindingValidationError> {
        crate::binding_validation::validate_candidate(&self.entries, uid, keys)
    }

    pub fn validate_scheme_set(&self, uid: u32, schemes: &[Vec<KeyToken>]) -> Result<(), BindingValidationError> {
        validate_scheme_set(&self.entries, uid, schemes)
    }

    pub fn conflicting_entries(&self, uid: u32, schemes: &[Vec<KeyToken>]) -> Vec<&BindingEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.uid != uid && schemes.iter().any(|keys| *keys == entry.keys))
            .collect()
    }
}

pub fn parse_names(cfg_path: &Path) -> HashMap<u32, String> {
    let Ok(text) = fs::read_to_string(cfg_path) else {
        return HashMap::new();
    };
    let mut names = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let tokens: Vec<&str> = line
            .split_whitespace()
            .take_while(|token| !token.starts_with('#'))
            .collect();
        if tokens.len() < 2 {
            continue;
        }
        let Ok(uid) = tokens[0].parse::<u32>() else {
            continue;
        };
        if category_for_uid(uid).is_none() {
            continue;
        }
        let rest = &tokens[1..];
        let last_is_motion = rest.last().is_some_and(|token| {
            token.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(
                        c,
                        '↑' | '↓' | '←' | '→' | 'r' | 'l' | 'e' | 'f' | 'q' | 't' | '/' | '-' | '.' | 's'
                    )
            })
        });
        let name_end = if rest.len() >= 2 && last_is_motion {
            rest.len() - 1
        } else {
            rest.len()
        };
        let name = rest[..name_end].join(" ");
        if !name.is_empty() {
            names.entry(uid).or_insert(name);
        }
    }
    names
}

static RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn request_reload() {
    RELOAD_REQUESTED.store(true, Ordering::Release);
}

pub fn take_reload_request() -> bool {
    RELOAD_REQUESTED.swap(false, Ordering::AcqRel)
}

pub fn load_effective_config(cfg_path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(cfg_path)?;
    load_effective_config_from_text(cfg_path, &text)
}

/// Parse the cfg once and apply the persisted UI bindings. Startup passes the
/// same text to Emblem/Flow/PaperGuard instead of reading the file four times.
pub fn load_effective_config_from_text(cfg_path: &Path, text: &str) -> Result<Config> {
    let mut config = Config::from(text);
    let ui = UiBindings::load(cfg_path)?;
    apply_to_config(&mut config, &ui)?;
    Ok(config)
}
pub fn apply_to_config(config: &mut Config, ui: &UiBindings) -> Result<()> {
    let instant_uids: HashSet<u32> = config.instant_arts.values().copied().collect();
    let managed_uids: HashSet<u32> = ui.entries.keys().chain(ui.disabled.iter()).copied().collect();
    for uid in managed_uids {
        config.arts.retain(|_, value| *value != uid);
        config.instant_arts.retain(|_, value| *value != uid);
        config.tools.retain(|_, value| *value != uid);
        if config.tool_on_t == Some(uid) {
            config.tool_on_t = None;
        }
        if config.tool_on_q == Some(uid) {
            config.tool_on_q = None;
        }
    }

    let mut seen = HashMap::<Vec<KeyToken>, u32>::new();
    for (uid, schemes) in &ui.entries {
        if ui.disabled.contains(uid) {
            continue;
        }
        validate_scheme_set(&[], *uid, schemes).map_err(|error| anyhow!("UID {uid}: {error}"))?;
        for keys in schemes {
            if let Some(previous) = seen.insert(keys.clone(), *uid) {
                return Err(anyhow!("键位方案冲突：UID {previous} 和 UID {uid} 都使用了 {keys:?}"));
            }

            match category_for_uid(*uid) {
                Some(BindingCategory::CombatArt) => {
                    let first = keys[0].to_art().ok_or_else(|| anyhow!("非法招式首键"))?;
                    let second = keys[1].to_art().ok_or_else(|| anyhow!("非法招式尾键"))?;
                    let combo = ArtCombo::pair(first, second);
                    if instant_uids.contains(uid) {
                        if config
                            .instant_arts
                            .insert(combo, *uid)
                            .is_some_and(|previous| previous != *uid)
                        {
                            return Err(anyhow!("招式键位冲突"));
                        }
                    } else if config.arts.insert(combo, *uid).is_some_and(|previous| previous != *uid) {
                        return Err(anyhow!("招式键位冲突"));
                    }
                }
                Some(BindingCategory::Prosthetic) => {
                    if keys.len() == 1 {
                        match keys[0] {
                            KeyToken::ProstheticUse => {
                                if config.tool_on_t.is_some_and(|previous| previous != *uid) {
                                    return Err(anyhow!("忍具键冲突"));
                                }
                                config.tool_on_t = Some(*uid);
                            }
                            KeyToken::ProstheticSwitch => {
                                if config.tool_on_q.is_some_and(|previous| previous != *uid) {
                                    return Err(anyhow!("忍具切换键冲突"));
                                }
                                config.tool_on_q = Some(*uid);
                            }
                            _ => return Err(anyhow!("单键忍具只能使用忍具键/忍具切换键")),
                        }
                    } else {
                        let first = keys[0]
                            .to_tool_first()
                            .ok_or_else(|| anyhow!("忍具键/忍具切换键只能作为尾键"))?;
                        let tail = keys[1].to_tool_tail().ok_or_else(|| anyhow!("忍具尾键不支持该键位"))?;
                        let combo = ToolCombo { first, tail };
                        if config
                            .tools
                            .insert(combo, *uid)
                            .is_some_and(|previous| previous != *uid)
                        {
                            return Err(anyhow!("忍具键位冲突"));
                        }
                    }
                }
                None => return Err(anyhow!("非法 UID {uid}")),
            }
        }
    }
    Ok(())
}

/// Persist every scheme for one UID. The UID is no longer disabled.
pub fn save_ui_schemes(
    cfg_path: &Path,
    snapshot: &BindingSnapshot,
    uid: u32,
    schemes: Vec<Vec<KeyToken>>,
) -> Result<()> {
    snapshot
        .validate_scheme_set(uid, &schemes)
        .map_err(|error| anyhow!(error.to_string()))?;
    let mut ui = UiBindings::load(cfg_path)?;
    ui.disabled.remove(&uid);
    ui.upsert(uid, schemes);
    ui.save(cfg_path)
}

/// Save a complete scheme set and remove only the exact conflicting schemes
/// owned by other UIDs. All their remaining schemes are preserved as UI
/// overrides.
pub fn save_ui_schemes_overwrite(
    cfg_path: &Path,
    snapshot: &BindingSnapshot,
    uid: u32,
    schemes: Vec<Vec<KeyToken>>,
) -> Result<()> {
    let mut seen = HashSet::new();
    for keys in &schemes {
        validate_scheme(uid, keys).map_err(|error| anyhow!(error.to_string()))?;
        if !seen.insert(keys.clone()) {
            return Err(anyhow!(BindingValidationError::DuplicateScheme.to_string()));
        }
    }
    if schemes.is_empty() {
        return Err(anyhow!(BindingValidationError::EmptySchemeSet.to_string()));
    }

    let conflicts = snapshot.conflicting_entries(uid, &schemes);
    let mut removed_by_uid = HashMap::<u32, HashSet<Vec<KeyToken>>>::new();
    for entry in conflicts {
        removed_by_uid.entry(entry.uid).or_default().insert(entry.keys.clone());
    }

    let mut ui = UiBindings::load(cfg_path)?;
    for (conflict_uid, removed) in removed_by_uid {
        let remaining: Vec<Vec<KeyToken>> = snapshot
            .entries_for_uid(conflict_uid)
            .into_iter()
            .filter(|entry| !removed.contains(&entry.keys))
            .map(|entry| entry.keys.clone())
            .collect();
        if remaining.is_empty() {
            ui.entries.remove(&conflict_uid);
            ui.disabled.insert(conflict_uid);
        } else {
            ui.entries.insert(conflict_uid, remaining);
            ui.disabled.remove(&conflict_uid);
        }
    }

    ui.disabled.remove(&uid);
    ui.upsert(uid, schemes);
    ui.save(cfg_path)
}

/// Remove every trigger binding for `uid`. The UID stays disabled in the UI
/// override file so a cfg-sourced bind cannot reappear after the reset.
pub fn reset_ui_binding(cfg_path: &Path, uid: u32) -> Result<()> {
    if category_for_uid(uid).is_none() {
        return Err(anyhow!("invalid UID {uid}"));
    }
    let mut ui = UiBindings::load(cfg_path)?;
    ui.entries.remove(&uid);
    ui.disabled.insert(uid);
    ui.save(cfg_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "battle-instinct-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_name_and_key_json() {
        let text = r#"{
          "version": 1,
          "bindings": [
            {"uid":7100,"keys":["Interact","Attack"]},
            {"uid":70500,"keys":["ProstheticUse"]}
          ]
        }"#;
        let ui = UiBindings::parse(text).unwrap();
        assert_eq!(ui.entries[&7100], vec![vec![KeyToken::Interact, KeyToken::Attack]]);
        assert_eq!(ui.entries[&70500], vec![vec![KeyToken::ProstheticUse]]);
    }

    #[test]
    fn parse_groups_repeated_uid_into_schemes_in_order() {
        let text = r#"{
          "version": 3,
          "bindings": [
            {"uid":7100,"keys":["Interact","Attack"]},
            {"uid":7100,"keys":["Block","Attack"]},
            {"uid":70500,"keys":["ProstheticUse"]}
          ]
        }"#;
        let ui = UiBindings::parse(text).unwrap();
        assert_eq!(
            ui.entries[&7100],
            vec![
                vec![KeyToken::Interact, KeyToken::Attack],
                vec![KeyToken::Block, KeyToken::Attack],
            ]
        );
    }

    #[test]
    fn rejects_prosthetic_key_as_first() {
        let snap = BindingSnapshot::default();
        assert!(
            snap.validate_candidate(7100, &[KeyToken::ProstheticUse, KeyToken::Attack])
                .is_err()
        );
    }

    #[test]
    fn conflict_is_checked_across_all_bindings() {
        let snap = BindingSnapshot {
            entries: vec![BindingEntry {
                uid: 7100,
                keys: vec![KeyToken::Interact, KeyToken::Attack],
                source: BindingSource::Cfg,
            }],
            names: HashMap::new(),
        };
        let error = snap
            .validate_candidate(5200, &[KeyToken::Interact, KeyToken::Attack])
            .unwrap_err();
        assert_eq!(error.to_string(), "当前键位已被占用");
    }

    #[test]
    fn json_save_roundtrip_preserves_all_schemes_and_order() {
        let dir = temp_dir("bindings");
        let cfg = dir.join("battle_instinct.cfg");
        std::fs::write(
            &cfg,
            "7100 一字斩·二连 ee
70500 手里剑 t-0s
",
        )
        .unwrap();
        let snapshot = BindingSnapshot::load(&cfg).unwrap();
        let schemes = vec![
            vec![KeyToken::Interact, KeyToken::Attack],
            vec![KeyToken::Block, KeyToken::Attack],
        ];
        save_ui_schemes(&cfg, &snapshot, 7100, schemes.clone()).unwrap();

        let ui = UiBindings::load(&cfg).unwrap();
        assert_eq!(ui.entries[&7100], schemes);
        assert_eq!(ui.entries.get(&70500), None);

        let text = std::fs::read_to_string(UiBindings::path_for_cfg(&cfg)).unwrap();
        assert!(text.contains("\"version\": 3"));
        assert_eq!(text.matches("\"uid\":7100").count(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn disabled_uids_are_parsed_and_excluded_from_snapshot() {
        let config = Config::from("7100 test ee\n5200 test el\n");
        let ui = UiBindings::parse(
            r#"{
              "version": 2,
              "disabled": [7100],
              "bindings": [
                {"uid":5200,"keys":["Interact","Attack"]}
              ]
            }"#,
        )
        .unwrap();
        assert!(ui.disabled.contains(&7100));
        let snapshot = BindingSnapshot::from_config(Path::new("__missing_cfg__"), &config, &ui);
        assert!(snapshot.current_entry(7100).is_none());
        assert_eq!(snapshot.current_entry(5200).unwrap().source, BindingSource::UiOverride);
    }

    #[test]
    fn overwrite_removes_only_conflicting_scheme_and_keeps_other_schemes() {
        let dir = temp_dir("overwrite");
        let cfg = dir.join("battle_instinct.cfg");
        std::fs::write(&cfg, "7100 test ee\n7100 test ↑l\n5200 test rl\n").unwrap();
        let snapshot = BindingSnapshot::load(&cfg).unwrap();
        assert_eq!(snapshot.entries_for_uid(7100).len(), 2);

        save_ui_schemes_overwrite(
            &cfg,
            &snapshot,
            5200,
            vec![vec![KeyToken::Interact, KeyToken::Interact]],
        )
        .unwrap();

        let ui = UiBindings::load(&cfg).unwrap();
        assert_eq!(ui.entries[&7100], vec![vec![KeyToken::Up, KeyToken::Attack]]);
        assert!(!ui.disabled.contains(&7100));
        assert_eq!(ui.entries[&5200], vec![vec![KeyToken::Interact, KeyToken::Interact]]);

        let mut effective = Config::open(&cfg).unwrap();
        apply_to_config(&mut effective, &ui).unwrap();
        assert_eq!(
            effective.art(ArtCombo::pair(ArtToken::Interact, ArtToken::Interact)),
            Some(5200)
        );
        assert_eq!(
            effective.art(ArtCombo::pair(ArtToken::Up, ArtToken::Attack)),
            Some(7100)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ui_override_replaces_existing_uid_binding() {
        let mut config = Config::from("7100 一字斩·二连 ee\n70500 手里剑 t-0s\n");
        let mut ui = UiBindings::default();
        ui.entries
            .insert(7100, vec![vec![KeyToken::Interact, KeyToken::Attack]]);
        ui.entries.insert(70500, vec![vec![KeyToken::ProstheticSwitch]]);
        apply_to_config(&mut config, &ui).unwrap();
        assert!(config.arts.iter().any(|(combo, uid)| *uid == 7100
            && combo.first() == Some(ArtToken::Interact)
            && combo.second() == Some(ArtToken::Attack)));
        assert_eq!(config.tool_on_q, Some(70500));
    }

    #[test]
    fn multiple_art_schemes_are_applied_for_same_uid() {
        let mut config = Config::from("7100 test ee\n");
        let mut ui = UiBindings::default();
        ui.entries.insert(
            7100,
            vec![
                vec![KeyToken::Jump, KeyToken::Crouch],
                vec![KeyToken::Block, KeyToken::Attack],
            ],
        );
        apply_to_config(&mut config, &ui).unwrap();
        assert_eq!(config.art(ArtCombo::pair(ArtToken::Jump, ArtToken::Crouch)), Some(7100));
        assert_eq!(
            config.art(ArtCombo::pair(ArtToken::Block, ArtToken::Attack)),
            Some(7100)
        );
    }

    #[test]
    fn multiple_prosthetic_schemes_are_applied_for_same_uid() {
        let mut config = Config::from(
            "70500 test t-0s
",
        );
        let mut ui = UiBindings::default();
        ui.entries.insert(
            70500,
            vec![
                vec![KeyToken::ProstheticUse],
                vec![KeyToken::Up, KeyToken::ProstheticUse],
            ],
        );
        apply_to_config(&mut config, &ui).unwrap();
        assert_eq!(config.tool_on_t, Some(70500));
        assert_eq!(
            config.tool(ToolCombo {
                first: ToolFirst::Up,
                tail: ToolTail::Use
            }),
            Some(70500)
        );
    }

    #[test]
    fn parses_and_applies_jump_sprint_crouch_tokens() {
        let ui = UiBindings::parse(
            r#"{
              "version": 2,
              "bindings": [
                {"uid":5100,"keys":["Jump","Crouch"]},
                {"uid":70500,"keys":["Sprint","ProstheticUse"]}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(
            ui.entries.get(&5100),
            Some(&vec![vec![KeyToken::Jump, KeyToken::Crouch]])
        );
        assert_eq!(
            ui.entries.get(&70500),
            Some(&vec![vec![KeyToken::Sprint, KeyToken::ProstheticUse]])
        );

        let mut config = Config::from("5100 art jc\n70500 tool st\n");
        apply_to_config(&mut config, &ui).unwrap();
        assert_eq!(config.art(ArtCombo::pair(ArtToken::Jump, ArtToken::Crouch)), Some(5100));
        assert_eq!(
            config.tool(ToolCombo {
                first: ToolFirst::Sprint,
                tail: ToolTail::Use
            }),
            Some(70500)
        );
    }

    #[test]
    fn reset_disables_cfg_and_ui_binding_for_uid() {
        let dir = temp_dir("reset");
        let cfg = dir.join("battle_instinct.cfg");
        std::fs::write(&cfg, "7100 test ee\n5200 test ↑l\n").unwrap();
        let snapshot = BindingSnapshot::load(&cfg).unwrap();
        save_ui_schemes(&cfg, &snapshot, 7100, vec![vec![KeyToken::Jump, KeyToken::Crouch]]).unwrap();
        reset_ui_binding(&cfg, 7100).unwrap();

        let ui = UiBindings::load(&cfg).unwrap();
        assert!(ui.disabled.contains(&7100));
        assert!(!ui.entries.contains_key(&7100));
        let reloaded = BindingSnapshot::load(&cfg).unwrap();
        assert!(reloaded.current_entry(7100).is_none());

        let mut effective = Config::open(&cfg).unwrap();
        apply_to_config(&mut effective, &ui).unwrap();
        assert!(!effective.arts.values().any(|uid| *uid == 7100));
        let _ = std::fs::remove_dir_all(dir);
    }
}
