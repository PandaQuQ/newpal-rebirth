//! ACT action tables (`all_*.act` entries).
//!
//! ```text
//! u16 nActions; u32 off[nActions] (relative to entry start, 0 = none)
//! action = stream of u16 words; opcode = two ASCII bytes, then u16 args (argc table below)
//! ```
use crate::bytes::{u16le, u32le};
use crate::Result;

/// Known opcodes and their argument counts (from a histogram over all six .act files).
pub const ARGC: &[(&str, usize)] = &[
    ("AP", 1), ("DE", 0), ("HW", 2), ("YX", 2), ("NT", 1), ("FR", 1), ("VO", 0), ("ON", 0), ("DS", 1),
    ("IV", 0), ("SG", 1), ("2O", 0), ("VW", 1), ("PL", 3), ("TA", 1), ("CS", 1), ("AO", 1), ("XO", 1),
    ("YO", 1), ("OR", 1), ("GC", 0), ("XA", 1), ("YA", 1), ("BC", 0), ("LC", 0), ("RC", 0), ("LF", 0),
    ("DL", 0), ("EQ", 0), ("MS", 0), ("WT", 1), ("ED", 0),
];

fn op_name(w: u16) -> String {
    // like JS String.fromCharCode(lo, hi): bytes map 1:1 to chars
    [(w & 0xFF) as u8 as char, (w >> 8) as u8 as char].iter().collect()
}

fn argc(name: &str) -> Option<usize> {
    ARGC.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActOp {
    pub name: String,
    pub args: Vec<u16>,
}

impl ActOp {
    /// "AP 3" style text (matches the JS reference output).
    pub fn to_text(&self) -> String {
        let mut s = self.name.clone();
        for a in &self.args {
            s.push(' ');
            s.push_str(&a.to_string());
        }
        s
    }
}

#[derive(Clone, Debug)]
pub struct Action {
    pub index: usize,
    pub offset: u32,
    pub ops: Vec<ActOp>,
}

pub fn parse_act(buf: &[u8]) -> Result<Vec<Option<Action>>> {
    let n = u16le(buf, 0)? as usize;
    let mut offs = Vec::with_capacity(n);
    for i in 0..n {
        offs.push(u32le(buf, 2 + i * 4)?);
    }
    let is_op = |w: u16| argc(&op_name(w)).is_some();
    let mut out = Vec::with_capacity(n);
    for (i, &o) in offs.iter().enumerate() {
        if o == 0 {
            out.push(None);
            continue;
        }
        let end = offs[i + 1..].iter().copied().find(|&x| x != 0).map(|x| x as usize).unwrap_or(buf.len());
        let mut ops = Vec::new();
        let mut p = o as usize;
        while p + 1 < end {
            let w = u16le(buf, p)?;
            p += 2;
            let name = op_name(w);
            let k = argc(&name).unwrap_or(0);
            let mut args = Vec::new();
            let mut j = 0;
            while j < k && p + 1 < end {
                args.push(u16le(buf, p)?);
                p += 2;
                j += 1;
            }
            // tolerate extra trailing words (seen on DE/DS/ON/SG)
            while p + 1 < end && !is_op(u16le(buf, p)?) {
                args.push(u16le(buf, p)?);
                p += 2;
            }
            let stop = name == "VO" || name == "2O";
            ops.push(ActOp { name, args });
            if stop {
                break;
            }
        }
        out.push(Some(Action { index: i, offset: o, ops }));
    }
    Ok(out)
}

/// One displayed animation frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// tsw entry id (1-based; slot = id-1; id >= 3000 -> other file, assumption)
    pub sprite: Option<u32>,
    pub frame: u16,
    /// DS value in main-loop frames; the original loop runs at 35 ms/frame (the original sleep argument is 0x23; 70 ms in some cutscenes)
    pub delay: u16,
    pub anchor: (u16, u16),
}

pub fn steps(a: &Action) -> Vec<Step> {
    let (mut fr, mut delay, mut anchor) = (None, 1u16, (0u16, 0u16));
    let mut out = Vec::new();
    for op in &a.ops {
        let a0 = op.args.first().copied().unwrap_or(0);
        match op.name.as_str() {
            "FR" => fr = Some(a0 as u32),
            "DS" => delay = if a0 == 0 { 1 } else { a0 },
            "YX" => anchor = (a0, op.args.get(1).copied().unwrap_or(0)),
            "AP" => out.push(Step { sprite: fr, frame: a0, delay, anchor }),
            _ => {}
        }
    }
    out
}