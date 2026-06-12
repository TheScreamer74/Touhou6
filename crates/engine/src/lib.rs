//! Minimal wgpu sprite renderer: textured quads in a 640x480 logical space,
//! headless render-to-image for verification plus a winit window mode.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// One textured quad. `dst` is x, y, w, h in 640x480 screen pixels;
/// `src` is u0, v0, u1, v1 in normalized texture coordinates.
#[derive(Clone, Copy)]
pub struct DrawCmd {
    pub tex: usize,
    pub dst: [f32; 4],
    pub src: [f32; 4],
    pub tint: [f32; 4],
}

pub struct Texture {
    bind_group: wgpu::BindGroup,
}

pub struct Engine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    instance: wgpu::Instance,
    bind_layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
}

impl Engine {
    pub fn new() -> Self {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("no GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

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

        Self { device, queue, instance, bind_layout, shader }
    }

    fn make_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&self.bind_layout],
            push_constant_ranges: &[],
        });
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
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
            // Screen pixels -> NDC (y down in screen space).
            let nx = |px: f32| px / SCREEN_W as f32 * 2.0 - 1.0;
            let ny = |py: f32| 1.0 - py / SCREEN_H as f32 * 2.0;
            let quad = [
                ([nx(x), ny(y)], [u0, v0]),
                ([nx(x + w), ny(y)], [u1, v0]),
                ([nx(x + w), ny(y + h)], [u1, v1]),
                ([nx(x), ny(y)], [u0, v0]),
                ([nx(x + w), ny(y + h)], [u1, v1]),
                ([nx(x), ny(y + h)], [u0, v1]),
            ];
            for (pos, uv) in quad {
                verts.push(Vertex { pos, uv, tint: c.tint });
            }
        }
        verts
    }

    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        vbuf: &wgpu::Buffer,
        cmds: &[DrawCmd],
        textures: &[&Texture],
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, vbuf.slice(..));
        // One draw per run of consecutive commands sharing a texture.
        let mut start = 0usize;
        while start < cmds.len() {
            let tex = cmds[start].tex;
            let mut end = start;
            while end < cmds.len() && cmds[end].tex == tex {
                end += 1;
            }
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
    pub fn render_to_image(&self, cmds: &[DrawCmd], textures: &[&Texture]) -> Vec<u8> {
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
        let vbuf = self.vertex_buffer(cmds);

        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.encode_pass(&mut encoder, &view, &pipeline, &vbuf, cmds, textures);

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

    /// Open a window and run the game loop. `update` is called at a fixed
    /// 60 Hz regardless of display refresh rate (the original game tied
    /// logic speed to frame timing — this is the stable-FPS fix). Returns
    /// the frame's draw commands; `Frame::quit` exits.
    pub fn run_game(
        self,
        title: &str,
        textures: Vec<Texture>,
        update: impl FnMut(&Input) -> Frame + 'static,
    ) {
        let event_loop = EventLoop::new().expect("event loop");
        let mut app = App {
            engine: self,
            title: title.to_string(),
            cmds: Vec::new(),
            textures,
            update: Box::new(update),
            input: Input::default(),
            last_tick: Instant::now(),
            accumulator: Duration::ZERO,
            window: None,
            surface: None,
            pipeline: None,
        };
        event_loop.run_app(&mut app).expect("run app");
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
    pub quit: bool,
}

const TICK: Duration = Duration::from_nanos(1_000_000_000 / 60);

struct App {
    engine: Engine,
    title: String,
    cmds: Vec<DrawCmd>,
    textures: Vec<Texture>,
    update: Box<dyn FnMut(&Input) -> Frame>,
    input: Input,
    last_tick: Instant,
    accumulator: Duration,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    pipeline: Option<wgpu::RenderPipeline>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(&self.title)
                        .with_inner_size(winit::dpi::LogicalSize::new(SCREEN_W, SCREEN_H))
                        .with_resizable(false),
                )
                .expect("create window"),
        );
        let surface = self.engine.instance.create_surface(window.clone()).expect("surface");
        let size = window.inner_size();
        let format = wgpu::TextureFormat::Bgra8Unorm;
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
                }

                let (Some(surface), Some(pipeline)) = (&self.surface, &self.pipeline) else {
                    return;
                };
                let frame = surface.get_current_texture().expect("acquire frame");
                let view = frame.texture.create_view(&Default::default());
                let vbuf = self.engine.vertex_buffer(&self.cmds);
                let tex_refs: Vec<&Texture> = self.textures.iter().collect();
                let mut encoder = self.engine.device.create_command_encoder(&Default::default());
                self.engine.encode_pass(&mut encoder, &view, pipeline, &vbuf, &self.cmds, &tex_refs);
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
