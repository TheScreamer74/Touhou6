//! BulletManager/Player projectile states from the v1.02h decompilation.
//!
//! Collision callbacks use the state before its clock advances, including the
//! original laser warmup/despawn midpoint hitbox bug and switch fallthrough.

use crate::anm_vm::AnmRunner;
use crate::ecl_vm::{Bullet, Laser, normalize_angle, player_angle_from, sincos_mul};
use th06_formats::anm0::Entry;

pub const SPAWN_COLORS_16: [u32; 16] = [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 0];
const SPAWN_COLORS_8: [u32; 8] = [0, 1, 1, 2, 2, 3, 4, 0];

#[derive(Clone)]
pub struct BulletVisuals {
    pub normal: AnmRunner,
    pub spawning: [AnmRunner; 3],
    pub despawn: AnmRunner,
    pub effect_texture: bool,
}

impl BulletVisuals {
    /// BulletManager_AddedCallback's g_BulletTypeInfos, pre-executed once.
    pub fn templates(etama: &Entry, effects: &Entry) -> Vec<Self> {
        if etama.scripts.is_empty() {
            return Vec::new();
        }
        (0..10)
            .map(|kind| {
                let (entry, normal, spawn, donut) = match kind {
                    0 => (etama, 0, [14, 15, 16], 11),
                    1..=5 => (etama, kind, [17, 18, 19], 12),
                    6..=8 => (etama, kind, [20, 20, 20], 13),
                    _ => (effects, 0, [2, 2, 2], 1),
                };
                let runner = |id| {
                    AnmRunner::new(
                        entry
                            .scripts
                            .iter()
                            .find(|(n, _)| *n == id)
                            .map(|(_, instrs)| instrs.clone())
                            .unwrap_or_default(),
                    )
                };
                Self {
                    normal: runner(normal),
                    spawning: spawn.map(runner),
                    despawn: runner(donut),
                    effect_texture: kind == 9,
                }
            })
            .collect()
    }

    pub fn set_color(&mut self, color: u32, height: f32, kind: usize) {
        let offset = if height <= 16.0 {
            SPAWN_COLORS_16.get(color as usize).copied().unwrap_or(0)
        } else if height <= 32.0 {
            if kind == 7 {
                1
            } else {
                SPAWN_COLORS_8.get(color as usize).copied().unwrap_or(0)
            }
        } else {
            color
        };
        for runner in self
            .spawning
            .iter_mut()
            .chain(std::iter::once(&mut self.despawn))
        {
            if let Some(sprite) = runner.sprite {
                runner.set_sprite(sprite + offset);
            }
        }
        if let Some(sprite) = self.normal.sprite {
            self.normal.set_sprite(sprite + color);
        }
    }
}

impl Bullet {
    pub fn visual(&self) -> Option<&AnmRunner> {
        let visuals = self.visuals.as_ref()?;
        if self.despawn_timer > 0 {
            Some(&visuals.despawn)
        } else if self.spawn_delay > 0 {
            Some(&visuals.spawning[self.spawn_kind()])
        } else {
            Some(&visuals.normal)
        }
    }

    fn spawn_kind(&self) -> usize {
        if self.ex_flags & 2 != 0 {
            0
        } else if self.ex_flags & 4 != 0 {
            1
        } else {
            2
        }
    }

    pub fn begin_despawn(&mut self) {
        if self.despawn_timer == 0 {
            // Real duration comes from the cached original ANM, including the
            // huge etama4 bubble's 16-frame fade. The counter is a fallback for
            // code-only tests which deliberately omit game data.
            self.despawn_timer = if self.type_index == 9 { 16 } else { 12 };
        }
    }

    /// Movement portion of BulletManager::OnUpdate. Returns true only when the
    /// FIRED case reaches its collision callbacks; spawn states never graze.
    pub fn step(&mut self, player: [f32; 2], region: [f32; 2]) -> bool {
        if self.despawn_timer > 0 {
            self.pos[0] += self.velocity[0] / 2.0;
            self.pos[1] += self.velocity[1] / 2.0;
            let ended = if let Some(visuals) = &mut self.visuals {
                visuals.despawn.tick();
                visuals.despawn.ended()
            } else {
                self.despawn_timer == 1
            };
            self.despawn_timer -= 1;
            if ended {
                self.oob_count = i32::MAX;
            }
            // Keep the despawn state set until the slot is reclaimed.
            self.despawn_timer = self.despawn_timer.max(1);
            self.timer += 1;
            return false;
        }
        if self.spawn_delay > 0 {
            self.pos[0] += self.velocity[0] * self.spawn_factor;
            self.pos[1] += self.velocity[1] * self.spawn_factor;
            let kind = self.spawn_kind();
            let ended = if let Some(visuals) = &mut self.visuals {
                visuals.spawning[kind].tick();
                visuals.spawning[kind].ended()
            } else {
                self.spawn_delay == 1
            };
            self.spawn_delay -= 1;
            if !ended {
                // ANM controls lifetime. This also supports bubble spawn's
                // 24-frame script instead of assuming 32 for every huge type.
                self.spawn_delay = self.spawn_delay.max(1);
                self.timer += 1;
                return false;
            }
            self.spawn_delay = 0;
            self.timer = 0;
        }
        if self.ex_flags & 1 != 0 {
            if self.timer <= 16 {
                self.set_velocity(5.0 - self.timer as f32 * 5.0 / 16.0 + self.speed);
            } else {
                self.ex_flags &= !1;
            }
        } else if self.ex_flags & 0x10 != 0 {
            if self.timer >= self.ex_int0 {
                self.ex_flags &= !0x10;
            } else {
                self.velocity[0] += self.ex_accel[0];
                self.velocity[1] += self.ex_accel[1];
                self.angle = self.velocity[1].atan2(self.velocity[0]);
            }
        } else if self.ex_flags & 0x20 != 0 {
            if self.timer >= self.ex_int0 {
                self.ex_flags &= !0x20;
            } else {
                self.angle = normalize_angle(self.angle + self.ex_f[1]);
                self.speed += self.ex_f[0];
                self.set_velocity(self.speed);
            }
        }
        let mode = self.ex_flags & 0x1c0;
        if mode != 0 {
            let speed = if self.timer >= self.ex_int0 * (self.ex_count + 1) {
                self.ex_count += 1;
                let flag = if mode & 0x40 != 0 {
                    0x40
                } else if mode & 0x100 != 0 {
                    0x100
                } else {
                    0x80
                };
                if self.ex_count >= self.ex_int1 {
                    self.ex_flags &= !flag;
                }
                self.angle = if flag == 0x40 {
                    self.angle + self.ex_f[0]
                } else if flag == 0x100 {
                    self.ex_f[0]
                } else {
                    player_angle_from(player, self.pos) + self.ex_f[0]
                };
                self.speed = self.ex_f[1];
                self.speed
            } else {
                self.speed
                    - (self.timer - self.ex_int0 * self.ex_count) as f32 * self.speed
                        / self.ex_int0 as f32
            };
            self.set_velocity(speed);
        } else if self.ex_flags & 0xc00 != 0 && !self.in_bounds(region) {
            let flag = if self.ex_flags & 0x400 != 0 {
                0x400
            } else {
                0x800
            };
            if self.pos[0] < 0.0 || self.pos[0] >= 384.0 {
                self.angle = normalize_angle(-self.angle - std::f32::consts::PI);
            }
            if self.pos[1] < 0.0 || (flag == 0x400 && self.pos[1] >= 448.0) {
                self.angle = -self.angle;
            }
            self.speed = self.ex_f[1];
            self.set_velocity(self.speed);
            self.ex_count += 1;
            if self.ex_count >= self.ex_int1 {
                self.ex_flags &= !flag;
            }
        }
        self.pos[0] += self.velocity[0];
        self.pos[1] += self.velocity[1];
        if self.in_bounds(region) {
            self.oob_count = 0;
        } else if self.ex_flags & 0xdc0 == 0 && self.oob_count == 0 {
            self.oob_count = i32::MAX;
        } else {
            self.oob_count += 1;
        }
        self.oob_count < 256
    }

    pub fn finish_fired_step(&mut self) {
        if let Some(visuals) = &mut self.visuals {
            if visuals.normal.sprite != Some(self.sprite) {
                visuals.normal.set_sprite(self.sprite);
            }
            visuals.normal.tick();
            if let Some(sprite) = visuals.normal.sprite {
                self.sprite = sprite;
            }
        }
        self.timer += 1;
    }

    fn set_velocity(&mut self, speed: f32) {
        self.velocity = sincos_mul(self.angle, speed);
    }

    fn in_bounds(&self, region: [f32; 2]) -> bool {
        self.pos[0] + self.width / 2.0 >= 0.0
            && self.pos[0] - self.width / 2.0 <= region[0]
            && self.pos[1] + self.height / 2.0 >= 0.0
            && self.pos[1] - self.height / 2.0 <= region[1]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LaserCollision {
    pub origin: [f32; 2],
    pub angle: f32,
    pub center: f32,
    pub length: f32,
    pub width: f32,
    pub can_graze: bool,
}

impl Laser {
    /// Produces each callback reached by the original switch, including two
    /// calls on START_DELAY→ACTIVE and ACTIVE→DESPAWNING transition frames.
    pub fn step(&mut self) -> Vec<LaserCollision> {
        let mut collisions = Vec::with_capacity(3);
        if !self.in_use {
            return collisions;
        }
        self.end_offset += self.speed;
        if self.start_length < self.end_offset - self.start_offset {
            self.start_offset = self.end_offset - self.start_length;
        }
        self.start_offset = self.start_offset.max(0.0);
        let length = self.end_offset - self.start_offset;
        let center = length / 2.0 + self.start_offset;
        let mut hit_length = length;
        self.draw_width = self.width;
        self.draw_alpha = 1.0;
        let push = |collisions: &mut Vec<_>, laser: &Self, hit_length| {
            collisions.push(LaserCollision {
                origin: laser.pos,
                angle: laser.angle,
                center,
                length: hit_length,
                width: laser.width / 2.0,
                can_graze: laser.timer % 12 == 0,
            });
        };
        if self.state == 0 {
            if self.flags & 1 != 0 {
                self.draw_alpha = laser_alpha(self.timer, self.start_time);
            } else {
                self.draw_width = if self.start_time - self.start_time.min(30) < self.timer {
                    self.timer as f32 * self.width / self.start_time as f32
                } else {
                    1.2
                };
                // Original bug changes longitudinal size, not beam thickness.
                hit_length = self.draw_width / 2.0;
            }
            if self.timer >= self.hitbox_start {
                push(&mut collisions, self, hit_length);
            }
            if self.timer >= self.start_time {
                self.timer = 0;
                self.state = 1;
            }
        }
        if self.state == 1 {
            push(&mut collisions, self, hit_length);
            if self.timer >= self.duration {
                self.timer = 0;
                self.state = 2;
                if self.despawn_duration == 0 {
                    self.in_use = false;
                    return collisions;
                }
            }
        }
        if self.state == 2 {
            if self.flags & 1 != 0 {
                // Preserve v1.02h's use of startTime and increasing alpha.
                self.draw_alpha = laser_alpha(self.timer, self.start_time);
            } else if self.despawn_duration > 0 {
                self.draw_width =
                    self.width - self.timer as f32 * self.width / self.despawn_duration as f32;
                hit_length = self.draw_width / 2.0;
            }
            if self.timer < self.hitbox_end {
                push(&mut collisions, self, hit_length);
            }
            if self.timer >= self.despawn_duration {
                self.in_use = false;
                return collisions;
            }
        }
        if self.start_offset >= 640.0 {
            self.in_use = false;
        }
        self.timer += 1;
        collisions
    }
}

// Original __ftol2 (102h.exe 0x45ba78, FISTP qword) returns a 64-bit
// integer, then BulletManager uses its low dword. Non-finite input becomes
// integer-indefinite 0x8000000000000000, whose low dword and alpha are zero.
fn laser_alpha(timer: i32, start_time: i32) -> f32 {
    let value = timer as f32 * 255.0 / start_time as f32;
    let integer = if !value.is_finite()
        || !(-9223372036854775808.0..9223372036854775808.0).contains(&value)
    {
        i64::MIN
    } else {
        value as i64
    } as i32;
    integer.min(255) as u8 as f32 / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bullet(flags: u32) -> Bullet {
        Bullet {
            pool_index: 0,
            pos: [100.0, 100.0],
            angle: 0.0,
            speed: 1.0,
            velocity: [1.0, 0.0],
            type_index: 0,
            visuals: None,
            sprite: 14,
            base_sprite: 14,
            sprite_offset: 0,
            height: 8.0,
            width: 8.0,
            spawn_delay: 0,
            spawn_factor: 1.0,
            oob_count: 0,
            timer: 0,
            ex_flags: flags,
            ex_accel: [0.0, 0.5],
            ex_f: [0.0, 0.0],
            ex_int0: 2,
            ex_int1: 0,
            ex_count: 0,
            grazed: false,
            despawn_timer: 0,
        }
    }
    #[test]
    fn first_fired_tick_is_zero_and_acceleration_keeps_cached_velocity() {
        let mut boost = bullet(1);
        assert!(boost.step([0.0; 2], [384.0, 448.0]));
        assert_eq!(boost.pos, [106.0, 100.0]);
        boost.finish_fired_step();
        assert!(boost.step([0.0; 2], [384.0, 448.0]));
        assert_eq!(boost.velocity, [5.6875, 0.0]);
        let mut acceleration = bullet(0x10);
        for _ in 0..3 {
            assert!(acceleration.step([0.0; 2], [384.0, 448.0]));
            acceleration.finish_fired_step();
        }
        assert_eq!(acceleration.pos, [103.0, 102.5]);
        assert_eq!(acceleration.velocity, [1.0, 1.0]);
        assert_eq!(acceleration.speed, 1.0); // speed is not magnitude(velocity)
        assert_eq!(acceleration.ex_flags, 0);
    }
    #[test]
    fn screen_shake_region_changes_culling_before_collision() {
        // GameManager::IsInBounds reads gameRegionSize, which ScreenEffect's
        // previous tick shortened. Original demo00 first reaches this case at
        // frame 2416: a bomb shake removes slot 26 below the cropped viewport.
        let mut full = bullet(0);
        full.pos = [160.0, 448.75];
        full.velocity = [0.0, 1.25];
        let mut shaking = bullet(0);
        shaking.pos = full.pos;
        shaking.velocity = full.velocity;
        assert!(full.step([0.0; 2], [384.0, 448.0]));
        assert!(!shaking.step([0.0; 2], [384.0, 442.0]));
        assert_eq!(shaking.oob_count, i32::MAX);

        // A direction-changing bullet instead preserves its slot for 256
        // out-of-bounds ticks, as on an uncropped field.
        let mut returning = bullet(0x40);
        returning.pos = [386.0, 100.0];
        returning.ex_int0 = 1000;
        assert!(returning.step([0.0; 2], [378.0, 448.0]));
        assert_eq!(returning.oob_count, 1);
    }
    #[test]
    fn spawn_transition_resets_clock_and_fade_preserves_last_velocity() {
        let mut b = bullet(1 | 2);
        b.spawn_factor = 0.5;
        b.spawn_delay = 2;
        assert!(!b.step([0.0; 2], [384.0, 448.0]));
        assert_eq!((b.pos, b.timer), ([100.5, 100.0], 1));
        assert!(b.step([0.0; 2], [384.0, 448.0]));
        assert_eq!((b.pos, b.timer, b.spawn_delay), ([107.0, 100.0], 0, 0));
        b.finish_fired_step();
        b.begin_despawn();
        assert!(!b.step([0.0; 2], [384.0, 448.0]));
        assert_eq!(b.pos, [110.0, 100.0]);
        for _ in 0..11 {
            b.step([0.0; 2], [384.0, 448.0]);
        }
        assert_eq!(b.oob_count, i32::MAX);
    }
    #[test]
    #[ignore = "requires original extracted etama3/4 ANM in TH06_REFERENCE_DIR"]
    fn original_bullet_anm_controls_all_spawn_and_cancellation_lifetimes() {
        use th06_formats::anm0::Anm0;
        let root =
            std::env::var("TH06_REFERENCE_DIR").unwrap_or_else(|_| "/tmp/th06-reference".into());
        let parse = |name| {
            Anm0::parse(&std::fs::read(format!("{root}/cm-assets/{name}")).unwrap()).unwrap()
        };
        let etama = parse("etama3.anm");
        let effects = parse("etama4.anm");
        let templates = BulletVisuals::templates(&etama.entries[0], &effects.entries[0]);
        for (kind, template) in templates.iter().enumerate() {
            for flags in [2, 4, 8] {
                let mut b = bullet(flags);
                b.type_index = kind;
                b.visuals = Some(template.clone());
                b.spawn_delay = 32;
                b.spawn_factor = 0.0;
                let expected = if kind == 9 {
                    24
                } else if kind >= 6 {
                    32
                } else {
                    match flags {
                        2 => 10,
                        4 => 16,
                        _ => 32,
                    }
                };
                for frame in 1..=expected {
                    let fired = b.step([0.0; 2], [384.0, 448.0]);
                    assert_eq!(
                        fired,
                        frame == expected,
                        "type {kind}, flags {flags}, frame {frame}"
                    );
                }
                b.begin_despawn();
                let expected = if kind == 9 { 16 } else { 12 };
                for frame in 1..=expected {
                    assert!(!b.step([0.0; 2], [384.0, 448.0]));
                    assert_eq!(
                        b.oob_count == i32::MAX,
                        frame == expected,
                        "type {kind}, fade frame {frame}"
                    );
                }
            }
        }
    }
}
