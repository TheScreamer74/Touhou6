//! Runs the port's real `spawn_bullet_pattern` over the same battery + RNG seed
//! as `oracle/bullet_oracle.cpp` (the decomp's exact math), printing each
//! bullet's (angle, speed). Diffing the two outputs proves the port's bullet
//! aim math matches the decomp. Run: `cargo run -p th06 --example oracle_dump`.

use th06::ecl_vm::{spawn_bullet_pattern, BulletProps, Rng, World};

fn make_world() -> World {
    World {
        rng: Rng::new(0x1234),
        difficulty: 1,
        rank: 16,
        sub_rank: 0,
        min_rank: 10,
        max_rank: 32,
        // pos is [0,0]; player at angle 1.2345 so AngleToPlayer == 1.2345.
        player_pos: [1.2345f32.cos(), 1.2345f32.sin()],
        bullets: Vec::new(),
        lasers: Vec::new(),
        events: Vec::new(),
        pending_spawns: Vec::new(),
        kill_trash: false,
        boss_present: false,
        power: 0,
        character: 0,
        shot_type: 0,
        time_stopped: false,
        bomb_active: false,
        bullet_heights: [0.0; 10],
        bullet_widths: [0.0; 10],
        effects: Vec::new(),
        eff_scripts: std::collections::HashMap::new(),
        random_item_spawn_index: 0,
        random_item_table_index: 0,
    }
}

fn main() {
    // (aim_mode, count1, count2, speed1, speed2, angle1, angle2)
    let battery: [(u16, i16, i16, f32, f32, f32, f32); 10] = [
        (0, 1, 1, 3.0, 1.0, 0.1, 0.3),
        (0, 4, 1, 3.0, 1.0, 0.1, 0.3),
        (1, 5, 2, 3.0, 1.0, 0.0, 0.26),
        (2, 12, 1, 2.5, 2.5, 0.0, 0.1),
        (3, 16, 3, 2.0, 1.0, 0.2, 0.05),
        (4, 8, 1, 2.0, 2.0, 0.0, 0.0),
        (5, 24, 1, 1.5, 1.5, 0.1, 0.0),
        (6, 6, 1, 2.0, 2.0, 1.0, -1.0),
        (7, 8, 2, 3.0, 1.0, 0.0, 0.2),
        (8, 10, 1, 3.0, 1.0, 1.0, -1.0),
    ];
    let mut world = make_world();
    for (aim, c1, c2, s1, s2, a1, a2) in battery {
        world.bullets.clear();
        let props = BulletProps {
            aim_mode: aim,
            count1: c1,
            count2: c2,
            speed1: s1,
            speed2: s2,
            angle1: a1,
            angle2: a2,
            pos: [0.0, 0.0],
            ..Default::default()
        };
        spawn_bullet_pattern(&mut world, &props);
        for b in &world.bullets {
            println!("{:.6} {:.6}", b.angle, b.speed);
        }
    }
}
