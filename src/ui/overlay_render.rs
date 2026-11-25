//! Minimal Present-thread overlay renderer.
//!
//! Deliberately mirrors the fast path used by lightweight ImGui overlays:
//! all D3D11 work happens on the Present thread, there is no worker-thread
//! context access, no ID3D11Multithread protection and no full pipeline
//! save/restore. The cached UI texture is uploaded only when the CPU painter
//! publishes a new generation.

use std::{ffi::c_void, mem::size_of, ptr, slice};

use anyhow::{Result, anyhow};
use windows::{
    Win32::{
        Foundation::BOOL,
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
        },
    },
    core::{Interface, PCSTR, s},
};

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct ShaderParams {
    panel: [f32; 4],
    screen: [f32; 4],
}

/// Snapshot produced by the CPU paint worker. `pixels` is tightly packed BGRA.
pub(crate) struct PaintFrame {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub panel_x: f32,
    pub panel_y: f32,
    pub panel_w: f32,
    pub panel_h: f32,
    pub pixels: Vec<u8>,
}

unsafe impl Send for OverlayRenderer {}

pub(crate) struct OverlayRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    rasterizer: ID3D11RasterizerState,
    depth_stencil: ID3D11DepthStencilState,
    sampler: ID3D11SamplerState,
    constant_buffer: ID3D11Buffer,
    solid_view: ID3D11ShaderResourceView,
    texture: Option<ID3D11Texture2D>,
    texture_view: Option<ID3D11ShaderResourceView>,
    texture_size: Option<(u32, u32)>,
    render_target: Option<ID3D11RenderTargetView>,
    render_target_swapchain: *mut c_void,
    render_target_size: (u32, u32),
}

impl OverlayRenderer {
    pub(crate) fn new(swapchain: &IDXGISwapChain) -> Result<Self> {
        let device: ID3D11Device =
            unsafe { swapchain.GetDevice() }.map_err(|error| anyhow!("IDXGISwapChain::GetDevice failed: {error}"))?;
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

        let solid_view = create_solid_view(&device)?;

        Ok(Self {
            device,
            context,
            vertex_shader: vertex_shader.ok_or_else(|| anyhow!("vertex shader was not created"))?,
            pixel_shader: pixel_shader.ok_or_else(|| anyhow!("pixel shader was not created"))?,
            rasterizer: rasterizer.ok_or_else(|| anyhow!("rasterizer state was not created"))?,
            depth_stencil: depth_stencil.ok_or_else(|| anyhow!("depth-stencil state was not created"))?,
            sampler: sampler.ok_or_else(|| anyhow!("sampler state was not created"))?,
            constant_buffer: constant_buffer.ok_or_else(|| anyhow!("constant buffer was not created"))?,
            solid_view,
            texture: None,
            texture_view: None,
            texture_size: None,
            render_target: None,
            render_target_swapchain: ptr::null_mut(),
            render_target_size: (0, 0),
        })
    }

    /// Upload a new UI snapshot. Present thread only; no other thread touches
    /// the immediate context.
    pub(crate) fn update_texture(&mut self, frame: &PaintFrame) -> Result<()> {
        let size = (frame.width, frame.height);
        if self.texture.is_none() || self.texture_size != Some(size) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: size.0,
                Height: size.1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
            };
            let mut texture = None;
            unsafe {
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
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
        }

        let texture = self
            .texture
            .as_ref()
            .ok_or_else(|| anyhow!("UI texture is unavailable"))?
            .clone();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&texture, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .map_err(|error| anyhow!("Map UI texture failed: {error}"))?;
        }
        let src_pitch = frame.width as usize * 4;
        for row in 0..frame.height as usize {
            unsafe {
                ptr::copy_nonoverlapping(
                    frame.pixels.as_ptr().add(row * src_pitch),
                    (mapped.pData as *mut u8).add(row * mapped.RowPitch as usize),
                    src_pitch,
                );
            }
        }
        unsafe { self.context.Unmap(&texture, 0) };
        Ok(())
    }

    pub(crate) fn draw(
        &mut self,
        swapchain: &IDXGISwapChain,
        panel_x: f32,
        panel_y: f32,
        panel_w: f32,
        panel_h: f32,
        screen_w: u32,
        screen_h: u32,
        cursor: Option<(f32, f32)>,
    ) -> Result<()> {
        self.ensure_render_target(swapchain, screen_w, screen_h)?;
        let render_target = self
            .render_target
            .as_ref()
            .ok_or_else(|| anyhow!("render target view is unavailable"))?;
        let panel_view = self
            .texture_view
            .as_ref()
            .ok_or_else(|| anyhow!("UI texture view is unavailable"))?
            .clone();
        let solid_view = self.solid_view.clone();

        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(render_target.clone())]), None::<&ID3D11DepthStencilView>);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: screen_w as f32,
                Height: screen_h as f32,
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
        }

        self.draw_quad(
            &panel_view,
            panel_x,
            panel_y,
            panel_w,
            panel_h,
            screen_w as f32,
            screen_h as f32,
        )?;

        if let Some((x, y)) = cursor {
            let bar = 7.0f32;
            let thickness = 2.0f32;
            self.draw_quad(
                &solid_view,
                x - bar,
                y - thickness * 0.5,
                bar * 2.0,
                thickness,
                screen_w as f32,
                screen_h as f32,
            )?;
            self.draw_quad(
                &solid_view,
                x - thickness * 0.5,
                y - bar,
                thickness,
                bar * 2.0,
                screen_w as f32,
                screen_h as f32,
            )?;
        }
        Ok(())
    }

    fn ensure_render_target(&mut self, swapchain: &IDXGISwapChain, width: u32, height: u32) -> Result<()> {
        let raw = Interface::as_raw(swapchain);
        let size = (width.max(1), height.max(1));
        if self.render_target.is_none() || self.render_target_swapchain != raw || self.render_target_size != size {
            let back_buffer: ID3D11Texture2D =
                unsafe { swapchain.GetBuffer(0) }.map_err(|error| anyhow!("GetBuffer(0) failed: {error}"))?;
            let mut render_target = None;
            unsafe {
                self.device
                    .CreateRenderTargetView(&back_buffer, None, Some(&mut render_target))
                    .map_err(|error| anyhow!("CreateRenderTargetView failed: {error}"))?;
            }
            self.render_target = Some(render_target.ok_or_else(|| anyhow!("CreateRenderTargetView returned no view"))?);
            self.render_target_swapchain = raw;
            self.render_target_size = size;
        }
        Ok(())
    }

    fn draw_quad(
        &mut self,
        view: &ID3D11ShaderResourceView,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        screen_w: f32,
        screen_h: f32,
    ) -> Result<()> {
        let params = ShaderParams {
            panel: [x, y, w, h],
            screen: [screen_w, screen_h, 0.0, 0.0],
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&self.constant_buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .map_err(|error| anyhow!("Map constant buffer failed: {error}"))?;
            ptr::copy_nonoverlapping(&params, mapped.pData as *mut ShaderParams, 1);
            self.context.Unmap(&self.constant_buffer, 0);
            self.context
                .VSSetConstantBuffers(0, Some(&[Some(self.constant_buffer.clone())]));
            self.context.PSSetShaderResources(0, Some(&[Some(view.clone())]));
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context.Draw(4, 0);
        }
        Ok(())
    }
}

fn create_solid_view(device: &ID3D11Device) -> Result<ID3D11ShaderResourceView> {
    let pixel: [u8; 4] = [0, 255, 255, 255]; // BGRA yellow software cursor
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 1,
        Height: 1,
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
        pSysMem: pixel.as_ptr() as *const c_void,
        SysMemPitch: 4,
        SysMemSlicePitch: 4,
    };
    let mut texture = None;
    unsafe {
        device
            .CreateTexture2D(&desc, Some(&data), Some(&mut texture))
            .map_err(|error| anyhow!("CreateTexture2D solid failed: {error}"))?;
    }
    let texture = texture.ok_or_else(|| anyhow!("solid texture was not created"))?;
    let mut view = None;
    unsafe {
        device
            .CreateShaderResourceView(&texture, None, Some(&mut view))
            .map_err(|error| anyhow!("CreateShaderResourceView solid failed: {error}"))?;
    }
    view.ok_or_else(|| anyhow!("solid shader resource view was not created"))
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

unsafe fn compile_stage(source: &str, entry: PCSTR, target: PCSTR) -> Result<Vec<u8>> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    let result = unsafe {
        D3DCompile(
            source.as_ptr() as *const c_void,
            source.len(),
            s!("paper_jutsu_overlay.hlsl"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    result.map_err(|error| {
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
