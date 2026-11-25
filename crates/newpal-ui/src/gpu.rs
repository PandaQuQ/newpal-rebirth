//! GPU side of the UI layer: vello renders the vector scene (compute) into an RGBA8 storage texture at the
//! target's physical size; a tiny blit pass then alpha-composites it over the game frame (sRGB-correct).
use vello::peniko::Color;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

const BLIT: &str = r#"
@group(0) @binding(0) var ui: texture_2d<f32>;
struct VOut { @builtin(position) pos: vec4<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}
@fragment fn fs(v: VOut) -> @location(0) vec4<f32> {
    let c = textureLoad(ui, vec2<i32>(v.pos.xy), 0);
    if (SRGB_TARGET) { return vec4<f32>(to_linear(c.rgb), c.a); }
    return c;
}
"#;

struct Target {
    _tex: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    w: u32,
    h: u32,
}

pub struct UiGpu {
    renderer: Renderer,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    target: Option<Target>,
    /// vello frame time of the last render (diagnostics)
    pub last_ms: f32,
}

impl UiGpu {
    /// `out_format` = format of the texture the UI is composited onto (swapchain or offscreen).
    pub fn new(device: &wgpu::Device, out_format: wgpu::TextureFormat) -> Result<Self, String> {
        let renderer = Renderer::new(
            device,
            RendererOptions { use_cpu: false, antialiasing_support: AaSupport::area_only(), num_init_threads: None, pipeline_cache: None },
        )
        .map_err(|e| format!("vello: {e}"))?;
        let src = BLIT.replace("SRGB_TARGET", if out_format.is_srgb() { "true" } else { "false" });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ui-blit"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-blit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ui-blit"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui-blit"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: out_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self { renderer, pipeline, layout, target: None, last_ms: 0.0 })
    }

    fn ensure_target(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.target.as_ref().is_some_and(|t| t.w == w && t.h == h) {
            return;
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui-layer"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-blit"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        self.target = Some(Target { _tex: tex, view, bind, w, h });
    }

    /// Rasterize `scene` at `w`x`h` physical pixels and composite it over `out` (load + alpha blend).
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene, out: &wgpu::TextureView, w: u32, h: u32) -> Result<(), String> {
        let (w, h) = (w.max(1), h.max(1));
        let t0 = std::time::Instant::now();
        self.ensure_target(device, w, h);
        let t = self.target.as_ref().unwrap();
        self.renderer
            .render_to_texture(device, queue, scene, &t.view, &RenderParams { base_color: Color::TRANSPARENT, width: w, height: h, antialiasing_method: AaConfig::Area })
            .map_err(|e| format!("vello render: {e}"))?;
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui-composite") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui-composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &t.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([enc.finish()]);
        self.last_ms = t0.elapsed().as_secs_f32() * 1000.0;
        Ok(())
    }
}
