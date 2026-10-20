//! The original default presentation preserves the color back buffer between
//! draw chains. Keep it at 640×480 and only scale the completed image when a
//! window requests a refresh; refresh rate must not repeat alpha composition.
use super::{BgScene, DrawCmd, Engine, SCREEN_H, SCREEN_W, Texture};
use std::collections::HashMap;

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
    return vec4f(textureLoad(previous, point, 0).rgb, 1.0);
}
"#;

pub(super) struct Renderer {
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    sprite: [wgpu::RenderPipeline; 8],
    background: [wgpu::RenderPipeline; 4],
    initialized: bool,
    present_layout: wgpu::BindGroupLayout,
    present_bind: wgpu::BindGroup,
    present_shader: wgpu::ShaderModule,
    present: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Renderer {
    pub(super) fn new(engine: &Engine) -> Self {
        let color = engine.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("persistent 32-bit back buffer"),
            size: wgpu::Extent3d {
                width: SCREEN_W,
                height: SCREEN_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = color.create_view(&Default::default());
        let present_layout =
            engine
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("back buffer display"),
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
        let present_bind = engine.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("back buffer display"),
            layout: &present_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        let present_shader = engine
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("back buffer display"),
                source: wgpu::ShaderSource::Wgsl(PRESENT.into()),
            });
        Self {
            view,
            depth: engine.depth_texture(SCREEN_W, SCREEN_H),
            sprite: engine.make_sprite_pipelines(wgpu::TextureFormat::Rgba8Unorm),
            background: std::array::from_fn(|index| {
                engine.make_bg_pipeline_mode(
                    wgpu::TextureFormat::Rgba8Unorm,
                    index & 2 != 0,
                    index & 1 != 0,
                )
            }),
            initialized: false,
            present_layout,
            present_bind,
            present_shader,
            present: HashMap::new(),
        }
    }

    pub(super) fn draw(
        &mut self,
        engine: &Engine,
        encoder: &mut wgpu::CommandEncoder,
        cmds: &[DrawCmd],
        textures: &[&Texture],
        bg: Option<&BgScene>,
    ) {
        if let Some(scene) = bg {
            engine.encode_bg(
                encoder,
                &self.view,
                &self.depth,
                &self.background,
                scene,
                textures,
                (SCREEN_W, SCREEN_H),
                self.initialized,
            );
        }
        let vertices = engine.vertex_buffer(cmds);
        engine.encode_pass(
            encoder,
            &self.view,
            &self.depth,
            &self.sprite,
            &vertices,
            cmds,
            textures,
            self.initialized || bg.is_some(),
            bg.is_some(),
            (SCREEN_W, SCREEN_H),
        );
        self.initialized = true;
    }

    pub(super) fn present(
        &mut self,
        engine: &Engine,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) {
        let pipeline = self.present.entry(format).or_insert_with(|| {
            let layout = engine
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&self.present_layout],
                    push_constant_ranges: &[],
                });
            engine
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("back buffer display"),
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
            label: Some("back buffer display"),
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
        pass.set_bind_group(0, &self.present_bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires GPU adapter"]
    fn interleaved_background_blends_preserve_source_order_in_both_modes() {
        use super::super::{BgDrawRange, Vertex3};
        let engine = Engine::new();
        let texture = engine.create_texture_with_format(&[255; 4], 1, 1, 1);
        let quad = |color| -> Vec<Vertex3> {
            [
                [-1.0, -1.0],
                [1.0, -1.0],
                [1.0, 1.0],
                [-1.0, -1.0],
                [1.0, 1.0],
                [-1.0, 1.0],
            ]
            .map(|p| Vertex3 {
                pos: [p[0], p[1], 0.5],
                uv: [0.5; 2],
                depth: 0.0,
                color,
            })
            .to_vec()
        };
        let mut alpha = quad([1.0, 0.0, 0.0, 1.0]);
        alpha.extend(quad([0.0, 0.0, 1.0, 1.0]));
        let mut scene = BgScene {
            mvp: glam::Mat4::IDENTITY.to_cols_array_2d(),
            fog_color: [0.0, 0.0, 0.0, 1.0],
            fog_near: 100.0,
            fog_far: 200.0,
            verts: alpha,
            verts_add: quad([0.0, 1.0, 0.0, 1.0]),
            draw_ranges: Vec::new(),
            tex: 0,
            arcade: [0.0, 0.0, 640.0, 480.0],
        };
        for limited in [false, true] {
            super::super::set_color_16bit(limited);
            // Former regrouping draws red -> blue -> additive green: cyan.
            scene.draw_ranges.clear();
            let legacy = engine.render_to_image(&[], &[&texture], Some(&scene));
            let center = ((240 * SCREEN_W + 320) * 4) as usize;
            assert_eq!(
                &legacy[center..center + 4],
                &[0, 255, 255, 255],
                "legacy mode={limited}"
            );
            // Stage.cpp draws each quad immediately: alpha red -> additive
            // green -> alpha blue. The final opaque VM must overwrite green.
            scene.draw_ranges = vec![
                BgDrawRange {
                    additive: false,
                    write_depth: true,
                    start: 0,
                    end: 6,
                },
                BgDrawRange {
                    additive: true,
                    write_depth: true,
                    start: 0,
                    end: 6,
                },
                BgDrawRange {
                    additive: false,
                    write_depth: true,
                    start: 6,
                    end: 12,
                },
            ];
            let ordered = engine.render_to_image(&[], &[&texture], Some(&scene));
            assert_eq!(
                &ordered[center..center + 4],
                &[0, 0, 255, 255],
                "ordered mode={limited}"
            );
            for (first_additive, first_write, expected) in [
                (true, true, [255, 0, 0, 255]),
                (true, false, [0, 0, 255, 255]),
                (false, false, [255, 0, 255, 255]),
                (false, true, [255, 0, 0, 255]),
            ] {
                let mut near = quad([1.0, 0.0, 0.0, 1.0]);
                let mut far = quad([0.0, 0.0, 1.0, 1.0]);
                for vertex in &mut near {
                    vertex.pos[2] = 0.2;
                }
                for vertex in &mut far {
                    vertex.pos[2] = 0.8;
                }
                if first_additive {
                    scene.verts_add = near;
                    scene.verts = far;
                } else {
                    scene.verts = near;
                    scene.verts_add = far;
                }
                scene.draw_ranges = vec![
                    BgDrawRange {
                        additive: first_additive,
                        write_depth: first_write,
                        start: 0,
                        end: 6,
                    },
                    BgDrawRange {
                        additive: !first_additive,
                        write_depth: first_additive,
                        start: 0,
                        end: 6,
                    },
                ];
                let image = engine.render_to_image(&[], &[&texture], Some(&scene));
                assert_eq!(
                    &image[center..center + 4],
                    &expected,
                    "ZWRITE mode={limited} additive={first_additive} write={first_write}"
                );
            }
            // Restore pools for the legacy-order check in the next mode.
            scene.verts = quad([1.0, 0.0, 0.0, 1.0]);
            scene.verts.extend(quad([0.0, 0.0, 1.0, 1.0]));
            scene.verts_add = quad([0.0, 1.0, 0.0, 1.0]);
        }
        super::super::set_color_16bit(false);
    }

    #[test]
    #[ignore = "requires GPU adapter"]
    fn projected_corners_keep_perspective_uv_depth_and_far_clipping_in_both_modes() {
        use super::super::{DepthState, DepthTest};
        let engine = Engine::new();
        let rgba: Vec<u8> = (0..=255u8).flat_map(|r| [r, 0, 0, 255]).collect();
        let texture = engine.create_texture(&rgba, 256, 1);
        let mut cmd = DrawCmd {
            projected: Some([
                [100.0, 100.0, 0.5, 1.0],
                [600.0, 200.0, 1.0, 2.0],
                [600.0, 600.0, 1.0, 2.0],
                [100.0, 300.0, 0.5, 1.0],
            ]),
            tex: 0,
            dst: [100.0, 100.0, 200.0, 200.0],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0; 4],
            rot: 0.0,
            additive: false,
            color_add: false,
            clip: None,
            depth: Some(DepthState {
                z: 0.0,
                test: DepthTest::LessEqual,
                write: true,
            }),
        };
        let pixel = (150 * SCREEN_W as usize + 200) * 4;
        for limited in [false, true] {
            super::super::set_color_16bit(limited);
            let image = engine.render_to_image(&[cmd], &[&texture], None);
            // At screen midpoint U is approximately1/3, not affine1/2.
            assert!(
                (78..=90).contains(&image[pixel]),
                "mode={limited} red={}",
                image[pixel]
            );
            let mut overlay = cmd;
            overlay.projected = None;
            overlay.tint = [1.0, 0.0, 0.0, 1.0];
            overlay.depth.as_mut().unwrap().z = 0.3;
            let occluded = engine.render_to_image(&[cmd, overlay], &[&texture], None);
            assert_eq!(occluded[pixel], image[pixel], "projected depth must be0.25");
            let corners = cmd.projected.unwrap();
            cmd.projected = Some(corners.map(|mut p| {
                p[2] = p[3] * 1.1;
                p
            }));
            let clipped = engine.render_to_image(&[cmd], &[&texture], None);
            assert_eq!(
                clipped[pixel], 0,
                "far clip must use full clip Z before viewport depth mapping"
            );
            cmd.projected = Some(corners);
        }
        super::super::set_color_16bit(false);
    }

    #[test]
    #[ignore = "requires GPU adapter"]
    fn sprite_depth_test_write_alpha_and_frame_reset_match_in_both_modes() {
        use super::super::{DepthState, DepthTest, Vertex3};
        let engine = Engine::new();
        let texture = engine.create_texture(&[255, 255, 255, 255], 1, 1);
        let mut commands = Vec::new();
        let red = [1.0, 0.0, 0.0, 1.0];
        let green = [0.0, 1.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        use DepthTest::{Always, LessEqual};
        {
            let mut quad = |column, tint, z, test, write, additive| {
                commands.push(DrawCmd {
                    projected: None,
                    clip: None,
                    tex: 0,
                    dst: [16.0 + column as f32 * 64.0, 32.0, 48.0, 48.0],
                    src: [0.0, 0.0, 1.0, 1.0],
                    tint,
                    rot: 0.0,
                    additive,
                    color_add: false,
                    depth: Some(DepthState { z, test, write }),
                });
            };
            // Behind an opaque depth-writing sprite.
            quad(0, red, 0.2, LessEqual, true, false);
            quad(0, blue, 0.8, LessEqual, true, false);
            // A successful no-write sprite must not occlude later geometry.
            quad(1, green, 0.2, LessEqual, false, false);
            quad(1, blue, 0.8, LessEqual, true, false);
            // Alpha-test failures must leave depth untouched.
            quad(2, [1.0, 0.0, 0.0, 3.0 / 255.0], 0.2, LessEqual, true, false);
            quad(2, blue, 0.8, LessEqual, true, false);
            // ALWAYS still writes when requested, replacing the nearer depth.
            quad(3, red, 0.2, LessEqual, true, false);
            quad(3, blue, 0.8, Always, true, false);
            quad(3, green, 0.4, LessEqual, true, false);
            // ALWAYS with writes disabled preserves the preceding nearer depth.
            quad(4, red, 0.2, LessEqual, true, false);
            quad(4, blue, 0.8, Always, false, false);
            quad(4, green, 0.4, LessEqual, true, false);
            // Additive blending has the same test/write rules.
            quad(5, red, 0.2, LessEqual, true, false);
            quad(5, blue, 0.8, LessEqual, false, true);
            quad(5, green, 0.1, LessEqual, false, true);
            quad(6, red, 0.4, LessEqual, true, false);
            quad(6, blue, 0.4, LessEqual, true, false);
            // ALPHAREF itself passes and writes, even if RGB565 rounds to black.
            quad(7, [1.0, 0.0, 0.0, 4.0 / 255.0], 0.2, LessEqual, true, false);
            quad(7, blue, 0.8, LessEqual, true, false);
            quad(8, red, 0.2, LessEqual, true, false);
        }
        let mut overlay = commands[0];
        overlay.dst[0] += 8.0 * 64.0;
        overlay.tint = blue;
        overlay.depth = None;
        commands.push(overlay);
        overlay.tint = green;
        overlay.depth = Some(DepthState {
            z: 0.8,
            test: LessEqual,
            write: true,
        });
        commands.push(overlay);
        for limited in [false, true] {
            super::super::set_color_16bit(limited);
            let image = engine.render_to_image(&commands, &[&texture], None);
            let expected = [
                [255, 0, 0],
                [0, 0, 255],
                [0, 0, 255],
                [0, 255, 0],
                [0, 0, 255],
                [255, 255, 0],
                [0, 0, 255],
                if limited { [0, 0, 0] } else { [4, 0, 0] },
                [0, 0, 255],
            ];
            for (column, rgb) in expected.iter().enumerate() {
                let offset = ((48 * SCREEN_W + 32 + column as u32 * 64) * 4) as usize;
                assert_eq!(
                    &image[offset..offset + 3],
                    rgb,
                    "mode={limited} case={column}"
                );
            }
            // The original requests D16. Nearly coplanar enemy layers must
            // compare equal after depth quantization, rather than retaining
            // a binary32 distinction that incorrectly rejects the later one.
            let mut close = commands[0];
            close.dst = [16.0, 120.0, 48.0, 48.0];
            close.depth.as_mut().unwrap().z = 0.4;
            let mut later = close;
            later.tint = blue;
            later.depth.as_mut().unwrap().z = 0.400_000_1;
            let image = engine.render_to_image(&[close, later], &[&texture], None);
            let offset = ((136 * SCREEN_W + 32) * 4) as usize;
            assert_eq!(
                &image[offset..offset + 3],
                &[0, 0, 255],
                "D16 mode={limited}"
            );

            let mut next = commands[0];
            next.dst = [0.0, 0.0, SCREEN_W as f32, SCREEN_H as f32];
            next.tint = green;
            next.depth.as_mut().unwrap().z = 0.9;
            engine.advance_frame(&[next], &[&texture], None);
            assert_eq!(&engine.read_frame()[..4], &[0, 255, 0, 255]);

            // Background writes use the same attachment as sprites. Its
            // original viewport range .5..1 maps clip-space .2 to depth .6.
            let verts = [
                [-1.0, -1.0],
                [1.0, -1.0],
                [1.0, 1.0],
                [-1.0, -1.0],
                [1.0, 1.0],
                [-1.0, 1.0],
            ]
            .map(|p| Vertex3 {
                pos: [p[0], p[1], 0.2],
                uv: [0.5, 0.5],
                depth: 0.0,
                color: red,
            });
            let scene = BgScene {
                mvp: glam::Mat4::IDENTITY.to_cols_array_2d(),
                fog_color: [0.0; 4],
                fog_near: 100.0,
                fog_far: 200.0,
                verts: verts.to_vec(),
                verts_add: Vec::new(),
                draw_ranges: Vec::new(),
                tex: 0,
                arcade: [0.0, 0.0, SCREEN_W as f32, SCREEN_H as f32],
            };
            next.tint = blue;
            next.depth.as_mut().unwrap().z = 0.7;
            let image = engine.render_to_image(&[next], &[&texture], Some(&scene));
            assert_eq!(&image[40 * SCREEN_W as usize * 4..][..4], &[255, 0, 0, 255]);
            next.tint = green;
            next.depth.as_mut().unwrap().z = 0.55;
            let image = engine.render_to_image(&[next], &[&texture], Some(&scene));
            assert_eq!(&image[40 * SCREEN_W as usize * 4..][..4], &[0, 255, 0, 255]);
        }
        super::super::set_color_16bit(false);
    }

    #[test]
    #[ignore = "requires GPU adapter"]
    fn history_accumulates_each_draw_once_and_readback_never_redraws() {
        let engine = Engine::new();
        let texture = engine.create_texture(&[255, 255, 255, 255], 1, 1);
        let mut command = DrawCmd {
            projected: None,
            clip: None,
            tex: 0,
            dst: [0.0, 0.0, 640.0, 480.0],
            src: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 0.0, 0.0, 1.0],
            rot: 0.0,
            additive: false,
            color_add: false,
            depth: None,
        };
        for limited in [false, true] {
            super::super::set_color_16bit(limited);
            super::super::PRESENTED_FRAMES.with(|frames| frames.set(Some(0)));
            engine.reset_frame_history();
            command.tint = [1.0, 0.0, 0.0, 1.0];
            engine.advance_frame(&[command], &[&texture], None);
            assert_eq!(&engine.read_frame()[..4], &[255, 0, 0, 255]);
            engine.advance_frame(&[], &[&texture], None);
            assert_eq!(&engine.read_frame()[..4], &[255, 0, 0, 255]);
            command.tint = [0.0, 1.0, 0.0, 0.5];
            engine.advance_frame(&[command], &[&texture], None);
            let expected = if limited {
                [132, 130, 0, 255]
            } else {
                [128, 128, 0, 255]
            };
            let pixels = engine.read_frame();
            for (actual, expected) in pixels[..4].iter().zip(expected) {
                assert!(actual.abs_diff(expected) <= u8::from(!limited));
            }
            assert_eq!(engine.read_frame(), pixels);
            assert_eq!(super::super::presentation_clock().unwrap().1, 4);
            engine.advance_frame(&[command], &[&texture], None);
            let expected = if limited {
                [66, 195, 0, 255]
            } else {
                [64, 192, 0, 255]
            };
            for (actual, expected) in engine.read_frame()[..4].iter().zip(expected) {
                assert!(actual.abs_diff(expected) <= u8::from(!limited));
            }
            // Four draw chains plus the pending calculation; the repeated
            // readback blits above have not advanced the original FPS clock.
            assert_eq!(super::super::presentation_clock().unwrap().1, 5);
        }
        super::super::PRESENTED_FRAMES.with(|frames| frames.set(None));
        super::super::set_color_16bit(false);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "requires GPU adapter and process memory inspection; 6000 draw chains"]
    fn thousands_of_unselected_frames_keep_gpu_resources_bounded() {
        fn resident_bytes() -> u64 {
            let output = std::process::Command::new("/bin/ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .expect("read current process memory");
            assert!(
                output.status.success(),
                "process memory inspection unavailable"
            );
            std::str::from_utf8(&output.stdout)
                .unwrap()
                .trim()
                .parse::<u64>()
                .unwrap()
                * 1024
        }
        let engine = Engine::new();
        let texture = engine.create_texture(&[255, 255, 255, 255], 1, 1);
        // Full overlap forces individual 16-bit composition passes. No image
        // readback is permitted in this loop: it used to hide the queue growth.
        let commands = vec![
            DrawCmd {
                projected: None,
                clip: None,
                tex: 0,
                dst: [0.0, 0.0, 640.0, 480.0],
                src: [0.0, 0.0, 1.0, 1.0],
                tint: [0.3, 0.6, 0.9, 0.5],
                rot: 0.0,
                additive: false,
                color_add: false,
                depth: None,
            };
            32
        ];
        for limited in [false, true] {
            super::super::set_color_16bit(limited);
            engine.reset_frame_history();
            for _ in 0..100 {
                engine.advance_frame(&commands, &[&texture], None);
            }
            let baseline = resident_bytes();
            let mut samples = Vec::new();
            for frame in 1..=3000 {
                engine.advance_frame(&commands, &[&texture], None);
                assert!(
                    engine
                        .frames_in_flight
                        .load(std::sync::atomic::Ordering::Relaxed)
                        <= 1
                );
                if frame % 500 == 0 {
                    samples.push(resident_bytes());
                }
            }
            let peak = samples.iter().copied().max().unwrap();
            eprintln!(
                "color_bits={} warm_rss={} samples_rss={samples:?} peak_growth={}",
                if limited { 16 } else { 32 },
                baseline,
                peak.saturating_sub(baseline)
            );
            // Allow driver/allocator warm caches, but reject the old unbounded
            // multi-gigabyte growth and any large sustained resource backlog.
            assert!(
                peak <= baseline + 128 * 1024 * 1024,
                "GPU submission memory grew beyond its bounded warm cache"
            );
        }
        super::super::set_color_16bit(false);
    }
}
