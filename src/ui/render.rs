//! GDI panel painter and layout helpers.
//!
//! All D3D11 work lives in `overlay_render.rs`. This module only knows how to
//! paint the editor panel into a CPU DIB, so it can run on a worker thread
//! without ever touching the game's immediate context.

use std::{mem::size_of, ptr};

use anyhow::{Result, anyhow};
use windows::Win32::{
    Foundation::{COLORREF, RECT},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
        CreateDIBSection, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER,
        DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, FillRect,
        GdiFlush, HBITMAP, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
};
use windows::core::PCWSTR;

use super::{
    editor::{BindingMode, Editor, GridItem, ModalButtonStyle, Rect, layout},
    i18n,
};

const UI_TEXTURE_WIDTH_LIMIT: u32 = 8192;

pub(crate) struct DibSurface {
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
    pitch: u32,
}

impl DibSurface {
    fn new(width: i32, height: i32) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.0.is_null() {
                return Err(anyhow!("CreateCompatibleDC failed"));
            }

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = ptr::null_mut();
            let bitmap = match CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(anyhow!("CreateDIBSection failed: {error}"));
                }
            };
            let old_bitmap = SelectObject(dc, bitmap);
            if bits.is_null() {
                let _ = SelectObject(dc, old_bitmap);
                let _ = DeleteObject(bitmap);
                let _ = DeleteDC(dc);
                return Err(anyhow!("CreateDIBSection returned a null pixel buffer"));
            }
            Ok(Self {
                dc,
                bitmap,
                old_bitmap,
                bits: bits as *mut u8,
                width,
                height,
                pitch: (width * 4) as u32,
            })
        }
    }
}

impl Drop for DibSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.dc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.dc);
        }
    }
}

/// CPU-only GDI painter for the overlay. Owned exclusively by the paint
/// worker thread; it never touches D3D11.
pub(crate) struct UiPainter {
    dib: Option<DibSurface>,
}

unsafe impl Send for UiPainter {}

impl UiPainter {
    pub(crate) fn new() -> Self {
        Self { dib: None }
    }

    pub(crate) fn paint(&mut self, editor: &Editor, width: i32, height: i32) -> Result<()> {
        self.draw_ui(editor, width, height)
    }

    /// Copy the completed panel into a packed BGRA buffer for the Present
    /// thread. This is CPU-only; the worker never touches D3D.
    pub(crate) fn copy_pixels(&self) -> Option<(Vec<u8>, i32, i32)> {
        let dib = self.dib.as_ref()?;
        let row_bytes = dib.pitch as usize;
        let len = row_bytes * dib.height as usize;
        let mut pixels = vec![0u8; len];
        unsafe {
            std::ptr::copy_nonoverlapping(dib.bits as *const u8, pixels.as_mut_ptr(), len);
        }
        Some((pixels, dib.width, dib.height))
    }

    fn draw_ui(&mut self, editor: &Editor, width: i32, height: i32) -> Result<()> {
        if width <= 0 || height <= 0 || width as u32 > UI_TEXTURE_WIDTH_LIMIT {
            return Ok(());
        }
        if self
            .dib
            .as_ref()
            .is_none_or(|dib| dib.width != width || dib.height != height)
        {
            self.dib = Some(DibSurface::new(width, height)?);
        }
        let dib = self.dib.as_ref().ok_or_else(|| anyhow!("UI DIB is unavailable"))?;
        let hdc = dib.dc;
        let l = layout(editor, width, height);
        let scale = scale_for(width, height);

        fill(hdc, l.panel, BG);
        frame(hdc, l.panel, GOLD_BRIGHT, 2);
        fill(hdc, Rect::new(2, 2, width - 2, 6), GOLD);

        text(
            hdc,
            l.title,
            &editor.name,
            (28 * scale / 100).clamp(20, 40),
            GOLD_BRIGHT,
            true,
            true,
            false,
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
            false,
        );

        fill(hdc, l.mode, PANEL);
        frame(hdc, l.mode, GOLD, 1);
        text(
            hdc,
            l.mode,
            editor.mode().label(),
            (20 * scale / 100).clamp(16, 30),
            TEXT,
            true,
            true,
            false,
        );
        if editor.mode_is_switchable() {
            text(
                hdc,
                l.mode_left,
                "<",
                (26 * scale / 100).clamp(20, 38),
                if editor.mode_can_prev() { GOLD_BRIGHT } else { MUTED },
                true,
                true,
                false,
            );
            text(
                hdc,
                l.mode_right,
                ">",
                (26 * scale / 100).clamp(20, 38),
                if editor.mode_can_next() { GOLD_BRIGHT } else { MUTED },
                true,
                true,
                false,
            );
        }

        text(
            hdc,
            l.scheme_title,
            i18n::pick("搓招方案", "Key schemes"),
            (16 * scale / 100).clamp(12, 23),
            MUTED,
            false,
            false,
            false,
        );
        for (index, rect) in l.scheme_buttons.iter().copied().enumerate() {
            let selected = index == editor.selected_index;
            fill(hdc, rect, if selected { PANEL_2 } else { PANEL });
            frame(
                hdc,
                rect,
                if selected { GOLD_BRIGHT } else { GOLD },
                if selected { 2 } else { 1 },
            );
            text(
                hdc,
                rect,
                &editor.scheme_label(index),
                (15 * scale / 100).clamp(12, 22),
                if selected { TEXT } else { MUTED },
                selected,
                true,
                false,
            );
        }

        fill(hdc, l.add_scheme, GREEN);
        frame(hdc, l.add_scheme, GOLD, 1);
        text(
            hdc,
            l.add_scheme,
            i18n::pick("添加方案", "Add scheme"),
            (15 * scale / 100).clamp(12, 22),
            TEXT,
            true,
            true,
            false,
        );
        fill(hdc, l.delete_scheme, RED);
        frame(hdc, l.delete_scheme, GOLD, 1);
        text(
            hdc,
            l.delete_scheme,
            i18n::pick("删除选中方案", "Delete selected"),
            (15 * scale / 100).clamp(12, 22),
            TEXT,
            true,
            true,
            false,
        );

        let mode = editor.mode();
        let slots = editor.slots();
        for (i, rect) in l.slots.iter().copied().enumerate() {
            let label = match (mode, i) {
                (BindingMode::Single, _) => i18n::pick("键位（点击移除）", "Key (click to remove)").to_string(),
                (BindingMode::Two | BindingMode::Three, 0) => {
                    i18n::pick("首键（点击移除）", "First key (click to remove)").to_string()
                }
                (BindingMode::Three, 1) => i18n::pick("中键（点击移除）", "Middle key (click to remove)").to_string(),
                (BindingMode::Two, 1) | (BindingMode::Three, 2) => {
                    i18n::pick("尾键（点击移除）", "Tail key (click to remove)").to_string()
                }
                (BindingMode::Three, _) => i18n::pick("尾键（点击移除）", "Tail key (click to remove)").to_string(),
                (BindingMode::Two, _) => i18n::pick("尾键（点击移除）", "Tail key (click to remove)").to_string(),
            };
            fill(hdc, rect, PANEL);
            frame(hdc, rect, GOLD, 1);
            text(
                hdc,
                Rect::new(rect.l + 8, rect.t + 8, rect.r - 8, rect.t + 30),
                &label,
                (14 * scale / 100).clamp(11, 21),
                MUTED,
                false,
                false,
                false,
            );
            let value = slots
                .get(i)
                .copied()
                .flatten()
                .map(|key| i18n::key_label(key).to_string())
                .unwrap_or_else(|| i18n::pick("（点击下方键位填入）", "(Click a key below)").to_string());
            text(
                hdc,
                Rect::new(rect.l + 8, rect.t + 32, rect.r - 8, rect.b - 6),
                &value,
                (19 * scale / 100).clamp(14, 29),
                if slots.get(i).copied().flatten().is_some() {
                    TEXT
                } else {
                    MUTED
                },
                true,
                false,
                false,
            );
        }

        for item in &l.grid {
            draw_grid_item(hdc, item, scale);
        }
        text(
            hdc,
            l.footnote,
            i18n::pick(
                "点击方案切换；添加/删除只修改本技能方案，保存后全部生效。重置会清空全部触发键位。",
                "Click a scheme to select it. Add/delete edits this skill only; save applies all schemes. Reset clears every trigger.",
            ),
            (12 * scale / 100).clamp(10, 18),
            MUTED,
            false,
            true,
            false,
        );

        fill(hdc, l.save, GREEN);
        frame(hdc, l.save, GOLD, 1);
        text(
            hdc,
            l.save,
            i18n::pick("保存", "Save"),
            (20 * scale / 100).clamp(15, 30),
            TEXT,
            true,
            true,
            false,
        );
        fill(hdc, l.reset, RESET_BG);
        frame(hdc, l.reset, GOLD, 1);
        text(
            hdc,
            l.reset,
            i18n::pick("重置", "Reset"),
            (20 * scale / 100).clamp(15, 30),
            TEXT,
            true,
            true,
            false,
        );
        fill(hdc, l.cancel, RED);
        frame(hdc, l.cancel, GOLD, 1);
        text(
            hdc,
            l.cancel,
            i18n::pick("取消", "Cancel"),
            (20 * scale / 100).clamp(15, 30),
            TEXT,
            true,
            true,
            false,
        );

        if let Some(modal_panel) = l.modal_panel {
            if let Some(modal) = &editor.modal {
                fill(hdc, modal_panel, PANEL);
                frame(hdc, modal_panel, GOLD_BRIGHT, 2);
                text(
                    hdc,
                    Rect::new(
                        modal_panel.l + 16,
                        modal_panel.t + 16,
                        modal_panel.r - 16,
                        modal_panel.b - 68,
                    ),
                    &modal.message,
                    (18 * scale / 100).clamp(14, 27),
                    TEXT,
                    false,
                    true,
                    true,
                );
                for (index, rect) in l.modal_buttons.iter().copied().enumerate() {
                    let Some(button) = modal.buttons.get(index) else {
                        continue;
                    };
                    let bg = match button.style {
                        ModalButtonStyle::Primary => GREEN,
                        ModalButtonStyle::Danger => RED,
                        ModalButtonStyle::Neutral => PANEL_2,
                    };
                    fill(hdc, rect, bg);
                    frame(hdc, rect, GOLD, 1);
                    text(
                        hdc,
                        rect,
                        &button.label,
                        (16 * scale / 100).clamp(13, 24),
                        TEXT,
                        true,
                        true,
                        false,
                    );
                }
            }
        }

        let _ = unsafe { GdiFlush() };
        Ok(())
    }
}

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

fn text(hdc: HDC, rect: Rect, value: &str, size: i32, color: COLORREF, bold: bool, center: bool, wrap: bool) {
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
        let align = if center { DT_CENTER } else { DT_LEFT };
        let flags = if wrap {
            align | DT_WORDBREAK
        } else {
            align | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS
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
    frame(hdc, item.rect, GOLD, 1);
    text(
        hdc,
        item.rect,
        &item.label,
        (18 * scale / 100).clamp(13, 28),
        fg,
        false,
        true,
        false,
    );
}

fn scale_for(_w: i32, h: i32) -> i32 {
    ((h as f32 / 760.0) * 100.0).round().clamp(75.0, 135.0) as i32
}

pub(crate) fn panel_geometry(editor: &Editor, frame_w: i32, frame_h: i32) -> (i32, i32, f32, f32) {
    let margin = 24;
    let max_w = (frame_w - margin).max(1);
    let max_h = (frame_h - margin).max(1);
    let w = ((frame_w as f32 * 0.43).round() as i32).clamp(560.min(max_w), max_w);
    let scheme_rows = editor.schemes.len().div_ceil(2).max(1) as i32;
    let extra_scheme_height = (scheme_rows - 1) * 38;
    let base_h = (frame_h as f32 * 0.74).round() as i32;
    let h = (base_h + extra_scheme_height).clamp(620.min(max_h), max_h);
    let desired_x = frame_w - w - (frame_w as f32 * 0.015).round() as i32;
    let x = desired_x.clamp(0, (frame_w - w).max(0));
    let desired_y = (frame_h as f32 * 0.145).round() as i32;
    let y = desired_y.clamp(0, (frame_h - h).max(0));
    (w, h, x as f32, y as f32)
}

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
