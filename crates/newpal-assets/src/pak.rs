//! Index archives: `.tsw` (sprites), `.act` (action tables), `all.snd` (sounds).
//!
//! ```text
//! 0x00  [u8; 0x18] header (unknown)     0x18 u32 declared count (may exceed 3000)
//! 0x1C  3000 x Slot (44 bytes)          data starts at 0x1C + 3000*44 = 132028
//! Slot { name: [u8;20] Big5 NUL-padded, size: u32, offset: u32 (absolute), id: u32 (= slot+1), pad: [u8;12] }
//! ```
use crate::bytes::{read_at, u32le};
use crate::legacy::decode_cstr;
use crate::{Error, Result};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const SLOTS: usize = 3000;
pub const SLOT_SIZE: usize = 44;
pub const HEADER_SIZE: usize = 0x1C;
pub const DATA_START: usize = HEADER_SIZE + SLOTS * SLOT_SIZE;

#[derive(Clone, Debug)]
pub struct PakEntry {
    pub slot: usize,
    pub name: String,
    pub size: u32,
    pub offset: u32,
    pub id: u32,
}

pub struct IndexArchive {
    pub path: PathBuf,
    file: Mutex<File>,
    pub file_size: u64,
    pub declared: u32,
    pub entries: Vec<PakEntry>,
}

impl IndexArchive {
    /// Open read-only. Never writes to the file.
    pub fn open(path: &Path) -> Result<Self> {
        let ctx = path.display().to_string();
        let mut f = File::open(path).map_err(|e| Error::io(e, ctx.clone()))?;
        let file_size = f.metadata().map_err(|e| Error::io(e, ctx.clone()))?.len();
        let mut head = [0u8; HEADER_SIZE];
        f.read_exact(&mut head).map_err(|e| Error::io(e, ctx.clone()))?;
        let declared = u32le(&head, 0x18)?;
        let count = (declared as usize).min(SLOTS);
        let mut idx = vec![0u8; count * SLOT_SIZE];
        f.read_exact(&mut idx).map_err(|e| Error::io(e, ctx.clone()))?;
        let mut entries = Vec::with_capacity(count);
        for slot in 0..count {
            let r = &idx[slot * SLOT_SIZE..(slot + 1) * SLOT_SIZE];
            entries.push(PakEntry {
                slot,
                name: decode_cstr(&r[0..20]),
                size: u32le(r, 20)?,
                offset: u32le(r, 24)?,
                id: u32le(r, 28)?,
            });
        }
        Ok(Self { path: path.to_path_buf(), file: Mutex::new(f), file_size, declared, entries })
    }

    pub fn entry(&self, slot: usize) -> Result<&PakEntry> {
        self.entries.get(slot).ok_or_else(|| Error::not_found(format!("{}: slot {slot}", self.path.display())))
    }

    /// Read the raw bytes of a slot.
    pub fn read(&self, slot: usize) -> Result<Vec<u8>> {
        let e = self.entry(slot)?;
        if e.size == 0 {
            return Err(Error::not_found(format!("{}: slot {slot} is empty", self.path.display())));
        }
        if e.offset as u64 + e.size as u64 > self.file_size {
            return Err(Error::corrupt(format!("{}: slot {slot} out of range", self.path.display())));
        }
        read_at(&self.file, e.offset as u64, e.size as usize, &self.path.display().to_string())
    }

    pub fn find(&self, name: &str) -> Option<&PakEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// `all.snd` entry: 16-byte WAVEFORMAT (PCM) + 2 junk bytes + PCM. Returns a complete RIFF/WAVE file.
pub fn snd_to_wav(entry: &[u8]) -> Result<Vec<u8>> {
    if entry.len() < 18 {
        return Err(Error::corrupt("snd entry too short"));
    }
    let pcm = &entry[18..];
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&entry[0..16]);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    Ok(out)
}