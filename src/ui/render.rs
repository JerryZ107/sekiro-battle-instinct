use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, RECT},
        Graphics::Gdi::{
            BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
            DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
            FillRect, HDC, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
        },
        UI::WindowsAndMessaging::GetClientRect,
    },
    core::PCWSTR,
};

use super::editor::{BindingMode, Editor, GridItem, Rect, layout};

const BG: COLORREF = COLORREF(0x001d1b19);
const PANEL: COLORREF = COLORREF(0x00282420);
const PANEL_2: COLORREF = COLORREF(0x00322c25);
const GOLD: COLORREF = COLORREF(0x007b6747);
const GOLD_BRIGHT: COLORREF = COLORREF(0x00c7a66a);
const TEXT: COLORREF = COLORREF(0x00e8dcc5);
const MUTED: COLORREF = COLORREF(0x00958a78);
const DISABLED_BG: COLORREF = COLORREF(0x00231f1c);
const DISABLED_TEXT: COLORREF = COLORREF(0x00665f54);
const GREEN: COLORREF = COLORREF(0x00465d3d);
const RESET_BG: COLORREF = COLORREF(0x00483f2e);
const RED: COLORREF = COLORREF(0x005c3838);

fn fill(hdc: HDC, rect: Rect, color: COLORREF) {
    let brush = unsafe { CreateSolidBrush(color) };
    let r = RECT {
        left: rect.l,
        top: rect.t,
        right: rect.r,
        bottom: rect.b,
    };
    unsafe {
        FillRect(hdc, &r, brush);
        let _ = DeleteObject(brush);
    }
}

fn frame(hdc: HDC, rect: Rect, color: COLORREF, width: i32) {
    fill(hdc, Rect::new(rect.l, rect.t, rect.r, rect.t + width), color);
    fill(hdc, Rect::new(rect.l, rect.b - width, rect.r, rect.b), color);
    fill(hdc, Rect::new(rect.l, rect.t, rect.l + width, rect.b), color);
    fill(hdc, Rect::new(rect.r - width, rect.t, rect.r, rect.b), color);
}

fn text(hdc: HDC, rect: Rect, value: &str, size: i32, color: COLORREF, bold: bool, center: bool) {
    let mut wide: Vec<u16> = value.encode_utf16().collect();
    let face: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let font = unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            if bold { 700 } else { 400 },
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            PCWSTR::from_raw(face.as_ptr()),
        )
    };
    let mut r = RECT {
        left: rect.l,
        top: rect.t,
        right: rect.r,
        bottom: rect.b,
    };
    unsafe {
        let old = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color);
        let flags = DT_SINGLELINE
            | DT_VCENTER
            | DT_END_ELLIPSIS
            | if center {
                DT_CENTER
            } else {
                windows::Win32::Graphics::Gdi::DT_LEFT
            };
        let _ = DrawTextW(hdc, &mut wide, &mut r, flags);
        SelectObject(hdc, old);
        let _ = DeleteObject(font);
    }
}

fn draw_grid_item(hdc: HDC, item: &GridItem, scale: i32) {
    let bg = if item.disabled { DISABLED_BG } else { PANEL_2 };
    let fg = if item.disabled { DISABLED_TEXT } else { TEXT };
    fill(hdc, item.rect, bg);
    frame(hdc, item.rect, if item.disabled { GOLD } else { GOLD }, 1);
    text(
        hdc,
        item.rect,
        &item.label,
        (18 * scale / 100).clamp(13, 28),
        fg,
        false,
        true,
    );
}

fn scale_for(_w: i32, h: i32) -> i32 {
    ((h as f32 / 760.0) * 100.0).round().clamp(75.0, 135.0) as i32
}

pub fn paint(hwnd: HWND, editor: &Editor) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let w = client.right - client.left;
        let h = client.bottom - client.top;
        let l = layout(editor, w, h);
        let scale = scale_for(w, h);

        fill(hdc, l.panel, BG);
        frame(hdc, l.panel, GOLD_BRIGHT, 2);
        fill(hdc, Rect::new(2, 2, w - 2, 6), GOLD);

        text(
            hdc,
            l.title,
            &editor.name,
            (28 * scale / 100).clamp(20, 40),
            GOLD_BRIGHT,
            true,
            true,
        );
        text(
            hdc,
            l.category,
            &format!(
                "{}  ·  UID {}  ·  {}",
                editor.category_label(),
                editor.uid,
                editor.source_label()
            ),
            (15 * scale / 100).clamp(12, 22),
            MUTED,
            false,
            true,
        );

        // Mode row
        fill(hdc, l.mode, PANEL);
        frame(hdc, l.mode, GOLD, 1);
        text(
            hdc,
            l.mode,
            editor.mode.label(),
            (20 * scale / 100).clamp(16, 30),
            TEXT,
            true,
            true,
        );
        if editor.mode_is_switchable() {
            text(
                hdc,
                l.mode_left,
                "‹",
                (26 * scale / 100).clamp(20, 38),
                if editor.mode_can_prev() { GOLD_BRIGHT } else { MUTED },
                true,
                true,
            );
            text(
                hdc,
                l.mode_right,
                "›",
                (26 * scale / 100).clamp(20, 38),
                if editor.mode_can_next() { GOLD_BRIGHT } else { MUTED },
                true,
                true,
            );
        }

        // Key display slots
        for (i, rect) in l.slots.iter().copied().enumerate() {
            let label = match editor.mode {
                BindingMode::Single => "键位（点击移除）".to_string(),
                BindingMode::Two if i == 0 => "首键（点击移除）".to_string(),
                BindingMode::Two => "尾键（点击移除）".to_string(),
            };
            fill(hdc, rect, PANEL);
            frame(hdc, rect, GOLD, 1);
            text(
                hdc,
                Rect::new(rect.l + 8, rect.t + 8, rect.r - 8, rect.t + 32),
                &label,
                (15 * scale / 100).clamp(12, 22),
                MUTED,
                false,
                false,
            );
            let value = editor.slots[i]
                .map(|key| key.label().to_string())
                .unwrap_or_else(|| "（点击下区键位填入）".to_string());
            text(
                hdc,
                Rect::new(rect.l + 8, rect.t + 34, rect.r - 8, rect.b - 8),
                &value,
                (21 * scale / 100).clamp(15, 31),
                if editor.slots[i].is_some() { TEXT } else { MUTED },
                true,
                false,
            );
        }

        // Grid
        for item in &l.grid {
            draw_grid_item(hdc, item, scale);
        }
        text(
            hdc,
            l.footnote,
            "点击下方键位填入，点击上方键位移除。重置会删除本技能/忍具的全部触发键位。",
            (13 * scale / 100).clamp(11, 19),
            MUTED,
            false,
            true,
        );

        // Buttons
        fill(hdc, l.save, GREEN);
        frame(hdc, l.save, GOLD, 1);
        text(hdc, l.save, "保存", (20 * scale / 100).clamp(15, 30), TEXT, true, true);
        fill(hdc, l.reset, RESET_BG);
        frame(hdc, l.reset, GOLD, 1);
        text(hdc, l.reset, "重置", (20 * scale / 100).clamp(15, 30), TEXT, true, true);
        fill(hdc, l.cancel, RED);
        frame(hdc, l.cancel, GOLD, 1);
        text(
            hdc,
            l.cancel,
            "取消",
            (20 * scale / 100).clamp(15, 30),
            TEXT,
            true,
            true,
        );

        // Modal
        if let Some(modal_panel) = l.modal_panel {
            if let Some(modal) = &editor.modal {
                fill(hdc, modal_panel, PANEL);
                frame(hdc, modal_panel, GOLD_BRIGHT, 2);
                text(
                    hdc,
                    Rect::new(
                        modal_panel.l + 16,
                        modal_panel.t + 18,
                        modal_panel.r - 16,
                        modal_panel.b - 70,
                    ),
                    &modal.message,
                    (18 * scale / 100).clamp(14, 27),
                    TEXT,
                    false,
                    true,
                );
                if let Some(confirm) = l.modal_confirm {
                    fill(hdc, confirm, GREEN);
                    frame(hdc, confirm, GOLD, 1);
                    text(
                        hdc,
                        confirm,
                        &modal.confirm,
                        (16 * scale / 100).clamp(13, 24),
                        TEXT,
                        true,
                        true,
                    );
                }
                if let Some(cancel) = l.modal_cancel {
                    fill(hdc, cancel, RED);
                    frame(hdc, cancel, GOLD, 1);
                    text(
                        hdc,
                        cancel,
                        modal.cancel.as_deref().unwrap_or("取消"),
                        (16 * scale / 100).clamp(13, 24),
                        TEXT,
                        true,
                        true,
                    );
                }
            }
        }

        let _ = EndPaint(hwnd, &paint);
    }
}
