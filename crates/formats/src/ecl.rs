//! ECL enemy-script files (th06).
//!
//! Layout per the th06 decompilation (EclManager.hpp): header of
//! `i16 subCount, i16 mainCount`, 3 timeline offsets, then `subCount`
//! sub offsets. Sub instructions: `i32 time, i16 opcode, i16 offsetToNext,
//! u8 _, u8 skipForDifficulty, u16 _, args...`. Timeline instructions:
//! `i16 time, i16 arg0, i16 opcode, i16 size, 20-byte args`.
//!
//! Jump offsets are byte-relative to the current instruction, so the VM
//! addresses instructions by byte offset and parses on demand.

pub struct Ecl {
    pub data: Vec<u8>,
    pub sub_offsets: Vec<u32>,
    pub timeline_offset: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Instr<'a> {
    pub time: i32,
    pub opcode: i16,
    pub offset_to_next: i16,
    pub skip_for_difficulty: u8,
    pub args: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
pub struct TimelineInstr<'a> {
    pub time: i16,
    pub arg0: i16,
    pub opcode: i16,
    pub size: i16,
    pub args: &'a [u8],
}

#[derive(Debug)]
pub enum Error {
    Truncated,
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(d[o..o + 2].try_into().unwrap())
}
fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

impl Ecl {
    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        if data.len() < 16 {
            return Err(Error::Truncated);
        }
        let sub_count = u16_at(&data, 0) as usize;
        let timeline_offset = u32_at(&data, 4);
        let mut sub_offsets = Vec::with_capacity(sub_count);
        for i in 0..sub_count {
            let off = 16 + i * 4;
            if off + 4 > data.len() {
                return Err(Error::Truncated);
            }
            sub_offsets.push(u32_at(&data, off));
        }
        Ok(Self {
            data,
            sub_offsets,
            timeline_offset,
        })
    }

    /// Parse the sub instruction at an absolute byte offset.
    pub fn instr_at(&self, offset: u32) -> Option<Instr<'_>> {
        let o = offset as usize;
        if o + 12 > self.data.len() {
            return None;
        }
        let offset_to_next = u16_at(&self.data, o + 6) as i16;
        let args_end = o + offset_to_next.unsigned_abs() as usize;
        Some(Instr {
            time: u32_at(&self.data, o) as i32,
            opcode: u16_at(&self.data, o + 4) as i16,
            offset_to_next,
            skip_for_difficulty: self.data[o + 9],
            args: self.data.get(o + 12..args_end.min(self.data.len()))?,
        })
    }

    /// Parse the timeline instruction at an absolute byte offset.
    pub fn timeline_at(&self, offset: u32) -> Option<TimelineInstr<'_>> {
        let o = offset as usize;
        if o + 8 > self.data.len() {
            return None;
        }
        let size = u16_at(&self.data, o + 6) as i16;
        let args_end = o + size.unsigned_abs() as usize;
        Some(TimelineInstr {
            time: u16_at(&self.data, o) as i16,
            arg0: u16_at(&self.data, o + 2) as i16,
            opcode: u16_at(&self.data, o + 4) as i16,
            size,
            args: self.data.get(o + 8..args_end.min(self.data.len()))?,
        })
    }
}

impl Instr<'_> {
    pub fn arg_i32(&self, i: usize) -> i32 {
        i32::from_le_bytes(self.args[i * 4..i * 4 + 4].try_into().unwrap())
    }
    pub fn arg_f32(&self, i: usize) -> f32 {
        f32::from_bits(self.arg_i32(i) as u32)
    }
    pub fn arg_i16(&self, byte: usize) -> i16 {
        i16::from_le_bytes(self.args[byte..byte + 2].try_into().unwrap())
    }
    /// Trailing C string (spell card names).
    pub fn arg_str(&self, from: usize) -> String {
        let bytes = &self.args[from.min(self.args.len())..];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
}

impl TimelineInstr<'_> {
    /// All accessors take byte offsets into the args block.
    pub fn arg_i32(&self, byte: usize) -> i32 {
        i32::from_le_bytes(self.args[byte..byte + 4].try_into().unwrap())
    }
    pub fn arg_f32(&self, byte: usize) -> f32 {
        f32::from_bits(self.arg_i32(byte) as u32)
    }
    pub fn arg_u16(&self, byte: usize) -> u16 {
        u16::from_le_bytes(self.args[byte..byte + 2].try_into().unwrap())
    }
}
