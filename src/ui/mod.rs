mod editor;
mod game_overlay;
mod i18n;
mod overlay_render;
mod paint_worker;
mod render;

use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
};

use anyhow::{Result, anyhow};
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::{
        Input::KeyboardAndMouse::{VK_ESCAPE, VK_F},
        WindowsAndMessaging::GetClientRect,
    },
};

use crate::{
    binding_validation::BindingValidationError,
    bindings::{self, BindingSnapshot, KeyToken},
    game_menu, menu_probe,
};

use self::editor::{Editor, ModalAction, layout};

static UI_STATE: OnceLock<Mutex<UiState>> = OnceLock::new();
static UI_OPEN: AtomicBool = AtomicBool::new(false);
static FRAME_W: AtomicI32 = AtomicI32::new(0);
static FRAME_H: AtomicI32 = AtomicI32::new(0);
static CURSOR_X: AtomicI32 = AtomicI32::new(0);
static CURSOR_Y: AtomicI32 = AtomicI32::new(0);
static CURSOR_VISIBLE: AtomicBool = AtomicBool::new(false);

pub(crate) struct UiState {
    pub(crate) editor: Option<Editor>,
    pub(crate) frame_width: i32,
    pub(crate) frame_height: i32,
    pub(crate) cursor_x: f32,
    pub(crate) cursor_y: f32,
    cfg_path: PathBuf,
    game_hwnd: isize,
    client_width: i32,
    client_height: i32,
}

pub(crate) fn state() -> &'static Mutex<UiState> {
    UI_STATE.get().expect("UI state must be initialized before hooks")
}

#[inline]
pub(crate) fn overlay_open() -> bool {
    UI_OPEN.load(Ordering::Acquire)
}

pub(crate) fn frame_size() -> (i32, i32) {
    (FRAME_W.load(Ordering::Acquire), FRAME_H.load(Ordering::Acquire))
}

pub(crate) fn set_frame_size(width: i32, height: i32) {
    let width = width.max(1);
    let height = height.max(1);
    let changed = FRAME_W.swap(width, Ordering::AcqRel) != width || FRAME_H.swap(height, Ordering::AcqRel) != height;
    if changed && overlay_open() {
        paint_worker::request_repaint();
    }
}

pub(crate) fn cursor_position() -> Option<(f32, f32)> {
    if !overlay_open() || !CURSOR_VISIBLE.load(Ordering::Acquire) {
        return None;
    }
    Some((
        CURSOR_X.load(Ordering::Acquire) as f32,
        CURSOR_Y.load(Ordering::Acquire) as f32,
    ))
}

fn lock_state() -> std::sync::MutexGuard<'static, UiState> {
    match state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::error!("ui: state mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

pub fn start(cfg_path: PathBuf) -> Result<()> {
    i18n::set_lang(i18n::load_lang_from_cfg(&cfg_path));
    UI_STATE
        .set(Mutex::new(UiState {
            editor: None,
            frame_width: 0,
            frame_height: 0,
            cursor_x: 0.0,
            cursor_y: 0.0,
            cfg_path,
            game_hwnd: 0,
            client_width: 0,
            client_height: 0,
        }))
        .map_err(|_| anyhow!("UI state was already initialized"))?;

    paint_worker::start()?;
    game_overlay::install()?;
    log::warn!("ui: in-game overlay input/rendering ready");
    Ok(())
}

fn current_row_is_bind_target() -> bool {
    game_menu::current_selection().is_some_and(|selection| bindings::category_for_uid(selection.uid).is_some())
}

/// Called from the game's WndProc. Returns true if the message was consumed.
pub(crate) fn on_key(hwnd: HWND, vk: u32) -> bool {
    if overlay_open() {
        if vk == VK_ESCAPE.0 as u32 || vk == VK_F.0 as u32 {
            request_close();
            if overlay_open() {
                paint_worker::request_repaint();
            }
        }
        return true;
    }

    let menu_open = menu_probe::menu_open_cached() || menu_probe::menu_open();
    if vk == VK_F.0 as u32 && menu_open && current_row_is_bind_target() {
        if let Err(error) = open_overlay(hwnd) {
            log::warn!("ui: cannot open key config: {error:#}");
        }
        return true;
    }
    false
}

pub(crate) fn on_mouse_move(hwnd: HWND, client_x: i32, client_y: i32) {
    if !overlay_open() {
        return;
    }
    let mut state = lock_state();
    if state.editor.is_none() {
        return;
    }
    let _ = hwnd;
    let client_w = state.client_width.max(1);
    let client_h = state.client_height.max(1);
    let (frame_w, frame_h) = frame_size();
    let frame_w = if frame_w > 1 { frame_w } else { state.frame_width.max(1) };
    let frame_h = if frame_h > 1 {
        frame_h
    } else {
        state.frame_height.max(1)
    };
    let x = (client_x as f32 * frame_w as f32 / client_w as f32).clamp(0.0, frame_w as f32 - 1.0);
    let y = (client_y as f32 * frame_h as f32 / client_h as f32).clamp(0.0, frame_h as f32 - 1.0);
    state.cursor_x = x;
    state.cursor_y = y;
    CURSOR_X.store(x.round() as i32, Ordering::Release);
    CURSOR_Y.store(y.round() as i32, Ordering::Release);
    CURSOR_VISIBLE.store(true, Ordering::Release);
}

pub(crate) fn on_mouse_click(_hwnd: HWND) {
    let mut state = lock_state();
    let Some(editor) = state.editor.as_ref() else {
        return;
    };
    let (frame_w, frame_h) = frame_size();
    let frame_w = if frame_w > 1 { frame_w } else { state.frame_width.max(1) };
    let frame_h = if frame_h > 1 {
        frame_h
    } else {
        state.frame_height.max(1)
    };
    let (panel_w, panel_h, panel_x, panel_y) = render::panel_geometry(editor, frame_w, frame_h);
    let local_x = state.cursor_x.round() as i32 - panel_x as i32;
    let local_y = state.cursor_y.round() as i32 - panel_y as i32;
    if local_x < 0 || local_y < 0 || local_x >= panel_w || local_y >= panel_h {
        return;
    }
    handle_click(&mut state, local_x, local_y, panel_w, panel_h);
}

fn open_overlay(hwnd: HWND) -> Result<()> {
    let cfg_path = {
        let state = lock_state();
        if state.editor.is_some() {
            return Ok(());
        }
        state.cfg_path.clone()
    };

    let Some(editor) = open_editor(&cfg_path)? else {
        log::debug!("ui: F ignored; current menu row is not a keybind target");
        return Ok(());
    };

    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let client_width = (client.right - client.left).max(1);
    let client_height = (client.bottom - client.top).max(1);
    let (frame_w, frame_h) = frame_size();
    let frame_w = if frame_w > 1 { frame_w } else { client_width };
    let frame_h = if frame_h > 1 { frame_h } else { client_height };
    FRAME_W.store(frame_w, Ordering::Release);
    FRAME_H.store(frame_h, Ordering::Release);

    let uid = editor.uid;
    let name = editor.name.clone();
    let menu_uid = editor.menu_uid;
    let menu_category = editor.menu_category;
    let (panel_w, panel_h, panel_x, panel_y) = render::panel_geometry(&editor, frame_w, frame_h);

    let mut state = lock_state();
    state.game_hwnd = hwnd.0 as isize;
    state.client_width = client_width;
    state.client_height = client_height;
    state.frame_width = frame_w;
    state.frame_height = frame_h;
    state.cursor_x = panel_x + panel_w as f32 * 0.5;
    state.cursor_y = panel_y + panel_h as f32 * 0.5;
    state.editor = Some(editor);
    CURSOR_X.store(state.cursor_x.round() as i32, Ordering::Release);
    CURSOR_Y.store(state.cursor_y.round() as i32, Ordering::Release);
    CURSOR_VISIBLE.store(true, Ordering::Release);
    UI_OPEN.store(true, Ordering::Release);
    game_overlay::release_cursor_clip();
    game_overlay::reset_surface();
    paint_worker::clear();
    paint_worker::request_repaint();
    crate::frame_perf::mark(crate::frame_perf::EventId::UiOpen);
    log::warn!(
        "ui: opened in-game key config uid={} name={} from menu uid={} category={}",
        uid,
        name,
        menu_uid,
        menu_category
    );
    Ok(())
}

fn open_editor(cfg_path: &std::path::Path) -> Result<Option<Editor>> {
    let snapshot = BindingSnapshot::load(cfg_path)?;
    let Some(selection) = game_menu::current_selection() else {
        return Ok(None);
    };
    if bindings::category_for_uid(selection.uid).is_none() {
        return Ok(None);
    }
    let uid = selection.uid;
    let Some(mut editor) = Editor::new(uid, snapshot) else {
        return Ok(None);
    };
    editor.menu_uid = selection.uid;
    editor.menu_category = selection.category;
    Ok(Some(editor))
}

fn request_close() {
    let mut state = lock_state();
    request_close_state(&mut state);
}

fn request_close_state(state: &mut UiState) {
    let close = if let Some(editor) = state.editor.as_mut() {
        if editor.modal.is_some() {
            // Esc / F while a dialog is open only dismisses that dialog.
            editor.modal_cancel();
            false
        } else {
            editor.request_close();
            editor.modal.is_none()
        }
    } else {
        false
    };
    if close {
        close_overlay(state);
    }
}

fn close_overlay(state: &mut UiState) {
    UI_OPEN.store(false, Ordering::Release);
    CURSOR_VISIBLE.store(false, Ordering::Release);
    game_overlay::reset_surface();
    paint_worker::clear();
    crate::frame_perf::mark(crate::frame_perf::EventId::UiClose);
    state.editor = None;
    state.game_hwnd = 0;
}

#[derive(Debug)]
enum ClickAction {
    None,
    Close,
    Save,
    SaveSchemes(Vec<Vec<KeyToken>>, bool),
    ResetNow,
    Cancel,
}

fn click_action(editor: &mut Editor, w: i32, h: i32, x: i32, y: i32) -> ClickAction {
    let l = layout(editor, w, h);

    if let Some(panel) = l.modal_panel {
        if panel.contains(x, y) {
            for (index, button) in l.modal_buttons.iter().copied().enumerate() {
                if button.contains(x, y) {
                    return match editor.take_modal_action(index) {
                        Some(ModalAction::Dismiss) | None => ClickAction::None,
                        Some(ModalAction::SaveAndClose) => ClickAction::Save,
                        Some(ModalAction::DiscardAndClose) => ClickAction::Close,
                        Some(ModalAction::ApplyMode(mode)) => {
                            editor.apply_mode(mode);
                            ClickAction::None
                        }
                        Some(ModalAction::OverwriteAndSave(schemes)) => ClickAction::SaveSchemes(schemes, true),
                        Some(ModalAction::ResetAndClose) => ClickAction::ResetNow,
                        Some(ModalAction::DeleteSelectedScheme) => {
                            editor.delete_selected_scheme();
                            ClickAction::None
                        }
                    };
                }
            }
        }
        return ClickAction::None;
    }
    if editor.modal.is_some() {
        return ClickAction::None;
    }

    for (index, rect) in l.scheme_buttons.iter().copied().enumerate() {
        if rect.contains(x, y) {
            editor.select_scheme(index);
            return ClickAction::None;
        }
    }
    if l.add_scheme.contains(x, y) {
        editor.add_scheme();
        return ClickAction::None;
    }
    if l.delete_scheme.contains(x, y) {
        editor.request_delete_scheme();
        return ClickAction::None;
    }

    if editor.mode_is_switchable() && l.mode_left.contains(x, y) {
        editor.mode_prev();
        return ClickAction::None;
    }
    if editor.mode_is_switchable() && l.mode_right.contains(x, y) {
        editor.mode_next();
        return ClickAction::None;
    }
    for (index, slot) in l.slots.iter().enumerate() {
        if slot.contains(x, y) {
            editor.click_slot(index);
            return ClickAction::None;
        }
    }
    for item in &l.grid {
        if item.rect.contains(x, y) {
            if item.disabled {
                editor.show_message(i18n::pick(
                    "当前版本暂不支持该键位",
                    "This key is not supported in this version.",
                ));
            } else if let Some(token) = item.token {
                editor.click_key(token);
            }
            return ClickAction::None;
        }
    }
    if l.save.contains(x, y) {
        ClickAction::Save
    } else if l.reset.contains(x, y) {
        editor.request_reset();
        ClickAction::None
    } else if l.cancel.contains(x, y) {
        ClickAction::Cancel
    } else {
        ClickAction::None
    }
}

fn handle_click(state: &mut UiState, x: i32, y: i32, w: i32, h: i32) {
    paint_worker::request_repaint();
    let action = {
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        click_action(editor, w, h, x, y)
    };

    match action {
        ClickAction::None => {}
        ClickAction::Close => close_overlay(state),
        ClickAction::Save => save_editor(state),
        ClickAction::SaveSchemes(schemes, overwrite) => save_editor_schemes(state, schemes, overwrite),
        ClickAction::ResetNow => reset_editor_binding(state),
        ClickAction::Cancel => request_close_state(state),
    }
}

fn save_editor(state: &mut UiState) {
    let schemes = {
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        if !editor.complete() {
            editor.show_message(i18n::pick(
                "请先完成所有方案的键位配置",
                "Complete every scheme before saving.",
            ));
            return;
        }

        let schemes = editor.all_keys();
        match editor.snapshot.validate_scheme_set(editor.uid, &schemes) {
            Ok(()) => schemes,
            Err(BindingValidationError::Occupied) => {
                let conflicts = editor.snapshot.conflicting_entries(editor.uid, &schemes);
                let mut names = Vec::new();
                for entry in conflicts {
                    let name = editor
                        .snapshot
                        .name(entry.uid)
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| format!("UID {}", entry.uid));
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                let conflict_names = if names.is_empty() {
                    i18n::pick("其他技能/忍具", "another combat art / prosthetic").to_string()
                } else {
                    names.join("、")
                };
                editor.show_overwrite_confirm(&conflict_names, schemes);
                return;
            }
            Err(error) => {
                editor.show_message(&i18n::validation_error(error));
                return;
            }
        }
    };

    save_editor_schemes(state, schemes, false);
}

fn save_editor_schemes(state: &mut UiState, schemes: Vec<Vec<KeyToken>>, overwrite: bool) {
    let result = {
        let Some(editor) = state.editor.as_ref() else {
            return;
        };
        if overwrite {
            bindings::save_ui_schemes_overwrite(&state.cfg_path, &editor.snapshot, editor.uid, schemes)
        } else {
            bindings::save_ui_schemes(&state.cfg_path, &editor.snapshot, editor.uid, schemes)
        }
    };

    match result {
        Ok(()) => {
            let (uid, name, mode, scheme_count) = state
                .editor
                .as_ref()
                .map(|editor| (editor.uid, editor.name.clone(), editor.mode(), editor.schemes.len()))
                .unwrap_or((0, String::new(), editor::BindingMode::Two, 0));
            bindings::request_reload();
            log::warn!("ui: saved uid={uid} name={name} mode={mode:?} schemes={scheme_count} overwrite={overwrite}");
            close_overlay(state);
        }
        Err(error) => {
            if let Some(editor) = state.editor.as_mut() {
                editor.show_message(&format!("{}: {error:#}", i18n::pick("保存失败", "Save failed"),));
            }
        }
    }
}

fn reset_editor_binding(state: &mut UiState) {
    let (uid, name) = match state.editor.as_ref() {
        Some(editor) => (editor.uid, editor.name.clone()),
        None => return,
    };
    match bindings::reset_ui_binding(&state.cfg_path, uid) {
        Ok(()) => {
            bindings::request_reload();
            log::warn!("ui: reset uid={uid} name={name}");
            close_overlay(state);
        }
        Err(error) => {
            if let Some(editor) = state.editor.as_mut() {
                editor.show_message(&format!("{}: {error:#}", i18n::pick("重置失败", "Reset failed"),));
            }
        }
    }
}
