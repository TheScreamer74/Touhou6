//! R5G6B5 composition. A final posterisation pass loses the rounding after
//! each alpha/additive blend, so this path keeps the actual 5/6/5 integer
//! channels in an RGBA8 carrier and explicitly blends each overlapping draw.
//! Non-overlapping quads share a pass; no framebuffer fetch extension needed.
use super::{BgScene, BgUniform, DrawCmd, Engine, SCREEN_H, SCREEN_W, Texture, Vertex, Vertex3};
use std::collections::HashMap;

const SAMPLE: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> levels: vec4f;
fn original_texel(p: vec2i) -> vec4f {
    let dimensions = vec2i(textureDimensions(tex));
    let wrapped = ((p % dimensions) + dimensions) % dimensions;
    return round(textureLoad(tex, wrapped, 0) * 255.0) / levels;
}
fn original_sample(uv: vec2f) -> vec4f {
    let point = uv * vec2f(textureDimensions(tex)) - vec2f(0.5);
    let base = vec2i(floor(point));
    let weight = fract(point);
    return mix(mix(original_texel(base), original_texel(base + vec2i(1, 0)), weight.x),
               mix(original_texel(base + vec2i(0, 1)), original_texel(base + vec2i(1, 1)), weight.x), weight.y);
}
@group(2) @binding(0) var previous: texture_2d<f32>;
fn compose(pos: vec2f, source: vec4f, additive: bool, premultiplied: bool) -> vec4f {
    if (source.a < 4.0 / 255.0) { discard; }
    let maximum = vec3f(31.0, 63.0, 31.0);
    let dest = round(textureLoad(previous, vec2i(pos), 0).rgb * 255.0) / maximum;
    let source_factor = select(source.a, 1.0, premultiplied);
    let dest_factor = select(1.0 - source.a, 1.0, additive);
    let rgb = clamp(source.rgb * source_factor + dest * dest_factor, vec3f(0.0), vec3f(1.0));
    return vec4f(floor(rgb * maximum + vec3f(0.5)) / 255.0, 1.0);
}
"#;

const PRESENT: &str = r#"
@group(0) @binding(0) var previous: texture_2d<f32>;
struct Out { @builtin(position) pos: vec4f, @location(0) uv: vec2f };
@vertex fn vs(@builtin(vertex_index) vertex: u32) -> Out {
    var out: Out;
    let point = vec2f(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    out.pos = vec4f(point * 2.0 - vec2f(1.0), 0.0, 1.0);
    out.uv = vec2f(point.x, 1.0 - point.y);
    return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4f {
    let point = clamp(vec2i(in.uv * vec2f(640.0, 480.0)), vec2i(0), vec2i(639, 479));
    let rgb = round(textureLoad(previous, point, 0).rgb * 255.0) / vec3f(31.0, 63.0, 31.0);
    return vec4f(rgb, 1.0);
}
"#;

pub(super) fn texture_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("original 16-bit texture"),
        entries: &[
            texture_entry(),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}
fn texture_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(super) fn texture(
    engine: &Engine,
    rgba: &[u8],
    width: u32,
    height: u32,
    format: u32,
) -> wgpu::BindGroup {
    let levels = match format {
        2 => [31u32, 31, 31, 1],
        3 | 4 => [31, 63, 31, 1],
        _ => [15, 15, 15, 15],
    };
    let bytes: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|pixel| {
            std::array::from_fn::<_, 4, _>(|channel| {
                if channel == 3 && (format == 3 || format == 4) {
                    return 1;
                }
                pixel[channel]
                    >> match levels[channel] {
                        63 => 2,
                        31 => 3,
                        15 => 4,
                        _ => 7,
                    }
            })
        })
        .collect();
    let texture = color_texture(engine, width, height, "16-bit texture codes");
    engine.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    let levels = levels.map(|level| level as f32);
    let buffer = engine.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("16-bit texture channel sizes"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    engine
        .queue
        .write_buffer(&buffer, 0, bytemuck::cast_slice(&levels));
    engine.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &engine.bind_layout_16bit,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffer.as_entire_binding(),
            },
        ],
    })
}

fn color_texture(engine: &Engine, width: u32, height: u32, label: &str) -> wgpu::Texture {
    engine.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

pub(super) struct Renderer {
    color: wgpu::Texture,
    view: wgpu::TextureView,
    previous: wgpu::Texture,
    previous_bind: wgpu::BindGroup,
    color_bind: wgpu::BindGroup,
    previous_layout: wgpu::BindGroupLayout,
    depth: wgpu::TextureView,
    sprite: [wgpu::RenderPipeline; 2],
    background: [wgpu::RenderPipeline; 2],
    present_shader: wgpu::ShaderModule,
    present: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Renderer {
    pub(super) fn new(engine: &Engine) -> Self {
        let device = &engine.device;
        let color = color_texture(engine, SCREEN_W, SCREEN_H, "R5G6B5 back buffer codes");
        let view = color.create_view(&Default::default());
        let previous = color_texture(engine, SCREEN_W, SCREEN_H, "R5G6B5 previous draw");
        let previous_view = previous.create_view(&Default::default());
        let previous_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("previous R5G6B5 buffer"),
            entries: &[texture_entry()],
        });
        let bind = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &previous_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                }],
            })
        };
        let color_bind = bind(&view);
        let previous_bind = bind(&previous_view);
        let make_pipeline = |background: bool, additive: bool| {
            let original = if background {
                super::BG_SHADER
            } else {
                super::SHADER
            };
            let mut source = original.replace("@group(0) @binding(0) var tex: texture_2d<f32>;\n@group(0) @binding(1) var samp: sampler;", "");
            source = source.replace("textureSample(tex, samp, in.uv)", "original_sample(in.uv)");
            if background {
                source = source.replace(
                    "return vec4f(c.rgb * c.a * f, c.a);",
                    "return compose(in.pos.xy, vec4f(c.rgb * c.a * f, c.a), true, true);",
                );
                source = source.replace(
                    "return vec4f(rgb, c.a);",
                    "return compose(in.pos.xy, vec4f(rgb, c.a), false, false);",
                );
                source.push_str(SAMPLE);
            } else {
                source = source.replace(
                    "return color;",
                    &format!("return compose(in.pos.xy, color, {additive}, false);"),
                );
                source.push_str(&SAMPLE.replace("@group(2)", "@group(1)"));
            }
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("R5G6B5 draw"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let bind_group_layouts = if background {
                vec![
                    &engine.bind_layout_16bit,
                    &engine.bg_uniform_layout,
                    &previous_layout,
                ]
            } else {
                vec![&engine.bind_layout_16bit, &previous_layout]
            };
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &bind_group_layouts,
                push_constant_ranges: &[],
            });
            let sprite_attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Uint32];
            let bg_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32, 3 => Float32x4];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("R5G6B5 draw"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: if background {
                            std::mem::size_of::<Vertex3>()
                        } else {
                            std::mem::size_of::<Vertex>()
                        } as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: if background {
                            &bg_attributes
                        } else {
                            &sprite_attributes
                        },
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: background.then_some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: !additive,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let sprite = [make_pipeline(false, false), make_pipeline(false, true)];
        let background = [make_pipeline(true, false), make_pipeline(true, true)];
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("R5G6B5 display"),
            source: wgpu::ShaderSource::Wgsl(PRESENT.into()),
        });
        Self {
            color,
            view,
            previous,
            previous_bind,
            color_bind,
            previous_layout,
            depth: engine.depth_texture(SCREEN_W, SCREEN_H),
            sprite,
            background,
            present_shader,
            present: HashMap::new(),
        }
    }

    fn copy_previous(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_texture(
            self.color.as_image_copy(),
            self.previous.as_image_copy(),
            wgpu::Extent3d {
                width: SCREEN_W,
                height: SCREEN_H,
                depth_or_array_layers: 1,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode(
        &mut self,
        engine: &Engine,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        cmds: &[DrawCmd],
        textures: &[&Texture],
        bg: Option<&BgScene>,
    ) {
        let clear = bg
            .map(|scene| scene.fog_color)
            .unwrap_or([0.0, 0.0, 0.0, 1.0]);
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("R5G6B5 clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from((clear[0] * 31.0 + 0.5).floor() / 255.0),
                            g: f64::from((clear[1] * 63.0 + 0.5).floor() / 255.0),
                            b: f64::from((clear[2] * 31.0 + 0.5).floor() / 255.0),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        if let Some(scene) = bg {
            for additive in [false, true] {
                let verts = if additive {
                    &scene.verts_add
                } else {
                    &scene.verts
                };
                if verts.is_empty() {
                    continue;
                }
                let vbuf = engine.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: std::mem::size_of_val(verts.as_slice()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                engine
                    .queue
                    .write_buffer(&vbuf, 0, bytemuck::cast_slice(verts));
                let uniform = BgUniform {
                    mvp: scene.mvp,
                    fog_color: scene.fog_color,
                    fog_params: [scene.fog_near, scene.fog_far, f32::from(additive), 0.0],
                };
                let ubuf = engine.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: std::mem::size_of::<BgUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                engine
                    .queue
                    .write_buffer(&ubuf, 0, bytemuck::bytes_of(&uniform));
                let uniform = engine.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &engine.bg_uniform_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    }],
                });
                let matrix = glam::Mat4::from_cols_array_2d(&scene.mvp);
                let bounds: Vec<_> = verts
                    .chunks(6)
                    .map(|quad| {
                        let projected: Vec<_> = quad
                            .iter()
                            .map(|vertex| {
                                matrix
                                    * glam::Vec4::new(
                                        vertex.pos[0],
                                        vertex.pos[1],
                                        vertex.pos[2],
                                        1.0,
                                    )
                            })
                            .collect();
                        if projected.iter().any(|point| point.w <= 0.0) {
                            return [0.0, 0.0, 640.0, 480.0];
                        }
                        rectangle(projected.iter().map(|point| {
                            [
                                scene.arcade[0] + (point.x / point.w + 1.0) * scene.arcade[2] / 2.0,
                                scene.arcade[1] + (1.0 - point.y / point.w) * scene.arcade[3] / 2.0,
                            ]
                        }))
                    })
                    .collect();
                for (start, end) in batches(&bounds) {
                    self.copy_previous(encoder);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("R5G6B5 background"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &self.depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    let [x, y, width, height] = scene.arcade;
                    pass.set_viewport(x, y, width, height, 0.0, 1.0);
                    pass.set_pipeline(&self.background[usize::from(additive)]);
                    pass.set_bind_group(0, &textures[scene.tex].bind_group_16bit, &[]);
                    pass.set_bind_group(1, &uniform, &[]);
                    pass.set_bind_group(2, &self.previous_bind, &[]);
                    pass.set_vertex_buffer(0, vbuf.slice(..));
                    pass.draw(
                        start as u32 * 6..(end as u32 * 6).min(verts.len() as u32),
                        0..1,
                    );
                }
            }
        }
        let vertices = Engine::build_vertices(cmds);
        let bounds: Vec<_> =
            vertices
                .chunks(6)
                .map(|quad| {
                    rectangle(quad.iter().map(|vertex| {
                        [(vertex.pos[0] + 1.0) * 320.0, (1.0 - vertex.pos[1]) * 240.0]
                    }))
                })
                .collect();
        let vbuf = engine.vertex_buffer(cmds);
        for (start, end) in batches(&bounds) {
            self.copy_previous(encoder);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("R5G6B5 sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_bind_group(1, &self.previous_bind, &[]);
            for (index, cmd) in cmds.iter().enumerate().take(end).skip(start) {
                pass.set_pipeline(&self.sprite[usize::from(cmd.additive)]);
                pass.set_bind_group(0, &textures[cmd.tex].bind_group_16bit, &[]);
                pass.draw(index as u32 * 6..index as u32 * 6 + 6, 0..1);
            }
        }
        let pipeline = self.present.entry(format).or_insert_with(|| {
            let layout = engine
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&self.previous_layout],
                    push_constant_ranges: &[],
                });
            engine
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("R5G6B5 display"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &self.present_shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &self.present_shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview: None,
                    cache: None,
                })
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("R5G6B5 display"),
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
        let [x, y, width, height] = super::display_viewport(size);
        pass.set_viewport(x, y, width, height, 0.0, 1.0);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.color_bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn rectangle(points: impl Iterator<Item = [f32; 2]>) -> [f32; 4] {
    points.fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |bounds, point| {
            [
                bounds[0].min(point[0]),
                bounds[1].min(point[1]),
                bounds[2].max(point[0]),
                bounds[3].max(point[1]),
            ]
        },
    )
}
fn batches(bounds: &[[f32; 4]]) -> Vec<(usize, usize)> {
    let mut batches = Vec::new();
    let mut start = 0;
    for (index, current) in bounds.iter().enumerate() {
        // A 1-pixel conservative guard includes rasterisation's edge rules.
        if bounds[start..index].iter().any(|other| {
            current[0] <= other[2] + 1.0
                && current[2] + 1.0 >= other[0]
                && current[1] <= other[3] + 1.0
                && current[3] + 1.0 >= other[1]
        }) {
            batches.push((start, index));
            start = index;
        }
    }
    if start < bounds.len() {
        batches.push((start, bounds.len()));
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batches_never_combine_overlapping_quads() {
        assert_eq!(
            batches(&[
                [0.0, 0.0, 10.0, 10.0],
                [20.0, 0.0, 30.0, 10.0],
                [5.0, 0.0, 8.0, 10.0]
            ]),
            vec![(0, 2), (2, 3)]
        );
    }
    #[test]
    #[ignore = "requires GPU adapter"]
    fn actual_gpu_rounds_every_blend_and_uses_original_texture_precision() {
        super::super::set_color_16bit(true);
        let engine = Engine::new();
        let texture = engine.create_texture_with_format(&[255, 0, 0, 255], 1, 1, 1);
        let mut cmd = DrawCmd {
            tex: 0,
            dst: [0.0, 0.0, 640.0, 480.0],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 0.5],
            rot: 0.0,
            additive: false,
            color_add: false,
        };
        let pixels = engine.render_to_image(&[cmd, cmd], &[&texture], None);
        assert_eq!(&pixels[..4], &[197, 0, 0, 255]); // 16/31 after first draw; 24/31 after second.
        cmd.additive = true;
        let pixels = engine.render_to_image(&[cmd, cmd], &[&texture], None);
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
        let texture = engine.create_texture_with_format(&[15, 31, 63, 255], 1, 1, 1);
        cmd.tint[3] = 1.0;
        cmd.additive = false;
        let pixels = engine.render_to_image(&[cmd], &[&texture], None);
        assert_eq!(&pixels[..4], &[0, 16, 49, 255]); // A4R4G4B4 source codes 0,1,3.
        let texture = engine.create_texture_with_format(&[15, 31, 63, 255], 1, 1, 4);
        let pixels = engine.render_to_image(&[cmd], &[&texture], None);
        assert_eq!(&pixels[..4], &[8, 28, 58, 255]); // R5G6B5 source codes 1,7,7.
        super::super::set_color_16bit(false);
    }

    #[test]
    #[ignore = "requires GPU adapter"]
    fn texture_stage_color_add_preserves_alpha_in_both_display_modes() {
        let engine = Engine::new();
        let texture = engine.create_texture_with_format(&[64, 96, 128, 128], 1, 1, 1);
        let cmd = DrawCmd {
            tex: 0,
            dst: [0.0, 0.0, 640.0, 480.0],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [0.25, 0.125, 0.75, 0.5],
            rot: 0.0,
            additive: false,
            color_add: true,
        };
        super::super::set_color_16bit(false);
        let pixels = engine.render_to_image(&[cmd], &[&texture], None);
        assert_eq!(&pixels[..4], &[32, 32, 64, 255]);
        super::super::set_color_16bit(true);
        let pixels = engine.render_to_image(&[cmd], &[&texture], None);
        assert_eq!(&pixels[..4], &[33, 36, 66, 255]);
        super::super::set_color_16bit(false);
    }

    #[test]
    #[ignore = "requires GPU adapter"]
    fn background_alpha_and_additive_share_quantized_depth_buffer() {
        super::super::set_color_16bit(true);
        let engine = Engine::new();
        let texture = engine.create_texture_with_format(&[255, 0, 0, 255], 1, 1, 1);
        let points = [
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ];
        let quad: Vec<_> = points
            .iter()
            .map(|p| Vertex3 {
                pos: [p[0], p[1], 0.5],
                uv: [0.5, 0.5],
                depth: 0.0,
                color: [1.0, 1.0, 1.0, 0.5],
            })
            .collect();
        let mut scene = BgScene {
            mvp: glam::Mat4::IDENTITY.to_cols_array_2d(),
            fog_color: [0.0, 0.0, 0.0, 1.0],
            fog_near: 0.0,
            fog_far: 1.0,
            verts: quad.clone(),
            verts_add: quad.clone(),
            tex: 0,
            arcade: [0.0, 0.0, 640.0, 480.0],
        };
        let pixels = engine.render_to_image(&[], &[&texture], Some(&scene));
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
        scene.verts_add.clear();
        scene.verts.extend(quad);
        let pixels = engine.render_to_image(&[], &[&texture], Some(&scene));
        assert_eq!(&pixels[..4], &[197, 0, 0, 255]);
        super::super::set_color_16bit(false);
    }
}
