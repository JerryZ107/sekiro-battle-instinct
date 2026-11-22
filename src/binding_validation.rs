use std::{error::Error, fmt};

use crate::bindings::{BindingCategory, BindingEntry, KeyToken, category_for_uid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingValidationError {
    InvalidUid(u32),
    ArtNeedsTwoNormalKeys,
    ProstheticKeyCount,
    ProstheticSingleKey,
    ProstheticKeyAsFirst,
    UnsupportedProstheticTail,
    Occupied,
}

impl fmt::Display for BindingValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUid(uid) => write!(f, "非法技能 UID：{uid}"),
            Self::ArtNeedsTwoNormalKeys => write!(f, "流派招式必须配置为两个普通键位"),
            Self::ProstheticKeyCount => write!(f, "义手忍具键位数量非法"),
            Self::ProstheticSingleKey => write!(f, "单键忍具只能使用忍具键/忍具切换键"),
            Self::ProstheticKeyAsFirst => write!(f, "忍具键/忍具切换键只能作为尾键"),
            Self::UnsupportedProstheticTail => write!(f, "忍具尾键不支持该键位"),
            Self::Occupied => write!(f, "当前键位已被占用"),
        }
    }
}

impl Error for BindingValidationError {}

pub fn validate_candidate(entries: &[BindingEntry], uid: u32, keys: &[KeyToken]) -> Result<(), BindingValidationError> {
    let category = category_for_uid(uid).ok_or(BindingValidationError::InvalidUid(uid))?;
    match category {
        BindingCategory::CombatArt => {
            if keys.len() != 2 || keys.iter().any(|key| key.is_prosthetic()) {
                return Err(BindingValidationError::ArtNeedsTwoNormalKeys);
            }
        }
        BindingCategory::Prosthetic => {
            if keys.is_empty() || keys.len() > 2 {
                return Err(BindingValidationError::ProstheticKeyCount);
            }
            if keys.len() == 1 && !keys[0].is_prosthetic() {
                return Err(BindingValidationError::ProstheticSingleKey);
            }
            if keys.len() == 2 && keys[0].is_prosthetic() {
                return Err(BindingValidationError::ProstheticKeyAsFirst);
            }
            if keys.len() == 2 && keys[1].to_tool_tail().is_none() {
                return Err(BindingValidationError::UnsupportedProstheticTail);
            }
        }
    }

    if entries.iter().any(|entry| entry.uid != uid && entry.keys == keys) {
        return Err(BindingValidationError::Occupied);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::BindingSource;

    fn entry(uid: u32, keys: Vec<KeyToken>) -> BindingEntry {
        BindingEntry {
            uid,
            keys,
            source: BindingSource::Cfg,
        }
    }

    #[test]
    fn occupied_error_has_required_text() {
        let entries = [entry(7100, vec![KeyToken::Interact, KeyToken::Attack])];
        let error = validate_candidate(&entries, 5200, &[KeyToken::Interact, KeyToken::Attack]).unwrap_err();
        assert_eq!(error.to_string(), "当前键位已被占用");
    }

    #[test]
    fn occupied_check_covers_combat_arts_and_prosthetics() {
        let entries = [entry(7100, vec![KeyToken::Up, KeyToken::Attack])];
        let error = validate_candidate(&entries, 70500, &[KeyToken::Up, KeyToken::Attack]).unwrap_err();
        assert_eq!(error.to_string(), "当前键位已被占用");
    }

    #[test]
    fn re_saving_same_uid_is_not_a_conflict() {
        let entries = [entry(7100, vec![KeyToken::Up, KeyToken::Attack])];
        assert!(validate_candidate(&entries, 7100, &[KeyToken::Up, KeyToken::Attack]).is_ok());
    }

    #[test]
    fn invalid_prosthetic_tail_is_not_reported_as_occupied() {
        let error = validate_candidate(&[], 70500, &[KeyToken::Interact, KeyToken::Up]).unwrap_err();
        assert_eq!(error.to_string(), "忍具尾键不支持该键位");
    }

    #[test]
    fn prosthetic_keys_cannot_be_first() {
        let error = validate_candidate(&[], 70500, &[KeyToken::ProstheticUse, KeyToken::Attack]).unwrap_err();
        assert_eq!(error.to_string(), "忍具键/忍具切换键只能作为尾键");
    }

    #[test]
    fn single_prosthetic_accepts_only_shortcut_keys() {
        assert!(validate_candidate(&[], 70500, &[KeyToken::ProstheticUse]).is_ok());
        assert!(validate_candidate(&[], 70500, &[KeyToken::Attack]).is_err());
    }

    #[test]
    fn combat_art_requires_two_normal_keys() {
        assert!(validate_candidate(&[], 7100, &[KeyToken::Interact, KeyToken::Attack]).is_ok());
        assert!(validate_candidate(&[], 7100, &[KeyToken::Interact]).is_err());
        assert!(validate_candidate(&[], 7100, &[KeyToken::Interact, KeyToken::ProstheticUse]).is_err());
    }

    #[test]
    fn jump_sprint_crouch_are_normal_and_prosthetic_tail_keys() {
        assert!(validate_candidate(&[], 7100, &[KeyToken::Jump, KeyToken::Crouch]).is_ok());
        assert!(validate_candidate(&[], 70500, &[KeyToken::Sprint, KeyToken::ProstheticUse]).is_ok());
        assert!(validate_candidate(&[], 70500, &[KeyToken::Crouch, KeyToken::Jump]).is_ok());
    }
}
