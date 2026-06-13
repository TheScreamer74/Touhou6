//! STD stage-background files (th06): 3D scenery as textured quads plus a
//! camera/fog timeline. Layout from the decompilation (Stage.hpp):
//!
//! Header (0x490): i16 nbObjects, i16 nbFaces, i32 facesOffset (→ instance
//! list), i32 scriptOffset, i32 unk, char name[128], songNames[4][128],
//! songPaths[4][128]. Immediately after the header: nbObjects file-relative
//! i32 offsets to objects. Each object: i16 id, i8 zLevel, i8 flags,
//! vec3 position, vec3 size, then inline quads (type, byteSize, anmScript,
//! vmIdx, vec3 pos, vec2 size) chained by byteSize until type < 0.
//! Instances: i16 id, i16 unk, vec3 pos, until id < 0. Script: i32 frame,
//! i16 opcode, i16 size, i32 args[3], until frame < 0.

fn i16(d: &[u8], o: usize) -> i16 {
    i16::from_le_bytes(d[o..o + 2].try_into().unwrap())
}
fn i32_(d: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}
fn u32_(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}
fn f32_(d: &[u8], o: usize) -> f32 {
    f32::from_bits(u32_(d, o))
}

#[derive(Debug, Clone, Copy)]
pub struct Quad {
    pub anm_script: i16,
    pub pos: [f32; 3],
    pub size: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct Object {
    pub z_level: i8,
    pub pos: [f32; 3],
    pub size: [f32; 3],
    pub quads: Vec<Quad>,
}

#[derive(Debug, Clone, Copy)]
pub struct Instance {
    pub id: i16,
    pub pos: [f32; 3],
}

#[derive(Debug, Clone, Copy)]
pub struct ScriptInstr {
    pub frame: i32,
    pub opcode: i16,
    pub args: [i32; 3],
}

pub struct Std {
    pub objects: Vec<Object>,
    pub instances: Vec<Instance>,
    pub script: Vec<ScriptInstr>,
}

impl Std {
    pub fn parse(d: &[u8]) -> Option<Self> {
        let nb_objects = i16(d, 0) as usize;
        let faces_offset = i32_(d, 4) as usize; // instance list
        let script_offset = i32_(d, 8) as usize;
        const HEADER: usize = 0x490;

        // Object offset table follows the header.
        let mut objects = Vec::with_capacity(nb_objects);
        for i in 0..nb_objects {
            let obj_off = u32_(d, HEADER + i * 4) as usize;
            objects.push(parse_object(d, obj_off)?);
        }

        // Instances.
        let mut instances = Vec::new();
        let mut o = faces_offset;
        while o + 16 <= d.len() {
            let id = i16(d, o);
            if id < 0 {
                break;
            }
            instances.push(Instance {
                id,
                pos: [f32_(d, o + 4), f32_(d, o + 8), f32_(d, o + 12)],
            });
            o += 16;
        }

        // Script.
        let mut script = Vec::new();
        let mut o = script_offset;
        while o + 20 <= d.len() {
            let frame = i32_(d, o);
            let opcode = i16(d, o + 4);
            script.push(ScriptInstr {
                frame,
                opcode,
                args: [i32_(d, o + 8), i32_(d, o + 12), i32_(d, o + 16)],
            });
            if frame < 0 {
                break;
            }
            o += 20;
        }

        Some(Self { objects, instances, script })
    }
}

fn parse_object(d: &[u8], base: usize) -> Option<Object> {
    let z_level = d.get(base + 2).copied()? as i8;
    let pos = [f32_(d, base + 4), f32_(d, base + 8), f32_(d, base + 12)];
    let size = [f32_(d, base + 16), f32_(d, base + 20), f32_(d, base + 24)];
    // Quads start at the inline firstQuad (offset 0x1c: after id/flags,
    // vec3 position and vec3 size).
    let mut quads = Vec::new();
    let mut o = base + 0x1c;
    loop {
        let ty = i16(d, o);
        if ty < 0 {
            break;
        }
        let byte_size = i16(d, o + 2) as usize;
        quads.push(Quad {
            anm_script: i16(d, o + 4),
            pos: [f32_(d, o + 8), f32_(d, o + 12), f32_(d, o + 16)],
            size: [f32_(d, o + 20), f32_(d, o + 24)],
        });
        if byte_size == 0 {
            break;
        }
        o += byte_size;
    }
    Some(Object { z_level, pos, size, quads })
}
