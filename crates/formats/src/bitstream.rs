/// MSB-first bit reader over a byte slice, as used by PBG3 archives.
pub struct BitStream<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> BitStream<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, bit: 0 }
    }

    pub fn seek(&mut self, byte_pos: usize) {
        self.pos = byte_pos;
        self.bit = 0;
    }

    pub fn read_bit(&mut self) -> Option<bool> {
        let byte = *self.data.get(self.pos)?;
        let value = byte & (0x80 >> self.bit) != 0;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Some(value)
    }

    pub fn read(&mut self, nbits: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..nbits {
            value = (value << 1) | self.read_bit()? as u32;
        }
        Some(value)
    }

    /// PBG3 variable-length integer: 2 bits give the byte count minus one.
    pub fn read_int(&mut self) -> Option<u32> {
        let size = self.read(2)?;
        self.read((size + 1) * 8)
    }

    /// Null-terminated string of 8-bit characters, at most `max` bytes.
    pub fn read_string(&mut self, max: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        for _ in 0..max {
            let byte = self.read(8)? as u8;
            if byte == 0 {
                break;
            }
            out.push(byte);
        }
        Some(out)
    }
}
