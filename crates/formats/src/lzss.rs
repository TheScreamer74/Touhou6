use crate::bitstream::BitStream;

const DICT_SIZE: usize = 0x2000;
const OFFSET_BITS: u32 = 13;
const LENGTH_BITS: u32 = 4;
const MIN_MATCH: u32 = 3;

/// LZSS decompression as used by PBG3: control bit 1 = literal byte,
/// 0 = back-reference (13-bit absolute dictionary offset, offset 0 ends
/// the stream; 4-bit length + 3).
pub fn decompress(bits: &mut BitStream, expected_size: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(expected_size);
    let mut dict = [0u8; DICT_SIZE];
    let mut head: usize = 1;

    while out.len() < expected_size {
        if bits.read_bit()? {
            let byte = bits.read(8)? as u8;
            out.push(byte);
            dict[head] = byte;
            head = (head + 1) % DICT_SIZE;
        } else {
            let offset = bits.read(OFFSET_BITS)? as usize;
            if offset == 0 {
                break;
            }
            let length = bits.read(LENGTH_BITS)? + MIN_MATCH;
            for i in 0..length as usize {
                let byte = dict[(offset + i) % DICT_SIZE];
                out.push(byte);
                dict[head] = byte;
                head = (head + 1) % DICT_SIZE;
            }
        }
    }
    Some(out)
}
