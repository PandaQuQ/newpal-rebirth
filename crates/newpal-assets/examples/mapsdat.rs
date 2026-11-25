//! Research dump of maps.dat (prints numbers only).
fn u16a(b: &[u8], o: usize) -> u16 { u16::from_le_bytes([b[o], b[o + 1]]) }
fn u32a(b: &[u8], o: usize) -> u32 { u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) }
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let f = std::fs::read(std::path::Path::new(&a[1]).join("maps.dat")).unwrap();
    let b = &f[0x200..];
    print!("header:");
    for i in 0..28 { print!(" +{:02x}={:x}", i * 4, u32a(b, i * 4)); }
    println!();
    let city = u32a(b, 0xc) as usize;
    let mut p = city; let mut n = 0;
    while u16a(b, p) != 0xFFFF { if n < 40 { println!("city {:?}", (0..7).map(|k| u16a(b, p + 2 * k)).collect::<Vec<_>>()); } p += 14; n += 1; }
    println!("cities {n}");
    let roles = u32a(b, 4) as usize;
    let mut p = roles; let mut n = 0;
    let want: Vec<u16> = a[2..].iter().map(|s| s.parse().unwrap()).collect();
    while u16a(b, p) != 0xFFFF { let r: Vec<u16> = (0..13).map(|k| u16a(b, p + 2 * k)).collect(); if want.contains(&r[0]) { println!("role {:?}", r); } p += 26; n += 1; }
    println!("roles {n}");
    let t = u32a(b, 0x54) as usize; let mut p = t; let mut n = 0;
    while u16a(b, p) != 0 { if n < 12 { println!("talk54 {:?}", (0..3).map(|k| u16a(b, p + 2 * k)).collect::<Vec<_>>()); } p += 6; n += 1; }
    println!("talk54 n {n}");
    let md = newpal_assets::mapsdat::MapsDat::parse(&f).unwrap();
    println!("music names {}: {:?}", md.music_names.len(), md.music_names.iter().take(12).collect::<Vec<_>>());
    println!("city music n {}: {:?}", md.city_music.len(), md.city_music.iter().take(8).collect::<Vec<_>>());
    for c in [1u16, 2, 8] { println!("city {c} music {:?}", md.music_for(c)); }
    let s = u32a(b, 0x10) as usize; println!("start {:?}", (0..8).map(|k| u16a(b, s + 2 * k)).collect::<Vec<_>>());
}