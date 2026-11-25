//! Golden tests: the Rust readers must match a locally generated hash file.
//!
//! `tests/golden.json` is **not** in the repository. Generate it from your own install:
//! `NEWPAL_WRITE_GOLDEN=1 cargo test -p newpal-assets --test golden write_golden`
//! Tests need the player's own install (`NEWPAL_GAME_DIR`, newpal.toml or Steam auto-detect)
//! and are skipped when the install or the local hash file is missing.
use newpal_assets::gamedir::{self, Config};
use newpal_assets::{act, g7f, lmf::Lmf, pak::IndexArchive, talk, tsw::SpriteEntry};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden.json")
}

fn golden() -> Option<Value> {
    let p = golden_path();
    match std::fs::read_to_string(&p) {
        Ok(s) => Some(serde_json::from_str(&s).expect("golden.json")),
        Err(_) => {
            eprintln!("SKIP golden test: {} not found (generate locally from your own install; see write_golden)", p.display());
            None
        }
    }
}

fn game() -> Option<PathBuf> {
    let p = Config::default_path();
    let cfg = Config::load_from(&p).unwrap_or_default();
    match gamedir::resolve(None, &cfg, &p) {
        Ok(r) => Some(r.dir),
        Err(e) => {
            eprintln!("SKIP golden test: {e}");
            None
        }
    }
}

#[test]
fn pak_index() {
    let Some(d) = game() else { return };
    let Some(g) = golden() else { return };
    let g = &g["pak"];
    let a = IndexArchive::open(&d.join("all_char.tsw")).unwrap();
    assert_eq!(a.entries.len() as u64, g["slots"].as_u64().unwrap());
    assert_eq!(a.entries.iter().filter(|e| e.size > 0).count() as u64, g["nonEmpty"].as_u64().unwrap());
    let names: Vec<_> = a.entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(sha(names.join("\n").as_bytes()), g["namesSha"].as_str().unwrap(), "legacy names differ");
}

#[test]
fn tsw_lzo_rle() {
    let Some(d) = game() else { return };
    let Some(g) = golden() else { return };
    let a = IndexArchive::open(&d.join("all_char.tsw")).unwrap();
    for t in g["tsw"].as_array().unwrap() {
        let slot = t["slot"].as_u64().unwrap() as usize;
        let e = SpriteEntry::parse(a.read(slot).unwrap()).unwrap();
        assert_eq!(e.bpp as u64, t["bpp"].as_u64().unwrap());
        let frames = t["frames"].as_array().unwrap();
        assert_eq!(e.frame_count(), frames.len());
        for (i, f) in frames.iter().enumerate() {
            let raw = e.frame_raw(i).unwrap();
            assert_eq!(sha(&raw), f["rawSha"].as_str().unwrap(), "LZO slot {slot} frame {i}");
            let img = e.frame_image(i).unwrap();
            assert_eq!((img.width as u64, img.height as u64), (f["w"].as_u64().unwrap(), f["h"].as_u64().unwrap()));
            assert_eq!(sha(&img.pixels), f["rgbaSha"].as_str().unwrap(), "RLE slot {slot} frame {i}");
        }
    }
}

#[test]
fn lmf_maps() {
    let Some(d) = game() else { return };
    let Some(g) = golden() else { return };
    for m in g["lmf"].as_array().unwrap() {
        let l = Lmf::open(&d.join(m["file"].as_str().unwrap())).unwrap();
        assert_eq!(l.maps.len() as u64, m["maps"].as_u64().unwrap());
        let id = m["id"].as_u64().unwrap() as u32;
        let map = l.load_map(id).unwrap();
        assert_eq!(map.w as u64, m["w"].as_u64().unwrap());
        assert_eq!(map.h as u64, m["h"].as_u64().unwrap());
        assert_eq!(map.layers as u64, m["layers"].as_u64().unwrap());
        assert_eq!(map.tiles.len() as u64, m["tilesLen"].as_u64().unwrap());
        assert_eq!(map.regions.len() as u64, m["regions"].as_u64().unwrap());
        assert_eq!(map.placements.len() as u64, m["placements"].as_u64().unwrap());
        for (p, exp) in map.placements.iter().zip(m["firstPlacements"].as_array().unwrap()) {
            let v: Vec<u64> = exp.as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).collect();
            assert_eq!(vec![p.id as u64, p.x0 as u64, p.y0 as u64, p.x1 as u64, p.y1 as u64, p.flags as u64], v);
        }
        assert_eq!(sha(&map.render_bg().pixels), m["bgSha"].as_str().unwrap(), "bg map {id}");
        if let Some(o) = m["obj0"].as_object() {
            let img = l.object_image(&map, map.placements[0].id).unwrap().unwrap();
            assert_eq!(img.width as u64, o["w"].as_u64().unwrap());
            assert_eq!(sha(&img.pixels), o["rgbaSha"].as_str().unwrap(), "obj0 map {id}");
        }
    }
}

#[test]
fn act_tables() {
    let Some(d) = game() else { return };
    let Some(g) = golden() else { return };
    let g = &g["act"];
    let a = IndexArchive::open(&d.join("all_char.act")).unwrap();
    let acts = act::parse_act(&a.read(0).unwrap()).unwrap();
    assert_eq!(acts.len() as u64, g["actions"].as_u64().unwrap());
    for (i, exp) in g["ops"].as_array().unwrap().iter().enumerate() {
        let got = acts[i].as_ref().map(|a| a.ops.iter().map(|o| o.to_text()).collect::<Vec<_>>().join("|"));
        assert_eq!(got.as_deref(), exp.as_str(), "action {i}");
    }
}

#[test]
fn talk_disasm() {
    let Some(d) = game() else { return };
    let Some(g) = golden() else { return };
    for t in g["talk"].as_array().unwrap() {
        let n = t["id"].as_u64().unwrap() as u32;
        let s = talk::script_from_file(&std::fs::read(d.join(talk::file_for(n))).unwrap(), n).unwrap();
        assert_eq!(s.code.len() as u64, t["len"].as_u64().unwrap());
        // the reference tools produce the original control-code form; the engine's own text is markup
        let r = talk::disasm_raw_text(&s.code);
        assert_eq!(r.instrs.len() as u64, t["instrs"].as_u64().unwrap(), "script {n} instr count");
        assert_eq!((r.ok as u64, r.unk as u64), (t["ok"].as_u64().unwrap(), t["unk"].as_u64().unwrap()));
        let ops: Vec<String> = r.instrs.iter().map(|i| i.op_key()).collect();
        assert_eq!(sha(ops.join(",").as_bytes()), t["ops"].as_str().unwrap(), "script {n} ops");
        let texts: Vec<String> = r.instrs.iter().filter_map(|i| if let talk::Instr::Op { text: Some(x), .. } = i { Some(x.clone()) } else { None }).collect();
        assert_eq!(texts.len() as u64, t["texts"].as_u64().unwrap());
        assert_eq!(sha(texts.join("\n").as_bytes()), t["textSha"].as_str().unwrap(), "script {n} text (legacy)");
    }
}

#[test]
fn value_tables() {
    let Some(d) = game() else { return };
    let Some(g) = golden() else { return };
    for t in g["g7f"].as_array().unwrap() {
        let b = std::fs::read(d.join(t["file"].as_str().unwrap())).unwrap();
        let (recs, bad) = g7f::parse(&b, 0x200);
        assert_eq!(recs.len() as u64, t["records"].as_u64().unwrap());
        assert_eq!(bad as u64, t["skipped"].as_u64().unwrap());
        let fr: Vec<_> = recs.iter().map(|r| g7f::friendly(r)).collect();
        let ids: Vec<String> = fr.iter().map(|r| r.id.map(|i| i.to_string()).unwrap_or_default()).collect();
        assert_eq!(sha(ids.join(",").as_bytes()), t["ids"].as_str().unwrap());
        let attrs: Vec<String> = fr
            .iter()
            .map(|r| format!("[{}]", r.attrs.iter().map(|(k, v)| format!("[{k},{v}]")).collect::<Vec<_>>().join(",")))
            .collect();
        assert_eq!(sha(attrs.join("\n").as_bytes()), t["attrsSha"].as_str().unwrap(), "{} attrs", t["file"]);
        let names: Vec<String> = fr.iter().map(|r| r.name.clone().unwrap_or_default()).collect();
        assert_eq!(sha(names.join("\n").as_bytes()), t["namesSha"].as_str().unwrap(), "{} names", t["file"]);
    }
}

/// Cell attributes (bit 30 = blocked, low byte = region) are consistent on real maps,
/// and an exit region of the inn (huge:2) is reachable from the hall.
/// Region names come from the player's files and are never hard-coded.
#[test]
fn lmf_cells_nav() {
    use newpal_assets::nav::{NavGrid, REGION_MASK};
    let Some(d) = game() else { return };
    for (file, id) in [("huge.lmf", 1), ("huge.lmf", 2), ("huge.lmf", 30), ("huge2.lmf", 277)] {
        let l = Lmf::open(&d.join(file)).unwrap();
        let m = l.load_map(id).unwrap();
        assert_eq!(m.cells.len(), m.w as usize * m.h as usize, "{file}:{id}");
        let blocked = m.cells.iter().filter(|&&c| c & 0x4000_0000 != 0).count() as f32 / m.cells.len() as f32;
        assert!((0.02..0.9).contains(&blocked), "{file}:{id} blocked {blocked}");
        assert!(m.cells.iter().all(|&c| (c & REGION_MASK) as usize <= m.regions.len()), "{file}:{id} region idx");
    }
    let m = Lmf::open(&d.join("huge.lmf")).unwrap().load_map(2).unwrap();
    assert_eq!(m.cells.iter().filter(|&&c| c & 0x4000_0000 != 0).count(), 853);
    let g = NavGrid::from_map(&m);
    // Exit regions use script ids >= 6000 (original convention).
    let exit = m
        .regions
        .iter()
        .position(|r| r.v[1] >= 6000)
        .expect("inn map should have an exit region") as u32
        + 1;
    let i = m.cells.iter().position(|&c| c & REGION_MASK == exit && c & 0x4000_0000 == 0).unwrap() as i32;
    let goal = NavGrid::center(i % g.w, i / g.w);
    let p = g.find_path((470.0, 430.0), goal).expect("path to exit");
    assert!(p.len() >= 2);
    let mut cur = (470.0, 430.0);
    for &q in &p { assert!(g.line_clear(cur, q)); cur = q; }
}

/// maps.dat: scene table + role placement for the inn opening (map 2).
/// Actor names come from the player's files and are never hard-coded.
#[test]
fn maps_dat_roles() {
    use newpal_assets::mapsdat::MapsDat;
    let Some(d) = game() else { return };
    let md = MapsDat::parse(&std::fs::read(d.join("maps.dat")).unwrap()).unwrap();
    assert_eq!(md.cities.len(), 564);
    assert_eq!(md.roles.len(), 4117);
    assert_eq!(md.city_for_map(2).unwrap().id, 2);
    let actor = md.roles_in(2).find(|r| r.actor() == 9).unwrap();
    assert_eq!((actor.cell(), actor.script(), actor.action()), ((11, 34), 110, 26));
    let a = IndexArchive::open(&d.join("all_char.act")).unwrap();
    assert!(!a.entry(8).unwrap().name.is_empty());
}

#[test]
fn audio_sources() {
    let Some(d) = game() else { return };
    let v = newpal_assets::vfs::Vfs::new(d, Vec::new());
    let md = v.maps_dat().unwrap();
    assert!(md.music_names.len() >= 70);
    let (a, _, _) = md.music_for(2).unwrap();
    assert_eq!(a, Some("M027.mp3"));
    let (bytes, ext) = v.music(a.unwrap()).unwrap().expect("Music/m027.mp3");
    assert_eq!(ext, "mp3");
    assert!(bytes.len() > 100_000);
    // inn -> town track in the table, played from Music/m005.mp3
    assert!(v.music(md.music_for(8).unwrap().0.unwrap()).unwrap().is_some());
    let wav = v.sfx(1).unwrap().expect("sfx 1");
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert!(v.sfx(0).unwrap().is_none());
}

/// Write `tests/golden.json` from the player's own install. Not run unless `NEWPAL_WRITE_GOLDEN` is set.
#[test]
fn write_golden() {
    if std::env::var_os("NEWPAL_WRITE_GOLDEN").is_none() {
        return;
    }
    let Some(d) = game() else { panic!("write_golden needs the original game install"); };
    let out = build_golden(&d);
    let p = golden_path();
    std::fs::write(&p, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
    eprintln!("wrote {}", p.display());
}

fn build_golden(d: &std::path::Path) -> Value {
    let a = IndexArchive::open(&d.join("all_char.tsw")).unwrap();
    let names: Vec<_> = a.entries.iter().map(|e| e.name.clone()).collect();
    let pak = json!({
        "file": "all_char.tsw",
        "slots": a.entries.len(),
        "nonEmpty": a.entries.iter().filter(|e| e.size > 0).count(),
        "namesSha": sha(names.join("\n").as_bytes()),
    });

    let mut tsw = Vec::new();
    for slot in [0usize, 186, 207] {
        let e = SpriteEntry::parse(a.read(slot).unwrap()).unwrap();
        let mut frames = Vec::new();
        for i in 0..e.frame_count() {
            let raw = e.frame_raw(i).unwrap();
            let img = e.frame_image(i).unwrap();
            frames.push(json!({
                "w": img.width, "h": img.height,
                "rawSha": sha(&raw), "rgbaSha": sha(&img.pixels),
            }));
        }
        tsw.push(json!({ "slot": slot, "bpp": e.bpp, "frames": frames }));
    }

    let mut lmf = Vec::new();
    for (file, id) in [("huge.lmf", 1u32), ("huge.lmf", 2), ("huge.lmf", 30), ("huge2.lmf", 277)] {
        let l = Lmf::open(&d.join(file)).unwrap();
        let map = l.load_map(id).unwrap();
        let first: Vec<Value> = map.placements.iter().take(4).map(|p| {
            json!([p.id, p.x0, p.y0, p.x1, p.y1, p.flags])
        }).collect();
        let mut rec = json!({
            "file": file, "id": id, "maps": l.maps.len(),
            "w": map.w, "h": map.h, "layers": map.layers,
            "tilesLen": map.tiles.len(), "regions": map.regions.len(),
            "placements": map.placements.len(),
            "firstPlacements": first,
            "bgSha": sha(&map.render_bg().pixels),
        });
        if let Some(p0) = map.placements.first() {
            if let Ok(Some(img)) = l.object_image(&map, p0.id) {
                rec["obj0"] = json!({ "w": img.width, "rgbaSha": sha(&img.pixels) });
            }
        }
        lmf.push(rec);
    }

    let act_a = IndexArchive::open(&d.join("all_char.act")).unwrap();
    let acts = act::parse_act(&act_a.read(0).unwrap()).unwrap();
    let ops: Vec<Value> = acts.iter().take(16).map(|a| {
        match a {
            Some(x) => Value::String(x.ops.iter().map(|o| o.to_text()).collect::<Vec<_>>().join("|")),
            None => Value::Null,
        }
    }).collect();

    let mut talk_arr = Vec::new();
    for n in [100u32, 104, 1001, 2050] {
        let s = talk::script_from_file(&std::fs::read(d.join(talk::file_for(n))).unwrap(), n).unwrap();
        let r = talk::disasm_raw_text(&s.code);
        let opk: Vec<String> = r.instrs.iter().map(|i| i.op_key()).collect();
        let texts: Vec<String> = r.instrs.iter().filter_map(|i| if let talk::Instr::Op { text: Some(x), .. } = i { Some(x.clone()) } else { None }).collect();
        talk_arr.push(json!({
            "id": n, "len": s.code.len(), "instrs": r.instrs.len(),
            "ok": r.ok, "unk": r.unk,
            "ops": sha(opk.join(",").as_bytes()),
            "texts": texts.len(),
            "textSha": sha(texts.join("\n").as_bytes()),
        }));
    }

    let mut g7f_arr = Vec::new();
    for file in ["ghero.dat", "gitem.dat", "gmagic.dat", "gmon.dat", "glevel.dat"] {
        let b = std::fs::read(d.join(file)).unwrap();
        let (recs, bad) = g7f::parse(&b, 0x200);
        let fr: Vec<_> = recs.iter().map(|r| g7f::friendly(r)).collect();
        let ids: Vec<String> = fr.iter().map(|r| r.id.map(|i| i.to_string()).unwrap_or_default()).collect();
        let attrs: Vec<String> = fr.iter().map(|r| {
            format!("[{}]", r.attrs.iter().map(|(k, v)| format!("[{k},{v}]")).collect::<Vec<_>>().join(","))
        }).collect();
        let names: Vec<String> = fr.iter().map(|r| r.name.clone().unwrap_or_default()).collect();
        g7f_arr.push(json!({
            "file": file, "records": recs.len(), "skipped": bad,
            "ids": sha(ids.join(",").as_bytes()),
            "attrsSha": sha(attrs.join("\n").as_bytes()),
            "namesSha": sha(names.join("\n").as_bytes()),
        }));
    }

    json!({
        "pak": pak,
        "tsw": tsw,
        "lmf": lmf,
        "act": { "actions": acts.len(), "ops": ops },
        "talk": talk_arr,
        "g7f": g7f_arr,
    })
}