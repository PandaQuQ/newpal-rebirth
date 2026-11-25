//! Prepared smooth-animation cache (`smooth.pak`), generated locally by the launcher's prepare step from
//! the player's own game data (never distributed).
//!
//! Layout: `b"NPSMPAK1"`, `u32 manifest_len`, manifest JSON, then PNG blobs. Offsets in the manifest are
//! relative to the first blob byte. Writes go to `<file>.tmp` and are renamed into place (atomic).
use crate::image::RgbaImage;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MAGIC: &[u8; 8] = b"NPSMPAK1";

/// One prepared animation (`<act>/<slot>/<action>`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// sha256 of the source frames (pixels + durations)
    pub src: String,
    /// backend id incl. model/version, e.g. "cpu-crossfade/1" or "rife-ncnn-vulkan-20221029/rife-v4.6"
    pub backend: String,
    pub factor: u32,
    /// generator algorithm version (bump = regenerate)
    pub algo: u32,
    pub canvas: [u32; 2],
    pub durations_ms: Vec<f32>,
    /// (offset, len) of each PNG
    pub frames: Vec<(u64, u64)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub entries: BTreeMap<String, Entry>,
    /// free-form generation info (backend, workers, time)
    #[serde(default)]
    pub info: BTreeMap<String, String>,
}

pub fn key(act: &str, slot: usize, action: usize) -> String {
    format!("{act}/{slot}/{action}")
}

pub struct SmoothPak {
    pub path: PathBuf,
    pub manifest: Manifest,
    base: u64,
    file: Mutex<File>,
}

impl SmoothPak {
    pub fn open(path: &Path) -> Result<Self> {
        let mut f = File::open(path).map_err(|e| Error::io(e, path.display().to_string()))?;
        let mut head = [0u8; 12];
        f.read_exact(&mut head).map_err(|e| Error::io(e, path.display().to_string()))?;
        if &head[..8] != MAGIC {
            return Err(Error::corrupt("smooth.pak: bad magic"));
        }
        let n = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
        let mut js = vec![0u8; n];
        f.read_exact(&mut js).map_err(|e| Error::io(e, path.display().to_string()))?;
        let manifest: Manifest = serde_json::from_slice(&js).map_err(|e| Error::corrupt(format!("smooth.pak manifest: {e}")))?;
        Ok(Self { path: path.to_path_buf(), manifest, base: 12 + n as u64, file: Mutex::new(f) })
    }

    pub fn get(&self, k: &str) -> Option<&Entry> {
        self.manifest.entries.get(k)
    }

    pub fn read_blob(&self, off: u64, len: u64) -> Result<Vec<u8>> {
        let mut f = self.file.lock().unwrap();
        f.seek(SeekFrom::Start(self.base + off)).map_err(|e| Error::io(e, "smooth.pak seek"))?;
        let mut v = vec![0u8; len as usize];
        f.read_exact(&mut v).map_err(|e| Error::io(e, "smooth.pak read"))?;
        Ok(v)
    }

    /// Decoded frames of an entry.
    pub fn frames(&self, e: &Entry) -> Result<Vec<RgbaImage>> {
        e.frames.iter().map(|&(o, l)| RgbaImage::from_png(&self.read_blob(o, l)?)).collect()
    }
}

/// An entry with its PNG blobs in memory (staging / writing).
#[derive(Clone, Debug)]
pub struct OwnedEntry {
    pub key: String,
    pub meta: Entry,
    pub pngs: Vec<Vec<u8>>,
}

impl OwnedEntry {
    pub fn from_pak(p: &SmoothPak, key: &str) -> Result<Self> {
        let meta = p.get(key).ok_or_else(|| Error::not_found(key.to_string()))?.clone();
        let pngs = meta.frames.iter().map(|&(o, l)| p.read_blob(o, l)).collect::<Result<Vec<_>>>()?;
        Ok(Self { key: key.to_string(), meta, pngs })
    }
}

/// Write a pak atomically from owned entries.
pub fn write(path: &Path, entries: &[OwnedEntry], info: BTreeMap<String, String>) -> Result<u64> {
    let mut m = Manifest { entries: BTreeMap::new(), info };
    let mut off = 0u64;
    for e in entries {
        let mut meta = e.meta.clone();
        meta.frames = e.pngs.iter().map(|b| {
            let r = (off, b.len() as u64);
            off += b.len() as u64;
            r
        }).collect();
        m.entries.insert(e.key.clone(), meta);
    }
    let js = serde_json::to_vec(&m).map_err(|e| Error::corrupt(e.to_string()))?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::io(e, d.display().to_string()))?;
    }
    let tmp = path.with_extension("pak.tmp");
    {
        let f = File::create(&tmp).map_err(|e| Error::io(e, tmp.display().to_string()))?;
        let mut w = std::io::BufWriter::new(f);
        let io = |e| Error::io(e, "smooth.pak write");
        w.write_all(MAGIC).map_err(io)?;
        w.write_all(&(js.len() as u32).to_le_bytes()).map_err(io)?;
        w.write_all(&js).map_err(io)?;
        // blobs in the same order as the offsets
        for e in entries {
            for b in &e.pngs {
                w.write_all(b).map_err(io)?;
            }
        }
        w.flush().map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| Error::io(e, path.display().to_string()))?;
    Ok(12 + js.len() as u64 + off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("npsm-test-{}", std::process::id()));
        let p = dir.join("smooth.pak");
        let mut img = RgbaImage::new(2, 3);
        img.put(1, 2, [9, 8, 7, 255]);
        let png = img.to_png().unwrap();
        let meta = Entry { src: "abc".into(), backend: "cpu-crossfade/1".into(), factor: 2, algo: 1, canvas: [2, 3], durations_ms: vec![10.0, 10.0], frames: vec![] };
        let e = OwnedEntry { key: key("all_char", 0, 8), meta, pngs: vec![png.clone(), png] };
        write(&p, &[e], BTreeMap::new()).unwrap();
        let pak = SmoothPak::open(&p).unwrap();
        let en = pak.get("all_char/0/8").unwrap();
        assert_eq!(en.frames.len(), 2);
        let fr = pak.frames(en).unwrap();
        assert_eq!(fr[1].pixels, img.pixels);
        let o = OwnedEntry::from_pak(&pak, "all_char/0/8").unwrap();
        assert_eq!(o.pngs.len(), 2);
        drop(pak);
        let _ = std::fs::remove_dir_all(&dir);
    }
}