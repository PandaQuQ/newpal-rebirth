//! LMF map archives (`huge.lmf`, `huge2.lmf`). See FINDINGS.md section 3.
//!
//! ```text
//! u32 dirOff @0; directory @dirOff: n = (fileSize - dirOff - 1) >> 4 entries { u32 mapOffset, u32, u32 mapId, u32 }
//! map: "MSF3", header H @ map+4 (offsets relative to map start)
//!   H+0x10/0x14 regions   H+0x18/0x1C objtab   H+0x20/0x24 placements   H+0x38 background block
//!   H+0x100 u16 w, u16 h (16px cells)   H+0x108 u16 layers
//! tile grid: u32[w*h*layers] @ map+0x118, low 16 bits = tile index into the background tile pool
//! background block: 0x400 header {u32 1, u32 chunkUnpacked, u32 total, .., @0x10 (u32 offRelMap, u32 packed) pairs}
//!   each chunk LZO1X -> concatenated 16x16 RGB555 tiles (512 bytes each)
//! ```
use crate::bytes::{read_at, u16le, u32le};
use crate::image::{rgb555, RgbaImage};
use crate::legacy::decode_cstr;
use crate::tsw::decode_rle;
use crate::{lzo, Error, Result};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const TILE: u32 = 16;

#[derive(Clone, Debug)]
pub struct ObjRec {
    pub off: u32,
    pub packed: u32,
    pub unpacked: u32,
    pub id: u16,
    pub flags: u16,
}

/// Object placement. The object frame drawn at (x0, y0) is a pixel-exact cut-out of the background;
/// `y1` is the depth baseline: actors whose feet are above `y1` are drawn behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub id: u16,
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
    pub flags: u16,
}

#[derive(Clone, Debug)]
pub struct Region {
    pub v: [u32; 4],
    pub name: String,
}

pub struct LmfMap {
    pub id: u32,
    pub offset: u64,
    /// size in 16px cells
    pub w: u16,
    pub h: u16,
    pub layers: u16,
    pub grid: Vec<u32>,
    pub tiles: Vec<u8>,
    pub objtab: HashMap<u16, ObjRec>,
    pub placements: Vec<Placement>,
    pub regions: Vec<Region>,
    /// per-cell attribute words (H+0x08 LZO section, first w*h u32). See [cell] for bits.
    pub cells: Vec<u32>,
}

impl LmfMap {
    pub fn pixel_size(&self) -> (u32, u32) {
        (self.w as u32 * TILE, self.h as u32 * TILE)
    }

    /// Compose the background (layer 0) from the tile pool.
    pub fn render_bg(&self) -> RgbaImage {
        let (pw, ph) = self.pixel_size();
        let mut img = RgbaImage::new(pw, ph);
        for cy in 0..self.h as u32 {
            for cx in 0..self.w as u32 {
                let t = (self.grid[(cy * self.w as u32 + cx) as usize] & 0xFFFF) as usize;
                let base = t * 512;
                if base + 512 > self.tiles.len() {
                    continue;
                }
                for y in 0..16u32 {
                    for x in 0..16u32 {
                        let o = base + ((y * 16 + x) * 2) as usize;
                        let v = u16::from_le_bytes([self.tiles[o], self.tiles[o + 1]]);
                        img.put(cx * 16 + x, cy * 16 + y, rgb555(v));
                    }
                }
            }
        }
        img
    }
}

pub struct Lmf {
    pub path: PathBuf,
    file: Mutex<File>,
    pub size: u64,
    /// map id -> absolute offset
    pub maps: BTreeMap<u32, u64>,
}

impl Lmf {
    pub fn open(path: &Path) -> Result<Self> {
        let ctx = path.display().to_string();
        let f = File::open(path).map_err(|e| Error::io(e, ctx.clone()))?;
        let size = f.metadata().map_err(|e| Error::io(e, ctx.clone()))?.len();
        let file = Mutex::new(f);
        let dir_off = u32le(&read_at(&file, 0, 4, &ctx)?, 0)? as u64;
        if dir_off >= size {
            return Err(Error::corrupt("lmf directory offset"));
        }
        let n = ((size - dir_off - 1) >> 4) as usize;
        let dir = read_at(&file, dir_off, n * 16, &ctx)?;
        let mut maps = BTreeMap::new();
        for i in 0..n {
            maps.insert(u32le(&dir, i * 16 + 8)?, u32le(&dir, i * 16)? as u64);
        }
        Ok(Self { path: path.to_path_buf(), file, size, maps })
    }

    fn rd(&self, off: u64, len: usize) -> Result<Vec<u8>> {
        if off + len as u64 > self.size {
            return Err(Error::corrupt(format!("lmf read out of range @{off:#x}+{len}")));
        }
        read_at(&self.file, off, len, &self.path.display().to_string())
    }

    pub fn load_map(&self, id: u32) -> Result<LmfMap> {
        let m = *self.maps.get(&id).ok_or_else(|| Error::not_found(format!("map {id}")))?;
        if self.rd(m, 4)? != b"MSF3" {
            return Err(Error::corrupt(format!("map {id}: bad magic")));
        }
        let hdr = self.rd(m + 4, 0x400)?;
        let w = u16le(&hdr, 0x100)?;
        let h = u16le(&hdr, 0x102)?;
        let layers = match u16le(&hdr, 0x108)? { 0 => 1, l => l };
        let cells = w as usize * h as usize;
        let gb = self.rd(m + 0x118, cells * 4 * layers as usize)?;
        let grid = gb.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();

        // background tile pool
        let bh = self.rd(m + u32le(&hdr, 0x38)? as u64, 0x400)?;
        let chunk_unpacked = u32le(&bh, 4)? as usize;
        let total = u32le(&bh, 8)? as usize;
        let mut tiles = Vec::with_capacity(total);
        let mut k = 0usize;
        while tiles.len() < total && k < 120 {
            let off = u32le(&bh, 0x10 + k * 4)? as u64;
            let sz = u32le(&bh, 0x14 + k * 4)? as usize;
            let d = lzo::decompress(&self.rd(m + off, sz)?, chunk_unpacked)?;
            tiles.extend_from_slice(&d);
            k += 2;
        }

        // cell attributes (LZO-decompressed into a w*h*8 buffer; the engine reads a u32 per cell)
        let cl = u32le(&hdr, 0x0C)? as usize;
        let raw = lzo::decompress(&self.rd(m + u32le(&hdr, 0x08)? as u64, cl)?, cells * 8)?;
        let cells_v: Vec<u32> = raw.chunks_exact(4).take(cells).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();

        // regions
        let b = self.rd(m + u32le(&hdr, 0x10)? as u64, u32le(&hdr, 0x14)? as usize)?;
        let mut regions = Vec::new();
        for i in 0..u32le(&b, 0)? as usize {
            let p = 4 + i * 48;
            let name_bytes = b.get(p + 16..p + 48).ok_or_else(|| Error::corrupt("region name"))?;
            regions.push(Region {
                v: [u32le(&b, p)?, u32le(&b, p + 4)?, u32le(&b, p + 8)?, u32le(&b, p + 12)?],
                name: decode_cstr(name_bytes),
            });
        }

        // object table
        let c = self.rd(m + u32le(&hdr, 0x18)? as u64, u32le(&hdr, 0x1C)? as usize)?;
        let mut objtab = HashMap::new();
        for i in 0..u16le(&c, 0)? as usize {
            let p = 8 + i * 16;
            let rec = ObjRec {
                off: u32le(&c, p)?,
                packed: u32le(&c, p + 4)?,
                unpacked: u32le(&c, p + 8)?,
                id: u16le(&c, p + 12)?,
                flags: u16le(&c, p + 14)?,
            };
            objtab.insert(rec.id, rec);
        }

        // placements
        let d = self.rd(m + u32le(&hdr, 0x20)? as u64, u32le(&hdr, 0x24)? as usize)?;
        let mut placements = Vec::new();
        for i in 0..u16le(&d, 0)? as usize {
            let p = 8 + i * 16;
            placements.push(Placement {
                id: u16le(&d, p)?,
                x0: u16le(&d, p + 2)?,
                y0: u16le(&d, p + 4)?,
                x1: u16le(&d, p + 6)?,
                y1: u16le(&d, p + 8)?,
                flags: u16le(&d, p + 10)?,
            });
        }
        Ok(LmfMap { id, offset: m, w, h, layers, grid, tiles, objtab, placements, regions, cells: cells_v })
    }

    /// Decode an object (background cut-out) frame. `None` when the id is not in the object table.
    pub fn object_image(&self, map: &LmfMap, oid: u16) -> Result<Option<RgbaImage>> {
        let Some(t) = map.objtab.get(&oid) else { return Ok(None) };
        let raw = self.rd(map.offset + t.off as u64, t.packed as usize)?;
        let raw = if t.packed == t.unpacked { raw } else { lzo::decompress(&raw, t.unpacked as usize)? };
        decode_rle(&raw, None).map(Some)
    }
}