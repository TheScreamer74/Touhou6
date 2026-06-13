//! Stage 3D background: drives the STD camera/fog script and builds the
//! textured-quad scene each frame. Mirrors the decompilation's Stage
//! OnUpdate (camera position keys + facing) and RenderObjects (quads
//! placed in world space, projected by a 30-degree LH perspective camera).

use std::collections::HashMap;

use glam::{Mat4, Vec3};
use th06_engine::{BgScene, Vertex3};
use th06_formats::anm0::Entry;
use th06_formats::std::Std;

const FIELD_W: f32 = 384.0;
const FIELD_H: f32 = 448.0;

pub struct Background {
    std: Std,
    /// anm script id -> (sprite pixel rect [x, y, w, h] in stg1bg, scale
    /// [sx, sy] from the script's op2). Tiles are authored at 16x16 but the
    /// floor scripts scale them 2x, so the quads tile seamlessly.
    sprites: HashMap<i32, ([f32; 4], [f32; 2])>,
    tex_size: [f32; 2],
    tex_slot: usize,

    time: f32,
    script_idx: usize,
    cam: Vec3,
    cam_init: Vec3,
    cam_final: Vec3,
    interp_start: f32,
    interp_end: f32,
    facing: Vec3,
    fog_color: [f32; 4],
    fog_near: f32,
    fog_far: f32,
}

fn fbits(i: i32) -> f32 {
    f32::from_bits(i as u32)
}

fn color_argb(c: i32) -> [f32; 4] {
    let u = c as u32;
    [
        ((u >> 16) & 0xff) as f32 / 255.0,
        ((u >> 8) & 0xff) as f32 / 255.0,
        (u & 0xff) as f32 / 255.0,
        ((u >> 24) & 0xff) as f32 / 255.0,
    ]
}

impl Background {
    pub fn new(std: Std, bg: &Entry, tex_slot: usize) -> Self {
        // Each stg1bg script sets one sprite (op 1); collect that mapping.
        let sprite_tbl: HashMap<u32, [f32; 4]> = bg
            .sprites
            .iter()
            .map(|s| (s.index, [s.x, s.y, s.width, s.height]))
            .collect();
        let mut sprites = HashMap::new();
        for (id, instrs) in &bg.scripts {
            if let Some(i) = instrs.iter().find(|i| i.opcode == 1) {
                let sp = u32::from_le_bytes(i.args[0..4].try_into().unwrap());
                if let Some(rect) = sprite_tbl.get(&sp) {
                    let scale = instrs
                        .iter()
                        .find(|i| i.opcode == 2)
                        .map(|i| {
                            [
                                f32::from_le_bytes(i.args[0..4].try_into().unwrap()),
                                f32::from_le_bytes(i.args[4..8].try_into().unwrap()),
                            ]
                        })
                        .unwrap_or([1.0, 1.0]);
                    sprites.insert(*id as i32, (*rect, scale));
                }
            }
        }
        Self {
            std,
            sprites,
            tex_size: [bg.width as f32, bg.height as f32],
            tex_slot,
            time: 0.0,
            script_idx: 0,
            cam: Vec3::ZERO,
            cam_init: Vec3::ZERO,
            cam_final: Vec3::ZERO,
            interp_start: 0.0,
            interp_end: 1.0,
            facing: Vec3::new(0.0, 0.0, 1.0),
            fog_color: [0.05, 0.05, 0.12, 1.0],
            fog_near: 200.0,
            fog_far: 3000.0,
        }
    }

    pub fn tick(&mut self) {
        // Process due script instructions (subset: position key, facing, fog).
        loop {
            let Some(ins) = self.std.script.get(self.script_idx) else { break };
            if ins.frame < 0 {
                break;
            }
            if (self.time as i32) < ins.frame {
                break;
            }
            match ins.opcode {
                0 => {
                    // CAMERA_POSITION_KEY: set current key, then scan ahead
                    // for the next key to interpolate toward.
                    let pos = Vec3::new(fbits(ins.args[0]), fbits(ins.args[1]), fbits(ins.args[2]));
                    self.cam = pos;
                    self.cam_init = pos;
                    self.interp_start = ins.frame as f32;
                    self.interp_end = ins.frame as f32 + 1.0;
                    self.cam_final = pos;
                    for next in &self.std.script[self.script_idx + 1..] {
                        if next.opcode == 0 {
                            self.interp_end = next.frame as f32;
                            self.cam_final =
                                Vec3::new(fbits(next.args[0]), fbits(next.args[1]), fbits(next.args[2]));
                            break;
                        }
                    }
                    self.script_idx += 1;
                }
                1 => {
                    // FOG: color, near, far.
                    self.fog_color = color_argb(ins.args[0]);
                    self.fog_color[3] = 1.0;
                    self.fog_near = fbits(ins.args[1]);
                    self.fog_far = fbits(ins.args[2]);
                    self.script_idx += 1;
                }
                2 => {
                    // CAMERA_FACING.
                    self.facing =
                        Vec3::new(fbits(ins.args[0]), fbits(ins.args[1]), fbits(ins.args[2]));
                    self.script_idx += 1;
                }
                _ => self.script_idx += 1, // facing-interp / fog-interp / pause: skipped
            }
        }

        // Interpolate camera position between the current keys.
        if self.interp_end > self.interp_start {
            let r = ((self.time - self.interp_start) / (self.interp_end - self.interp_start))
                .clamp(0.0, 1.0);
            self.cam = self.cam_init.lerp(self.cam_final, r);
        }
        self.time += 1.0;
    }

    fn view_proj(&self) -> Mat4 {
        let mid_w = FIELD_W / 2.0;
        let mid_h = FIELD_H / 2.0;
        let fov = 30.0_f32.to_radians();
        let cam_dist = mid_h / (fov / 2.0).tan();
        let eye = Vec3::new(mid_w, -mid_h, -cam_dist * self.facing.z);
        let at = Vec3::new(mid_w + self.facing.x, -mid_h + self.facing.y, 0.0);
        let up = Vec3::Y;
        let view = Mat4::look_at_lh(eye, at, up);
        let proj = Mat4::perspective_lh(fov, FIELD_W / FIELD_H, 100.0, 20000.0);
        proj * view
    }

    fn view_matrix(&self) -> Mat4 {
        let mid_w = FIELD_W / 2.0;
        let mid_h = FIELD_H / 2.0;
        let fov = 30.0_f32.to_radians();
        let cam_dist = mid_h / (fov / 2.0).tan();
        let eye = Vec3::new(mid_w, -mid_h, -cam_dist * self.facing.z);
        let at = Vec3::new(mid_w + self.facing.x, -mid_h + self.facing.y, 0.0);
        Mat4::look_at_lh(eye, at, Vec3::Y)
    }

    pub fn scene(&self) -> BgScene {
        let mvp = self.view_proj();
        let view = self.view_matrix();
        let [tw, th] = self.tex_size;
        let mut verts = Vec::new();

        for inst in &self.std.instances {
            let Some(obj) = self.std.objects.get(inst.id as usize) else { continue };
            for q in &obj.quads {
                let Some(&([sx, sy, sw, sh], scale)) = self.sprites.get(&(q.anm_script as i32)) else {
                    continue;
                };
                let qw = if q.size[0] != 0.0 { q.size[0] } else { sw * scale[0] };
                let qh = if q.size[1] != 0.0 { q.size[1] } else { sh * scale[1] };
                // World origin of the quad (camera subtracted; y points up).
                let wx = obj.pos[0] + q.pos[0] + inst.pos[0] - self.cam.x;
                let wy = obj.pos[1] + q.pos[1] + inst.pos[1] - self.cam.y;
                let wz = obj.pos[2] + q.pos[2] + inst.pos[2] - self.cam.z;
                let ox = wx;
                let oy = -wy;
                let oz = wz;

                // Fog from true view-space distance: 1 = clear (near), 0 =
                // fully fogged (far). Computed at the quad origin.
                let view_z = (view * glam::Vec4::new(ox, oy, oz, 1.0)).z;
                let span = (self.fog_far - self.fog_near).max(1.0);
                let fog = ((self.fog_far - view_z) / span).clamp(0.0, 1.0);

                let u0 = sx / tw;
                let v0 = sy / th;
                let u1 = (sx + sw) / tw;
                let v1 = (sy + sh) / th;
                let tl = Vertex3 { pos: [ox, oy, oz], uv: [u0, v0], fog };
                let tr = Vertex3 { pos: [ox + qw, oy, oz], uv: [u1, v0], fog };
                let br = Vertex3 { pos: [ox + qw, oy - qh, oz], uv: [u1, v1], fog };
                let bl = Vertex3 { pos: [ox, oy - qh, oz], uv: [u0, v1], fog };
                verts.extend_from_slice(&[tl, tr, br, tl, br, bl]);
            }
        }

        BgScene { mvp: mvp.to_cols_array_2d(), fog_color: self.fog_color, verts, tex: self.tex_slot }
    }
}
