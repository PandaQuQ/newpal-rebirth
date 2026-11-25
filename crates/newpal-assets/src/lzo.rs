//! LZO1X-1 decompressor, a line-for-line port of the original engine decompressor
//! (validated byte-exact against the JS reference `tools/tsw-sprite.mjs`).
use crate::{Error, Result};

#[inline]
fn byte(src: &[u8], i: usize) -> Result<usize> {
    src.get(i).map(|&b| b as usize).ok_or_else(|| Error::corrupt("lzo: input overrun"))
}

#[inline]
fn lit(dst: &mut Vec<u8>, src: &[u8], ip: &mut usize, n: usize) -> Result<()> {
    let s = src.get(*ip..*ip + n).ok_or_else(|| Error::corrupt("lzo: literal overrun"))?;
    dst.extend_from_slice(s);
    *ip += n;
    Ok(())
}

#[inline]
fn copy_match(dst: &mut Vec<u8>, dist: usize, len: usize) -> Result<()> {
    if dist == 0 || dist > dst.len() {
        return Err(Error::corrupt("lzo: bad match distance"));
    }
    let start = dst.len() - dist;
    for k in 0..len {
        let v = dst[start + k];
        dst.push(v);
    }
    Ok(())
}

/// Decompress an LZO1X stream. `expected` is the declared unpacked size (used for capacity and a sanity cap).
pub fn decompress(src: &[u8], expected: usize) -> Result<Vec<u8>> {
    let cap = expected.saturating_mul(2) + 4096;
    let mut dst: Vec<u8> = Vec::with_capacity(expected + 64);
    let mut ip = 0usize;
    // state 0: expect literal run; 1: after short literal (<4); 2: after long literal
    let mut state = 0u8;
    let mut t = byte(src, ip)?;
    if t > 17 {
        ip += 1;
        t -= 17;
        lit(&mut dst, src, &mut ip, t)?;
        state = if t < 4 { 1 } else { 2 };
    }
    loop {
        if dst.len() > cap {
            return Err(Error::corrupt("lzo: output exceeds declared size"));
        }
        t = byte(src, ip)?;
        ip += 1;
        if state == 0 {
            if t < 16 {
                if t == 0 {
                    while byte(src, ip)? == 0 {
                        t += 255;
                        ip += 1;
                    }
                    t += 15 + byte(src, ip)?;
                    ip += 1;
                }
                lit(&mut dst, src, &mut ip, t + 3)?;
                state = 2;
                continue;
            }
        } else if t < 16 {
            if state == 2 {
                let m = (1 + 0x0800) + (t >> 2) + (byte(src, ip)? << 2);
                ip += 1;
                copy_match(&mut dst, m, 3)?;
            } else {
                let m = 1 + (t >> 2) + (byte(src, ip)? << 2);
                ip += 1;
                copy_match(&mut dst, m, 2)?;
            }
            let s = t & 3;
            if s != 0 {
                lit(&mut dst, src, &mut ip, s)?;
                state = 1;
            } else {
                state = 0;
            }
            continue;
        }
        // t >= 16: match
        let (len, dist);
        if t >= 64 {
            dist = 1 + ((t >> 2) & 7) + (byte(src, ip)? << 3);
            ip += 1;
            len = (t >> 5) - 1 + 2;
        } else if t >= 32 {
            let mut l = t & 31;
            if l == 0 {
                while byte(src, ip)? == 0 {
                    l += 255;
                    ip += 1;
                }
                l += 31 + byte(src, ip)?;
                ip += 1;
            }
            len = l + 2;
            dist = 1 + ((byte(src, ip)? | (byte(src, ip + 1)? << 8)) >> 2);
            ip += 2;
        } else {
            let mut d = (t & 8) << 11;
            let mut l = t & 7;
            if l == 0 {
                while byte(src, ip)? == 0 {
                    l += 255;
                    ip += 1;
                }
                l += 7 + byte(src, ip)?;
                ip += 1;
            }
            len = l + 2;
            let w = byte(src, ip)? | (byte(src, ip + 1)? << 8);
            ip += 2;
            d += w >> 2;
            if d == 0 {
                break; // end of stream marker
            }
            dist = d + 0x4000;
        }
        copy_match(&mut dst, dist, len)?;
        let s = byte(src, ip - 2)? & 3;
        if s != 0 {
            lit(&mut dst, src, &mut ip, s)?;
            state = 1;
        } else {
            state = 0;
        }
    }
    Ok(dst)
}