//! Bounds-checked little-endian readers and positional file reads.
use crate::{Error, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

#[inline]
pub fn u16le(b: &[u8], o: usize) -> Result<u16> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| Error::corrupt(format!("u16 read past end at {o:#x}")))
}

#[inline]
pub fn u32le(b: &[u8], o: usize) -> Result<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| Error::corrupt(format!("u32 read past end at {o:#x}")))
}

#[inline]
pub fn i32le(b: &[u8], o: usize) -> Result<i32> {
    u32le(b, o).map(|v| v as i32)
}

/// Find `needle` in `hay` starting at `from`.
pub fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Read `len` bytes at absolute `off` from a shared file handle.
pub fn read_at(file: &Mutex<File>, off: u64, len: usize, ctx: &str) -> Result<Vec<u8>> {
    let mut f = file.lock().map_err(|_| Error::corrupt("file lock poisoned"))?;
    f.seek(SeekFrom::Start(off)).map_err(|e| Error::io(e, ctx.to_string()))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|e| Error::io(e, format!("{ctx} @{off:#x}+{len}")))?;
    Ok(buf)
}