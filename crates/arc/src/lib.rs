//! MT Framework `.arc` archives (version 8, as shipped with Lost Planet 2 PC).
//!
//! Layout: `"ARC\0"`, `u16 version`, `u16 count`, then `count` 80-byte entries
//! (`name[64]` without extension, `u32 type hash`, `u32 compressed size`,
//! `u32 size | flags`, `u32 offset`), then zlib-compressed file data.

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};

pub mod sbc;
pub mod navi;

pub const VERSION: u16 = 8;
const ENTRY: usize = 80;
const NAME: usize = 64;
const SIZE_MASK: u32 = 0x1FFF_FFFF;
/// Where LP2's archives start their data when the table fits before it.
const DATA_ALIGN: usize = 0x8000;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Path inside the archive with `\` separators and no extension.
    pub name: String,
    /// MT Framework resource type hash (identifies the class, e.g. rTexture).
    pub type_hash: u32,
    /// Top 3 bits of the size field, preserved on repack.
    pub flags: u32,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub enum Error {
    NotArc,
    Version(u16),
    Truncated(String),
    Inflate(String, std::io::Error),
    SizeMismatch(String),
    NameTooLong(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::NotArc => write!(f, "not an ARC file"),
            Error::Version(v) => write!(f, "unsupported ARC version {v}"),
            Error::Truncated(n) => write!(f, "{n}: data runs past the end of the file"),
            Error::Inflate(n, e) => write!(f, "{n}: {e}"),
            Error::SizeMismatch(n) => write!(f, "{n}: decompressed size does not match the table"),
            Error::NameTooLong(n) => write!(f, "{n}: name longer than 63 bytes"),
        }
    }
}

impl std::error::Error for Error {}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

pub fn read(file: &[u8]) -> Result<Vec<Entry>, Error> {
    if file.len() < 8 || &file[..4] != b"ARC\0" {
        return Err(Error::NotArc);
    }
    let version = u16::from_le_bytes([file[4], file[5]]);
    if version != VERSION {
        return Err(Error::Version(version));
    }
    let count = u16::from_le_bytes([file[6], file[7]]) as usize;
    if file.len() < 8 + count * ENTRY {
        return Err(Error::Truncated("entry table".into()));
    }
    (0..count)
        .map(|i| {
            let e = &file[8 + i * ENTRY..8 + (i + 1) * ENTRY];
            let name = String::from_utf8_lossy(e[..NAME].split(|&b| b == 0).next().unwrap()).into_owned();
            let (packed, size_flags, offset) = (u32_at(e, 68) as usize, u32_at(e, 72), u32_at(e, 76) as usize);
            let raw = file.get(offset..offset + packed).ok_or_else(|| Error::Truncated(name.clone()))?;
            let size = (size_flags & SIZE_MASK) as usize;
            let mut data = Vec::with_capacity(size);
            ZlibDecoder::new(raw).read_to_end(&mut data).map_err(|err| Error::Inflate(name.clone(), err))?;
            if data.len() != size {
                return Err(Error::SizeMismatch(name));
            }
            Ok(Entry { name, type_hash: u32_at(e, 64), flags: size_flags & !SIZE_MASK, data })
        })
        .collect()
}

pub fn write(entries: &[Entry]) -> Result<Vec<u8>, Error> {
    let table_end = 8 + entries.len() * ENTRY;
    let data_start = if table_end <= DATA_ALIGN { DATA_ALIGN } else { table_end.next_multiple_of(16) };
    let mut out = vec![0u8; data_start];
    out[..4].copy_from_slice(b"ARC\0");
    out[4..6].copy_from_slice(&VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        if e.name.len() >= NAME {
            return Err(Error::NameTooLong(e.name.clone()));
        }
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(&e.data).and_then(|_| z.flush()).map_err(|err| Error::Inflate(e.name.clone(), err))?;
        let packed = z.finish().map_err(|err| Error::Inflate(e.name.clone(), err))?;
        let t = 8 + i * ENTRY;
        out[t..t + e.name.len()].copy_from_slice(e.name.as_bytes());
        out[t + 64..t + 68].copy_from_slice(&e.type_hash.to_le_bytes());
        out[t + 68..t + 72].copy_from_slice(&(packed.len() as u32).to_le_bytes());
        out[t + 72..t + 76].copy_from_slice(&(e.data.len() as u32 & SIZE_MASK | e.flags).to_le_bytes());
        let offset = out.len() as u32;
        out[t + 76..t + 80].copy_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&packed);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let entries = vec![
            Entry { name: "a\\b".into(), type_hash: 0x241F_5DEB, flags: 0x4000_0000, data: b"hello".repeat(50) },
            Entry { name: "c".into(), type_hash: 1, flags: 0, data: vec![] },
        ];
        let file = write(&entries).unwrap();
        assert_eq!(read(&file).unwrap(), entries);
        assert!(matches!(read(b"nope"), Err(Error::NotArc)));
        assert!(matches!(read(&file[..100]), Err(Error::Truncated(_))));
    }
}
