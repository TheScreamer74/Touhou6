//! Stage 3D background: drives the STD camera/fog script and runs every
//! background quad as a live ANM VM each frame, mirroring the decompilation's
//! `Stage::OnUpdate` (camera position/facing/fog) and `RenderObjects` +
//! `AnmManager::Draw3` (quads placed in world space, oriented by their anm
//! rotation, projected by a 30-degree LH perspective camera).

use std::collections::HashMap;

use glam::{Mat4, Vec3};
use th06_engine::{BgScene, Vertex3};
use th06_formats::anm0::{Entry, Instr};
use th06_formats::std::Std;

const FIELD_W: f32 = 384.0;
const FIELD_H: f32 = 448.0;

/// A live ANM script instance for one background quad. Models the same
/// clock/jump machinery as `anm_vm::AnmRunner`, but tracks the 3D state the
/// background needs (rotation + angular velocity, uv scroll, auto-rotate) and
/// omits position (the STD quad position is authoritative for backgrounds).
struct BgQuadVm {
    instrs: Vec<Instr>,
    pc: usize,
    clock: u16,
    halted: bool,
    dead: bool,
    visible: bool,
    sprite: Option<u32>,
    scale: [f32; 2],
    /// op11 SetScaleSpeed: per-frame scale delta.
    scale_vel: [f32; 2],
    /// op9 SetRotation, integrated each frame by `angle_vel` (op10).
    rot: [f32; 3],
    angle_vel: [f32; 3],
    /// op26 SetAutoRotate (2 = always face the camera).
    auto_rotate: i32,
    /// op27/28 UVScroll accumulators, wrapped to [0, 1).
    uv: [f32; 2],
    /// op13/14: true = additive (One/One) blend.
    blend_add: bool,
    /// vm->color modulation [r, g, b, a] (op3 alpha, op4 color, op12 fade).
    color: [f32; 4],
    /// op12 Fade: alpha interpolation (from, to, end_time, timer).
    fade: [f32; 2],
    fade_end: u16,
    fade_t: u16,
}

impl BgQuadVm {
    fn new(instrs: Vec<Instr>) -> Self {
        let mut vm = Self {
            instrs,
            pc: 0,
            clock: 0,
            halted: false,
            dead: false,
            visible: true,
            sprite: None,
            scale: [1.0, 1.0],
            scale_vel: [0.0, 0.0],
            rot: [0.0, 0.0, 0.0],
            angle_vel: [0.0, 0.0, 0.0],
            auto_rotate: 0,
            uv: [0.0, 0.0],
            blend_add: false,
            color: [1.0, 1.0, 1.0, 1.0],
            fade: [0.0, 0.0],
            fade_end: 0,
            fade_t: 0,
        };
        vm.exec_ready();
        vm
    }

    fn tick(&mut self) {
        if self.dead {
            return;
        }
        self.clock = self.clock.saturating_add(1);
        self.exec_ready();
        // Per-frame integration (AnmManager::OnTick `stop:` block).
        for i in 0..3 {
            if self.angle_vel[i] != 0.0 {
                self.rot[i] += self.angle_vel[i];
            }
        }
        self.scale[0] += self.scale_vel[0];
        self.scale[1] += self.scale_vel[1];
        if self.fade_end > 0 {
            self.fade_t += 1;
            let r = (self.fade_t as f32 / self.fade_end as f32).min(1.0);
            self.color[3] = self.fade[0] + (self.fade[1] - self.fade[0]) * r;
            if self.fade_t >= self.fade_end {
                self.fade_end = 0;
            }
        }
    }

    fn exec_ready(&mut self) {
        let mut budget = 1000;
        while !self.halted && !self.dead && self.pc < self.instrs.len() {
            budget -= 1;
            if budget == 0 {
                break;
            }
            let i = &self.instrs[self.pc];
            if i.time > self.clock {
                break;
            }
            match i.opcode {
                0 => self.dead = true,
                1 => {
                    self.sprite = Some(i.arg_u32(0));
                    self.visible = true;
                }
                2 => self.scale = [i.arg_f32(0), i.arg_f32(1)],
                3 => self.color[3] = (i.arg_u32(0) & 0xff) as f32 / 255.0,
                4 => {
                    let c = i.arg_u32(0);
                    self.color[0] = ((c >> 16) & 0xff) as f32 / 255.0;
                    self.color[1] = ((c >> 8) & 0xff) as f32 / 255.0;
                    self.color[2] = (c & 0xff) as f32 / 255.0;
                }
                5 => {
                    let target = i.arg_u32(0);
                    if let Some(idx) = self.instrs.iter().position(|x| x.offset == target) {
                        self.pc = idx;
                        self.clock = self.instrs[idx].time;
                        continue;
                    }
                }
                7 => self.scale[0] = -self.scale[0],
                8 => self.scale[1] = -self.scale[1],
                9 => self.rot = [i.arg_f32(0), i.arg_f32(1), i.arg_f32(2)],
                10 => self.angle_vel = [i.arg_f32(0), i.arg_f32(1), i.arg_f32(2)],
                11 => self.scale_vel = [i.arg_f32(0), i.arg_f32(1)],
                12 => {
                    // Fade: interpolate alpha to target over a duration.
                    self.fade = [self.color[3], (i.arg_u32(0) & 0xff) as f32 / 255.0];
                    self.fade_end = i.arg_u32(1) as u16;
                    self.fade_t = 0;
                }
                13 => self.blend_add = true,
                14 => self.blend_add = false,
                15 => self.halted = true,
                16 => {
                    // SetRandomSprite: deterministic (use the base index).
                    self.sprite = Some(i.arg_u32(0));
                    self.visible = true;
                }
                21 | 24 => {
                    self.pc += 1;
                    self.halted = true;
                    return;
                }
                26 => self.auto_rotate = i.arg_u32(0) as i32,
                27 => {
                    self.uv[0] = (self.uv[0] + i.arg_f32(0)).rem_euclid(1.0);
                }
                28 => {
                    self.uv[1] = (self.uv[1] + i.arg_f32(0)).rem_euclid(1.0);
                }
                29 => self.visible = i.arg_u32(0) != 0,
                // 3 alpha / 12 fade / 13-14 blend need engine support; 17-20
                // position is overridden by the STD quad position. Ignored here.
                _ => {}
            }
            self.pc += 1;
        }
    }
}

/// A background quad: its world position (STD quad + instance) and z-level,
/// plus the live ANM VM driving its sprite/rotation/scroll.
struct DrawQuad {
    base: [f32; 3],
    size: [f32; 2],
    z: i8,
    vm: BgQuadVm,
}

pub struct Background {
    std: Std,
    /// sprite index in the bg anm file -> pixel rect [x, y, w, h].
    sprite_tbl: HashMap<u32, [f32; 4]>,
    quads: Vec<DrawQuad>,
    /// quad indices sorted back-to-front by z-level (decomp draws zLevel 0..3).
    draw_order: Vec<usize>,
    tex_size: [f32; 2],
    tex_slot: usize,

    time: f32,
    script_idx: usize,
    cam: Vec3,
    cam_init: Vec3,
    cam_final: Vec3,
    interp_start: f32,
    interp_end: f32,
    /// Camera look direction (STD op2/op3), interpolated like the position.
    facing: Vec3,
    facing_init: Vec3,
    facing_final: Vec3,
    facing_dur: i32,
    facing_timer: i32,
    fog_color: [f32; 4],
    fog_near: f32,
    fog_far: f32,
    /// STDOP_FOG_INTERP (op4): gradual fog transition (Stage::OnUpdate). The
    /// fog lerps from `fog_init` to `fog_final` over `fog_interp_dur` frames.
    fog_init: ([f32; 4], f32, f32),
    fog_final: ([f32; 4], f32, f32),
    fog_interp_dur: i32,
    fog_interp_timer: i32,
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
        let sprite_tbl: HashMap<u32, [f32; 4]> = bg
            .sprites
            .iter()
            .map(|s| (s.index, [s.x, s.y, s.width, s.height]))
            .collect();
        // Background anm scripts by id, to instantiate per quad.
        let script_map: HashMap<i32, &Vec<Instr>> =
            bg.scripts.iter().map(|(id, instrs)| (*id as i32, instrs)).collect();

        // One live VM per drawn quad (instance x object quad). Draw position is
        // quad + instance - stage (obj.pos is only the cull bound).
        let mut quads = Vec::new();
        for inst in &std.instances {
            let Some(obj) = std.objects.get(inst.id as usize) else { continue };
            for q in &obj.quads {
                let Some(instrs) = script_map.get(&(q.anm_script as i32)) else { continue };
                quads.push(DrawQuad {
                    base: [
                        q.pos[0] + inst.pos[0],
                        q.pos[1] + inst.pos[1],
                        q.pos[2] + inst.pos[2],
                    ],
                    size: q.size,
                    z: obj.z_level,
                    vm: BgQuadVm::new((*instrs).clone()),
                });
            }
        }
        let mut draw_order: Vec<usize> = (0..quads.len()).collect();
        draw_order.sort_by_key(|&i| quads[i].z);


        Self {
            std,
            sprite_tbl,
            quads,
            draw_order,
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
            facing_init: Vec3::new(0.0, 0.0, 1.0),
            facing_final: Vec3::new(0.0, 0.0, 1.0),
            facing_dur: 1,
            facing_timer: 0,
            // Stage::AddedCallback initial skyFog: black, near 200, far 500
            // (overwritten by the script's frame-0 FOG instruction).
            fog_color: [0.0, 0.0, 0.0, 1.0],
            fog_near: 200.0,
            fog_far: 500.0,
            fog_init: ([0.0, 0.0, 0.0, 1.0], 200.0, 500.0),
            fog_final: ([0.0, 0.0, 0.0, 1.0], 200.0, 500.0),
            fog_interp_dur: 0,
            fog_interp_timer: 0,
        }
    }

    pub fn tick(&mut self) {
        // Camera/fog script (STD): position keys, fog, facing.
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
                    // CAMERA_POSITION_KEY: set current key, scan ahead for the
                    // next key to interpolate toward.
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
                    // FOG: color, near, far. skyFog is set to the target; when a
                    // FOG_INTERP is active it becomes the interp final and the
                    // step below lerps toward it (Stage::OnUpdate STDOP_FOG).
                    let mut col = color_argb(ins.args[0]);
                    col[3] = 1.0;
                    let near = fbits(ins.args[1]);
                    let far = fbits(ins.args[2]);
                    self.fog_color = col;
                    self.fog_near = near;
                    self.fog_far = far;
                    self.fog_final = (col, near, far);
                    self.script_idx += 1;
                }
                2 => {
                    // CAMERA_FACING: new look direction (interpolated via the
                    // active duration; default 1 = instant).
                    self.facing_init = self.facing_final;
                    self.facing_final =
                        Vec3::new(fbits(ins.args[0]), fbits(ins.args[1]), fbits(ins.args[2]));
                    self.script_idx += 1;
                }
                3 => {
                    // CAMERA_FACING_INTERP_LINEAR: duration; restart the timer so
                    // the interp begins at ratio 0 on this frame (the decomp's
                    // facing ramp is one frame behind a naive timer).
                    self.facing_dur = ins.args[0];
                    self.facing_timer = -1;
                    self.script_idx += 1;
                }
                4 => {
                    // FOG_INTERP: capture the current fog as the interp start and
                    // begin a `dur`-frame transition toward the next FOG target.
                    // Timer starts at -1 (ZunTimer::InitializeForPopup); the step
                    // below Ticks it to 0 (ratio 0) on this frame, matching the
                    // decomp's one-frame-later ramp.
                    self.fog_init = (self.fog_color, self.fog_near, self.fog_far);
                    self.fog_interp_dur = ins.args[0];
                    self.fog_interp_timer = -1;
                    self.script_idx += 1;
                }
                _ => self.script_idx += 1,
            }
        }

        // Interpolate camera position between keys.
        if self.interp_end > self.interp_start {
            let r = ((self.time - self.interp_start) / (self.interp_end - self.interp_start))
                .clamp(0.0, 1.0);
            self.cam = self.cam_init.lerp(self.cam_final, r);
        }
        // Interpolate camera facing (op2/op3), as Stage::OnUpdate does.
        if self.facing_dur != 0 {
            if self.facing_timer < self.facing_dur {
                self.facing_timer += 1;
            }
            let r = self.facing_timer as f32 / self.facing_dur as f32;
            // Exact decomp order: (final - init) * ratio + init (Stage::OnUpdate),
            // not glam's init + (final-init)*ratio — they differ by 1 ulp.
            let d = self.facing_final - self.facing_init;
            self.facing = Vec3::new(
                d.x * r + self.facing_init.x,
                d.y * r + self.facing_init.y,
                d.z * r + self.facing_init.z,
            );
        }
        // Interpolate fog (op4 FOG_INTERP): lerp colour/near/far from start to the
        // FOG target over the duration, then clear (Stage::OnUpdate fog block).
        if self.fog_interp_dur != 0 {
            self.fog_interp_timer += 1;
            let r = (self.fog_interp_timer as f32 / self.fog_interp_dur as f32).min(1.0);
            let (ic, inear, ifar) = self.fog_init;
            let (fc, fnear, ffar) = self.fog_final;
            // Per-byte lerp truncated to u8, matching the decomp's COLOR_SET_-
            // COMPONENT((u8)((finalByte - initByte) * ratio + initByte)).
            for k in 0..4 {
                let ib = ic[k] * 255.0;
                let fb = fc[k] * 255.0;
                let b = ((fb - ib) * r + ib) as i32;
                self.fog_color[k] = b as f32 / 255.0;
            }
            self.fog_near = inear + (fnear - inear) * r;
            self.fog_far = ifar + (ffar - ifar) * r;
            if self.fog_interp_timer >= self.fog_interp_dur {
                self.fog_interp_dur = 0;
            }
        }
        self.time += 1.0;

        // Advance every background quad's ANM script.
        for dq in &mut self.quads {
            dq.vm.tick();
        }
    }

    /// Per-frame bg state (camera pos, facing, fog) for the bg-state oracle diff.
    /// Returns (pos, facing, fog_argb_packed, fog_near, fog_far).
    pub fn dbg_state(&self) -> ([f32; 3], [f32; 3], u32, f32, f32) {
        let b = |v: f32| (v * 255.0) as u32 & 0xff;
        let argb = (b(self.fog_color[3]) << 24)
            | (b(self.fog_color[0]) << 16)
            | (b(self.fog_color[1]) << 8)
            | b(self.fog_color[2]);
        (
            [self.cam.x, self.cam.y, self.cam.z],
            [self.facing.x, self.facing.y, self.facing.z],
            argb,
            self.fog_near,
            self.fog_far,
        )
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
        // Far plane 10000 (GameManager::SetupCamera with extraRenderDistance 0),
        // not 20000 — the original clips quads past 10000 (they are fully fogged
        // by then anyway); rendering them produced an over-distant smear band.
        let proj = Mat4::perspective_lh(fov, FIELD_W / FIELD_H, 100.0, 10000.0);
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

    /// Project a playfield point (x, y, z) through a straight-on SetupCamera to
    /// field-pixel screen coords — used to render Draw3 effects (spellcard
    /// bubbles) at their 3D positions. The gameplay layer (enemies/bullets) is
    /// drawn screen-space 1:1 (XYZRHW), so effects use the matching un-pitched
    /// camera (facing 0,0,1): playfield x/y map 1:1 and the bubble's z adds the
    /// perspective swirl. World Y is negated to match the camera's convention.
    pub fn project_point(pf: [f32; 3]) -> Option<[f32; 2]> {
        let mid_w = FIELD_W / 2.0;
        let mid_h = FIELD_H / 2.0;
        let fov = 30.0_f32.to_radians();
        let cam_dist = mid_h / (fov / 2.0).tan();
        let eye = Vec3::new(mid_w, -mid_h, -cam_dist);
        let at = Vec3::new(mid_w, -mid_h, 0.0);
        let view = Mat4::look_at_lh(eye, at, Vec3::Y);
        let proj = Mat4::perspective_lh(fov, FIELD_W / FIELD_H, 100.0, 10000.0);
        let clip = (proj * view) * glam::Vec4::new(pf[0], -pf[1], pf[2], 1.0);
        if clip.w <= 1e-3 {
            return None;
        }
        let nx = clip.x / clip.w;
        let ny = clip.y / clip.w;
        Some([(nx * 0.5 + 0.5) * FIELD_W, (1.0 - (ny * 0.5 + 0.5)) * FIELD_H])
    }

    pub fn scene(&self) -> BgScene {
        let mvp = self.view_proj();
        let view = self.view_matrix();
        let [tw, th] = self.tex_size;
        let mut verts = Vec::new();
        let mut verts_add = Vec::new();

        for &qi in &self.draw_order {
            let dq = &self.quads[qi];
            let vm = &dq.vm;
            if vm.dead || !vm.visible {
                continue;
            }
            let Some(sprite) = vm.sprite else { continue };
            let Some(&[sx, sy, sw, sh]) = self.sprite_tbl.get(&sprite) else { continue };

            // Exact decomp sizing: a 256-unit base quad (vertices at ±128)
            // scaled by scaleX/scaleY, where scaleX = quad.size.x / sprite.widthPx
            // when the quad size is set, else the anm scale (op2). So the on-screen
            // half-extent is 128 * scaleX (Draw3 + SetupVertexBuffer).
            let scale_x = if dq.size[0] != 0.0 { dq.size[0] / sw } else { vm.scale[0] };
            let scale_y = if dq.size[1] != 0.0 { dq.size[1] / sh } else { vm.scale[1] };
            let hw = 128.0 * scale_x;
            let hh = 128.0 * scale_y;

            // World center (camera subtracted; y up). Quads are centered on
            // their position — this matches the original's on-screen layout in
            // this view model; applying the literal AnchorTopLeft (op23) +hw/-hh
            // shift offsets the scene sideways here.
            let ox = dq.base[0] - self.cam.x;
            let oy = -(dq.base[1] - self.cam.y);
            let oz = dq.base[2] - self.cam.z;

            // 3D orientation from the anm rotation (Draw3: Rx*Ry*Rz). auto_rotate
            // == 2 is a camera-facing billboard, left axis-aligned.
            let rotm = if (vm.rot[0] != 0.0 || vm.rot[1] != 0.0 || vm.rot[2] != 0.0)
                && vm.auto_rotate != 2
            {
                Some(
                    glam::Mat3::from_rotation_z(vm.rot[2])
                        * glam::Mat3::from_rotation_y(vm.rot[1])
                        * glam::Mat3::from_rotation_x(vm.rot[0]),
                )
            } else {
                None
            };

            // UV with the live scroll offset baked in (op27/28).
            let u0 = sx / tw + vm.uv[0];
            let v0 = sy / th + vm.uv[1];
            let u1 = (sx + sw) / tw + vm.uv[0];
            let v1 = (sy + sh) / th + vm.uv[1];

            // Pass view-space depth per vertex; the fog factor is computed
            // per-pixel in the shader (matches D3DFOG_LINEAR table fog).
            let color = vm.color;
            let vtx = |lx: f32, ly: f32, u: f32, v: f32| -> Vertex3 {
                let p = glam::Vec3::new(lx, ly, 0.0);
                let p = if let Some(m) = rotm { m * p } else { p };
                let pos = [ox + p.x, oy + p.y, oz + p.z];
                let depth = (view * glam::Vec4::new(pos[0], pos[1], pos[2], 1.0)).z;
                Vertex3 { pos, uv: [u, v], depth, color }
            };

            let tl = vtx(-hw, hh, u0, v0);
            let tr = vtx(hw, hh, u1, v0);
            let br = vtx(hw, -hh, u1, v1);
            let bl = vtx(-hw, -hh, u0, v1);
            let out = if vm.blend_add { &mut verts_add } else { &mut verts };
            out.extend_from_slice(&[tl, tr, br, tl, br, bl]);
        }

        BgScene {
            mvp: mvp.to_cols_array_2d(),
            fog_color: self.fog_color,
            fog_near: self.fog_near,
            fog_far: self.fog_far,
            verts,
            verts_add,
            tex: self.tex_slot,
        }
    }
}
