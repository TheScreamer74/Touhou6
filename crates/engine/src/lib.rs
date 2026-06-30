//! Minimal wgpu sprite renderer: textured quads in a 640x480 logical space,
//! headless render-to-image for verification plus a winit window mode.

pub mod audio;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

use bytemuck::{Pod, Zeroable};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

pub const SCREEN_W: u32 = 640;
pub const SCREEN_H: u32 = 480;

const SHADER: &str = r#"
struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) tint: vec4f,
};

@vertex
fn vs(@location(0) pos: vec2f, @location(1) uv: vec2f, @location(2) tint: vec4f) -> VOut {
    var out: VOut;
    out.pos = vec4f(pos, 0.0, 1.0);
    out.uv = uv;
    out.tint = tint;
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs(in: VOut) -> @location(0) vec4f {
    return textureSample(tex, samp, in.uv) * in.tint;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    tint: [f32; 4],
}

/// 3D background shader: an MVP transform plus per-pixel linear fog
/// (matches the original's D3DRS_FOGTABLEMODE = D3DFOG_LINEAR). The view-space
/// depth is interpolated perspective-correct and the fog factor is computed in
/// the fragment shader from fog_params = (start, end).
const BG_SHADER: &str = r#"
// fog_params = (start, end, additive_flag, 0).
struct U { mvp: mat4x4<f32>, fog_color: vec4f, fog_params: vec4f };
@group(1) @binding(0) var<uniform> u: U;

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) depth: f32,
    @location(2) color: vec4f,
};

@vertex
fn vs(@location(0) pos: vec3f, @location(1) uv: vec2f, @location(2) depth: f32, @location(3) color: vec4f) -> VOut {
    var out: VOut;
    out.pos = u.mvp * vec4f(pos, 1.0);
    out.uv = uv;
    out.depth = depth;
    out.color = color;
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs(in: VOut) -> @location(0) vec4f {
    let c = textureSample(tex, samp, in.uv) * in.color;
    // Alpha test (D3DRS_ALPHATESTENABLE, ALPHAREF=4, GREATEREQUAL): discard
    // near-transparent fragments so they don't write depth and occlude the
    // tiles behind them (GameWindow.cpp:515-517).
    if (c.a < 4.0 / 255.0) {
        discard;
    }
    let span = max(u.fog_params.y - u.fog_params.x, 1.0);
    let f = clamp((u.fog_params.y - in.depth) / span, 0.0, 1.0);
    if (u.fog_params.z > 0.5) {
        // Additive glow: fades to nothing with distance, premultiplied for One/One.
        return vec4f(c.rgb * c.a * f, c.a);
    }
    let rgb = mix(u.fog_color.rgb, c.rgb, f);
    return vec4f(rgb, c.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex3 {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// View-space depth (distance from camera); fog is computed per-pixel.
    pub depth: f32,
    /// Per-vertex modulation colour (ANM vm->color: tint + alpha/fade).
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BgUniform {
    mvp: [[f32; 4]; 4],
    fog_color: [f32; 4],
    /// (fog_start, fog_end, 0, 0).
    fog_params: [f32; 4],
}

/// A 3D background frame: a camera matrix, fog colour + range, the textured
/// triangle soup (already in world space), and which texture slot to use.
pub struct BgScene {
    pub mvp: [[f32; 4]; 4],
    pub fog_color: [f32; 4],
    pub fog_near: f32,
    pub fog_far: f32,
    /// Alpha-blended quads.
    pub verts: Vec<Vertex3>,
    /// Additive-blended quads (op13), drawn after `verts` with no depth write.
    pub verts_add: Vec<Vertex3>,
    pub tex: usize,
}

/// One textured quad. `dst` is x, y, w, h in 640x480 screen pixels;
/// `src` is u0, v0, u1, v1 in normalized texture coordinates. `rot`
/// rotates the quad around its center (radians, clockwise).
#[derive(Clone, Copy)]
pub struct DrawCmd {
    pub tex: usize,
    pub dst: [f32; 4],
    pub src: [f32; 4],
    pub tint: [f32; 4],
    pub rot: f32,
    /// Additive (One) blend instead of alpha — glows/particles. Defaults false.
    pub additive: bool,
}

pub struct Texture {
    bind_group: wgpu::BindGroup,
}

pub struct Engine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    // Native creates surfaces from this in `resumed`; on web the surface is
    // built up front, so the stored instance is only kept alive here.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    instance: wgpu::Instance,
    // Only read on web (to pick a surface format from caps).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    adapter: wgpu::Adapter,
    bind_layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    bg_shader: wgpu::ShaderModule,
    bg_uniform_layout: wgpu::BindGroupLayout,
}

impl Engine {
    /// Native: block on adapter/device acquisition.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Self {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("no GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");
        Self::from_device(instance, adapter, device, queue)
    }

    /// Web: build the engine *from a canvas*. WebGL's GL context comes from
    /// the canvas, so the surface must exist before the adapter is requested
    /// (and passed as `compatible_surface`). Returns the surface so the run
    /// loop can reuse it rather than create a second context on the canvas.
    #[cfg(target_arch = "wasm32")]
    pub async fn new_web(
        canvas: web_sys::HtmlCanvasElement,
    ) -> (Self, wgpu::Surface<'static>) {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .expect("create surface from canvas");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("no GPU adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                // WebGL2 has tighter limits than native; clamp to its defaults.
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                ..Default::default()
            })
            .await
            .expect("request device");
        (Self::from_device(instance, adapter, device, queue), surface)
    }

    fn from_device(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
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

        let bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bg"),
            source: wgpu::ShaderSource::Wgsl(BG_SHADER.into()),
        });
        let bg_uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bg uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        Self { device, queue, instance, adapter, bind_layout, shader, bg_shader, bg_uniform_layout }
    }

    fn make_bg_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        self.make_bg_pipeline_mode(format, false)
    }

    /// `additive` = One/One blend with depth-write disabled, for the bg's
    /// additive glow layer (op13). Otherwise standard alpha blend + depth write.
    fn make_bg_pipeline_mode(&self, format: wgpu::TextureFormat, additive: bool) -> wgpu::RenderPipeline {
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bg"),
            bind_group_layouts: &[&self.bind_layout, &self.bg_uniform_layout],
            push_constant_ranges: &[],
        });
        let blend = if additive {
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            }
        } else {
            wgpu::BlendState::ALPHA_BLENDING
        };
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bg"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.bg_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex3>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32, 3 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.bg_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: !additive,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// Depth attachment sized to match the colour target (the surface may
    /// be larger than the 640x480 logical size on HiDPI displays).
    fn depth_texture(&self, width: u32, height: u32) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    /// Render the 3D background into `target` (clearing it to the fog
    /// colour), scissored to the 384x448 play field at (32, 16).
    fn encode_bg(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        pipeline_add: &wgpu::RenderPipeline,
        scene: &BgScene,
        textures: &[&Texture],
        target_size: (u32, u32),
    ) {
        let make_vbuf = |verts: &[Vertex3]| {
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bg verts"),
                size: (verts.len().max(1) * std::mem::size_of::<Vertex3>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if !verts.is_empty() {
                self.queue.write_buffer(&buf, 0, bytemuck::cast_slice(verts));
            }
            buf
        };
        let vbuf = make_vbuf(&scene.verts);
        let vbuf_add = make_vbuf(&scene.verts_add);

        let make_ubind = |additive: f32| {
            let uniform = BgUniform {
                mvp: scene.mvp,
                fog_color: scene.fog_color,
                fog_params: [scene.fog_near, scene.fog_far, additive, 0.0],
            };
            let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bg uniform"),
                size: std::mem::size_of::<BgUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&uniform));
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.bg_uniform_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
            })
        };
        let ubind = make_ubind(0.0);
        let ubind_add = make_ubind(1.0);

        let [r, g, b, _] = scene.fog_color;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bg"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: r as f64,
                        g: g as f64,
                        b: b as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        // Confine the background to the play field, scaled to the target
        // (the surface is larger than 640x480 on HiDPI displays).
        let sx = target_size.0 as f32 / SCREEN_W as f32;
        let sy = target_size.1 as f32 / SCREEN_H as f32;
        pass.set_viewport(32.0 * sx, 16.0 * sy, 384.0 * sx, 448.0 * sy, 0.0, 1.0);
        pass.set_bind_group(0, &textures[scene.tex].bind_group, &[]);
        if !scene.verts.is_empty() {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &ubind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.draw(0..scene.verts.len() as u32, 0..1);
        }
        if !scene.verts_add.is_empty() {
            pass.set_pipeline(pipeline_add);
            pass.set_bind_group(1, &ubind_add, &[]);
            pass.set_vertex_buffer(0, vbuf_add.slice(..));
            pass.draw(0..scene.verts_add.len() as u32, 0..1);
        }
    }

    fn make_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        self.make_pipeline_mode(format, false)
    }

    /// `additive` = SrcAlpha/One blend (glow particles, laser balls), matching
    /// the original's AnmVmBlendMode_One; otherwise standard alpha.
    fn make_pipeline_mode(&self, format: wgpu::TextureFormat, additive: bool) -> wgpu::RenderPipeline {
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&self.bind_layout],
            push_constant_ranges: &[],
        });
        let blend = if additive {
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            }
        } else {
            wgpu::BlendState::ALPHA_BLENDING
        };
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    pub fn create_texture(&self, rgba: &[u8], width: u32, height: u32) -> Texture {
        self.create_texture_mode(rgba, width, height, false)
    }

    /// Like `create_texture`, but the sampler wraps (Repeat) so tiled/scrolling
    /// backgrounds blend seamlessly at quad edges.
    pub fn create_texture_wrapped(&self, rgba: &[u8], width: u32, height: u32) -> Texture {
        self.create_texture_mode(rgba, width, height, true)
    }

    fn create_texture_mode(&self, rgba: &[u8], width: u32, height: u32, wrap: bool) -> Texture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&Default::default());
        let address = if wrap {
            wgpu::AddressMode::Repeat
        } else {
            wgpu::AddressMode::ClampToEdge
        };
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        Texture { bind_group }
    }

    fn build_vertices(cmds: &[DrawCmd]) -> Vec<Vertex> {
        let mut verts = Vec::with_capacity(cmds.len() * 6);
        for c in cmds {
            let [x, y, w, h] = c.dst;
            let [u0, v0, u1, v1] = c.src;
            // Rotate corners around the quad center in screen space.
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let (sin, cos) = c.rot.sin_cos();
            let corner = |px: f32, py: f32| {
                let (dx, dy) = (px - cx, py - cy);
                (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
            };
            // Screen pixels -> NDC (y down in screen space).
            let nx = |px: f32| px / SCREEN_W as f32 * 2.0 - 1.0;
            let ny = |py: f32| 1.0 - py / SCREEN_H as f32 * 2.0;
            let p00 = corner(x, y);
            let p10 = corner(x + w, y);
            let p11 = corner(x + w, y + h);
            let p01 = corner(x, y + h);
            let quad = [
                (p00, [u0, v0]),
                (p10, [u1, v0]),
                (p11, [u1, v1]),
                (p00, [u0, v0]),
                (p11, [u1, v1]),
                (p01, [u0, v1]),
            ];
            for ((px, py), uv) in quad {
                verts.push(Vertex { pos: [nx(px), ny(py)], uv, tint: c.tint });
            }
        }
        verts
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        pipeline_add: &wgpu::RenderPipeline,
        vbuf: &wgpu::Buffer,
        cmds: &[DrawCmd],
        textures: &[&Texture],
        load: bool,
    ) {
        let load_op = if load {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: load_op, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_vertex_buffer(0, vbuf.slice(..));
        // One draw per run of consecutive commands sharing a texture AND blend
        // mode; the pipeline (alpha vs additive) is switched per run.
        let mut start = 0usize;
        while start < cmds.len() {
            let tex = cmds[start].tex;
            let add = cmds[start].additive;
            let mut end = start;
            while end < cmds.len() && cmds[end].tex == tex && cmds[end].additive == add {
                end += 1;
            }
            pass.set_pipeline(if add { pipeline_add } else { pipeline });
            pass.set_bind_group(0, &textures[tex].bind_group, &[]);
            pass.draw(start as u32 * 6..end as u32 * 6, 0..1);
            start = end;
        }
    }

    fn vertex_buffer(&self, cmds: &[DrawCmd]) -> wgpu::Buffer {
        let verts = Self::build_vertices(cmds);
        // Zero-size buffers cannot be sliced; keep a minimum allocation so
        // an empty scene still renders (as a plain clear).
        let size = (verts.len() * std::mem::size_of::<Vertex>()).max(std::mem::size_of::<Vertex>());
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !verts.is_empty() {
            self.queue.write_buffer(&buf, 0, bytemuck::cast_slice(&verts));
        }
        buf
    }

    /// Render one frame offscreen and return RGBA8 pixels (640x480).
    pub fn render_to_image(
        &self,
        cmds: &[DrawCmd],
        textures: &[&Texture],
        bg: Option<&BgScene>,
    ) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d { width: SCREEN_W, height: SCREEN_H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let pipeline = self.make_pipeline(format);
        let pipeline_add = self.make_pipeline_mode(format, true);
        let vbuf = self.vertex_buffer(cmds);

        let mut encoder = self.device.create_command_encoder(&Default::default());
        if let Some(scene) = bg {
            let bg_pipeline = self.make_bg_pipeline(format);
            let bg_pipeline_add = self.make_bg_pipeline_mode(format, true);
            let depth = self.depth_texture(SCREEN_W, SCREEN_H);
            self.encode_bg(&mut encoder, &view, &depth, &bg_pipeline, &bg_pipeline_add, scene, textures, (SCREEN_W, SCREEN_H));
        }
        self.encode_pass(&mut encoder, &view, &pipeline, &pipeline_add, &vbuf, cmds, textures, bg.is_some());

        let bytes_per_row = SCREEN_W * 4; // 2560, already 256-aligned
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (bytes_per_row * SCREEN_H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: SCREEN_W, height: SCREEN_H, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
        self.device.poll(wgpu::PollType::Wait).expect("poll");
        let data = slice.get_mapped_range().to_vec();
        data
    }

    fn make_app(
        self,
        title: &str,
        textures: Vec<Texture>,
        update: impl FnMut(&Input) -> Frame + 'static,
    ) -> App {
        App {
            engine: self,
            title: title.to_string(),
            cmds: Vec::new(),
            bg: None,
            bg_pipeline: None,
            bg_pipeline_add: None,
            depth: None,
            surface_size: (SCREEN_W, SCREEN_H),
            textures,
            update: Box::new(update),
            input: Input::default(),
            last_tick: Instant::now(),
            accumulator: Duration::ZERO,
            window: None,
            surface: None,
            pipeline: None,
            pipeline_add: None,
            autoshot: std::env::var("TH06_AUTOSHOT").ok().filter(|s| !s.is_empty()),
            tick_count: 0,
            #[cfg(target_arch = "wasm32")]
            web_surface: None,
            #[cfg(target_arch = "wasm32")]
            canvas: None,
        }
    }

    /// Open a window and run the game loop. `update` is called at a fixed
    /// 60 Hz regardless of display refresh rate (the original game tied
    /// logic speed to frame timing — this is the stable-FPS fix). Returns
    /// the frame's draw commands; `Frame::quit` exits.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run_game(
        self,
        title: &str,
        textures: Vec<Texture>,
        update: impl FnMut(&Input) -> Frame + 'static,
    ) {
        let event_loop = EventLoop::new().expect("event loop");
        let mut app = self.make_app(title, textures, update);
        if let Some(dir) = &app.autoshot {
            std::fs::create_dir_all(dir).expect("create autoshot dir");
        }
        event_loop.run_app(&mut app).expect("run app");
    }

    /// Web: hand the loop to the browser, reusing the canvas + surface that
    /// were created up front (see `new_web`). Returns immediately — winit
    /// drives ticks via requestAnimationFrame.
    #[cfg(target_arch = "wasm32")]
    pub fn run_game_web(
        self,
        canvas: web_sys::HtmlCanvasElement,
        surface: wgpu::Surface<'static>,
        title: &str,
        textures: Vec<Texture>,
        update: impl FnMut(&Input) -> Frame + 'static,
    ) {
        use winit::platform::web::EventLoopExtWebSys;
        let event_loop = EventLoop::new().expect("event loop");
        let mut app = self.make_app(title, textures, update);
        app.canvas = Some(canvas);
        app.web_surface = Some(surface);
        event_loop.spawn_app(app);
    }
}

/// Game-relevant keys, mapped from physical keyboard input.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Shoot, // Z
    Bomb,  // X
    Focus, // Shift
    Pause, // Esc
    Enter,
}

fn map_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::KeyZ => Key::Shoot,
        KeyCode::KeyX => Key::Bomb,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Key::Focus,
        KeyCode::Escape => Key::Pause,
        KeyCode::Enter => Key::Enter,
        _ => return None,
    })
}

#[derive(Default)]
pub struct Input {
    held: HashSet<Key>,
    pressed: HashSet<Key>,
}

impl Input {
    /// Build an input state by hand (used by headless/scripted runs).
    pub fn synthetic(held: &[Key], pressed: &[Key]) -> Self {
        Self {
            held: held.iter().copied().collect(),
            pressed: pressed.iter().copied().collect(),
        }
    }

    pub fn held(&self, key: Key) -> bool {
        self.held.contains(&key)
    }
    /// True only on the tick after the key went down.
    pub fn pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }
}

pub struct Frame {
    pub cmds: Vec<DrawCmd>,
    pub bg: Option<BgScene>,
    pub quit: bool,
}

const TICK: Duration = Duration::from_nanos(1_000_000_000 / 60);

struct App {
    engine: Engine,
    title: String,
    cmds: Vec<DrawCmd>,
    bg: Option<BgScene>,
    bg_pipeline: Option<wgpu::RenderPipeline>,
    bg_pipeline_add: Option<wgpu::RenderPipeline>,
    depth: Option<wgpu::TextureView>,
    surface_size: (u32, u32),
    textures: Vec<Texture>,
    update: Box<dyn FnMut(&Input) -> Frame>,
    input: Input,
    last_tick: Instant,
    accumulator: Duration,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    pipeline: Option<wgpu::RenderPipeline>,
    pipeline_add: Option<wgpu::RenderPipeline>,
    /// TH06_AUTOSHOT=<dir>: dump an offscreen PNG every 60 ticks during live
    /// play so playtests can be reviewed after the fact.
    autoshot: Option<String>,
    tick_count: u32,
    /// Web: surface + canvas are built up front (WebGL needs the canvas to
    /// exist before adapter acquisition) and handed to `resumed` to reuse.
    #[cfg(target_arch = "wasm32")]
    web_surface: Option<wgpu::Surface<'static>>,
    #[cfg(target_arch = "wasm32")]
    canvas: Option<web_sys::HtmlCanvasElement>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(winit::dpi::LogicalSize::new(SCREEN_W, SCREEN_H))
            .with_resizable(false);

        // Native: create the window, then a surface on it.
        #[cfg(not(target_arch = "wasm32"))]
        let (window, surface) = {
            let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
            let surface = self.engine.instance.create_surface(window.clone()).expect("surface");
            (window, surface)
        };
        // Web: bind the winit window to the existing canvas (for input/redraw)
        // and reuse the surface already created on it in `new_web` — making a
        // second surface would clash with the canvas's GL context.
        #[cfg(target_arch = "wasm32")]
        let (window, surface) = {
            use winit::platform::web::WindowAttributesExtWebSys;
            let canvas = self.canvas.clone().expect("web canvas");
            let attrs = attrs.with_canvas(Some(canvas));
            let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
            let surface = self.web_surface.take().expect("web surface");
            (window, surface)
        };

        // Native: use the real window size. Web: the canvas isn't laid out
        // yet at this point, so window.inner_size() is ~0x0 — configuring the
        // surface to that renders into a single pixel that CSS then stretches
        // (the "flat color" bug). Pin the web surface to the fixed play size;
        // configuring it also sets the canvas drawing-buffer to match.
        #[cfg(not(target_arch = "wasm32"))]
        let size = window.inner_size();
        #[cfg(target_arch = "wasm32")]
        let size = winit::dpi::PhysicalSize::new(SCREEN_W, SCREEN_H);
        // Native (Metal) uses Bgra8Unorm; WebGL/WebGPU surfaces advertise their
        // own preferred formats, so pick a supported non-sRGB one there.
        #[cfg(not(target_arch = "wasm32"))]
        let format = wgpu::TextureFormat::Bgra8Unorm;
        #[cfg(target_arch = "wasm32")]
        let format = {
            let caps = surface.get_capabilities(&self.engine.adapter);
            caps.formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .unwrap_or(caps.formats[0])
        };
        surface.configure(
            &self.engine.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );
        self.pipeline = Some(self.engine.make_pipeline(format));
        self.pipeline_add = Some(self.engine.make_pipeline_mode(format, true));
        self.bg_pipeline = Some(self.engine.make_bg_pipeline(format));
        self.bg_pipeline_add = Some(self.engine.make_bg_pipeline_mode(format, true));
        self.surface_size = (size.width.max(1), size.height.max(1));
        self.depth = Some(self.engine.depth_texture(self.surface_size.0, self.surface_size.1));
        self.surface = Some(surface);
        self.window = Some(window);
        self.last_tick = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event: KeyEvent { physical_key: PhysicalKey::Code(code), state, repeat: false, .. },
                ..
            } => {
                if let Some(key) = map_key(code) {
                    match state {
                        ElementState::Pressed => {
                            self.input.held.insert(key);
                            self.input.pressed.insert(key);
                        }
                        ElementState::Released => {
                            self.input.held.remove(&key);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                // Fixed-timestep logic: run 60 Hz ticks to catch up with
                // wall time, render the latest state once.
                let now = Instant::now();
                self.accumulator += now - self.last_tick;
                self.last_tick = now;
                // Cap catch-up so a stall doesn't fast-forward the game.
                if self.accumulator > TICK * 4 {
                    self.accumulator = TICK * 4;
                }
                // The first redraw can arrive before any tick is due;
                // run one anyway so there is a scene to draw.
                if self.cmds.is_empty() && self.accumulator < TICK {
                    self.accumulator = TICK;
                }
                while self.accumulator >= TICK {
                    self.accumulator -= TICK;
                    let frame = (self.update)(&self.input);
                    self.input.pressed.clear();
                    if frame.quit {
                        event_loop.exit();
                        return;
                    }
                    self.cmds = frame.cmds;
                    self.bg = frame.bg;
                    self.tick_count += 1;
                    if let Some(dir) = &self.autoshot {
                        if self.tick_count % 60 == 0 {
                            let tex_refs: Vec<&Texture> = self.textures.iter().collect();
                            let pixels = self.engine.render_to_image(&self.cmds, &tex_refs, self.bg.as_ref());
                            let path = format!("{dir}/shot_{:06}.png", self.tick_count);
                            let _ = image::save_buffer(&path, &pixels, SCREEN_W, SCREEN_H, image::ColorType::Rgba8);
                        }
                    }
                }

                let (Some(surface), Some(pipeline), Some(pipeline_add), Some(bg_pipeline), Some(bg_pipeline_add), Some(depth)) = (
                    &self.surface,
                    &self.pipeline,
                    &self.pipeline_add,
                    &self.bg_pipeline,
                    &self.bg_pipeline_add,
                    &self.depth,
                ) else {
                    return;
                };
                let frame = surface.get_current_texture().expect("acquire frame");
                let view = frame.texture.create_view(&Default::default());
                let vbuf = self.engine.vertex_buffer(&self.cmds);
                let tex_refs: Vec<&Texture> = self.textures.iter().collect();
                let mut encoder = self.engine.device.create_command_encoder(&Default::default());
                if let Some(scene) = &self.bg {
                    self.engine.encode_bg(
                        &mut encoder,
                        &view,
                        depth,
                        bg_pipeline,
                        bg_pipeline_add,
                        scene,
                        &tex_refs,
                        self.surface_size,
                    );
                }
                self.engine
                    .encode_pass(&mut encoder, &view, pipeline, pipeline_add, &vbuf, &self.cmds, &tex_refs, self.bg.is_some());
                self.engine.queue.submit([encoder.finish()]);
                frame.present();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// Decode a color image (JPEG or PNG) and an optional grayscale alpha mask
/// into a single RGBA8 buffer.
pub fn compose_rgba(color: &[u8], alpha: Option<&[u8]>) -> (Vec<u8>, u32, u32) {
    let img = image::load_from_memory(color).expect("decode color image").to_rgba8();
    let (w, h) = img.dimensions();
    let mut rgba = img.into_raw();
    if let Some(mask_bytes) = alpha {
        let mask = image::load_from_memory(mask_bytes).expect("decode alpha mask").to_luma8();
        assert_eq!(mask.dimensions(), (w, h), "alpha mask size mismatch");
        for (i, p) in mask.as_raw().iter().enumerate() {
            rgba[i * 4 + 3] = *p;
        }
    }
    (rgba, w, h)
}
