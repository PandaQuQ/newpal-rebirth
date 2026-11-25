//! Minimal wgpu sprite renderer. Everything is drawn as textured quads or triangle meshes in
//! *logical* units through a camera (origin + size). The world camera uses original-pixel units, so
//! the output resolution is independent of the content: HD textures, any window size and widescreen
//! just change the camera.
use bytemuck::{Pod, Zeroable};
use newpal_assets::image::RgbaImage;
use newpal_prepare::flow::FlowField;
use newpal_prepare::mesh::AnimMesh;

const SHADER: &str = r#"
struct Cam { origin: vec2<f32>, size: vec2<f32> };
@group(0) @binding(0) var<uniform> cam: Cam;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) texsz: vec2<f32>,
    @location(3) mode: f32,
};
@vertex fn vs(@builtin(vertex_index) vi: u32,
              @location(0) p: vec2<f32>, @location(1) s: vec2<f32>,
              @location(2) uv0: vec2<f32>, @location(3) uv1: vec2<f32>, @location(4) color: vec4<f32>,
              @location(5) texsz: vec2<f32>, @location(6) mode: f32, @location(7) _pad: f32) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let w = p + corner * s;
    let n = (w - cam.origin) / cam.size;
    var o: VOut;
    o.pos = vec4<f32>(n.x * 2.0 - 1.0, 1.0 - n.y * 2.0, 0.0, 1.0);
    o.uv = mix(uv0, uv1, corner);
    o.color = color;
    o.texsz = texsz;
    o.mode = mode;
    return o;
}
fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }
fn smp(uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(tex, samp, uv, 0.0); }
fn smp_off(uv: vec2<f32>, texsz: vec2<f32>, ox: f32, oy: f32) -> vec4<f32> {
    return smp(uv + vec2<f32>(ox, oy) / texsz);
}
// Catmull-Rom 1D
fn cr(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return b + ((-a + c) * 0.5) * t + (a - 2.5 * b + 2.0 * c - 0.5 * d) * t2 + ((-0.5 * a + 1.5 * b - 1.5 * c + 0.5 * d) * t3);
}
fn cubic(uv: vec2<f32>, texsz: vec2<f32>) -> vec4<f32> {
    let pos = uv * texsz - 0.5;
    let i0 = floor(pos);
    let f = pos - i0;
    let base = (i0 + 0.5) / texsz;
    var col: array<vec4<f32>, 4>;
    for (var j = -1; j < 3; j++) {
        var p: array<vec4<f32>, 4>;
        for (var i = -1; i < 3; i++) {
            p[i + 1] = smp(base + vec2<f32>(f32(i), f32(j)) / texsz);
        }
        col[j + 1] = cr(p[0], p[1], p[2], p[3], f.x);
    }
    return cr(col[0], col[1], col[2], col[3], f.y);
}
// Anime4K-style: cubic upsample + edge-aware blend + unsharp restore (no CNN weights).
fn anime4k(uv: vec2<f32>, texsz: vec2<f32>) -> vec4<f32> {
    let body = cubic(uv, texsz);
    let c00 = smp_off(uv, texsz, -1.0, -1.0);
    let c10 = smp_off(uv, texsz,  0.0, -1.0);
    let c20 = smp_off(uv, texsz,  1.0, -1.0);
    let c01 = smp_off(uv, texsz, -1.0,  0.0);
    let c11 = smp(uv);
    let c21 = smp_off(uv, texsz,  1.0,  0.0);
    let c02 = smp_off(uv, texsz, -1.0,  1.0);
    let c12 = smp_off(uv, texsz,  0.0,  1.0);
    let c22 = smp_off(uv, texsz,  1.0,  1.0);
    let gx = luma(c20.rgb) + 2.0 * luma(c21.rgb) + luma(c22.rgb)
           - luma(c00.rgb) - 2.0 * luma(c01.rgb) - luma(c02.rgb);
    let gy = luma(c02.rgb) + 2.0 * luma(c12.rgb) + luma(c22.rgb)
           - luma(c00.rgb) - 2.0 * luma(c10.rgb) - luma(c20.rgb);
    let g = sqrt(gx * gx + gy * gy);
    let dir = vec2<f32>(gx, gy) / max(g, 1e-5);
    let tanv = vec2<f32>(-dir.y, dir.x) / texsz;
    let along = (smp(uv - tanv) + smp(uv + tanv)) * 0.5;
    let edge_w = smoothstep(0.08, 0.35, g);
    let mixed = mix(body, along, edge_w * 0.65);
    let blur = (c00 + c20 + c02 + c22) * 0.0625 + (c10 + c01 + c21 + c12) * 0.125 + c11 * 0.25;
    let sharp = mixed + (mixed - blur) * mix(0.35, 0.7, edge_w);
    return vec4<f32>(clamp(sharp.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), body.a);
}
fn distc(a: vec3<f32>, b: vec3<f32>) -> f32 { return dot(abs(a - b), vec3<f32>(0.299, 0.587, 0.114)); }
// xBR / ScaleFX-style edge-directed pixel-art upscale (Hyllian-inspired, any scale).
fn xbr(uv: vec2<f32>, texsz: vec2<f32>) -> vec4<f32> {
    let pos = uv * texsz - 0.5;
    let i0 = floor(pos);
    let f = pos - i0;
    let base = (i0 + 0.5) / texsz;
    let A = smp(base + vec2<f32>(-1.0, -1.0) / texsz);
    let B = smp(base + vec2<f32>( 0.0, -1.0) / texsz);
    let C = smp(base + vec2<f32>( 1.0, -1.0) / texsz);
    let D = smp(base + vec2<f32>(-1.0,  0.0) / texsz);
    let E = smp(base);
    let F = smp(base + vec2<f32>( 1.0,  0.0) / texsz);
    let G = smp(base + vec2<f32>(-1.0,  1.0) / texsz);
    let H = smp(base + vec2<f32>( 0.0,  1.0) / texsz);
    let I = smp(base + vec2<f32>( 1.0,  1.0) / texsz);
    // 4 corners: keep a sharp edge when the diagonal is weaker than the axis (xBR lv2 idea).
    var e0 = E;
    var e1 = E;
    var e2 = E;
    var e3 = E;
    // top-left toward A/B/D
    if (distc(D.rgb, B.rgb) < distc(A.rgb, E.rgb)) {
        let w = 0.5;
        e0 = mix(E, mix(D, B, 0.5), w);
    }
    // top-right toward C/B/F
    if (distc(F.rgb, B.rgb) < distc(C.rgb, E.rgb)) {
        e1 = mix(E, mix(F, B, 0.5), 0.5);
    }
    // bottom-left toward G/D/H
    if (distc(D.rgb, H.rgb) < distc(G.rgb, E.rgb)) {
        e2 = mix(E, mix(D, H, 0.5), 0.5);
    }
    // bottom-right toward I/F/H
    if (distc(F.rgb, H.rgb) < distc(I.rgb, E.rgb)) {
        e3 = mix(E, mix(F, H, 0.5), 0.5);
    }
    let x = mix(e0, e1, f.x);
    let y = mix(e2, e3, f.x);
    return mix(x, y, f.y);
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
    var c: vec4<f32>;
    if (i.mode > 2.5 && i.texsz.x > 1.0) {
        c = xbr(i.uv, i.texsz);
    } else if (i.mode > 1.5 && i.texsz.x > 1.0) {
        c = anime4k(i.uv, i.texsz);
    } else {
        c = textureSample(tex, samp, i.uv);
    }
    c = c * i.color;
    if (c.a < 0.004) { discard; }
    return c;
}
"#;

const FLOW_SHADER: &str = r#"
struct Cam { origin: vec2<f32>, size: vec2<f32> };
@group(0) @binding(0) var<uniform> cam: Cam;
@group(1) @binding(0) var tex_a: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var tex_b: texture_2d<f32>;
@group(1) @binding(3) var flow_tex: texture_2d<f32>;
@group(1) @binding(4) var conf_tex: texture_2d<f32>;
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) texsz: vec2<f32>,
    @location(3) t: f32,
};
@vertex fn vs(@builtin(vertex_index) vi: u32,
              @location(0) p: vec2<f32>, @location(1) s: vec2<f32>,
              @location(2) uv0: vec2<f32>, @location(3) uv1: vec2<f32>, @location(4) color: vec4<f32>,
              @location(5) texsz: vec2<f32>, @location(6) mode: f32, @location(7) t: f32) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let w = p + corner * s;
    let n = (w - cam.origin) / cam.size;
    var o: VOut;
    o.pos = vec4<f32>(n.x * 2.0 - 1.0, 1.0 - n.y * 2.0, 0.0, 1.0);
    o.uv = mix(uv0, uv1, corner);
    o.color = color;
    o.texsz = texsz;
    o.t = t;
    return o;
}
fn smp(tex: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(tex, samp, uv, 0.0);
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
    let t = clamp(i.t, 0.0, 1.0);
    let fade = mix(smp(tex_a, i.uv), smp(tex_b, i.uv), t);
    let fdim = vec2<f32>(textureDimensions(flow_tex));
    let fxy = vec2<i32>(clamp(i.uv, vec2<f32>(0.0), vec2<f32>(0.9999)) * fdim);
    let fl = textureLoad(flow_tex, fxy, 0);
    let conf = smp(conf_tex, i.uv).r;
    // flow channels are pixels of the sprite (already scaled at upload)
    let inv = vec2<f32>(1.0 / max(i.texsz.x, 1.0), 1.0 / max(i.texsz.y, 1.0));
    let wa = smp(tex_a, i.uv + fl.rg * t * inv);
    let wb = smp(tex_b, i.uv + fl.ba * (1.0 - t) * inv);
    let warped = mix(wa, wb, t);
    let wgt = select(0.0, smoothstep(0.28, 0.55, conf), conf >= 0.28);
    var c = mix(fade, warped, wgt) * i.color;
    if (c.a < 0.004) { discard; }
    return c;
}
"#;

const MESH_SHADER: &str = r#"
struct Cam { origin: vec2<f32>, size: vec2<f32> };
@group(0) @binding(0) var<uniform> cam: Cam;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@vertex fn vs(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VOut {
    let n = (p - cam.origin) / cam.size;
    var o: VOut;
    o.pos = vec4<f32>(n.x * 2.0 - 1.0, 1.0 - n.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv) * i.color;
    if (c.a < 0.004) { discard; }
    return c;
}
"#;

pub const UPSCALE_NEAREST: u32 = 0;
pub const UPSCALE_LINEAR: u32 = 1;
pub const UPSCALE_ANIME4K: u32 = 2;
pub const UPSCALE_XBR: u32 = 3;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Quad {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub color: [f32; 4],
    pub tex_size: [f32; 2],
    pub mode: f32,
    pub _pad: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct MeshVert {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Camera {
    pub origin: [f32; 2],
    pub size: [f32; 2],
}

pub const CAM_WORLD: usize = 0;
pub const CAM_SCREEN: usize = 1;

struct GpuTexture {
    bind: wgpu::BindGroup,
    _tex: wgpu::Texture,
    size: [f32; 2],
}

enum Cmd {
    Quad { tex: usize, cam: usize, quad: Quad },
    Mesh { tex: usize, cam: usize, first: u32, count: u32 },
    #[allow(dead_code)]
    Flow { tex_a: usize, tex_b: usize, flow: usize, conf: usize, cam: usize, quad: Quad },
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    flow_pipeline: wgpu::RenderPipeline,
    tex_layout: wgpu::BindGroupLayout,
    flow_layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    cams: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    instances: wgpu::Buffer,
    capacity: u64,
    mesh_vbuf: wgpu::Buffer,
    mesh_ibuf: wgpu::Buffer,
    mesh_vcap: u64,
    mesh_icap: u64,
    mesh_verts: Vec<MeshVert>,
    mesh_indices: Vec<u32>,
    textures: Vec<Option<GpuTexture>>,
    cmds: Vec<Cmd>,
    pub white: usize,
}

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(MESH_SHADER.into()),
        });
        let cam_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cam"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite"),
            bind_group_layouts: &[Some(&cam_layout), Some(&tex_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4, 5 => Float32x2, 6 => Float32, 7 => Float32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Quad>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh-warp"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVert>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &mesh_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let flow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flow-warp"),
            source: wgpu::ShaderSource::Wgsl(FLOW_SHADER.into()),
        });
        let flow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flow-tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            ],
        });
        let flow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flow"),
            bind_group_layouts: &[Some(&cam_layout), Some(&flow_layout)],
            immediate_size: 0,
        });
        let flow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flow-warp"),
            layout: Some(&flow_pl),
            vertex: wgpu::VertexState {
                module: &flow_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Quad>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &flow_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |f: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: f, min_filter: f, ..Default::default() })
        };
        let (nearest, linear) = (sampler(wgpu::FilterMode::Nearest), sampler(wgpu::FilterMode::Linear));
        let cams = (0..2)
            .map(|_| {
                let buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cam"),
                    size: std::mem::size_of::<Camera>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cam"),
                    layout: &cam_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
                });
                (buf, bg)
            })
            .collect();
        let capacity = 1024u64;
        let instances = Self::make_instances(&device, capacity);
        let (mesh_vcap, mesh_icap) = (256u64, 768u64);
        let mesh_vbuf = Self::make_vbuf(&device, mesh_vcap);
        let mesh_ibuf = Self::make_ibuf(&device, mesh_icap);
        let mut r = Self {
            device, queue, format, pipeline, mesh_pipeline, flow_pipeline, tex_layout, flow_layout, nearest, linear, cams, instances, capacity,
            mesh_vbuf, mesh_ibuf,
            mesh_vcap, mesh_icap,
            mesh_verts: Vec::new(), mesh_indices: Vec::new(),
            textures: Vec::new(), cmds: Vec::new(), white: 0,
        };
        let mut w = RgbaImage::new(1, 1);
        w.pixels.copy_from_slice(&[255, 255, 255, 255]);
        r.white = r.upload(&w, false);
        r
    }

    fn make_instances(device: &wgpu::Device, n: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quads"),
            size: n * std::mem::size_of::<Quad>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_vbuf(device: &wgpu::Device, n: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh-v"),
            size: n * std::mem::size_of::<MeshVert>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_ibuf(device: &wgpu::Device, n: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh-i"),
            size: n * 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload an image; returns a texture id. `smooth` = linear filtering (HD / text), else nearest.
    pub fn upload(&mut self, img: &RgbaImage, smooth: bool) -> usize {
        let t = self.make_texture(img, smooth);
        self.textures.push(Some(t));
        self.textures.len() - 1
    }

    /// Replace texture `id` in place (e.g. re-rendered dialogue text).
    pub fn replace(&mut self, id: usize, img: &RgbaImage, smooth: bool) {
        let t = self.make_texture(img, smooth);
        self.textures[id] = Some(t);
    }

    fn make_texture(&self, img: &RgbaImage, smooth: bool) -> GpuTexture {
        let (w, h) = (img.width.max(1), img.height.max(1));
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let empty = [0u8; 4];
        let data: &[u8] = if img.width == 0 || img.height == 0 { &empty } else { &img.pixels };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            size,
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(if smooth { &self.linear } else { &self.nearest }),
                },
            ],
        });
        GpuTexture { bind, _tex: tex, size: [w as f32, h as f32] }
    }

    pub fn set_camera(&self, i: usize, cam: Camera) {
        self.queue.write_buffer(&self.cams[i].0, 0, bytemuck::bytes_of(&cam));
    }

    /// Queue a quad (drawn in push order = painter's order).
    pub fn push(&mut self, tex: usize, cam: usize, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.push_ex(tex, cam, x, y, w, h, color, UPSCALE_NEAREST);
    }

    pub fn push_ex(&mut self, tex: usize, cam: usize, x: f32, y: f32, w: f32, h: f32, color: [f32; 4], mode: u32) {
        let sz = self.textures.get(tex).and_then(|t| t.as_ref()).map(|t| t.size).unwrap_or([1.0, 1.0]);
        self.cmds.push(Cmd::Quad { tex, cam, quad: Quad { pos: [x, y], size: [w, h], uv0: [0.0, 0.0], uv1: [1.0, 1.0], color, tex_size: sz, mode: mode as f32, _pad: 0.0 } });
    }

    /// Upload a packed flow field. Channels are converted to sprite-pixel units.
    /// Returns (flow_tex, conf_tex) ids.
    #[allow(dead_code)]
    pub fn upload_flow(&mut self, field: &FlowField, sprite_w: f32, sprite_h: f32) -> (usize, usize) {
        let (w, h) = (field.width.max(1), field.height.max(1));
        let sx = sprite_w / w as f32;
        let sy = sprite_h / h as f32;
        let n = (w * h) as usize;
        let mut rgba = vec![0f32; n * 4];
        let mut conf = vec![0u8; n];
        for i in 0..n {
            rgba[i * 4]     = field.fx[i] as f32 / newpal_prepare::flow::FX as f32 * sx;
            rgba[i * 4 + 1] = field.fy[i] as f32 / newpal_prepare::flow::FX as f32 * sy;
            rgba[i * 4 + 2] = field.bx[i] as f32 / newpal_prepare::flow::FX as f32 * sx;
            rgba[i * 4 + 3] = field.by[i] as f32 / newpal_prepare::flow::FX as f32 * sy;
            conf[i] = field.conf[i];
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let flow_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flow"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &flow_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&rgba),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(16 * w), rows_per_image: Some(h) },
            size,
        );
        let conf_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flow-conf"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &conf_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &conf,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
            size,
        );
        // Draw-time path builds a 5-entry flow_layout bind group from `_tex`.
        // The stored `bind` is a dummy filterable 1x1 so Rgba32Float never hits tex_layout.
        let dummy_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flow-dummy"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &dummy_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[0u8, 0, 0, 0],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let dview = dummy_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let dbind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flow-dummy-bind"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&dview) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.nearest) },
            ],
        });
        self.textures.push(Some(GpuTexture { bind: dbind, _tex: flow_tex, size: [w as f32, h as f32] }));
        let fi = self.textures.len() - 1;
        // reuse the same dummy bind for the confidence texture slot
        let cbind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flow-dummy-bind2"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&dview) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.nearest) },
            ],
        });
        self.textures.push(Some(GpuTexture { bind: cbind, _tex: conf_tex, size: [w as f32, h as f32] }));
        let ci = self.textures.len() - 1;
        // Bind groups hold views; keep the 1x1 dummy allocated for the Renderer lifetime.
        std::mem::forget(dummy_tex);
        (fi, ci)
    }

    /// Timing-preserving warp: `t` is 0..1 inside the original keyframe delay (period unchanged).
    #[allow(dead_code)]
    pub fn push_flow(&mut self, tex_a: usize, tex_b: usize, flow: usize, conf: usize, cam: usize, x: f32, y: f32, w: f32, h: f32, color: [f32; 4], t: f32) {
        let sz = self.textures.get(tex_a).and_then(|t| t.as_ref()).map(|t| t.size).unwrap_or([1.0, 1.0]);
        self.cmds.push(Cmd::Flow { tex_a, tex_b, flow, conf, cam, quad: Quad { pos: [x, y], size: [w, h], uv0: [0.0, 0.0], uv1: [1.0, 1.0], color, tex_size: sz, mode: 0.0, _pad: t } });
    }

    pub fn push_mesh(&mut self, tex: usize, cam: usize, verts: &[MeshVert], indices: &[u32]) {
        if verts.is_empty() || indices.is_empty() { return; }
        let first_v = self.mesh_verts.len() as u32;
        self.mesh_verts.extend_from_slice(verts);
        let first = self.mesh_indices.len() as u32;
        self.mesh_indices.extend(indices.iter().map(|i| i + first_v));
        self.cmds.push(Cmd::Mesh { tex, cam, first, count: indices.len() as u32 });
    }

    /// GPU mesh-warp of an animation frame. `t` is 0..1 toward the next frame. Positions are in
    /// logical pixels; UVs sample the texture as the full logical canvas.
    pub fn push_anim_mesh(&mut self, tex: usize, cam: usize, mesh: &AnimMesh, frame: usize, t: f32, origin: [f32; 2], color: [f32; 4]) {
        let Some(f) = mesh.frames.get(frame) else { return; };
        let (lw, lh) = (mesh.logical_w.max(1) as f32, mesh.logical_h.max(1) as f32);
        let verts: Vec<MeshVert> = f.verts.iter().zip(&f.flow).map(|(v, fl)| MeshVert {
            pos: [origin[0] + v[0] + fl[0] * t, origin[1] + v[1] + fl[1] * t],
            uv: [v[0] / lw, v[1] / lh],
            color,
        }).collect();
        self.push_mesh(tex, cam, &verts, &mesh.indices);
    }

    /// Render all queued quads / meshes into `view` and clear the queue.
    pub fn render(&mut self, view: &wgpu::TextureView, clear: wgpu::Color) {
        let nq = self.cmds.iter().filter(|c| matches!(c, Cmd::Quad { .. } | Cmd::Flow { .. })).count() as u64;
        if nq > self.capacity {
            self.capacity = nq.next_power_of_two().max(1);
            self.instances = Self::make_instances(&self.device, self.capacity);
        }
        if !self.mesh_verts.is_empty() {
            let n = self.mesh_verts.len() as u64;
            if n > self.mesh_vcap {
                self.mesh_vcap = n.next_power_of_two();
                self.mesh_vbuf = Self::make_vbuf(&self.device, self.mesh_vcap);
            }
            self.queue.write_buffer(&self.mesh_vbuf, 0, bytemuck::cast_slice(&self.mesh_verts));
        }
        if !self.mesh_indices.is_empty() {
            let n = self.mesh_indices.len() as u64;
            if n > self.mesh_icap {
                self.mesh_icap = n.next_power_of_two();
                self.mesh_ibuf = Self::make_ibuf(&self.device, self.mesh_icap);
            }
            self.queue.write_buffer(&self.mesh_ibuf, 0, bytemuck::cast_slice(&self.mesh_indices));
        }
        let quads: Vec<Quad> = self.cmds.iter().filter_map(|c| match c {
            Cmd::Quad { quad, .. } | Cmd::Flow { quad, .. } => Some(*quad),
            _ => None,
        }).collect();
        if !quads.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&quads));
        }
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut quad_i = 0u32;
            let mut i = 0usize;
            while i < self.cmds.len() {
                match self.cmds[i] {
                    Cmd::Quad { tex, cam, .. } => {
                        let mut j = i + 1;
                        let mut n = 1u32;
                        while j < self.cmds.len() {
                            match self.cmds[j] {
                                Cmd::Quad { tex: t2, cam: c2, .. } if t2 == tex && c2 == cam => { n += 1; j += 1; }
                                _ => break,
                            }
                        }
                        if let Some(t) = self.textures.get(tex).and_then(|t| t.as_ref()) {
                            pass.set_pipeline(&self.pipeline);
                            pass.set_vertex_buffer(0, self.instances.slice(..));
                            pass.set_bind_group(0, &self.cams[cam].1, &[]);
                            pass.set_bind_group(1, &t.bind, &[]);
                            pass.draw(0..4, quad_i..quad_i + n);
                        }
                        quad_i += n;
                        i = j;
                    }
                    Cmd::Flow { tex_a, tex_b, flow, conf, cam, .. } => {
                        if let (Some(a), Some(b), Some(f), Some(c)) = (
                            self.textures.get(tex_a).and_then(|t| t.as_ref()),
                            self.textures.get(tex_b).and_then(|t| t.as_ref()),
                            self.textures.get(flow).and_then(|t| t.as_ref()),
                            self.textures.get(conf).and_then(|t| t.as_ref()),
                        ) {
                            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("flow-draw"),
                                layout: &self.flow_layout,
                                entries: &[
                                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&a._tex.create_view(&wgpu::TextureViewDescriptor::default())) },
                                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.linear) },
                                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&b._tex.create_view(&wgpu::TextureViewDescriptor::default())) },
                                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&f._tex.create_view(&wgpu::TextureViewDescriptor::default())) },
                                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&c._tex.create_view(&wgpu::TextureViewDescriptor::default())) },
                                ],
                            });
                            pass.set_pipeline(&self.flow_pipeline);
                            pass.set_vertex_buffer(0, self.instances.slice(..));
                            pass.set_bind_group(0, &self.cams[cam].1, &[]);
                            pass.set_bind_group(1, &bind, &[]);
                            pass.draw(0..4, quad_i..quad_i + 1);
                        }
                        quad_i += 1;
                        i += 1;
                    }
                    Cmd::Mesh { tex, cam, first, count } => {
                        let mut j = i + 1;
                        let mut last = first + count;
                        while j < self.cmds.len() {
                            match self.cmds[j] {
                                Cmd::Mesh { tex: t2, cam: c2, first: f2, count: n2 } if t2 == tex && c2 == cam && f2 == last => {
                                    last = f2 + n2;
                                    j += 1;
                                }
                                _ => break,
                            }
                        }
                        if let Some(t) = self.textures.get(tex).and_then(|t| t.as_ref()) {
                            pass.set_pipeline(&self.mesh_pipeline);
                            pass.set_vertex_buffer(0, self.mesh_vbuf.slice(..));
                            pass.set_index_buffer(self.mesh_ibuf.slice(..), wgpu::IndexFormat::Uint32);
                            pass.set_bind_group(0, &self.cams[cam].1, &[]);
                            pass.set_bind_group(1, &t.bind, &[]);
                            pass.draw_indexed(first..last, 0, 0..1);
                        }
                        i = j;
                    }
                }
            }
        }
        self.queue.submit([enc.finish()]);
        self.cmds.clear();
        self.mesh_verts.clear();
        self.mesh_indices.clear();
    }

    /// Render offscreen at `w`x`h` and read back RGBA8 (used by `--screenshot` / HD compare).
    pub fn render_to_image(&mut self, w: u32, h: u32, clear: wgpu::Color) -> RgbaImage {
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.render(&view, clear);
        let padded = (4 * w).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("copy") });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            size,
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().expect("map readback buffer");
        let mut img = RgbaImage::new(w, h);
        for y in 0..h as usize {
            let src = &data[y * padded as usize..y * padded as usize + 4 * w as usize];
            img.pixels[y * 4 * w as usize..(y + 1) * 4 * w as usize].copy_from_slice(src);
        }
        drop(data);
        buf.unmap();
        img
    }
}
