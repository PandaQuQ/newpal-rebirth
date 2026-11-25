//! `maps.dat` (RoleDataBase): scenes ("cities") -> LMF map, and the role (NPC) placement table.
//!
//! The original loader reads the file from offset 0x200 into one buffer; every table below is
//! `buf + u32 header[k]` (header = first 0x70 bytes of the buffer). Layout as used by the
//! original scene and city loaders:
//! ```text
//! +0x04 roles:  13 x u16 per record, until u16 0xFFFF
//!        [0] city  [1] role id  [2] actor (all_char act/tsw number, slot = n-1; >=699 special)
//!        [3] action override  [4] direction?  [5] x cell  [6] y cell  [7] talk script
//!        [8] [9] unknown  [10] flags (&3 passed as facing; 0x80, 0x200, 0x8000 tested)  [11..12] bytes
//! +0x0C cities: 7 x u16 per record, until 0xFFFF: [0] city id [1] map id [2] flags (0x8000/0x4000/0x2000/
//!        0x1000/0x800/0x400 -> story flags f,5,6,7,8) [3] & 0xf [4] music? [5] [6]
//! +0x10 new-game start record (u16 x 8)
//! +0x54 role id -> (u16, u16) table (3 x u16, until 0)
//! +0x08 music block B: u32 [B+0] -> name table (u32 per music id -> string @ +4, Big5/ASCII),
//!        [B+4] -> u32 -> city music list {u16 city, u16 track a, u16 track b, u16 flags (&3 loop mode)} until 0
//!        (tracks are files in <game>/Music)
//! +0x64 i32: city -> flag table (u16 city, u16 flag) pairs set on first entry
//! ```
use crate::bytes::{u16le, u32le};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct City {
    pub id: u16,
    pub map: u16,
    pub raw: [u16; 7],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Role {
    pub raw: [u16; 13],
}

impl Role {
    pub fn city(&self) -> u16 { self.raw[0] }
    pub fn id(&self) -> u16 { self.raw[1] }
    /// actor number (all_char act/tsw slot = actor - 1)
    pub fn actor(&self) -> u16 { self.raw[2] }
    pub fn action(&self) -> u16 { self.raw[3] }
    pub fn dir(&self) -> u16 { self.raw[4] }
    /// feet position in 16 px cells
    pub fn cell(&self) -> (u16, u16) { (self.raw[5], self.raw[6]) }
    pub fn script(&self) -> u16 { self.raw[7] }
    pub fn flags(&self) -> u16 { self.raw[10] }
}

#[derive(Clone, Debug)]
pub struct MapsDat {
    pub cities: Vec<City>,
    pub roles: Vec<Role>,
    pub start: [u16; 8],
    /// music id -> file stem (e.g. "m003")
    pub music_names: Vec<String>,
    /// city -> (track a, track b, flags)
    pub city_music: Vec<(u16, u16, u16, u16)>,
}

impl MapsDat {
    pub fn parse(file: &[u8]) -> Result<Self> {
        let b = file.get(0x200..).ok_or_else(|| Error::corrupt("maps.dat too short"))?;
        let tab = |k: usize| -> Result<usize> { Ok(u32le(b, k)? as usize) };
        let mut cities = Vec::new();
        let mut p = tab(0x0C)?;
        while u16le(b, p)? != 0xFFFF {
            let mut raw = [0u16; 7];
            for (k, r) in raw.iter_mut().enumerate() { *r = u16le(b, p + 2 * k)?; }
            cities.push(City { id: raw[0], map: raw[1], raw });
            p += 14;
        }
        let mut roles = Vec::new();
        let mut p = tab(0x04)?;
        while u16le(b, p)? != 0xFFFF {
            let mut raw = [0u16; 13];
            for (k, r) in raw.iter_mut().enumerate() { *r = u16le(b, p + 2 * k)?; }
            roles.push(Role { raw });
            p += 26;
        }
        let s = tab(0x10)?;
        let mut start = [0u16; 8];
        for (k, r) in start.iter_mut().enumerate() { *r = u16le(b, s + 2 * k)?; }
        let (mut music_names, mut city_music) = (Vec::new(), Vec::new());
        if let Ok(blk) = tab(0x08) {
            if let (Ok(names), Ok(list)) = (u32le(b, blk), u32le(b, blk + 4)) {
                let names = names as usize;
                let mut p = u32le(b, list as usize).map(|v| v as usize).unwrap_or(usize::MAX);
                // name table: u32 offsets until one does not point at a printable string
                for i in 0..512usize {
                    let Ok(o) = u32le(b, names + i * 4) else { break };
                    let s = o as usize + 4;
                    let Some(bytes) = b.get(s..(s + 64).min(b.len())) else { break };
                    let end = bytes.iter().position(|&c| c == 0).unwrap_or(0);
                    let name = String::from_utf8_lossy(&bytes[..end]).to_string();
                    if end == 0 || !name.chars().all(|c| c.is_ascii_graphic()) { break }
                    music_names.push(name);
                }
                while let (true, Ok(c)) = (p.saturating_add(8) <= b.len(), u16le(b, p.min(b.len()))) {
                    if c == 0 { break }
                    city_music.push((c, u16le(b, p + 2)?, u16le(b, p + 4)?, u16le(b, p + 6)?));
                    p += 8;
                }
            }
        }
        Ok(Self { cities, roles, start, music_names, city_music })
    }
    /// Background music file stems for a city (track a, track b).
    pub fn music_for(&self, city: u16) -> Option<(Option<&str>, Option<&str>, u16)> {
        let &(_, a, b, fl) = self.city_music.iter().find(|m| m.0 == city)?;
        let n = |i: u16| self.music_names.get(i as usize).map(|s| s.as_str());
        Some((n(a), n(b), fl))
    }
    pub fn city(&self, id: u16) -> Option<&City> { self.cities.iter().find(|c| c.id == id) }
    /// First city that uses LMF map `map`.
    pub fn city_for_map(&self, map: u16) -> Option<&City> { self.cities.iter().find(|c| c.map == map) }
    pub fn roles_in(&self, city: u16) -> impl Iterator<Item = &Role> { self.roles.iter().filter(move |r| r.city() == city) }
    /// LMF file for a map id (ids >= 277 live in huge2.lmf).
    pub fn lmf_for(map: u16) -> &'static str { if map >= 277 { "huge2" } else { "huge" } }
}