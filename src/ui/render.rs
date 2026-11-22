use std::{ffi::c_void, mem::size_of, ptr, slice};

use anyhow::{Result, anyhow};
use windows::{
    Win32::{
        Foundation::{BOOL, COLORREF, RECT},
        Graphics::{
            Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, Fxc::D3DCompile, ID3DBlob},
            Direct3D11::{
                D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC, D3D11_COMPARISON_ALWAYS,
                D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE, D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ZERO,
                D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE,
                D3D11_RASTERIZER_DESC, D3D11_SAMPLER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE_ADDRESS_CLAMP,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT, ID3D11BlendState,
                ID3D11Buffer, ID3D11DepthStencilState, ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext,
                ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView,
                ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
            },
            Dxgi::{Common::DXGI_FORMAT_B8G8R8A8_UNORM, IDXGISwapChain},
            Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
                CreateDIBSection, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS,
                DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteDC, DeleteObject,
                DrawTextW, FillRect, GdiFlush, HBITMAP, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode,
                SetTextColor, TRANSPARENT,
            },
        },
    },
    core::{Interface, PCWSTR, s},
};

use super::editor::{BindingMode, Editor, GridItem, Rect, layout};

const UI_TEXTURE_WIDTH_LIMIT: u32 = 8192;

struct DibSurface {
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

struct DeviceResources {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    rasterizer: ID3D11RasterizerState,
    depth_stencil: ID3D11DepthStencilState,
    sampler: ID3D11SamplerState,
    constant_buffer: ID3D11Buffer,
    texture: Option<ID3D11Texture2D>,
    texture_view: Option<ID3D11ShaderResourceView>,
    texture_size: Option<(u32, u32)>,
}

#[repr(C)]
struct ShaderParams {
    panel: [f32; 4],
    screen: [f32; 4],
}

struct SavedState {
    render_target: Option<ID3D11RenderTargetView>,
    depth_stencil_view: Option<ID3D11DepthStencilView>,
    viewport: D3D11_VIEWPORT,
    viewport_count: u32,
    blend: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,
    depth_stencil: Option<ID3D11DepthStencilState>,
    stencil_ref: u32,
    rasterizer: Option<ID3D11RasterizerState>,
    topology: windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY,
    input_layout: Option<ID3D11InputLayout>,
    vertex_shader: Option<ID3D11VertexShader>,
    pixel_shader: Option<ID3D11PixelShader>,
    shader_resource: Option<ID3D11ShaderResourceView>,
    sampler: Option<ID3D11SamplerState>,
    constant_buffer: Option<ID3D11Buffer>,
}

/// Software-rendered UI texture plus a tiny D3D11 textured-quad renderer.
/// GDI is only used to paint into a CPU DIB; the game's own D3D11 device is
/// used for the final overlay draw, so this path also works when the game's
/// device was not created with BGRA support (which rules out Direct2D).
pub struct Renderer {
    dib: Option<DibSurface>,
    resources: Option<DeviceResources>,
}

unsafe impl Send for Renderer {}

impl Renderer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            dib: None,
            resources: None,
        })
    }

    pub fn paint(
        &mut self,
        swapchain: &IDXGISwapChain,
        editor: &Editor,
        frame_width: u32,
        frame_height: u32,
        redraw_ui: bool,
    ) -> Result<()> {
        if frame_width == 0 || frame_height == 0 {
            return Ok(());
        }

        let (panel_w, panel_h, panel_x, panel_y) = panel_geometry(frame_width as i32, frame_height as i32);
        if panel_w <= 0 || panel_h <= 0 {
            return Ok(());
        }

        let size_changed = self
            .dib
            .as_ref()
            .is_none_or(|dib| dib.width != panel_w || dib.height != panel_h);
        if redraw_ui || size_changed {
            self.draw_ui(editor, panel_w, panel_h)?;
        }

        let device: ID3D11Device =
            unsafe { swapchain.GetDevice() }.map_err(|error| anyhow!("IDXGISwapChain::GetDevice failed: {error}"))?;
        let device_changed = self
            .resources
            .as_ref()
            .is_none_or(|resources| Interface::as_raw(&resources.device) != Interface::as_raw(&device));
        if device_changed {
            self.resources = Some(DeviceResources::new(device)?);
        }

        let resources = self
            .resources
            .as_mut()
            .ok_or_else(|| anyhow!("D3D11 device resources are unavailable"))?;
        let size = (panel_w as u32, panel_h as u32);
        if redraw_ui || size_changed || resources.texture_size != Some(size) {
            let dib = self.dib.as_ref().ok_or_else(|| anyhow!("UI DIB is unavailable"))?;
            resources.update_texture(dib, size)?;
        }

        resources.draw(
            swapchain,
            panel_x,
            panel_y,
            panel_w as f32,
            panel_h as f32,
            frame_width as f32,
            frame_height as f32,
        )
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
            editor.mode.label(),
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
                false,
            );
            let value = editor.slots[i]
                .map(|key| key.label().to_string())
                .unwrap_or_else(|| "（点击下方键位填入）".to_string());
            text(
                hdc,
                Rect::new(rect.l + 8, rect.t + 34, rect.r - 8, rect.b - 8),
                &value,
                (21 * scale / 100).clamp(15, 31),
                if editor.slots[i].is_some() { TEXT } else { MUTED },
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
            "点击下方键位填入，点击上方键位移除。重置会删除本技能/忍具的全部触发键位。",
            (13 * scale / 100).clamp(11, 19),
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
            "保存",
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
            "重置",
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
            "取消",
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
                        modal_panel.t + 18,
                        modal_panel.r - 16,
                        modal_panel.b - 70,
                    ),
                    &modal.message,
                    (18 * scale / 100).clamp(14, 27),
                    TEXT,
                    false,
                    true,
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
                        false,
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
                        false,
                    );
                }
            }
        }

        let _ = unsafe { GdiFlush() };
        Ok(())
    }
}

impl DeviceResources {
    fn new(device: ID3D11Device) -> Result<Self> {
        let context =
            unsafe { device.GetImmediateContext() }.map_err(|error| anyhow!("GetImmediateContext failed: {error}"))?;

        let (vs_bytecode, ps_bytecode) = unsafe { compile_shaders() }?;
        let mut vertex_shader = None;
        unsafe {
            device
                .CreateVertexShader(&vs_bytecode, None, Some(&mut vertex_shader))
                .map_err(|error| anyhow!("CreateVertexShader failed: {error}"))?;
        }
        let mut pixel_shader = None;
        unsafe {
            device
                .CreatePixelShader(&ps_bytecode, None, Some(&mut pixel_shader))
                .map_err(|error| anyhow!("CreatePixelShader failed: {error}"))?;
        }

        let rasterizer_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: BOOL(0),
            DepthClipEnable: BOOL(0),
            ScissorEnable: BOOL(0),
            MultisampleEnable: BOOL(0),
            AntialiasedLineEnable: BOOL(0),
            ..Default::default()
        };
        let mut rasterizer = None;
        unsafe {
            device
                .CreateRasterizerState(&rasterizer_desc, Some(&mut rasterizer))
                .map_err(|error| anyhow!("CreateRasterizerState failed: {error}"))?;
        }

        let depth_desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: BOOL(0),
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
            DepthFunc: D3D11_COMPARISON_ALWAYS,
            StencilEnable: BOOL(0),
            ..Default::default()
        };
        let mut depth_stencil = None;
        unsafe {
            device
                .CreateDepthStencilState(&depth_desc, Some(&mut depth_stencil))
                .map_err(|error| anyhow!("CreateDepthStencilState failed: {error}"))?;
        }

        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            ComparisonFunc: D3D11_COMPARISON_ALWAYS,
            MinLOD: 0.0,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        unsafe {
            device
                .CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .map_err(|error| anyhow!("CreateSamplerState failed: {error}"))?;
        }

        let constant_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<ShaderParams>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut constant_buffer = None;
        unsafe {
            device
                .CreateBuffer(&constant_desc, None, Some(&mut constant_buffer))
                .map_err(|error| anyhow!("CreateBuffer failed: {error}"))?;
        }

        Ok(Self {
            device,
            context,
            vertex_shader: vertex_shader.ok_or_else(|| anyhow!("vertex shader was not created"))?,
            pixel_shader: pixel_shader.ok_or_else(|| anyhow!("pixel shader was not created"))?,
            rasterizer: rasterizer.ok_or_else(|| anyhow!("rasterizer state was not created"))?,
            depth_stencil: depth_stencil.ok_or_else(|| anyhow!("depth-stencil state was not created"))?,
            sampler: sampler.ok_or_else(|| anyhow!("sampler state was not created"))?,
            constant_buffer: constant_buffer.ok_or_else(|| anyhow!("constant buffer was not created"))?,
            texture: None,
            texture_view: None,
            texture_size: None,
        })
    }

    fn update_texture(&mut self, dib: &DibSurface, size: (u32, u32)) -> Result<()> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.0,
            Height: size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: dib.bits as *const c_void,
            SysMemPitch: dib.pitch,
            SysMemSlicePitch: dib.pitch * dib.height as u32,
        };
        let mut texture = None;
        unsafe {
            self.device
                .CreateTexture2D(&desc, Some(&data), Some(&mut texture))
                .map_err(|error| anyhow!("CreateTexture2D failed: {error}"))?;
        }
        let texture = texture.ok_or_else(|| anyhow!("CreateTexture2D returned no texture"))?;
        let mut view = None;
        unsafe {
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))
                .map_err(|error| anyhow!("CreateShaderResourceView failed: {error}"))?;
        }
        self.texture = Some(texture);
        self.texture_view = Some(view.ok_or_else(|| anyhow!("CreateShaderResourceView returned no view"))?);
        self.texture_size = Some(size);
        Ok(())
    }

    fn draw(
        &self,
        swapchain: &IDXGISwapChain,
        panel_x: f32,
        panel_y: f32,
        panel_w: f32,
        panel_h: f32,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<()> {
        let back_buffer: ID3D11Texture2D =
            unsafe { swapchain.GetBuffer(0) }.map_err(|error| anyhow!("GetBuffer(0) failed: {error}"))?;
        let mut render_target = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&back_buffer, None, Some(&mut render_target))
                .map_err(|error| anyhow!("CreateRenderTargetView failed: {error}"))?;
        }
        let render_target = render_target.ok_or_else(|| anyhow!("CreateRenderTargetView returned no view"))?;
        let texture_view = self
            .texture_view
            .as_ref()
            .ok_or_else(|| anyhow!("UI texture view is unavailable"))?;

        let saved = self.save_state();
        let result = (|| -> Result<()> {
            unsafe {
                self.context
                    .OMSetRenderTargets(Some(&[Some(render_target.clone())]), None::<&ID3D11DepthStencilView>);
                self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: screen_w,
                    Height: screen_h,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));
                self.context.OMSetBlendState(None::<&ID3D11BlendState>, None, u32::MAX);
                self.context.OMSetDepthStencilState(&self.depth_stencil, 0);
                self.context.RSSetState(&self.rasterizer);
                self.context.IASetInputLayout(None::<&ID3D11InputLayout>);
                self.context
                    .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
                self.context.VSSetShader(&self.vertex_shader, None);
                self.context.PSSetShader(&self.pixel_shader, None);

                let params = ShaderParams {
                    panel: [panel_x, panel_y, panel_w, panel_h],
                    screen: [screen_w, screen_h, 0.0, 0.0],
                };
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context
                    .Map(&self.constant_buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                    .map_err(|error| anyhow!("Map constant buffer failed: {error}"))?;
                ptr::copy_nonoverlapping(&params, mapped.pData as *mut ShaderParams, 1);
                self.context.Unmap(&self.constant_buffer, 0);
                self.context
                    .VSSetConstantBuffers(0, Some(&[Some(self.constant_buffer.clone())]));
                self.context
                    .PSSetShaderResources(0, Some(&[Some(texture_view.clone())]));
                self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
                self.context.Draw(4, 0);
            }
            Ok(())
        })();

        self.restore_state(saved);
        result
    }

    fn save_state(&self) -> SavedState {
        unsafe {
            let mut render_targets = [None];
            let mut depth_stencil_view = None;
            self.context
                .OMGetRenderTargets(Some(&mut render_targets), Some(&mut depth_stencil_view as *mut _));

            let mut viewport = D3D11_VIEWPORT::default();
            let mut viewport_count = 1;
            self.context.RSGetViewports(&mut viewport_count, Some(&mut viewport));

            let mut blend = None;
            let mut blend_factor = [0.0f32; 4];
            let mut sample_mask = 0u32;
            self.context.OMGetBlendState(
                Some(&mut blend as *mut _),
                Some(&mut blend_factor),
                Some(&mut sample_mask as *mut _),
            );

            let mut depth_stencil = None;
            let mut stencil_ref = 0u32;
            self.context
                .OMGetDepthStencilState(Some(&mut depth_stencil as *mut _), Some(&mut stencil_ref as *mut _));

            let rasterizer = self.context.RSGetState().ok();
            let topology = self.context.IAGetPrimitiveTopology();
            let input_layout = self.context.IAGetInputLayout().ok();

            let mut vertex_shader = None;
            self.context.VSGetShader(&mut vertex_shader, None, None);
            let mut pixel_shader = None;
            self.context.PSGetShader(&mut pixel_shader, None, None);

            let mut shader_resource = [None];
            self.context.PSGetShaderResources(0, Some(&mut shader_resource));
            let mut sampler = [None];
            self.context.PSGetSamplers(0, Some(&mut sampler));
            let mut constant_buffer = [None];
            self.context.VSGetConstantBuffers(0, Some(&mut constant_buffer));

            SavedState {
                render_target: render_targets[0].take(),
                depth_stencil_view,
                viewport,
                viewport_count,
                blend,
                blend_factor,
                sample_mask,
                depth_stencil,
                stencil_ref,
                rasterizer,
                topology,
                input_layout,
                vertex_shader,
                pixel_shader,
                shader_resource: shader_resource[0].take(),
                sampler: sampler[0].take(),
                constant_buffer: constant_buffer[0].take(),
            }
        }
    }

    fn restore_state(&self, saved: SavedState) {
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[saved.render_target]), saved.depth_stencil_view.as_ref());
            if saved.viewport_count > 0 {
                self.context.RSSetViewports(Some(&[saved.viewport]));
            }
            self.context
                .OMSetBlendState(saved.blend.as_ref(), Some(&saved.blend_factor), saved.sample_mask);
            self.context
                .OMSetDepthStencilState(saved.depth_stencil.as_ref(), saved.stencil_ref);
            self.context.RSSetState(saved.rasterizer.as_ref());
            self.context.IASetPrimitiveTopology(saved.topology);
            self.context.IASetInputLayout(saved.input_layout.as_ref());
            self.context.VSSetShader(saved.vertex_shader.as_ref(), None);
            self.context.PSSetShader(saved.pixel_shader.as_ref(), None);
            self.context.PSSetShaderResources(0, Some(&[saved.shader_resource]));
            self.context.PSSetSamplers(0, Some(&[saved.sampler]));
            self.context.VSSetConstantBuffers(0, Some(&[saved.constant_buffer]));
        }
    }
}

unsafe fn compile_shaders() -> Result<(Vec<u8>, Vec<u8>)> {
    const VS_SOURCE: &str = r#"
cbuffer Params : register(b0)
{
    float4 panel;
    float4 screen;
};

struct VSOut
{
    float4 pos : SV_POSITION;
    float2 uv : TEXCOORD0;
};

VSOut vs_main(uint id : SV_VertexID)
{
    float2 uv = float2((id == 1 || id == 3) ? 1.0 : 0.0,
                       (id >= 2) ? 1.0 : 0.0);
    float2 pixel = float2(panel.x + uv.x * panel.z,
                          panel.y + uv.y * panel.w);
    VSOut output;
    output.pos = float4(pixel.x / screen.x * 2.0 - 1.0,
                        1.0 - pixel.y / screen.y * 2.0,
                        0.0, 1.0);
    output.uv = uv;
    return output;
}
"#;

    const PS_SOURCE: &str = r#"
Texture2D ui_texture : register(t0);
SamplerState ui_sampler : register(s0);

struct VSOut
{
    float4 pos : SV_POSITION;
    float2 uv : TEXCOORD0;
};

float4 ps_main(VSOut input) : SV_TARGET
{
    return ui_texture.Sample(ui_sampler, input.uv);
}
"#;

    let vs = unsafe { compile_stage(VS_SOURCE, s!("vs_main"), s!("vs_4_0")) }?;
    let ps = unsafe { compile_stage(PS_SOURCE, s!("ps_main"), s!("ps_4_0")) }?;
    Ok((vs, ps))
}

unsafe fn compile_stage(source: &str, entry: windows::core::PCSTR, target: windows::core::PCSTR) -> Result<Vec<u8>> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    unsafe {
        D3DCompile(
            source.as_ptr() as *const c_void,
            source.len(),
            s!("battle_instinct_overlay.hlsl"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    }
    .map_err(|error| {
        let detail = errors.as_ref().map(blob_text).unwrap_or_else(|| format!("{error}"));
        anyhow!("D3DCompile failed: {detail}")
    })?;

    let code = code.ok_or_else(|| anyhow!("D3DCompile returned no bytecode"))?;
    let bytes = unsafe { slice::from_raw_parts(code.GetBufferPointer() as *const u8, code.GetBufferSize()) };
    Ok(bytes.to_vec())
}

fn blob_text(blob: &ID3DBlob) -> String {
    let size = unsafe { blob.GetBufferSize() };
    let ptr = unsafe { blob.GetBufferPointer() as *const u8 };
    if ptr.is_null() || size == 0 {
        return String::new();
    }
    String::from_utf8_lossy(unsafe { slice::from_raw_parts(ptr, size) }).into_owned()
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

pub(crate) fn panel_geometry(frame_w: i32, frame_h: i32) -> (i32, i32, f32, f32) {
    let margin = 24;
    let max_w = (frame_w - margin).max(1);
    let max_h = (frame_h - margin).max(1);
    let w = ((frame_w as f32 * 0.43).round() as i32).clamp(560.min(max_w), max_w);
    let h = ((frame_h as f32 * 0.74).round() as i32).clamp(620.min(max_h), max_h);
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
