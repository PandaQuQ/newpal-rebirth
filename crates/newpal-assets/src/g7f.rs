//! 0x7F tagged value tables (`ghero/gitem/gmagic/gmon/glevel.dat`, data from 0x200).
//!
//! ```text
//! u32 tag = 0x7F | B | u16 code, B = (argc << 4) | group
//!   argc = (tag >> 20) & 0xF: 0..=4 -> argc x i32 follow; 0xF -> Big5 string terminated by "%%"
//! 0x7F00FFFF = record end
//! heads: 7F100000 item/magic id, 7F101000 hero/monster/level id, 7F102000 code block, 7F103000 scene
//! 7FF2F000 name, 7FF2F001 description, 7F200001 (attr, val) item/magic, 7F201001 (attr, val) char/monster
//! ```
use crate::bytes::{find, i32le, u32le};
use crate::legacy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const END: u32 = 0x7F00_FFFF;
pub const TAG_NAME: u32 = 0x7FF2_F000;
pub const TAG_DESC: u32 = 0x7FF2_F001;
pub const TAG_ATTR_ITEM: u32 = 0x7F20_0001;
pub const TAG_ATTR_CHAR: u32 = 0x7F20_1001;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Ints(Vec<i32>),
    Str(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub tag: u32,
    pub value: Value,
}

/// Parse into raw records. Returns (records, skipped_bytes).
pub fn parse(b: &[u8], start: usize) -> (Vec<Vec<Item>>, usize) {
    let (mut recs, mut cur, mut p, mut bad) = (Vec::new(), Vec::new(), start, 0usize);
    while p + 4 <= b.len() {
        let tag = u32le(b, p).unwrap();
        if tag >> 24 != 0x7F {
            p += 1;
            bad += 1;
            continue;
        }
        p += 4;
        if tag == END {
            if !cur.is_empty() {
                recs.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let argc = (tag >> 20) & 0xF;
        if argc == 0xF {
            let e = find(b, b"%%", p);
            let end = e.unwrap_or(b.len());
            cur.push(Item { tag, value: Value::Str(legacy::markup::from_legacy(&legacy::decode_text(&b[p..end]), false)) });
            p = e.map(|e| e + 2).unwrap_or(b.len());
        } else {
            let mut a = Vec::new();
            for _ in 0..argc {
                match i32le(b, p) {
                    Ok(v) => a.push(v),
                    Err(_) => break,
                }
                p += 4;
            }
            cur.push(Item { tag, value: Value::Ints(a) });
        }
    }
    if !cur.is_empty() {
        recs.push(cur);
    }
    (recs, bad)
}

/// Friendly view of a record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub kind: u32,
    pub id: Option<i32>,
    pub name: Option<String>,
    pub desc: Option<String>,
    /// (attr id -> value) from 7F200001 / 7F201001 pairs
    pub attrs: BTreeMap<i32, i32>,
    /// everything else, in file order
    pub fields: Vec<Item>,
}

pub fn friendly(rec: &[Item]) -> Record {
    let mut r = Record::default();
    if let Some(h) = rec.first() {
        r.kind = h.tag;
        if let Value::Ints(a) = &h.value {
            r.id = a.first().copied();
        }
    }
    for it in rec.iter().skip(1) {
        match (&it.value, it.tag) {
            (Value::Str(s), TAG_NAME) => r.name = Some(s.clone()),
            (Value::Str(s), TAG_DESC) => r.desc = Some(s.clone()),
            (Value::Ints(a), TAG_ATTR_ITEM | TAG_ATTR_CHAR) if a.len() == 2 => {
                r.attrs.insert(a[0], a[1]);
            }
            _ => r.fields.push(it.clone()),
        }
    }
    r
}