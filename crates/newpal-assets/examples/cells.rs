//! Research: dump LMF cell attribute stats + overlay PNGs (written OUTSIDE the repo).
//! cargo run -p newpal-assets --example cells -- <gamedir> <outdir> huge:2 huge:30 ...
use newpal_assets::lmf::Lmf;
use std::collections::BTreeMap;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::PathBuf::from(&a[1]);
    let out = std::path::PathBuf::from(&a[2]);
    std::fs::create_dir_all(&out).unwrap();
    for spec in &a[3..] {
        let (file, id) = spec.split_once(':').unwrap();
        let id: u32 = id.parse().unwrap();
        let l = Lmf::open(&game.join(format!("{file}.lmf"))).unwrap();
        let m = l.load_map(id).unwrap();
        println!("== {spec} {}x{} cells={} grid={}", m.w, m.h, m.cells.len(), m.grid.len());
        let mut bits = [0usize; 32];
        let mut vals: BTreeMap<u32, usize> = BTreeMap::new();
        let mut hi: BTreeMap<u32, usize> = BTreeMap::new();
        for &c in &m.cells { for b in 0..32 { if c >> b & 1 == 1 { bits[b] += 1 } } *vals.entry(c & 0xFFFF_0000).or_default() += 1; }
        for &g in &m.grid { *hi.entry(g >> 16).or_default() += 1; }
        println!("bits: {}", (0..32).filter(|&b| bits[b] > 0).map(|b| format!("{b}:{}", bits[b])).collect::<Vec<_>>().join(" "));
        println!("hi16 cell vals: {:?}", vals.iter().take(24).map(|(k, v)| format!("{k:08x}x{v}")).collect::<Vec<_>>());
        println!("lo16 sample row 10: {:?}", m.cells.iter().skip(10 * m.w as usize).take(m.w as usize).map(|c| format!("{:x}", c & 0xFFFF)).collect::<Vec<_>>());
        println!("grid hi16: {:?}", hi.iter().take(12).collect::<Vec<_>>());
        let mut rc: BTreeMap<u32, (usize, u32, u32)> = BTreeMap::new(); for (i, &c) in m.cells.iter().enumerate() { if c & 0xF != 0 { let e = rc.entry(c & 0xF).or_insert((0, 0, 0)); e.0 += 1; e.1 = (i % m.w as usize) as u32; e.2 = (i / m.w as usize) as u32; } } println!("lo4 ids: {:?}", rc); for r in 1..=m.regions.len() as u32 { let v: Vec<(usize, usize)> = m.cells.iter().enumerate().filter(|(_, &c)| c & 0xFF == r && c & 0x4000_0000 == 0).map(|(i, _)| (i % m.w as usize, i / m.w as usize)).take(6).collect(); println!("region {r} walkable cells {:?}", v); } for r in &m.regions { println!("region {:?} {}", r.v, r.name); }
        let img = m.render_bg();
        for (masks, tag) in [(0x6000_0000u32, "block"), (0x2000_0000, "b29"), (0x4000_0000, "b30"), (0x8000_0000, "b31"), (0x1000_0000, "b28"), (0x0F00_0000, "b24-27"), (0x00FF_0000, "b16-23"), (0x0000_FFFF, "lo16"), (0x0000_000F, "lo4"), (0x0000_F000, "n3")] {
            let mut o = img.clone();
            for cy in 0..m.h as u32 { for cx in 0..m.w as u32 {
                let c = m.cells[(cy * m.w as u32 + cx) as usize];
                if c & masks != 0 { for y in 2..14 { for x in 2..14 {
                    let i = (((cy * 16 + y) * o.width + cx * 16 + x) * 4) as usize;
                    o.pixels[i] = ((o.pixels[i] as u32 + 255) / 2) as u8; o.pixels[i + 1] /= 2; o.pixels[i + 2] /= 2;
                } } }
            } }
            o.save_png(&out.join(format!("{}_{}_{tag}.png", file, id))).unwrap();
        }
    }
}