//! Original AnmManager Draw / DrawNoRotation / Draw2 geometry, expressed in
//! WebGPU pixel coordinates (pixel centers are +0.5 compared with D3D8).

pub struct Quad {
    pub dst: [f32; 4],
    pub rotation: f32,
}

/// Draw3's X/Y/Z rotations, before the neutral camera. Retain homogeneous W
/// for GPU clipping and perspective-correct texture interpolation. Positions
/// are playfield-local homogeneous pixels; clip Z uses the full0..1 range,
/// mapped into the original foreground depth0..0.5 by the draw viewport.
pub fn world_tilted(pos: [f32; 3], size: [f32; 2], rotation: [f32; 3]) -> [[f32; 4]; 4] {
    let transform = glam::Mat3::from_rotation_z(rotation[2])
        * glam::Mat3::from_rotation_y(rotation[1])
        * glam::Mat3::from_rotation_x(rotation[0]);
    let distance = 224.0 / (std::f32::consts::PI / 12.0).tan();
    let q = 10000.0 / (10000.0 - 100.0);
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| {
        let offset = transform * glam::Vec3::new(x * size[0] / 2.0, -y * size[1] / 2.0, 0.0);
        let w = distance + pos[2] + offset.z;
        [
            192.5 * w + (pos[0] + offset.x - 192.0) * distance,
            224.5 * w + (pos[1] - offset.y - 224.0) * distance,
            w * q - 100.0 * q,
            w,
        ]
    })
}

/// The stage's foreground viewport maps D3D normalized depth into0..0.5.
/// Wine11 keeps that range for XYZRHW vertices as well as projected XYZ.
pub fn screen_depth(z: f32) -> f32 {
    z * 0.5
}

/// Draw2/Draw3's neutral perspective camera, followed by the foreground
/// viewport depth range. A sprite plane parallel to the screen has uniformZ.
pub fn world_depth(z: f32) -> f32 {
    let distance = 224.0 / (std::f32::consts::PI / 12.0).tan();
    let camera_z = distance + z;
    let q = 10000.0 / (10000.0 - 100.0);
    ((camera_z * q - 100.0 * q) / camera_z) * 0.5
}

/// DrawNoRotation rounds each edge independently using x87 round-to-nearest.
pub fn screen(pos: [f32; 2], size: [f32; 2], corner: bool) -> Quad {
    let start = if corner {
        pos
    } else {
        [pos[0] - size[0] / 2.0, pos[1] - size[1] / 2.0]
    };
    let left = start[0].round_ties_even();
    let top = start[1].round_ties_even();
    Quad {
        dst: [
            left,
            top,
            (start[0] + size[0]).round_ties_even() - left,
            (start[1] + size[1]).round_ties_even() - top,
        ],
        rotation: 0.0,
    }
}

/// Draw rotates rounded half-sizes and the local (-0.5,-0.5) displacement.
/// The center correction is required for original rotated HUD/bomb sprites.
pub fn screen_rotated(pos: [f32; 2], size: [f32; 2], angle: f32, corner: bool) -> Quad {
    if angle == 0.0 {
        return screen(pos, size, corner);
    }
    let half = [
        (size[0] / 2.0).round_ties_even(),
        (size[1] / 2.0).round_ties_even(),
    ];
    let (sin, cos) = angle.sin_cos();
    let center = [
        pos[0].round_ties_even() + 0.5 - 0.5 * (cos + sin),
        pos[1].round_ties_even() + 0.5 - 0.5 * (cos - sin),
    ];
    Quad {
        dst: [
            center[0] - if corner { 0.0 } else { half[0] },
            center[1] - if corner { 0.0 } else { half[1] },
            half[0] * 2.0,
            half[1] * 2.0,
        ],
        rotation: -angle,
    }
}

/// Draw2 uses the neutral playfield perspective camera. A zero-angle VM uses
/// rounded coordinates with a half-pixel offset; Draw3 retains fractional
/// coordinates. Depth also scales position and sprite size, even at z=0.495.
/// Coordinates are playfield-local; the caller adds the viewport origin.
pub fn world(pos: [f32; 2], size: [f32; 2], angle: f32, corner: bool, z: f32) -> Quad {
    let mut center = if angle == 0.0 {
        [
            pos[0].round_ties_even() - 0.5,
            pos[1].round_ties_even() - 0.5,
        ]
    } else {
        pos
    };
    if corner {
        center[0] += size[0] / 2.0;
        center[1] += size[1] / 2.0;
    }
    let distance = 224.0 / (std::f32::consts::PI / 12.0).tan();
    let scale = distance / (distance + z);
    center = [
        192.0 + (center[0] - 192.0) * scale + 0.5,
        224.0 + (center[1] - 224.0) * scale + 0.5,
    ];
    Quad {
        dst: [
            center[0] - size[0] * scale / 2.0,
            center[1] - size[1] * scale / 2.0,
            size[0] * scale,
            size[1] * scale,
        ],
        rotation: -angle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn draw3_tilt_changes_corner_depth_and_applies_x_y_then_z() {
        let q = world_tilted(
            [192.0, 224.0, 0.0],
            [100.0, 50.0],
            [std::f32::consts::FRAC_PI_2; 3],
        );
        let distance = 224.0 / (std::f32::consts::PI / 12.0).tan();
        // TL (-50,+25,0) -> X (-50,0,+25) -> Y (+25,0,+50)
        // -> Z (0,+25,+50), then perspective. This differs from Z/Y/X.
        assert!((q[0][3] - (distance + 50.0)).abs() < 0.001);
        assert!((q[0][0] / q[0][3] - 192.5).abs() < 0.001);
        assert!((q[0][1] / q[0][3] - (224.5 - 25.0 * distance / (distance + 50.0))).abs() < 0.001);
        assert_ne!(q[0][3], q[1][3]);
        let flat = world_tilted([192.0, 224.0, 0.0], [100.0, 50.0], [0.0; 3]);
        assert_eq!(
            [flat[0][0] / flat[0][3], flat[0][1] / flat[0][3]],
            [142.5, 199.5]
        );
    }
    #[test]
    fn player_rhw_occludes_later_world_effects_in_original_viewport() {
        assert_eq!(screen_depth(0.49), 0.245);
        assert_eq!(screen_depth(0.491), 0.2455);
        assert!(world_depth(0.49) > 0.44 && world_depth(0.49) < 0.45);
        assert!(screen_depth(0.49) < world_depth(0.49));
    }
    #[test]
    fn d3d_pixel_center_and_x87_tie_rules() {
        // Original DrawInner: rint(edge)-0.5; WebGPU then adds 0.5.
        assert_eq!(
            screen([10.5, 11.5], [16.0; 2], false).dst,
            [2.0, 4.0, 16.0, 16.0]
        );
        assert_eq!(
            world([10.25, 11.75], [16.0; 2], 0.0, false, 0.0).dst,
            [2.0, 4.0, 16.0, 16.0]
        );
        // Draw3 doesn't perform the Draw2 no-rotation half-pixel correction.
        let quad = world([10.25, 11.75], [16.0; 2], 1.0, false, 0.0);
        assert_eq!(quad.dst, [2.75, 4.25, 16.0, 16.0]);
        assert_eq!(quad.rotation, -1.0);
    }
}
