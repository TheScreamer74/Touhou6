//! MSG dialogue scripts (th06).
//!
//! Header: i32 entry count, then that many i32 offsets (file-relative).
//! Instructions: u16 time, u8 opcode, u8 argSize, args.

pub struct Msg {
    pub data: Vec<u8>,
    pub offsets: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct Instr<'a> {
    pub time: u16,
    pub opcode: u8,
    pub args: &'a [u8],
    pub size: u32,
}

impl Msg {
    pub fn parse(data: Vec<u8>) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        let count = i32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            let o = 4 + i * 4;
            offsets.push(u32::from_le_bytes(data.get(o..o + 4)?.try_into().unwrap()));
        }
        Some(Self { data, offsets })
    }

    pub fn instr_at(&self, off: u32) -> Option<Instr<'_>> {
        let o = off as usize;
        let time = u16::from_le_bytes(self.data.get(o..o + 2)?.try_into().unwrap());
        let opcode = *self.data.get(o + 2)?;
        let arg_size = *self.data.get(o + 3)? as usize;
        Some(Instr {
            time,
            opcode,
            args: self.data.get(o + 4..o + 4 + arg_size)?,
            size: 4 + arg_size as u32,
        })
    }
}

impl Instr<'_> {
    pub fn arg_i16(&self, byte: usize) -> i16 {
        i16::from_le_bytes(self.args[byte..byte + 2].try_into().unwrap())
    }
    pub fn arg_i32(&self, byte: usize) -> i32 {
        i32::from_le_bytes(self.args[byte..byte + 4].try_into().unwrap())
    }
    /// Null-terminated text starting at `from`.
    pub fn arg_str(&self, from: usize) -> String {
        let bytes = &self.args[from.min(self.args.len())..];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
}
