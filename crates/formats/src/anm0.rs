//! ANM version 0 (th06) sprite/animation archives.
//!
//! Layout per entry: 64-byte header of 16 u32s, `num_sprites` u32 sprite
//! offsets, `num_scripts` (id, offset) u32 pairs, then null-terminated
//! texture names, 20-byte sprite records and bytecode scripts. Entries
//! chain through `next_offset`. Offsets are relative to the entry start.

#[derive(Debug, Clone)]
pub struct Sprite {
    pub index: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct Instr {
    pub time: u16,
    pub opcode: u8,
    pub args: Vec<u8>,
}

impl Instr {
    pub fn arg_f32(&self, i: usize) -> f32 {
        f32::from_le_bytes(self.args[i * 4..i * 4 + 4].try_into().unwrap())
    }
    pub fn arg_u32(&self, i: usize) -> u32 {
        u32::from_le_bytes(self.args[i * 4..i * 4 + 4].try_into().unwrap())
    }
}

#[derive(Debug)]
pub struct Entry {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub name: String,
    pub alpha_name: Option<String>,
    pub sprites: Vec<Sprite>,
    pub scripts: Vec<(u32, Vec<Instr>)>,
}

#[derive(Debug)]
pub struct Anm0 {
    pub entries: Vec<Entry>,
}

#[derive(Debug)]
pub enum Error {
    Truncated,
}

fn u32_at(data: &[u8], off: usize) -> Result<u32, Error> {
    data.get(off..off + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or(Error::Truncated)
}

fn f32_at(data: &[u8], off: usize) -> Result<f32, Error> {
    Ok(f32::from_bits(u32_at(data, off)?))
}

fn string_at(data: &[u8], off: usize) -> Result<String, Error> {
    let tail = data.get(off..).ok_or(Error::Truncated)?;
    let end = tail.iter().position(|&b| b == 0).ok_or(Error::Truncated)?;
    Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
}

impl Anm0 {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut entries = Vec::new();
        let mut base = 0usize;
        loop {
            let entry_data = data.get(base..).ok_or(Error::Truncated)?;
            let next_offset = u32_at(entry_data, 0x38)? as usize;
            let end = if next_offset != 0 { next_offset } else { entry_data.len() };
            entries.push(Self::parse_entry(&entry_data[..end])?);
            if next_offset == 0 {
                break;
            }
            base += next_offset;
        }
        Ok(Self { entries })
    }

    fn parse_entry(data: &[u8]) -> Result<Entry, Error> {
        let num_sprites = u32_at(data, 0x00)? as usize;
        let num_scripts = u32_at(data, 0x04)? as usize;
        let width = u32_at(data, 0x0c)?;
        let height = u32_at(data, 0x10)?;
        let format = u32_at(data, 0x14)?;
        let name_offset = u32_at(data, 0x1c)? as usize;
        let alpha_name_offset = u32_at(data, 0x24)? as usize;

        let name = string_at(data, name_offset)?;
        let alpha_name = if alpha_name_offset != 0 {
            Some(string_at(data, alpha_name_offset)?)
        } else {
            None
        };

        let mut sprites = Vec::with_capacity(num_sprites);
        for i in 0..num_sprites {
            let off = u32_at(data, 0x40 + i * 4)? as usize;
            sprites.push(Sprite {
                index: u32_at(data, off)?,
                x: f32_at(data, off + 4)?,
                y: f32_at(data, off + 8)?,
                width: f32_at(data, off + 12)?,
                height: f32_at(data, off + 16)?,
            });
        }

        // Script regions are bounded by the next script's offset (scripts
        // are stored last in the entry, in offset order).
        let pairs_base = 0x40 + num_sprites * 4;
        let mut script_offsets = Vec::with_capacity(num_scripts);
        for i in 0..num_scripts {
            let id = u32_at(data, pairs_base + i * 8)?;
            let off = u32_at(data, pairs_base + i * 8 + 4)? as usize;
            script_offsets.push((id, off));
        }
        let mut bounds: Vec<usize> = script_offsets.iter().map(|&(_, o)| o).collect();
        bounds.sort_unstable();

        let mut scripts = Vec::with_capacity(num_scripts);
        for &(id, off) in &script_offsets {
            let end = bounds
                .iter()
                .copied()
                .find(|&b| b > off)
                .unwrap_or(data.len());
            scripts.push((id, Self::parse_script(&data[..end], off)?));
        }

        Ok(Entry { width, height, format, name, alpha_name, sprites, scripts })
    }

    fn parse_script(data: &[u8], mut off: usize) -> Result<Vec<Instr>, Error> {
        let mut instrs = Vec::new();
        while off + 4 <= data.len() {
            let time = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
            let opcode = data[off + 2];
            let length = data[off + 3] as usize;
            let args = data
                .get(off + 4..off + 4 + length)
                .ok_or(Error::Truncated)?
                .to_vec();
            off += 4 + length;
            let is_end = opcode == 0 && time == 0;
            instrs.push(Instr { time, opcode, args });
            if is_end {
                break;
            }
        }
        Ok(instrs)
    }
}
