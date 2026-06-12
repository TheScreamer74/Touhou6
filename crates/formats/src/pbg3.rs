use crate::bitstream::BitStream;
use crate::lzss;

#[derive(Debug)]
pub struct Entry {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub checksum: u32,
}

pub struct Pbg3<'a> {
    data: &'a [u8],
    pub entries: Vec<Entry>,
}

#[derive(Debug)]
pub enum Error {
    BadMagic,
    Truncated,
}

impl<'a> Pbg3<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() < 4 || &data[..4] != b"PBG3" {
            return Err(Error::BadMagic);
        }
        let mut bits = BitStream::new(&data[4..]);
        let count = bits.read_int().ok_or(Error::Truncated)?;
        let table_offset = bits.read_int().ok_or(Error::Truncated)?;

        // Offsets in the header are absolute file positions; the bitstream
        // starts after the 4-byte magic.
        bits.seek(table_offset as usize - 4);
        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let _unknown1 = bits.read_int().ok_or(Error::Truncated)?;
            let _unknown2 = bits.read_int().ok_or(Error::Truncated)?;
            let checksum = bits.read_int().ok_or(Error::Truncated)?;
            let offset = bits.read_int().ok_or(Error::Truncated)?;
            let size = bits.read_int().ok_or(Error::Truncated)?;
            let name = bits.read_string(255).ok_or(Error::Truncated)?;
            entries.push(Entry {
                name: String::from_utf8_lossy(&name).into_owned(),
                offset,
                size,
                checksum,
            });
        }
        Ok(Self { data, entries })
    }

    pub fn extract(&self, entry: &Entry) -> Option<Vec<u8>> {
        let mut bits = BitStream::new(&self.data[4..]);
        bits.seek(entry.offset as usize - 4);
        lzss::decompress(&mut bits, entry.size as usize)
    }
}
