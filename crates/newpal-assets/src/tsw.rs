//! TSW sprite entries and the shared RLE frame format.
//!
//! ```text
//! entry: u32 size, u16 magic=0xABCD, u16 nframes, u16 bpp (8|16), u16 unk
//!        [bpp==8: 256 x u16 RGB555 palette]
//!        nframes x FrameDesc (9 x i32): [0] offset from entry start, [1] packed, [2] unpacked, [3] flag,
//!                                       [6] =1 blend?, [8] = (h<<16)|w
//! frame bytes: LZO1X(packed) -> unpacked (stored raw when packed == unpacked)
//! RLE frame: u16 0xFFFF, u16 w, u16 h, u16 bpp|flag; rows: u16 rowLen (&0x3FFF, bytes incl. itself; 0 = end)
//!            then u16 ctrl until 0: ctrl&0xC000 == 0 -> n literal pixels; 0x8000/0xC000 -> skip n; 0x4000 -> marker
//! ```
use crate::bytes::{i32le, u16le};
use crate::image::{rgb555, RgbaImage};
use crate::{lzo, Error, Result};

pub const MAGIC: u16 = 0xABCD;

#[derive(Clone, Debug)]
pub struct FrameDesc {
    pub raw: [i32; 9],
}

impl FrameDesc {
    pub fn offset(&self) -> usize { self.raw[0] as usize }
    pub fn packed(&self) -> usize { self.raw[1] as usize }
    pub fn unpacked(&self) -> usize { self.raw[2] as usize }
    pub fn width(&self) -> u32 { (self.raw[8] as u32) & 0xFFFF }
    pub fn height(&self) -> u32 { (self.raw[8] as u32) >> 16 }
}

pub struct SpriteEntry {
    pub bpp: u16,
    pub unk: u16,
    pub palette: Option<Vec<u16>>,
    pub descs: Vec<FrameDesc>,
    buf: Vec<u8>,
}

impl SpriteEntry {
    pub fn parse(buf: Vec<u8>) -> Result<Self> {
        let magic = u16le(&buf, 4)?;
        if magic != MAGIC {
            return Err(Error::corrupt(format!("tsw entry magic {magic:#06x}")));
        }
        let n = u16le(&buf, 6)? as usize;
        let bpp = u16le(&buf, 8)?;
        let unk = u16le(&buf, 10)?;
        let mut p = 12;
        let palette = if bpp == 8 {
            let mut pal = Vec::with_capacity(256);
            for i in 0..256 {
                pal.push(u16le(&buf, p + i * 2)?);
            }
            p += 512;
            Some(pal)
        } else {
            None
        };
        let mut descs = Vec::with_capacity(n);
        for i in 0..n {
            let mut raw = [0i32; 9];
            for (k, v) in raw.iter_mut().enumerate() {
                *v = i32le(&buf, p + i * 36 + k * 4)?;
            }
            descs.push(FrameDesc { raw });
        }
        Ok(Self { bpp, unk, palette, descs, buf })
    }

    pub fn frame_count(&self) -> usize {
        self.descs.len()
    }

    /// Decompressed RLE bytes of frame `i`.
    pub fn frame_raw(&self, i: usize) -> Result<Vec<u8>> {
        let d = self.descs.get(i).ok_or_else(|| Error::not_found(format!("frame {i}")))?;
        let packed = self
            .buf
            .get(d.offset()..d.offset() + d.packed())
            .ok_or_else(|| Error::corrupt(format!("frame {i} data out of range")))?;
        if d.packed() == d.unpacked() {
            Ok(packed.to_vec())
        } else {
            lzo::decompress(packed, d.unpacked())
        }
    }

    pub fn frame_image(&self, i: usize) -> Result<RgbaImage> {
        decode_rle(&self.frame_raw(i)?, self.palette.as_deref())
    }
}

/// Decode an RLE frame to RGBA8 (transparent = all zero).
pub fn decode_rle(f: &[u8], pal: Option<&[u16]>) -> Result<RgbaImage> {
    if u16le(f, 0)? != 0xFFFF {
        return Err(Error::corrupt("rle frame magic"));
    }
    let w = u16le(f, 2)? as u32;
    let h = u16le(f, 4)? as u32;
    let bpp = u16le(f, 6)? & 0x7FFF;
    let mut img = RgbaImage::new(w, h);
    let mut p = 8usize;
    let mut y = 0u32;
    while p + 2 <= f.len() {
        let rl = (u16le(f, p)? & 0x3FFF) as usize;
        if rl == 0 {
            break;
        }
        let row_end = p + rl;
        let mut q = p + 2;
        let mut x = 0u32;
        while q + 2 <= f.len() {
            let c = u16le(f, q)?;
            q += 2;
            if c == 0 {
                break;
            }
            let n = (c & 0x3FFF) as u32;
            match c & 0xC000 {
                0 => {
                    for _ in 0..n {
                        let v = if bpp == 16 {
                            let v = u16le(f, q)?;
                            q += 2;
                            v
                        } else {
                            let idx = *f.get(q).ok_or_else(|| Error::corrupt("rle overrun"))? as usize;
                            q += 1;
                            let pal = pal.ok_or_else(|| Error::corrupt("8bpp frame without palette"))?;
                            pal[idx]
                        };
                        img.put(x, y, rgb555(v));
                        x += 1;
                    }
                }
                0x8000 | 0xC000 => x += n,
                _ => {} // 0x4000 marker, no advance
            }
        }
        p = row_end;
        y += 1;
    }
    Ok(img)
}