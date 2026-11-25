//! `talk*.dat` scripts: loader + disassembler (the original talk-script interpreter).
//!
//! ```text
//! file: 0x200-byte "MZ" stub, u32 off[1001] @0x200 (slot = N % 1000; N % 1000 == 0 -> slot 1000)
//! script N lives in talk{N/1000+1}.dat; code @ off + 0x200, at most 0x8000 bytes
//! instruction: u16 w0 (op = w0 & 0x3FFF, top 2 bits = flags), then u16 args; 0xFFFF = end
//! ```
use crate::bytes::{find, u16le, u32le};
use crate::talk_ops_gen::OP_LENS;
use crate::legacy;
use crate::{Error, Result};

pub struct TalkScript {
    pub id: u32,
    pub file: String,
    pub start: usize,
    pub code: Vec<u8>,
    /// whole talk file (jumps may leave the script's own slice); `start` indexes into it
    pub image: std::sync::Arc<Vec<u8>>,
    /// jump targets are `u32` offsets; byte position in `image` = target + jump_base (0x200 for talk*.dat)
    pub jump_base: usize,
}

impl TalkScript {
    /// A standalone script (mod `scripts/talk/<n>.bin`): targets are offsets into the code itself.
    pub fn standalone(id: u32, file: String, code: Vec<u8>) -> Self {
        Self { id, file, start: 0, image: std::sync::Arc::new(code.clone()), code, jump_base: 0 }
    }
}

/// Which talk file holds script `n`.
pub fn file_for(n: u32) -> String {
    format!("talk{}.dat", n / 1000 + 1)
}

/// Extract script `n` from the bytes of its talk file.
pub fn script_from_file(file_bytes: &[u8], n: u32) -> Result<TalkScript> {
    let slot = if n % 1000 == 0 { 1000 } else { (n % 1000) as usize };
    let st = u32le(file_bytes, 0x200 + slot * 4)? as usize + 0x200;
    let mut nx = file_bytes.len();
    for s in 1..=1000usize {
        let o = u32le(file_bytes, 0x200 + s * 4)? as usize + 0x200;
        if o > st && o < nx {
            nx = o;
        }
    }
    let end = nx.min(st + 0x8000);
    let code = file_bytes.get(st..end).ok_or_else(|| Error::corrupt(format!("script {n} out of range")))?.to_vec();
    Ok(TalkScript { id: n, file: file_for(n), start: st, code, image: std::sync::Arc::new(file_bytes.to_vec()), jump_base: 0x200 })
}

/// Dialogue / text ops: `u16 speaker` + text up to the bytes "%Q".
const TEXT_OPS: &[u16] = &[1, 2, 3, 4, 5, 6, 0x59, 0x5a, 0x7d, 0xc2, 0x60];
/// String ops without a speaker word.
const STR_OPS: &[u16] = &[0xc2, 0x60];
/// Fixed lengths established by reading the decompiled switch arms.
const FIXED: &[(u16, usize)] = &[
    (0xa, 6), (0xe4, 6), (0xdf, 8), (0xb, 6), (0xe5, 6), (0xe0, 8), (0x2d, 6), (0xe3, 6), (0xde, 8),
    (0x28, 8), (0xe2, 8), (0xe8, 8), (0xdd, 10), (0xf, 6), (0xa1, 4), (0x309, 4), (0x400, 2), (0x401, 2), (0x402, 2),
    // var compare-and-jump (i16 / i32 forms): u16 var, value, u32 target
    (0x20, 10), (0x21, 10), (0xb8, 12), (0xb9, 12),
];

pub fn op_name(op: u16) -> Option<&'static str> {
    Some(match op {
        1 => "SAY", 2 => "SAY_W", 3 => "SAY2", 4 => "SAY2_W", 5 => "SAY3", 6 => "SAY3_W",
        0x59 => "SAY4", 0x5a => "SAY4_W", 0x7d => "TEXT", 0xf => "GOTO", 0xa1 => "CALLSCRIPT",
        0xcb => "SWITCH", 0xd5 => "IFCMP", 0xdc => "IFCMP2", 0xe1 => "IFCMP3", 0x17 => "IFLIST",
        0x29 => "JTAB", 0x57 => "JSEL", 0xc => "MOVE_NPC", 0xc2 => "PLAY_MOVIE", 0x60 => "PLAY_MANI",
        0x402 => "YIELD", 0x400 => "BLOCK_BEGIN", 0x401 => "BLOCK_END", 0x309 => "SET_EVENT",
        0x14 => "WALK", 0xa9 => "WALK_EX",
        _ => return None,
    })
}

pub fn is_dialogue(op: u16) -> bool {
    matches!(op, 1..=6 | 0x59 | 0x5a | 0x7d)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Instr {
    Op {
        pos: usize,
        op: u16,
        flag: u16,
        len: usize,
        args: Vec<u16>,
        /// dialogue text as inline markup v1 (`<color=6>`, `<br/>`, `<speaker>`; see `i18n::markup`); file name for movie ops
        text: Option<String>,
        /// binary parameter header preceding dialogue text (portrait/position; layout unknown)
        hdr: Option<Vec<u8>>,
        target: Option<u32>,
        /// length chosen by lookahead among several candidates
        guess: bool,
    },
    Unknown { pos: usize, bytes: Vec<u8> },
    End { pos: usize },
}

impl Instr {
    pub fn pos(&self) -> usize {
        match self {
            Instr::Op { pos, .. } | Instr::Unknown { pos, .. } | Instr::End { pos } => *pos,
        }
    }
    /// "1a" / "??" / "END" (matches the JS reference)
    pub fn op_key(&self) -> String {
        match self {
            Instr::Op { op, .. } => format!("{op:x}"),
            Instr::Unknown { .. } => "??".into(),
            Instr::End { .. } => "END".into(),
        }
    }
    pub fn to_text(&self) -> String {
        match self {
            Instr::End { pos } => format!("{pos:04x}  END"),
            Instr::Unknown { pos, bytes } => {
                format!("{pos:04x}  ?? {}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
            }
            Instr::Op { pos, op, flag, args, text, hdr, target, guess, .. } => {
                let nm = op_name(*op).map(str::to_string).unwrap_or_else(|| format!("op_{op:03x}"));
                let mut s = format!("{pos:04x}  {nm}");
                if *flag != 0 {
                    s.push_str(&format!("|{}", flag >> 14));
                }
                s.push(' ');
                s.push_str(&args.iter().map(|a| format!("{a:x}")).collect::<Vec<_>>().join(" "));
                if *guess {
                    s.push_str("  ;len?");
                }
                if let Some(h) = hdr {
                    s.push_str(&format!("  hdr={}", h.iter().map(|b| format!("{b:02x}")).collect::<String>()));
                }
                if let Some(t) = text {
                    s.push_str(&format!("  \u{300c}{}\u{300d}", t.replace('\n', "\\n")));
                }
                if let Some(t) = target {
                    s.push_str(&format!("  -> {t:x}"));
                }
                s
            }
        }
    }
}

pub struct Disasm {
    pub instrs: Vec<Instr>,
    pub ok: usize,
    pub unk: usize,
}

enum Len {
    One { op: u16, len: usize, text: bool, str_op: bool },
    Many { op: u16, lens: &'static [u16] },
}

fn table_lens(op: u16) -> Option<&'static [u16]> {
    OP_LENS.iter().find(|(o, _)| *o == op).map(|(_, l)| *l)
}

fn len_at(c: &[u8], p: usize) -> Option<Len> {
    if p + 2 > c.len() {
        return None;
    }
    let w = |k: usize| if p + 2 * k + 1 < c.len() { u16::from_le_bytes([c[p + 2 * k], c[p + 2 * k + 1]]) } else { 0 };
    let op = w(0) & 0x3FFF;
    if TEXT_OPS.contains(&op) {
        let e = find(c, b"%Q", p + 2)?;
        return Some(Len::One { op, len: e + 2 - p, text: true, str_op: STR_OPS.contains(&op) });
    }
    if let Some(&(_, l)) = FIXED.iter().find(|(o, _)| *o == op) {
        return Some(Len::One { op, len: l, text: false, str_op: false });
    }
    let var = match op {
        0x14 => Some(4 + 6 * (w(1) & 0x3FFF) as usize),
        0xa9 => Some(4 + 12 * (w(1) & 0x3FFF) as usize),
        0x6d => Some(4 + 2 * w(1) as usize),
        0x74 => Some(4 + 6 * w(1) as usize),
        0x85 => Some(4 + 2 * w(1) as usize),
        _ => None,
    };
    if let Some(len) = var {
        return Some(Len::One { op, len, text: false, str_op: false });
    }
    if op == 0x17 || op == 0x18 {
        // flag list until 0xFF00, then u32 target
        let (mut k, mut n) = (p + 2, 0usize);
        while k + 2 <= c.len() && u16::from_le_bytes([c[k], c[k + 1]]) != 0xFF00 {
            k += 2;
            n += 1;
        }
        return Some(Len::One { op, len: 2 + 2 * n + 2 + 4, text: false, str_op: false });
    }
    if op == 0xcb {
        let (mut k, mut n) = (p + 6, 0usize);
        while k + 4 <= c.len() && u32::from_le_bytes([c[k], c[k + 1], c[k + 2], c[k + 3]]) != 0xFF00_FF00 {
            k += 4;
            n += 1;
        }
        return Some(Len::One { op, len: 6 + 4 * n + 4, text: false, str_op: false });
    }
    let lens = table_lens(op)?;
    match lens.len() {
        0 => None,
        1 if lens[0] == 0 => None,
        1 => Some(Len::One { op, len: lens[0] as usize, text: false, str_op: false }),
        _ => Some(Len::Many { op, lens }),
    }
}

fn valid(c: &[u8], p: usize, depth: u32) -> bool {
    if p == c.len() {
        return true;
    }
    let Some(r) = len_at(c, p) else { return false };
    if depth == 0 {
        return true;
    }
    match r {
        Len::One { len, .. } => valid(c, p + len, depth - 1),
        Len::Many { lens, .. } => lens.iter().any(|&l| l > 0 && valid(c, p + l as usize, depth - 1)),
    }
}

/// Decode one instruction at byte `p` (no resync). `None` when the bytes do not form a known instruction.
pub fn decode_at(code: &[u8], p: usize) -> Option<Instr> {
    decode_at_mode(code, p, false)
}

/// `raw_text`: leave dialogue in the original control-code form (verification against the JS reference tools only).
fn decode_at_mode(code: &[u8], p: usize, raw_text: bool) -> Option<Instr> {
    let raw = u16le(code, p).ok()?;
    if raw == 0xFFFF || raw == 0x0C0C {
        return Some(Instr::End { pos: p });
    }
    let (op, len, text, str_op, guess) = match len_at(code, p)? {
        Len::Many { op, lens } => lens.iter().find(|&&l| l > 0 && valid(code, p + l as usize, 2)).map(|&l| (op, l as usize, false, false, true))?,
        Len::One { op, len, text, str_op } => (op, len, text, str_op, false),
    };
    let nw = if text { if str_op { 1 } else { 2 } } else { len.div_ceil(2) };
    let mut words = Vec::new();
    let mut k = 1;
    while k < nw && p + 2 * k + 1 < code.len() {
        words.push(u16::from_le_bytes([code[p + 2 * k], code[p + 2 * k + 1]]));
        k += 1;
    }
    let (mut txt, mut hdr) = (None, None);
    let args = if text {
        let a = if str_op { vec![] } else { words.first().copied().into_iter().collect() };
        let s = p + if str_op { 2 } else { 4 };
        let e = p + len - 2;
        let tb: &[u8] = if s < e { &code[s..e] } else { &[] };
        let mut k = if str_op { 0 } else { tb.iter().position(|&b| b == 0x25).unwrap_or(0) };
        if !str_op && !raw_text {
            // operand bytes before the text that contain a `%`: skip them (the reference tools keep them)
            k += legacy::binary_prefix(&tb[k..]);
        }
        if k > 0 {
            hdr = Some(tb[..k].to_vec());
        }
        // dialogue text leaves the import layer as markup (UTF-8); movie / animation names stay plain
        let decoded = legacy::decode_text(&tb[k..]);
        txt = Some(if str_op || raw_text { decoded } else { legacy::markup::from_legacy(&decoded, true) });
        a
    } else {
        words
    };
    let target = if op == 0xf { u32le(code, p + 2).ok() } else { None };
    Some(Instr::Op { pos: p, op, flag: raw & 0xC000, len, args, text: txt, hdr, target, guess })
}

/// Linear-sweep disassembly with lookahead disambiguation and resync on unknown bytes. Stops at the first
/// end marker (what falls through from the start); see [`disasm_full`] for code only reachable by jumps.
pub fn disasm(code: &[u8]) -> Disasm {
    disasm_opts(code, true, false)
}

/// [`disasm`] with dialogue text left in the original control-code form. Only for golden tests that compare
/// against the JS reference tools; the engine never uses it.
pub fn disasm_raw_text(code: &[u8]) -> Disasm {
    disasm_opts(code, true, true)
}

/// Like [`disasm`] but keeps sweeping *past* end markers: scripts are made of blocks separated by `0xFFFF`
/// that are entered by jumps (`GOTO`, compare-and-jump, `SWITCH`...), so a sweep that stops at the first end
/// misses their dialogue. Used by the translation export (every instruction counts, reachable or not).
pub fn disasm_full(code: &[u8]) -> Disasm {
    disasm_opts(code, false, false)
}

fn disasm_opts(code: &[u8], stop_at_end: bool, raw_text: bool) -> Disasm {
    let mut out = Vec::new();
    let (mut p, mut ok, mut unk) = (0usize, 0usize, 0usize);
    while p < code.len() {
        if p + 2 > code.len() {
            out.push(Instr::Unknown { pos: p, bytes: code[p..].to_vec() });
            unk += code.len() - p;
            break;
        }
        match decode_at_mode(code, p, raw_text) {
            Some(i @ Instr::End { .. }) => {
                out.push(i);
                if stop_at_end {
                    break;
                }
                p += 2;
            }
            Some(i) => {
                let len = match &i { Instr::Op { len, .. } => *len, _ => 2 };
                out.push(i);
                ok += len;
                p += len;
            }
            None => {
                let mut q = p + 1;
                while q < code.len() && !valid(code, q, 3) {
                    q += 1;
                }
                out.push(Instr::Unknown { pos: p, bytes: code[p..q].to_vec() });
                unk += q - p;
                p = q;
            }
        }
    }
    Disasm { instrs: out, ok, unk }
}

/// Next decodable position at or after `p` (same resync rule as [`disasm`]).
pub fn resync(code: &[u8], p: usize) -> usize {
    let mut q = p + 1;
    while q < code.len() && !valid(code, q, 3) {
        q += 1;
    }
    q
}
/// A styled run inside a dialogue line.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub color: u8,
    pub text: String,
}

/// Parse dialogue markup into styled lines (speaker / cues dropped; see [`crate::i18n::markup::parse`] for the
/// full result).
pub fn parse_dialogue(markup: &str) -> Vec<Vec<Span>> {
    crate::i18n::markup::parse(markup).lines
}
