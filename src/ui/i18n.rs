use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU8, Ordering},
};

use crate::{
    binding_validation::BindingValidationError,
    bindings::{BindingCategory, BindingSource, KeyToken},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lang {
    Zh,
    En,
}

impl Lang {
    pub(crate) fn pick(self, zh: &'static str, en: &'static str) -> &'static str {
        match self {
            Lang::Zh => zh,
            Lang::En => en,
        }
    }

    pub(crate) fn key_label(self, token: KeyToken) -> &'static str {
        match self {
            Lang::Zh => token.label(),
            Lang::En => match token {
                KeyToken::Up => "Move Up",
                KeyToken::Right => "Move Right",
                KeyToken::Down => "Move Down",
                KeyToken::Left => "Move Left",
                KeyToken::Block => "Block",
                KeyToken::Attack => "Attack",
                KeyToken::Interact => "Interact",
                KeyToken::Grapple => "Grapple",
                KeyToken::Jump => "Jump",
                KeyToken::Sprint => "Sprint",
                KeyToken::Crouch => "Crouch",
                KeyToken::ProstheticUse => "Use Tool",
                KeyToken::ProstheticSwitch => "Switch Tool",
            },
        }
    }

    pub(crate) fn category_label(self, category: BindingCategory) -> &'static str {
        match (self, category) {
            (Lang::Zh, BindingCategory::CombatArt) => "流派招式",
            (Lang::Zh, BindingCategory::Prosthetic) => "义手忍具",
            (Lang::En, BindingCategory::CombatArt) => "Combat Art",
            (Lang::En, BindingCategory::Prosthetic) => "Prosthetic",
        }
    }

    pub(crate) fn source_label(self, source: BindingSource) -> &'static str {
        match (self, source) {
            (Lang::Zh, BindingSource::Cfg) => "来源：cfg",
            (Lang::Zh, BindingSource::UiOverride) => "来源：UI 覆盖",
            (Lang::Zh, BindingSource::None) => "来源：未配置",
            (Lang::En, BindingSource::Cfg) => "Source: cfg",
            (Lang::En, BindingSource::UiOverride) => "Source: UI override",
            (Lang::En, BindingSource::None) => "Source: unset",
        }
    }

    pub(crate) fn validation_error(self, error: BindingValidationError) -> String {
        match (self, error) {
            (Lang::Zh, BindingValidationError::InvalidUid(uid)) => format!("非法技能 UID：{uid}"),
            (Lang::Zh, BindingValidationError::ArtNeedsTwoNormalKeys) => "流派招式必须配置为两个普通键位".to_string(),
            (Lang::Zh, BindingValidationError::ProstheticKeyCount) => "义手忍具键位数量非法".to_string(),
            (Lang::Zh, BindingValidationError::ProstheticSingleKey) => "单键忍具只能使用忍具键/忍具切换键".to_string(),
            (Lang::Zh, BindingValidationError::ProstheticKeyAsFirst) => "忍具键/忍具切换键只能作为尾键".to_string(),
            (Lang::Zh, BindingValidationError::UnsupportedProstheticTail) => "忍具尾键不支持该键位".to_string(),
            (Lang::Zh, BindingValidationError::Occupied) => "当前键位已被占用".to_string(),
            (Lang::En, BindingValidationError::InvalidUid(uid)) => format!("Invalid skill UID: {uid}"),
            (Lang::En, BindingValidationError::ArtNeedsTwoNormalKeys) => {
                "Combat arts require two normal keys.".to_string()
            }
            (Lang::En, BindingValidationError::ProstheticKeyCount) => "Invalid number of prosthetic keys.".to_string(),
            (Lang::En, BindingValidationError::ProstheticSingleKey) => {
                "Single-key mode only accepts Use Tool or Switch Tool.".to_string()
            }
            (Lang::En, BindingValidationError::ProstheticKeyAsFirst) => {
                "Use Tool / Switch Tool can only be the second key.".to_string()
            }
            (Lang::En, BindingValidationError::UnsupportedProstheticTail) => {
                "This key cannot be used as the second prosthetic key.".to_string()
            }
            (Lang::En, BindingValidationError::Occupied) => "That key is already in use.".to_string(),
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub(crate) fn set_lang(lang: Lang) {
    LANG.store(
        match lang {
            Lang::Zh => 0,
            Lang::En => 1,
        },
        Ordering::Release,
    );
}

pub(crate) fn lang() -> Lang {
    if LANG.load(Ordering::Acquire) == 1 {
        Lang::En
    } else {
        Lang::Zh
    }
}

pub(crate) fn pick(zh: &'static str, en: &'static str) -> &'static str {
    lang().pick(zh, en)
}

pub(crate) fn is_en() -> bool {
    lang() == Lang::En
}

pub(crate) fn key_label(token: KeyToken) -> &'static str {
    lang().key_label(token)
}

pub(crate) fn category_label(category: BindingCategory) -> &'static str {
    lang().category_label(category)
}

pub(crate) fn source_label(source: BindingSource) -> &'static str {
    lang().source_label(source)
}

pub(crate) fn validation_error(error: BindingValidationError) -> String {
    lang().validation_error(error)
}

pub(crate) fn load_lang_from_cfg(path: &Path) -> Lang {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_lang(&text))
        .unwrap_or(Lang::Zh)
}

fn parse_lang(text: &str) -> Option<Lang> {
    for line in text.lines() {
        let line = line
            .trim_start_matches('\u{feff}')
            .trim()
            .trim_start_matches('#')
            .trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if key != "ui language" && key != "界面语言" {
            continue;
        }
        let value = value.trim().to_ascii_lowercase();
        return match value.as_str() {
            "en" | "en-us" | "english" => Some(Lang::En),
            "zh" | "zh-cn" | "chinese" | "中文" => Some(Lang::Zh),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ui_language_comments() {
        assert_eq!(parse_lang("# ui language: en\n"), Some(Lang::En));
        assert_eq!(parse_lang("# UI Language: en-US\n"), Some(Lang::En));
        assert_eq!(parse_lang("# 界面语言: zh\n"), Some(Lang::Zh));
        assert_eq!(parse_lang("# ui language: english\n"), Some(Lang::En));
        assert_eq!(parse_lang("# ui language: unknown\n"), None);
    }

    #[test]
    fn shipped_cfgs_select_expected_ui_language() {
        assert_eq!(load_lang_from_cfg(Path::new("res/battle_instinct.cfg")), Lang::En);
        assert_eq!(load_lang_from_cfg(Path::new("res/battle_instinct_zh.cfg")), Lang::Zh);
    }

    #[test]
    fn english_ui_labels_are_localized() {
        assert_eq!(Lang::En.key_label(KeyToken::ProstheticSwitch), "Switch Tool");
        assert_eq!(Lang::En.category_label(BindingCategory::CombatArt), "Combat Art");
        assert_eq!(Lang::En.source_label(BindingSource::None), "Source: unset");
    }
}
