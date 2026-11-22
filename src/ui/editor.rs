use crate::binding_validation::BindingValidationError;
use crate::bindings::{BindingCategory, BindingSnapshot, BindingSource, KeyToken};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingMode {
    Single,
    Two,
}

impl BindingMode {
    pub fn label(self) -> &'static str {
        match self {
            BindingMode::Single => "单键触发",
            BindingMode::Two => "双键触发",
        }
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

#[derive(Clone, Debug)]
pub enum ModalAction {
    Dismiss,
    DiscardAndClose,
    ApplyMode(BindingMode),
    OverwriteAndSave(Vec<KeyToken>),
    ResetAndClose,
}

#[derive(Clone, Debug)]
pub struct Modal {
    pub message: String,
    pub confirm: String,
    pub cancel: Option<String>,
    pub action: ModalAction,
}

#[derive(Clone, Debug)]
pub struct Editor {
    pub uid: u32,
    pub name: String,
    pub category: BindingCategory,
    pub source: BindingSource,
    pub menu_uid: u32,
    pub menu_category: u32,
    pub mode: BindingMode,
    pub slots: Vec<Option<KeyToken>>,
    original: Vec<Option<KeyToken>>,
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
        let current = snapshot.current_entry(uid);
        let source = current.map(|entry| entry.source).unwrap_or(BindingSource::None);
        let existing = current.map(|entry| entry.keys.clone());
        let mode = match (category, existing.as_ref().map(Vec::len)) {
            (BindingCategory::Prosthetic, Some(1)) => BindingMode::Single,
            _ => BindingMode::Two,
        };
        let slots = match (mode, existing) {
            (BindingMode::Single, Some(keys)) => vec![keys.first().copied()],
            (BindingMode::Two, Some(keys)) => vec![keys.get(0).copied(), keys.get(1).copied()],
            (BindingMode::Single, None) => vec![None],
            (BindingMode::Two, None) => vec![None, None],
        };
        let original = slots.clone();
        Some(Editor {
            uid,
            name,
            category,
            source,
            menu_uid: 0,
            menu_category: 0,
            mode,
            slots,
            original,
            modal: None,
            snapshot,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.slots != self.original
    }

    pub fn complete(&self) -> bool {
        self.slots.iter().all(Option::is_some)
    }

    pub fn keys(&self) -> Vec<KeyToken> {
        self.slots.iter().flatten().copied().collect()
    }

    pub fn category_label(&self) -> &'static str {
        match self.category {
            BindingCategory::CombatArt => "流派招式",
            BindingCategory::Prosthetic => "义手忍具",
        }
    }

    pub fn source_label(&self) -> &'static str {
        match self.source {
            BindingSource::Cfg => "来源：cfg",
            BindingSource::UiOverride => "来源：UI 覆盖",
            BindingSource::None => "来源：未配置",
        }
    }

    pub fn mode_is_switchable(&self) -> bool {
        self.category == BindingCategory::Prosthetic
    }

    pub fn mode_can_prev(&self) -> bool {
        self.mode_is_switchable() && self.mode == BindingMode::Two
    }

    pub fn mode_can_next(&self) -> bool {
        self.mode_is_switchable() && self.mode == BindingMode::Single
    }

    /// `<`: move to the previous legal mode. Combat arts are fixed at Two.
    pub fn mode_prev(&mut self) {
        if !self.mode_can_prev() {
            return;
        }
        if self.slots.get(1).and_then(|v| *v).is_some() {
            self.modal = Some(Modal {
                message: "切换为单键触发后，第2键将被清空，是否继续？".into(),
                confirm: "确认切换".into(),
                cancel: Some("取消".into()),
                action: ModalAction::ApplyMode(BindingMode::Single),
            });
        } else {
            self.apply_mode(BindingMode::Single);
        }
    }

    /// `>`: move to the next legal mode. Combat arts are fixed at Two.
    pub fn mode_next(&mut self) {
        if self.mode_can_next() {
            self.apply_mode(BindingMode::Two);
        }
    }

    pub fn apply_mode(&mut self, mode: BindingMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.slots.resize(
            match mode {
                BindingMode::Single => 1,
                BindingMode::Two => 2,
            },
            None,
        );
    }

    pub fn click_slot(&mut self, index: usize) {
        if let Some(slot) = self.slots.get_mut(index) {
            if slot.is_some() {
                *slot = None;
            }
        }
    }

    pub fn click_key(&mut self, token: KeyToken) {
        if self.slots.iter().all(Option::is_some) {
            self.show_message("键位已满，请点击配置区键位去除");
            return;
        }
        if self.category == BindingCategory::CombatArt && token.is_prosthetic() {
            self.show_message("流派招式不能使用忍具键/忍具切换键");
            return;
        }
        if token.is_prosthetic() && self.mode == BindingMode::Two {
            if self.slots.first().is_some_and(Option::is_none) {
                self.show_message("忍具键/忍具切换键只能作为尾键");
                return;
            }
            if self.slots.get(1).is_some_and(Option::is_none) {
                self.slots[1] = Some(token);
                self.check_after_edit(1);
            } else {
                self.show_message("键位已满，请点击配置区键位去除");
            }
            return;
        }
        if self.mode == BindingMode::Single && !token.is_prosthetic() {
            return;
        }
        if let Some(index) = self.slots.iter().position(Option::is_none) {
            self.slots[index] = Some(token);
            self.check_after_edit(index);
        } else {
            self.show_message("键位已满，请点击配置区键位去除");
        }
    }

    /// Validate a completed draft. Invalid input and occupied sequences are
    /// distinct typed errors. The rollback removes only the slot that was just
    /// clicked, never an unrelated earlier slot.
    fn check_after_edit(&mut self, clicked_index: usize) {
        if !self.complete() {
            return;
        }
        let keys = self.keys();
        if let Err(error) = self.snapshot.validate_candidate(self.uid, &keys) {
            // Occupied key sequences are a save-time decision. Keep the draft
            // intact so the save button can offer overwrite/cancel.
            if error == BindingValidationError::Occupied {
                return;
            }
            if let Some(slot) = self.slots.get_mut(clicked_index) {
                *slot = None;
            }
            self.show_message(&error.to_string());
        }
    }

    pub fn show_message(&mut self, message: &str) {
        self.modal = Some(Modal {
            message: message.into(),
            confirm: "确定".into(),
            cancel: None,
            action: ModalAction::Dismiss,
        });
    }

    pub fn show_overwrite_confirm(&mut self, conflict_name: &str, keys: Vec<KeyToken>) {
        self.modal = Some(Modal {
            message: format!("当前键位已被「{conflict_name}」占用。\n覆盖后原键位将被清除，是否继续？"),
            confirm: "覆盖原键位".into(),
            cancel: Some("取消".into()),
            action: ModalAction::OverwriteAndSave(keys),
        });
    }

    pub fn request_close(&mut self) {
        if self.is_dirty() {
            self.modal = Some(Modal {
                message: "有未保存的键位修改，是否放弃？".into(),
                confirm: "放弃修改".into(),
                cancel: Some("继续编辑".into()),
                action: ModalAction::DiscardAndClose,
            });
        }
    }

    /// Reset removes every trigger binding for this skill/prosthetic. The
    /// confirmation also tells the user how to replace individual slots without
    /// using the reset path.
    pub fn request_reset(&mut self) {
        self.modal = Some(Modal {
            message: "确定要重置此技能/忍具的触发键位吗？\n\n（若要更换键位，请点击首键或尾键的对应键位以移除）".into(),
            confirm: "确认重置".into(),
            cancel: Some("取消".into()),
            action: ModalAction::ResetAndClose,
        });
    }

    pub fn modal_confirm(&mut self) -> Option<ModalAction> {
        let modal = self.modal.take()?;
        Some(modal.action)
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
    pub slots: Vec<Rect>,
    pub grid: Vec<GridItem>,
    pub save: Rect,
    pub reset: Rect,
    pub cancel: Rect,
    pub footnote: Rect,
    pub modal_panel: Option<Rect>,
    pub modal_confirm: Option<Rect>,
    pub modal_cancel: Option<Rect>,
}

pub fn grid_items(editor: &Editor) -> Vec<(String, Option<KeyToken>, bool)> {
    let mut out = Vec::new();
    if editor.mode == BindingMode::Single {
        out.push(("忍具键".into(), Some(KeyToken::ProstheticUse), false));
        out.push(("忍具切换键".into(), Some(KeyToken::ProstheticSwitch), false));
        return out;
    }
    for token in KeyToken::ALL_NORMAL {
        out.push((token.label().into(), Some(token), false));
    }
    match editor.category {
        BindingCategory::CombatArt => {
            out.push(("忍具键（仅义手忍具）".into(), Some(KeyToken::ProstheticUse), true));
            out.push((
                "忍具切换键（仅义手忍具）".into(),
                Some(KeyToken::ProstheticSwitch),
                true,
            ));
        }
        BindingCategory::Prosthetic => {
            out.push(("忍具键".into(), Some(KeyToken::ProstheticUse), false));
            out.push(("忍具切换键".into(), Some(KeyToken::ProstheticSwitch), false));
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
    let mode = Rect::new(content_l, 92, content_r, 134);
    let arrow_w = 42;
    let mode_left = Rect::new(content_l, mode.t, content_l + arrow_w, mode.b);
    let mode_right = Rect::new(content_r - arrow_w, mode.t, content_r, mode.b);
    let slots_top = 146;
    let slots_h = 96;
    let slot_gap = 10;
    let n = editor.slots.len() as i32;
    let mut slots = Vec::with_capacity(n as usize);
    if n == 1 {
        slots.push(Rect::new(
            content_l + 90,
            slots_top,
            content_r - 90,
            slots_top + slots_h,
        ));
    } else {
        let width = (content_r - content_l - slot_gap) / 2;
        slots.push(Rect::new(content_l, slots_top, content_l + width, slots_top + slots_h));
        slots.push(Rect::new(
            content_l + width + slot_gap,
            slots_top,
            content_r,
            slots_top + slots_h,
        ));
    }

    let grid_top = 294;
    let gap = 8;
    let items = grid_items(editor);
    let cols = 4usize;
    let rows = (items.len() + cols - 1) / cols;
    let cell_w = (content_r - content_l - gap * (cols as i32 - 1)) / cols as i32;
    let cell_h = 44;
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
    let footnote = Rect::new(content_l, grid_bottom + 10, content_r, grid_bottom + 34);
    let button_y = h - 56;
    let button_w = (content_r - content_l - gap * 2) / 3;
    let save = Rect::new(content_l, button_y, content_l + button_w, h - 18);
    let reset = Rect::new(
        content_l + button_w + gap,
        button_y,
        content_l + button_w * 2 + gap,
        h - 18,
    );
    let cancel = Rect::new(content_l + button_w * 2 + gap * 2, button_y, content_r, h - 18);

    let mut modal_panel = None;
    let mut modal_confirm = None;
    let mut modal_cancel = None;
    if editor.modal.is_some() {
        let mw = (w - 80).min(560).max(360);
        let mh = 180;
        let ml = (w - mw) / 2;
        let mt = (h - mh) / 2;
        let mp = Rect::new(ml, mt, ml + mw, mt + mh);
        modal_panel = Some(mp);
        if editor.modal.as_ref().and_then(|m| m.cancel.as_ref()).is_some() {
            let bw = (mw - 50) / 2;
            modal_confirm = Some(Rect::new(ml + 20, mt + mh - 56, ml + 20 + bw, mt + mh - 20));
            modal_cancel = Some(Rect::new(ml + 30 + bw, mt + mh - 56, ml + 30 + bw + bw, mt + mh - 20));
        } else {
            modal_confirm = Some(Rect::new(
                ml + mw / 2 - 70,
                mt + mh - 56,
                ml + mw / 2 + 70,
                mt + mh - 20,
            ));
        }
    }

    Layout {
        panel,
        title,
        category,
        mode,
        mode_left,
        mode_right,
        slots,
        grid,
        save,
        reset,
        cancel,
        footnote,
        modal_panel,
        modal_confirm,
        modal_cancel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingEntry, BindingSnapshot};
    use std::collections::HashMap;

    fn editor(uid: u32, keys: Vec<KeyToken>) -> Editor {
        let mut names = HashMap::new();
        names.insert(uid, "测试技能".to_string());
        let snapshot = BindingSnapshot {
            entries: vec![BindingEntry {
                uid,
                keys,
                source: crate::bindings::BindingSource::Cfg,
            }],
            names,
        };
        Editor::new(uid, snapshot).unwrap()
    }

    #[test]
    fn editor_prefills_existing_cfg_binding() {
        let e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        assert_eq!(e.slots, vec![Some(KeyToken::Interact), Some(KeyToken::Attack)]);
        assert_eq!(e.source, BindingSource::Cfg);
    }

    #[test]
    fn removing_a_slot_keeps_other_slots() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.click_slot(0);
        assert_eq!(e.slots, vec![None, Some(KeyToken::Attack)]);
        e.click_key(KeyToken::Block);
        assert_eq!(e.slots, vec![Some(KeyToken::Block), Some(KeyToken::Attack)]);
    }

    #[test]
    fn prosthetic_tail_key_cannot_fill_first_slot() {
        let mut e = editor(70500, vec![KeyToken::Up, KeyToken::ProstheticUse]);
        e.click_slot(0);
        e.click_slot(1);
        e.click_key(KeyToken::ProstheticUse);
        assert_eq!(e.slots, vec![None, None]);
        assert_eq!(e.modal.as_ref().unwrap().message, "忍具键/忍具切换键只能作为尾键");
    }

    #[test]
    fn mode_switch_is_directional_and_requires_confirmation() {
        let mut e = editor(70500, vec![KeyToken::Up, KeyToken::ProstheticUse]);
        e.mode_next();
        assert_eq!(e.mode, BindingMode::Two);
        e.mode_prev();
        assert_eq!(e.mode, BindingMode::Two);
        assert!(e.modal.is_some());
        match e.modal_confirm() {
            Some(ModalAction::ApplyMode(mode)) => e.apply_mode(mode),
            other => panic!("unexpected modal action: {other:?}"),
        }
        assert_eq!(e.mode, BindingMode::Single);
        assert_eq!(e.slots.len(), 1);
        e.mode_prev();
        assert_eq!(e.mode, BindingMode::Single);
        e.mode_next();
        assert_eq!(e.mode, BindingMode::Two);
    }

    #[test]
    fn invalid_tail_reports_specific_error_not_occupied() {
        let mut e = editor(70500, vec![KeyToken::Interact, KeyToken::ProstheticUse]);
        e.click_slot(1);
        e.click_key(KeyToken::Up);
        assert_eq!(e.modal.as_ref().unwrap().message, "忍具尾键不支持该键位");
        assert_eq!(e.slots, vec![Some(KeyToken::Interact), None]);
    }

    #[test]
    fn occupied_conflict_is_deferred_until_save() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.snapshot.entries.push(BindingEntry {
            uid: 5200,
            keys: vec![KeyToken::Block, KeyToken::Attack],
            source: crate::bindings::BindingSource::Cfg,
        });
        e.click_slot(0);
        e.click_key(KeyToken::Block);
        assert!(e.modal.is_none());
        assert_eq!(e.slots, vec![Some(KeyToken::Block), Some(KeyToken::Attack)]);
        assert_eq!(
            e.snapshot.validate_candidate(e.uid, &e.keys()).unwrap_err(),
            BindingValidationError::Occupied
        );
    }

    #[test]
    fn overwrite_modal_has_confirm_and_cancel_only() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.show_overwrite_confirm("other", vec![KeyToken::Interact, KeyToken::Interact]);
        let modal = e.modal.as_ref().unwrap();
        assert_eq!(modal.confirm, "覆盖原键位");
        assert_eq!(modal.cancel.as_deref(), Some("取消"));
        match &modal.action {
            ModalAction::OverwriteAndSave(keys) => {
                assert_eq!(keys, &vec![KeyToken::Interact, KeyToken::Interact]);
            }
            other => panic!("unexpected modal action: {other:?}"),
        }
    }
    #[test]
    fn full_slots_show_required_message() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.click_key(KeyToken::Block);
        assert_eq!(e.modal.as_ref().unwrap().message, "键位已满，请点击配置区键位去除");
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
                .any(|(_, _, disabled)| *disabled && items.iter().any(|(label, _, _)| label.contains("暂未支持")))
        );
    }

    #[test]
    fn reset_modal_requires_confirmation_and_mentions_slot_removal() {
        let mut e = editor(7100, vec![KeyToken::Interact, KeyToken::Attack]);
        e.request_reset();
        let modal = e.modal.as_ref().unwrap();
        assert!(modal.message.contains("若要更换键位，请点击首键或尾键的对应键位以移除"));
        assert_eq!(modal.confirm, "确认重置");
        assert_eq!(modal.cancel.as_deref(), Some("取消"));
        assert!(matches!(modal.action, ModalAction::ResetAndClose));
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
}
