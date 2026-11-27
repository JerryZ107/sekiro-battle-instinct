use super::i18n;
use crate::binding_validation::BindingValidationError;
use crate::bindings::{BindingCategory, BindingSnapshot, BindingSource, KeyToken};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingMode {
    Single,
    Two,
    Three,
}

impl BindingMode {
    pub fn label(self) -> &'static str {
        match self {
            BindingMode::Single => i18n::pick("单键触发", "Single-key"),
            BindingMode::Two => i18n::pick("双键触发", "Two-key"),
            BindingMode::Three => i18n::pick("三键触发", "Three-key"),
        }
    }

    fn slot_count(self) -> usize {
        match self {
            BindingMode::Single => 1,
            BindingMode::Two => 2,
            BindingMode::Three => 3,
        }
    }
}

const COMBAT_ART_MODES: &[BindingMode] = &[BindingMode::Two, BindingMode::Three];
const PROSTHETIC_MODES: &[BindingMode] = &[BindingMode::Single, BindingMode::Two, BindingMode::Three];

fn legal_modes(category: BindingCategory) -> &'static [BindingMode] {
    match category {
        BindingCategory::CombatArt => COMBAT_ART_MODES,
        BindingCategory::Prosthetic => PROSTHETIC_MODES,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub fn new(l: i32, t: i32, r: i32, b: i32) -> Rect {
        Rect { l, t, r, b }
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalButtonStyle {
    Primary,
    Danger,
    Neutral,
}

#[derive(Clone, Debug)]
pub enum ModalAction {
    Dismiss,
    SaveAndClose,
    DiscardAndClose,
    ApplyMode(BindingMode),
    OverwriteAndSave(Vec<Vec<KeyToken>>),
    ResetAndClose,
    DeleteSelectedScheme,
}

#[derive(Clone, Debug)]
pub struct ModalButton {
    pub label: String,
    pub action: ModalAction,
    pub style: ModalButtonStyle,
}

#[derive(Clone, Debug)]
pub struct Modal {
    pub message: String,
    pub buttons: Vec<ModalButton>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemeDraft {
    pub slots: Vec<Option<KeyToken>>,
}

impl SchemeDraft {
    fn blank(mode: BindingMode) -> SchemeDraft {
        SchemeDraft {
            slots: vec![None; mode.slot_count()],
        }
    }

    fn from_keys(keys: &[KeyToken]) -> SchemeDraft {
        SchemeDraft {
            slots: keys.iter().copied().map(Some).collect(),
        }
    }

    pub fn mode(&self) -> BindingMode {
        match self.slots.len() {
            0 | 1 => BindingMode::Single,
            2 => BindingMode::Two,
            _ => BindingMode::Three,
        }
    }

    pub fn complete(&self) -> bool {
        !self.slots.is_empty() && self.slots.iter().all(Option::is_some)
    }

    pub fn keys(&self) -> Vec<KeyToken> {
        self.slots.iter().flatten().copied().collect()
    }
}

#[derive(Clone, Debug)]
pub struct Editor {
    pub uid: u32,
    pub name: String,
    pub category: BindingCategory,
    pub source: BindingSource,
    pub menu_uid: u32,
    pub menu_category: u32,
    pub schemes: Vec<SchemeDraft>,
    pub selected_index: usize,
    original: Vec<SchemeDraft>,
    pub modal: Option<Modal>,
    pub snapshot: BindingSnapshot,
}

impl Editor {
    pub fn new(uid: u32, snapshot: BindingSnapshot) -> Option<Editor> {
        let category = crate::bindings::category_for_uid(uid)?;
        let name = snapshot
            .name(uid)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("UID {uid}"));
        let existing = snapshot.entries_for_uid(uid);
        let source = existing
            .first()
            .map(|entry| entry.source)
            .unwrap_or(BindingSource::None);
        let mut schemes: Vec<SchemeDraft> = existing
            .iter()
            .map(|entry| SchemeDraft::from_keys(&entry.keys))
            .collect();
        if schemes.is_empty() {
            schemes.push(SchemeDraft::blank(BindingMode::Two));
        }
        let original = schemes.clone();
        Some(Editor {
            uid,
            name,
            category,
            source,
            menu_uid: 0,
            menu_category: 0,
            schemes,
            selected_index: 0,
            original,
            modal: None,
            snapshot,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.schemes != self.original
    }

    pub fn selected_scheme(&self) -> &SchemeDraft {
        &self.schemes[self.selected_index]
    }

    fn selected_scheme_mut(&mut self) -> &mut SchemeDraft {
        &mut self.schemes[self.selected_index]
    }

    pub fn mode(&self) -> BindingMode {
        self.selected_scheme().mode()
    }

    pub fn slots(&self) -> &[Option<KeyToken>] {
        &self.selected_scheme().slots
    }

    pub fn complete(&self) -> bool {
        !self.schemes.is_empty() && self.schemes.iter().all(SchemeDraft::complete)
    }

    #[cfg(test)]
    pub fn selected_keys(&self) -> Vec<KeyToken> {
        self.selected_scheme().keys()
    }

    pub fn all_keys(&self) -> Vec<Vec<KeyToken>> {
        self.schemes.iter().map(SchemeDraft::keys).collect()
    }

    pub fn scheme_label(&self, index: usize) -> String {
        let Some(scheme) = self.schemes.get(index) else {
            return String::new();
        };
        if scheme.slots.iter().all(Option::is_none) {
            return i18n::pick("未配置", "Unassigned").to_string();
        }
        scheme
            .slots
            .iter()
            .map(|slot| {
                slot.map(i18n::key_label)
                    .unwrap_or_else(|| i18n::pick("未配置", "Unassigned"))
            })
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn category_label(&self) -> &'static str {
        match self.category {
            BindingCategory::CombatArt => i18n::category_label(BindingCategory::CombatArt),
            BindingCategory::Prosthetic => i18n::category_label(BindingCategory::Prosthetic),
        }
    }

    pub fn source_label(&self) -> &'static str {
        match self.source {
            BindingSource::Cfg => i18n::source_label(BindingSource::Cfg),
            BindingSource::UiOverride => i18n::source_label(BindingSource::UiOverride),
            BindingSource::None => i18n::source_label(BindingSource::None),
        }
    }

    pub fn mode_is_switchable(&self) -> bool {
        legal_modes(self.category).len() > 1
    }

    fn mode_position(&self) -> Option<usize> {
        legal_modes(self.category).iter().position(|mode| *mode == self.mode())
    }

    pub fn mode_can_prev(&self) -> bool {
        self.mode_position().is_some_and(|index| index > 0)
    }

    pub fn mode_can_next(&self) -> bool {
        self.mode_position()
            .is_some_and(|index| index + 1 < legal_modes(self.category).len())
    }

    /// `<`: move to the previous legal mode. Combat arts switch between
    /// two-key and three-key triggers; prosthetics between single, two and three.
    pub fn mode_prev(&mut self) {
        let Some(index) = self.mode_position() else {
            return;
        };
        if index > 0 {
            let target = legal_modes(self.category)[index - 1];
            self.request_mode(target);
        }
    }

    /// `>`: move to the next legal mode. In both editors the three-key mode
    /// follows (sits to the right of) the two-key mode.
    pub fn mode_next(&mut self) {
        let Some(index) = self.mode_position() else {
            return;
        };
        if index + 1 < legal_modes(self.category).len() {
            let target = legal_modes(self.category)[index + 1];
            self.request_mode(target);
        }
    }

    pub fn request_mode(&mut self, mode: BindingMode) {
        if mode == self.mode() {
            return;
        }
        let dropped_keys = self
            .selected_scheme()
            .slots
            .iter()
            .skip(mode.slot_count())
            .any(Option::is_some);
        if !dropped_keys {
            self.apply_mode(mode);
            return;
        }
        self.modal = Some(Modal {
            message: if i18n::is_en() {
                format!(
                    "Switching to {} mode will clear the extra key(s). Continue?",
                    mode.label()
                )
            } else {
                format!("切换为{}后，多出的键位将被清空，是否继续？", mode.label())
            },
            buttons: vec![
                ModalButton {
                    label: i18n::pick("确认切换", "Switch").into(),
                    action: ModalAction::ApplyMode(mode),
                    style: ModalButtonStyle::Primary,
                },
                ModalButton {
                    label: i18n::pick("取消", "Cancel").into(),
                    action: ModalAction::Dismiss,
                    style: ModalButtonStyle::Neutral,
                },
            ],
        });
    }

    pub fn apply_mode(&mut self, mode: BindingMode) {
        if mode == self.mode() || !legal_modes(self.category).contains(&mode) {
            return;
        }
        self.selected_scheme_mut().slots.resize(mode.slot_count(), None);
    }

    pub fn select_scheme(&mut self, index: usize) {
        if index < self.schemes.len() {
            self.selected_index = index;
        }
    }

    pub fn add_scheme(&mut self) {
        self.schemes.push(SchemeDraft::blank(self.mode()));
        self.selected_index = self.schemes.len() - 1;
    }

    pub fn request_delete_scheme(&mut self) {
        let label = self.scheme_label(self.selected_index);
        self.modal = Some(Modal {
            message: if i18n::is_en() {
                format!("Delete the selected scheme \"{label}\"?")
            } else {
                format!("确定删除选中的方案「{label}」吗？")
            },
            buttons: vec![
                ModalButton {
                    label: i18n::pick("确认删除", "Delete").into(),
                    action: ModalAction::DeleteSelectedScheme,
                    style: ModalButtonStyle::Danger,
                },
                ModalButton {
                    label: i18n::pick("取消", "Cancel").into(),
                    action: ModalAction::Dismiss,
                    style: ModalButtonStyle::Neutral,
                },
            ],
        });
    }

    pub fn delete_selected_scheme(&mut self) {
        if self.schemes.is_empty() {
            return;
        }
        let mode = self.mode();
        self.schemes.remove(self.selected_index);
        if self.schemes.is_empty() {
            self.schemes.push(SchemeDraft::blank(mode));
            self.selected_index = 0;
        } else if self.selected_index >= self.schemes.len() {
            self.selected_index = self.schemes.len() - 1;
        }
    }

    pub fn click_slot(&mut self, index: usize) {
        if let Some(slot) = self.selected_scheme_mut().slots.get_mut(index) {
            if slot.is_some() {
                *slot = None;
            }
        }
    }

    pub fn click_key(&mut self, token: KeyToken) {
        let mode = self.mode();
        if self.selected_scheme().slots.iter().all(Option::is_some) {
            self.show_message(i18n::pick(
                "键位已满，请点击配置区键位去移除。",
                "Key slots are full. Click a slot in the configuration area to remove it.",
            ));
            return;
        }
        if self.category == BindingCategory::CombatArt && token.is_prosthetic() {
            self.show_message(i18n::pick(
                "流派招式不能使用忍具键/忍具切换键。",
                "Combat arts cannot use Use Tool or Switch Tool keys.",
            ));
            return;
        }
        if token.is_prosthetic() && mode != BindingMode::Single {
            let tail_index = mode.slot_count() - 1;
            if self.selected_scheme().slots[..tail_index].iter().any(Option::is_none) {
                self.show_message(i18n::pick(
                    "忍具键/忍具切换键只能作为尾键。",
                    "Use Tool / Switch Tool can only be the tail key.",
                ));
                return;
            }
            self.selected_scheme_mut().slots[tail_index] = Some(token);
            self.check_after_edit(tail_index);
            return;
        }
        if mode == BindingMode::Single && !token.is_prosthetic() {
            return;
        }
        if let Some(index) = self.selected_scheme().slots.iter().position(Option::is_none) {
            self.selected_scheme_mut().slots[index] = Some(token);
            self.check_after_edit(index);
        } else {
            self.show_message(i18n::pick(
                "键位已满，请点击配置区键位去移除。",
                "Key slots are full. Click a slot in the configuration area to remove it.",
            ));
        }
    }

    /// Validate the selected scheme after an edit. Invalid shapes roll back
    /// only the slot that was just clicked. Cross-UID conflicts are intentionally
    /// deferred to save so the overwrite/cancel dialog can handle them.
    fn check_after_edit(&mut self, clicked_index: usize) {
        let selected = self.selected_index;
        let scheme = &self.schemes[selected];
        if !scheme.complete() {
            return;
        }
        let keys = scheme.keys();
        let duplicate = self
            .schemes
            .iter()
            .enumerate()
            .any(|(index, other)| index != selected && other.complete() && other.keys() == keys);
        if duplicate {
            if let Some(slot) = self.schemes[selected].slots.get_mut(clicked_index) {
                *slot = None;
            }
            self.show_message(&i18n::validation_error(BindingValidationError::DuplicateScheme));
            return;
        }

        if let Err(error) = self.snapshot.validate_candidate(self.uid, &keys) {
            if error == BindingValidationError::Occupied {
                return;
            }
            if let Some(slot) = self.schemes[selected].slots.get_mut(clicked_index) {
                *slot = None;
            }
            self.show_message(&i18n::validation_error(error));
        }
    }

    pub fn show_message(&mut self, message: &str) {
        self.modal = Some(Modal {
            message: message.into(),
            buttons: vec![ModalButton {
                label: i18n::pick("确定", "OK").into(),
                action: ModalAction::Dismiss,
                style: ModalButtonStyle::Primary,
            }],
        });
    }

    pub fn show_overwrite_confirm(&mut self, conflict_names: &str, schemes: Vec<Vec<KeyToken>>) {
        self.modal = Some(Modal {
            message: if i18n::is_en() {
                format!(
                    "The following keys are already used by \"{conflict_names}\".\nThe conflicting scheme(s) will be removed if you overwrite. Continue?"
                )
            } else {
                format!("当前键位已被「{conflict_names}」占用。\n覆盖后仅会清除冲突的方案，是否继续？")
            },
            buttons: vec![
                ModalButton {
                    label: i18n::pick("覆盖原键位", "Overwrite").into(),
                    action: ModalAction::OverwriteAndSave(schemes),
                    style: ModalButtonStyle::Primary,
                },
                ModalButton {
                    label: i18n::pick("取消", "Cancel").into(),
                    action: ModalAction::Dismiss,
                    style: ModalButtonStyle::Neutral,
                },
            ],
        });
    }

    pub fn request_close(&mut self) {
        if self.is_dirty() {
            self.modal = Some(Modal {
                message: i18n::pick(
                    "有未保存的键位修改，是否保存后退出？",
                    "You have unsaved changes. Save before closing?",
                )
                .into(),
                buttons: vec![
                    ModalButton {
                        label: i18n::pick("是", "Yes").into(),
                        action: ModalAction::SaveAndClose,
                        style: ModalButtonStyle::Primary,
                    },
                    ModalButton {
                        label: i18n::pick("否", "No").into(),
                        action: ModalAction::DiscardAndClose,
                        style: ModalButtonStyle::Danger,
                    },
                    ModalButton {
                        label: i18n::pick("取消", "Cancel").into(),
                        action: ModalAction::Dismiss,
                        style: ModalButtonStyle::Neutral,
                    },
                ],
            });
        }
    }

    /// Reset removes every trigger binding for this skill/prosthetic. The
    /// confirmation also tells the user how to replace individual slots without
    /// using the reset path.
    pub fn request_reset(&mut self) {
        self.modal = Some(Modal {
            message: i18n::pick(
                "确定要重置此技能/忍具的触发键位吗？\n\n（若要更换键位，请先点击对应槽位以移除。）",
                "Reset every trigger key for this combat art / prosthetic?\n\n(To change one key, click its slot to remove it first.)",
            )
            .into(),
            buttons: vec![
                ModalButton {
                    label: i18n::pick("确认重置", "Reset").into(),
                    action: ModalAction::ResetAndClose,
                    style: ModalButtonStyle::Danger,
                },
                ModalButton {
                    label: i18n::pick("取消", "Cancel").into(),
                    action: ModalAction::Dismiss,
                    style: ModalButtonStyle::Neutral,
                },
            ],
        });
    }

    pub fn take_modal_action(&mut self, button_index: usize) -> Option<ModalAction> {
        let action = self.modal.as_ref()?.buttons.get(button_index)?.action.clone();
        self.modal = None;
        Some(action)
    }

    pub fn modal_cancel(&mut self) {
        self.modal = None;
    }
}

#[derive(Clone, Debug)]
pub struct GridItem {
    pub rect: Rect,
    pub label: String,
    pub token: Option<KeyToken>,
    pub disabled: bool,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub panel: Rect,
    pub title: Rect,
    pub category: Rect,
    pub mode: Rect,
    pub mode_left: Rect,
    pub mode_right: Rect,
    pub scheme_title: Rect,
    pub scheme_buttons: Vec<Rect>,
    pub add_scheme: Rect,
    pub delete_scheme: Rect,
    pub slots: Vec<Rect>,
    pub grid: Vec<GridItem>,
    pub save: Rect,
    pub reset: Rect,
    pub cancel: Rect,
    pub footnote: Rect,
    pub modal_panel: Option<Rect>,
    pub modal_buttons: Vec<Rect>,
}

pub fn grid_items(editor: &Editor) -> Vec<(String, Option<KeyToken>, bool)> {
    let mut out = Vec::new();
    if editor.mode() == BindingMode::Single {
        out.push((
            i18n::key_label(KeyToken::ProstheticUse).into(),
            Some(KeyToken::ProstheticUse),
            false,
        ));
        out.push((
            i18n::key_label(KeyToken::ProstheticSwitch).into(),
            Some(KeyToken::ProstheticSwitch),
            false,
        ));
        return out;
    }
    for token in KeyToken::ALL_NORMAL {
        out.push((i18n::key_label(token).into(), Some(token), false));
    }
    match editor.category {
        BindingCategory::CombatArt => {
            out.push((
                i18n::pick("忍具键（仅义手忍具）", "Use Tool").into(),
                Some(KeyToken::ProstheticUse),
                true,
            ));
            out.push((
                i18n::pick("忍具切换键（仅义手忍具）", "Switch Tool").into(),
                Some(KeyToken::ProstheticSwitch),
                true,
            ));
        }
        BindingCategory::Prosthetic => {
            out.push((
                i18n::key_label(KeyToken::ProstheticUse).into(),
                Some(KeyToken::ProstheticUse),
                false,
            ));
            out.push((
                i18n::key_label(KeyToken::ProstheticSwitch).into(),
                Some(KeyToken::ProstheticSwitch),
                false,
            ));
        }
    }
    out
}

pub fn layout(editor: &Editor, w: i32, h: i32) -> Layout {
    let m = 18;
    let panel = Rect::new(0, 0, w, h);
    let content_l = m;
    let content_r = w - m;
    let title = Rect::new(content_l, 14, content_r, 58);
    let category = Rect::new(content_l, 58, content_r, 82);
    let mode_rect = Rect::new(content_l, 92, content_r, 134);
    let arrow_w = 42;
    let mode_left = Rect::new(content_l, mode_rect.t, content_l + arrow_w, mode_rect.b);
    let mode_right = Rect::new(content_r - arrow_w, mode_rect.t, content_r, mode_rect.b);
    let scheme_title = Rect::new(content_l, 142, content_r, 166);
    let scheme_top = 172;
    let scheme_gap = 6;
    let scheme_btn_h = 32;
    let scheme_cols = 2usize;
    let scheme_rows = (editor.schemes.len() + scheme_cols - 1) / scheme_cols;
    let scheme_rows = scheme_rows.max(1);
    let scheme_w = (content_r - content_l - scheme_gap) / 2;
    let mut scheme_buttons = Vec::with_capacity(editor.schemes.len());
    for index in 0..editor.schemes.len() {
        let col = index % scheme_cols;
        let row = index / scheme_cols;
        let l = content_l + col as i32 * (scheme_w + scheme_gap);
        let t = scheme_top + row as i32 * (scheme_btn_h + scheme_gap);
        scheme_buttons.push(Rect::new(l, t, l + scheme_w, t + scheme_btn_h));
    }
    let scheme_bottom = scheme_top + scheme_rows as i32 * (scheme_btn_h + scheme_gap);

    let actions_top = scheme_bottom + 2;
    let action_gap = 8;
    let action_w = (content_r - content_l - action_gap) / 2;
    let add_scheme = Rect::new(content_l, actions_top, content_l + action_w, actions_top + 32);
    let delete_scheme = Rect::new(
        content_l + action_w + action_gap,
        actions_top,
        content_r,
        actions_top + 32,
    );

    let slots_top = actions_top + 42;
    let slots_h = 82;
    let slot_gap = 10;
    let n = editor.slots().len() as i32;
    let mut slots = Vec::with_capacity(n as usize);
    if n == 1 {
        slots.push(Rect::new(
            content_l + 90,
            slots_top,
            content_r - 90,
            slots_top + slots_h,
        ));
    } else {
        // First → second → third are laid out left-to-right, so the third-key
        // slot/window naturally sits to the right of the two-key pair.
        let width = (content_r - content_l - slot_gap * (n - 1)) / n;
        for index in 0..n {
            let l = content_l + index * (width + slot_gap);
            let r = if index + 1 == n { content_r } else { l + width };
            slots.push(Rect::new(l, slots_top, r, slots_top + slots_h));
        }
    }

    let grid_top = slots_top + slots_h + 12;
    let gap = 6;
    let items = grid_items(editor);
    let cols = 4usize;
    let rows = (items.len() + cols - 1) / cols;
    let cell_w = (content_r - content_l - gap * (cols as i32 - 1)) / cols as i32;
    let cell_h = 40;
    let mut grid = Vec::with_capacity(items.len());
    for (i, (label, token, disabled)) in items.into_iter().enumerate() {
        let col = (i % cols) as i32;
        let row = (i / cols) as i32;
        let l = content_l + col * (cell_w + gap);
        let t = grid_top + row * (cell_h + gap);
        grid.push(GridItem {
            rect: Rect::new(l, t, l + cell_w, t + cell_h),
            label,
            token,
            disabled,
        });
    }
    let grid_bottom = grid_top + rows as i32 * (cell_h + gap);
    let footnote = Rect::new(content_l, grid_bottom + 8, content_r, grid_bottom + 30);

    let button_y = h - 56;
    let button_gap = 8;
    let button_w = (content_r - content_l - button_gap * 2) / 3;
    let save = Rect::new(content_l, button_y, content_l + button_w, h - 18);
    let reset = Rect::new(
        content_l + button_w + button_gap,
        button_y,
        content_l + button_w * 2 + button_gap,
        h - 18,
    );
    let cancel = Rect::new(content_l + button_w * 2 + button_gap * 2, button_y, content_r, h - 18);

    let mut modal_panel = None;
    let mut modal_buttons = Vec::new();
    if let Some(modal) = editor.modal.as_ref() {
        let button_count = modal.buttons.len().max(1) as i32;
        let mw = (w - 80).min(640).max(380);
        let mh = if button_count >= 3 { 210 } else { 180 };
        let ml = (w - mw) / 2;
        let mt = (h - mh) / 2;
        let mp = Rect::new(ml, mt, ml + mw, mt + mh);
        modal_panel = Some(mp);

        let by = mt + mh - 56;
        if button_count == 1 {
            modal_buttons.push(Rect::new(ml + mw / 2 - 70, by, ml + mw / 2 + 70, mt + mh - 20));
        } else {
            let margin = 20;
            let btn_gap = 8;
            let total_w = mw - margin * 2;
            let bw = (total_w - btn_gap * (button_count - 1)) / button_count;
            for index in 0..button_count {
                let l = ml + margin + index * (bw + btn_gap);
                modal_buttons.push(Rect::new(l, by, l + bw, mt + mh - 20));
            }
        }
    }

    Layout {
        panel,
        title,
        category,
        mode: mode_rect,
        mode_left,
        mode_right,
        scheme_title,
        scheme_buttons,
        add_scheme,
        delete_scheme,
        slots,
        grid,
        save,
        reset,
        cancel,
        footnote,
        modal_panel,
        modal_buttons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingEntry, BindingSnapshot};
    use std::collections::HashMap;

    fn snapshot_with(uid: u32, schemes: Vec<Vec<KeyToken>>) -> BindingSnapshot {
        let mut names = HashMap::new();
        names.insert(uid, "测试技能".to_string());
        BindingSnapshot {
            entries: schemes
                .into_iter()
                .map(|keys| BindingEntry {
                    uid,
                    keys,
                    source: BindingSource::Cfg,
                })
                .collect(),
            names,
        }
    }

    fn editor(uid: u32, keys: Vec<KeyToken>) -> Editor {
        Editor::new(uid, snapshot_with(uid, vec![keys])).unwrap()
    }

    fn editor_multi(uid: u32, schemes: Vec<Vec<KeyToken>>) -> Editor {
        Editor::new(uid, snapshot_with(uid, schemes)).unwrap()
    }

    #[test]
    fn editor_prefills_existing_cfg_binding() {
        let e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        assert_eq!(e.slots(), &[Some(KeyToken::Interact), Some(KeyToken::Attack)]);
        assert_eq!(e.source, BindingSource::Cfg);
        assert_eq!(e.selected_index, 0);
    }

    #[test]
    fn editor_loads_all_schemes_and_selects_first() {
        let e = editor_multi(
            7100,
            vec![
                vec![KeyToken::Interact, KeyToken::Attack],
                vec![KeyToken::Block, KeyToken::Attack],
            ],
        );
        assert_eq!(e.schemes.len(), 2);
        assert_eq!(e.selected_index, 0);
        assert_eq!(e.scheme_label(0), "交互-攻击");
        assert_eq!(e.scheme_label(1), "防御-攻击");
    }

    #[test]
    fn selecting_a_scheme_does_not_mark_dirty() {
        let mut e = editor_multi(
            7100,
            vec![
                vec![KeyToken::Interact, KeyToken::Attack],
                vec![KeyToken::Block, KeyToken::Attack],
            ],
        );
        e.select_scheme(1);
        assert_eq!(e.selected_index, 1);
        assert!(!e.is_dirty());
        assert_eq!(e.slots(), &[Some(KeyToken::Block), Some(KeyToken::Attack)]);
    }

    #[test]
    fn adding_scheme_selects_blank_scheme_and_marks_dirty() {
        let mut e = editor_multi(70500, vec![vec![KeyToken::ProstheticUse]]);
        e.add_scheme();
        assert_eq!(e.schemes.len(), 2);
        assert_eq!(e.selected_index, 1);
        assert_eq!(e.mode(), BindingMode::Single);
        assert_eq!(e.slots(), &[None]);
        assert!(e.is_dirty());
    }

    #[test]
    fn deleting_last_scheme_keeps_one_blank_scheme() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.delete_selected_scheme();
        assert_eq!(e.schemes.len(), 1);
        assert_eq!(e.selected_index, 0);
        assert_eq!(e.mode(), BindingMode::Two);
        assert_eq!(e.slots(), &[None, None]);
        assert!(e.is_dirty());
    }

    #[test]
    fn deleting_selected_scheme_selects_nearest_remaining() {
        let mut e = editor_multi(
            7100,
            vec![
                vec![KeyToken::Interact, KeyToken::Attack],
                vec![KeyToken::Block, KeyToken::Attack],
            ],
        );
        e.select_scheme(1);
        e.delete_selected_scheme();
        assert_eq!(e.schemes.len(), 1);
        assert_eq!(e.selected_index, 0);
    }

    #[test]
    fn removing_a_slot_keeps_other_slots() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.click_slot(0);
        assert_eq!(e.slots(), &[None, Some(KeyToken::Attack)]);
        e.click_key(KeyToken::Block);
        assert_eq!(e.slots(), &[Some(KeyToken::Block), Some(KeyToken::Attack)]);
    }

    #[test]
    fn prosthetic_tail_key_cannot_fill_first_slot() {
        let mut e = editor(70500, vec![KeyToken::Up, KeyToken::ProstheticUse]);
        e.click_slot(0);
        e.click_slot(1);
        e.click_key(KeyToken::ProstheticUse);
        assert_eq!(e.slots(), &[None, None]);
        assert_eq!(
            e.modal.as_ref().unwrap().message,
            i18n::pick(
                "忍具键/忍具切换键只能作为尾键。",
                "Use Tool / Switch Tool can only be the tail key.",
            )
        );
    }

    #[test]
    fn mode_switch_is_directional_and_requires_confirmation() {
        let mut e = editor(70500, vec![KeyToken::Up, KeyToken::ProstheticUse]);
        e.mode_next();
        assert_eq!(e.mode(), BindingMode::Three);
        assert_eq!(e.slots().len(), 3);
        e.mode_prev();
        assert_eq!(e.mode(), BindingMode::Two);
        e.mode_prev();
        assert_eq!(e.mode(), BindingMode::Two);
        assert!(e.modal.is_some());
        match e.take_modal_action(0) {
            Some(ModalAction::ApplyMode(mode)) => e.apply_mode(mode),
            other => panic!("unexpected modal action: {other:?}"),
        }
        assert_eq!(e.mode(), BindingMode::Single);
        assert_eq!(e.slots().len(), 1);
        e.mode_prev();
        assert_eq!(e.mode(), BindingMode::Single);
        e.mode_next();
        assert_eq!(e.mode(), BindingMode::Two);
        e.mode_next();
        assert_eq!(e.mode(), BindingMode::Three);
    }

    #[test]
    fn prosthetic_three_key_mode_uses_first_middle_tail_slots() {
        let mut e = editor(70500, vec![KeyToken::Up, KeyToken::ProstheticUse]);
        e.mode_next();
        assert_eq!(e.mode(), BindingMode::Three);
        e.click_slot(0);
        e.click_slot(1);
        e.click_key(KeyToken::Block);
        e.click_key(KeyToken::Attack);
        e.click_key(KeyToken::ProstheticUse);
        assert_eq!(
            e.slots(),
            &[
                Some(KeyToken::Block),
                Some(KeyToken::Attack),
                Some(KeyToken::ProstheticUse)
            ]
        );
        assert!(e.complete());
    }

    #[test]
    fn combat_art_three_key_mode_is_right_of_two_key_mode() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        assert_eq!(e.mode(), BindingMode::Two);
        assert!(e.mode_can_next());
        assert!(!e.mode_can_prev());

        e.mode_next();
        assert_eq!(e.mode(), BindingMode::Three);
        assert_eq!(e.slots().len(), 3);
        assert_eq!(e.slots()[2], None);
        e.click_key(KeyToken::Block);
        assert_eq!(
            e.slots(),
            &[Some(KeyToken::Interact), Some(KeyToken::Attack), Some(KeyToken::Block)]
        );

        // Dropping the third key requires confirmation.
        e.mode_prev();
        assert_eq!(e.mode(), BindingMode::Three);
        assert!(e.modal.is_some());
        match e.take_modal_action(0) {
            Some(ModalAction::ApplyMode(mode)) => e.apply_mode(mode),
            other => panic!("unexpected modal action: {other:?}"),
        }
        assert_eq!(e.mode(), BindingMode::Two);
        assert_eq!(e.slots().len(), 2);
    }

    #[test]
    fn arrow_from_two_key_mode_shows_three_slot_ui_for_both_categories() {
        for uid in [7100, 70500] {
            let mut e = editor(uid, vec![KeyToken::Up, KeyToken::Attack]);
            assert!(e.mode_is_switchable());
            e.mode_next();
            assert_eq!(e.mode(), BindingMode::Three);
            let l = layout(&e, 800, 700);
            assert_eq!(l.slots.len(), 3);
            assert!(l.slots[0].r <= l.slots[1].l);
            assert!(l.slots[1].r <= l.slots[2].l);
        }
    }

    #[test]
    fn adding_scheme_in_three_key_mode_keeps_three_slots() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.mode_next();
        e.add_scheme();
        assert_eq!(e.mode(), BindingMode::Three);
        assert_eq!(e.slots(), &[None, None, None]);
    }

    #[test]
    fn invalid_tail_reports_specific_error_not_occupied() {
        let mut e = editor(70500, vec![KeyToken::Interact, KeyToken::ProstheticUse]);
        e.click_slot(1);
        e.click_key(KeyToken::Up);
        assert_eq!(e.modal.as_ref().unwrap().message, "忍具尾键不支持该键位");
        assert_eq!(e.slots(), &[Some(KeyToken::Interact), None]);
    }

    #[test]
    fn occupied_conflict_is_deferred_until_save() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.snapshot.entries.push(BindingEntry {
            uid: 5200,
            keys: vec![KeyToken::Block, KeyToken::Attack],
            source: BindingSource::Cfg,
        });
        e.click_slot(0);
        e.click_key(KeyToken::Block);
        assert!(e.modal.is_none());
        assert_eq!(e.slots(), &[Some(KeyToken::Block), Some(KeyToken::Attack)]);
        assert_eq!(
            e.snapshot.validate_candidate(e.uid, &e.selected_keys()).unwrap_err(),
            BindingValidationError::Occupied
        );
    }

    #[test]
    fn duplicate_scheme_inside_skill_is_rejected_immediately() {
        let mut e = editor_multi(
            7100,
            vec![
                vec![KeyToken::Interact, KeyToken::Attack],
                vec![KeyToken::Block, KeyToken::Attack],
            ],
        );
        e.select_scheme(1);
        e.click_slot(0);
        e.click_key(KeyToken::Interact);
        assert_eq!(e.slots(), &[None, Some(KeyToken::Attack)]);
        assert_eq!(e.modal.as_ref().unwrap().message, "该技能内存在重复方案");
    }

    #[test]
    fn overwrite_modal_has_confirm_and_cancel_only() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.show_overwrite_confirm("other", vec![vec![KeyToken::Interact, KeyToken::Interact]]);
        let modal = e.modal.as_ref().unwrap();
        assert_eq!(modal.buttons.len(), 2);
        assert_eq!(modal.buttons[0].label, "覆盖原键位");
        assert_eq!(modal.buttons[1].label, "取消");
        match &modal.buttons[0].action {
            ModalAction::OverwriteAndSave(keys) => {
                assert_eq!(keys, &vec![vec![KeyToken::Interact, KeyToken::Interact]]);
            }
            other => panic!("unexpected modal action: {other:?}"),
        }
    }

    #[test]
    fn close_prompt_has_yes_no_cancel_actions() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.click_slot(0);
        e.request_close();
        let modal = e.modal.as_ref().unwrap();
        assert_eq!(modal.buttons.len(), 3);
        assert!(matches!(modal.buttons[0].action, ModalAction::SaveAndClose));
        assert!(matches!(modal.buttons[1].action, ModalAction::DiscardAndClose));
        assert!(matches!(modal.buttons[2].action, ModalAction::Dismiss));
    }

    #[test]
    fn delete_prompt_has_confirm_and_cancel_actions() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.request_delete_scheme();
        let modal = e.modal.as_ref().unwrap();
        assert_eq!(modal.buttons.len(), 2);
        assert!(matches!(modal.buttons[0].action, ModalAction::DeleteSelectedScheme));
        assert!(matches!(modal.buttons[1].action, ModalAction::Dismiss));
    }

    #[test]
    fn full_slots_show_required_message() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.click_key(KeyToken::Block);
        assert_eq!(
            e.modal.as_ref().unwrap().message,
            i18n::pick(
                "键位已满，请点击配置区键位去移除。",
                "Key slots are full. Click a slot in the configuration area to remove it.",
            )
        );
    }

    #[test]
    fn incomplete_binding_is_not_saveable() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.click_slot(1);
        assert!(!e.complete());
    }

    #[test]
    fn new_action_keys_are_enabled_in_grid() {
        let e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        let items = grid_items(&e);
        for token in [KeyToken::Jump, KeyToken::Sprint, KeyToken::Crouch] {
            let item = items.iter().find(|(_, t, _)| *t == Some(token)).unwrap();
            assert!(!item.2, "{} should be enabled", token.label());
        }
        assert!(
            !items
                .iter()
                .any(|(_, _, disabled)| *disabled && items.iter().any(|(label, _, _)| label.contains("暂不支持")))
        );
    }

    #[test]
    fn reset_modal_requires_confirmation_and_mentions_slot_removal() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.request_reset();
        let modal = e.modal.as_ref().unwrap();
        assert!(modal.message.contains("若要更换键位"));
        assert_eq!(modal.buttons[0].label, "确认重置");
        assert_eq!(modal.buttons[1].label, "取消");
        assert!(matches!(modal.buttons[0].action, ModalAction::ResetAndClose));
    }

    #[test]
    fn bottom_buttons_are_three_non_overlapping_rects() {
        let e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        let l = layout(&e, 760, 760);
        assert!(l.save.r <= l.reset.l);
        assert!(l.reset.r <= l.cancel.l);
        assert_eq!(l.save.t, l.reset.t);
        assert_eq!(l.reset.t, l.cancel.t);
    }

    #[test]
    fn schemes_use_two_columns_and_wrap_to_new_rows() {
        let e = editor_multi(
            7100,
            vec![
                vec![KeyToken::Interact, KeyToken::Attack],
                vec![KeyToken::Block, KeyToken::Attack],
                vec![KeyToken::Jump, KeyToken::Attack],
            ],
        );
        let l = layout(&e, 760, 760);
        assert_eq!(l.scheme_buttons.len(), 3);
        assert_eq!(l.scheme_buttons[0].t, l.scheme_buttons[1].t);
        assert!(l.scheme_buttons[0].r <= l.scheme_buttons[1].l);
        assert!(l.scheme_buttons[2].t > l.scheme_buttons[0].t);
        assert!(l.scheme_buttons[2].b <= l.add_scheme.t);
    }

    #[test]
    fn scheme_actions_are_above_first_and_second_key_slots() {
        let e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        let l = layout(&e, 760, 760);
        assert!(l.add_scheme.b <= l.slots[0].t);
        assert!(l.delete_scheme.b <= l.slots[0].t);
    }
}
