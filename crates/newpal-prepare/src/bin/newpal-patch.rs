//! `newpal-patch build|apply` — residual HD patches.
//!
//! A patch is `HD - bicubic4x(original)` keyed by sha256 of the original frame.
//! It is useless without the player's own original data. Not legal advice.
use newpal_assets::image::RgbaImage;
use newpal_assets::tsw::SpriteEntry;
use newpal_assets::vfs::Vfs;
use newpal_prepare::residual::{
    self, apply_residual, bicubic_4x, decode_residual_lossless, process_frames, source_hash,
    Measure, PatchManifest,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "help".into());
    match cmd.as_str() {
        "build" => build(args),
        "apply" => apply(args),
        "cmp" => cmp(args),
        _ => {
            eprintln!(
                "newpal-patch build|apply\n  build --game-dir DIR --hd-cache DIR --out DIR [--only sprites|maps|all|all_char] [--limit N] [--encoding lossless|offset8|zstd|brotli|zstd-45|brotli-45] [--workers N] [--chunk N] [--only-key SUBSTR]\n  cmp --game-dir DIR --hd-cache DIR --a PATCH --b PATCH --key KEY\n  apply --game-dir DIR --patch DIR --hd-cache DIR [--workers N]"
            );
            Ok(())
        }
    }
}

fn take(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("missing value for {flag}"))
}

struct Unit {
    key: String,
    kind: UnitKind,
    hd_path: PathBuf,
}

enum UnitKind {
    Sprite { tsw: String, id: u32, fi: usize },
    Map { lmf: String, id: u32 },
}

impl Unit {
    fn is_map(&self) -> bool {
        matches!(self.kind, UnitKind::Map { .. })
    }
}

fn build(mut it: impl Iterator<Item = String>) -> Result<(), String> {
    let mut game = None;
    let mut hd = None;
    let mut out = None;
    let mut only = "sprites".to_string();
    let mut limit = None;
    let mut encoding = "brotli-45".to_string();
    let mut workers = default_workers();
    let mut chunk = 64usize;
    let mut only_key: Option<String> = None;
    while let Some(k) = it.next() {
        match k.as_str() {
            "--game-dir" => game = Some(PathBuf::from(take(&mut it, &k)?)),
            "--hd-cache" => hd = Some(PathBuf::from(take(&mut it, &k)?)),
            "--out" => out = Some(PathBuf::from(take(&mut it, &k)?)),
            "--only-key" => only_key = Some(take(&mut it, &k)?),
            "--only" => only = take(&mut it, &k)?,
            "--limit" => limit = Some(take(&mut it, &k)?.parse().map_err(|e| format!("{e}"))?),
            "--encoding" => encoding = take(&mut it, &k)?,
            "--workers" => workers = take(&mut it, &k)?.parse().map_err(|e| format!("{e}"))?,
            "--chunk" => chunk = take(&mut it, &k)?.parse().map_err(|e| format!("{e}"))?,
            "--data-root" => {
                let r = PathBuf::from(take(&mut it, &k)?);
                std::env::set_var("NEWPAL_DATA_ROOT", &r);
                if hd.is_none() {
                    hd = Some(r.join("cache").join("hd"));
                }
            }
            other => return Err(format!("unknown {other}")),
        }
    }
    let game = game.ok_or("--game-dir")?;
    let hd = hd.ok_or("--hd-cache")?;
    let out = out.ok_or("--out")?;
    let vfs = Vfs::new(game, Vec::new());
    vfs.set_hd_enabled(false);
    let mut units = collect_units(&hd, &only, limit)?;
    if let Some(sub) = &only_key {
        units.retain(|u| u.key.contains(sub.as_str()));
    }
    let total = units.len();
    let n_maps = units.iter().filter(|u| u.is_map()).count();
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out.join("res")).map_err(|e| e.to_string())?;
    let mut all_man = load_resume(&out, &encoding)?;
    let done: HashSet<String> = all_man.entries.iter().map(|e| e.key.clone()).collect();
    let already = done.len();
    let units: Vec<Unit> = units.into_iter().filter(|u| !done.contains(&u.key)).collect();
    eprintln!(
        "[patch] collected {total} items ({n_maps} maps), encoding {encoding}, workers {workers}, chunk {chunk}"
    );
    if already > 0 {
        eprintln!(
            "[patch] resume: {already} already in manifest, {} remaining",
            units.len()
        );
    }

    let mut all_meas = Measure {
        offset8_psnr_min: f64::INFINITY,
        packed_psnr_min: f64::INFINITY,
        ..Measure::default()
    };
    // Seed packed size from existing entries (duplicates counted per key, same as a fresh build).
    all_meas.frames = already;
    all_meas.packed_ok = already;
    all_meas.packed_bytes = all_man.entries.iter().map(|e| e.bytes).sum();
    let t0 = Instant::now();
    let mut i = 0usize;
    let done_base = already;
    while i < units.len() {
        if units[i].is_map() {
            // Maps: never preload a multi-item batch. Large maps are 100–300 MiB
            // decoded RGBA and several GiB of encode working set; the old
            // (PNG-filesize × 12) budget still scheduled 22 at once and OOM'd.
            let u = &units[i];
            match png_ihdr_wh(&u.hd_path) {
                Ok((w, h)) => {
                    let est = map_workset_bytes(w, h);
                    eprintln!(
                        "[patch] map {} {w}x{h} workset_est={:.2} GiB, load+encode 1",
                        u.key,
                        est as f64 / (1024.0 * 1024.0 * 1024.0)
                    );
                }
                Err(e) => eprintln!("[patch] map {}: IHDR {e} (still trying)", u.key),
            }
            let t_load = Instant::now();
            match load_unit(&vfs, u) {
                Ok(it) => {
                    let load_s = t_load.elapsed().as_secs_f32();
                    let t_proc = Instant::now();
                    // One map at a time, but all workers cooperate inside it (striped encode).
                    let (man, meas) = process_frames(&[it], Some(&out), &encoding, workers)?;
                    eprintln!(
                        "[patch]   {}: load {load_s:.1}s, process {:.1}s",
                        u.key,
                        t_proc.elapsed().as_secs_f32()
                    );
                    all_man.entries.extend(man.entries);
                    all_meas.add(&meas);
                    if meas.packed_ok > 0 && meas.hd_png_bytes == 0 {
                        // large images skip PNG re-encoding for stats; use the cache file size
                        all_meas.hd_png_bytes += std::fs::metadata(&u.hd_path).map(|m| m.len()).unwrap_or(0);
                    }
                    all_man.entries.sort_by(|a, b| a.key.cmp(&b.key));
                    all_man.entries.dedup_by(|a, b| a.key == b.key);
                    write_manifest(&out, &all_man)?;
                }
                Err(e) => eprintln!("[patch] skip {}: {e}", u.key),
            }
            i += 1;
            eprintln!(
                "[patch] {}/{total} packed={:.1} MB hd_png={:.1} MB apply_ms={:.0} [maps]",
                done_base + i,
                all_meas.packed_bytes as f64 / 1e6,
                all_meas.hd_png_bytes as f64 / 1e6,
                all_meas.apply_packed_ms
            );
            continue;
        }

        let end = next_batch_end(&units, i, workers, chunk);
        let slice = &units[i..end];
        let mut items = Vec::with_capacity(slice.len());
        for u in slice {
            match load_unit(&vfs, u) {
                Ok(it) => items.push(it),
                Err(e) => eprintln!("[patch] skip {}: {e}", u.key),
            }
        }
        let w = workers.min(items.len().max(1));
        if !items.is_empty() {
            let (man, meas) = process_frames(&items, Some(&out), &encoding, w)?;
            all_man.entries.extend(man.entries);
            all_meas.add(&meas);
            all_man.entries.sort_by(|a, b| a.key.cmp(&b.key));
            all_man.entries.dedup_by(|a, b| a.key == b.key);
            write_manifest(&out, &all_man)?;
        }
        i = end;
        eprintln!(
            "[patch] {}/{total} packed={:.1} MB hd_png={:.1} MB apply_ms={:.0}",
            done_base + i,
            all_meas.packed_bytes as f64 / 1e6,
            all_meas.hd_png_bytes as f64 / 1e6,
            all_meas.apply_packed_ms
        );
    }
    all_meas.secs = t0.elapsed().as_secs_f32();
    all_man.entries.sort_by(|a, b| a.key.cmp(&b.key));
    write_manifest(&out, &all_man)?;
    eprintln!(
        "[patch] build done: frames={} encoding={} packed_ok={} packed={:.3} MB hd_png={:.3} MB orig_png={:.3} MB packed/hd={:.3} packed/orig={:.3} PSNR avg={:.2} min={:.2} q_avg={:.2} apply_ms={:.0} lossless={:.3} MB offset8={:.3} MB in {:.1}s -> {} ({} entries)",
        all_meas.frames,
        encoding,
        all_meas.packed_ok,
        all_meas.packed_bytes as f64 / 1e6,
        all_meas.hd_png_bytes as f64 / 1e6,
        all_meas.orig_png_bytes as f64 / 1e6,
        if all_meas.hd_png_bytes > 0 { all_meas.packed_bytes as f64 / all_meas.hd_png_bytes as f64 } else { 0.0 },
        if all_meas.orig_png_bytes > 0 { all_meas.packed_bytes as f64 / all_meas.orig_png_bytes as f64 } else { 0.0 },
        all_meas.packed_psnr_avg(),
        if all_meas.packed_psnr_min.is_finite() { all_meas.packed_psnr_min } else { 0.0 },
        all_meas.packed_q_avg(),
        all_meas.apply_packed_ms,
        all_meas.lossless_bytes as f64 / 1e6,
        all_meas.offset8_bytes as f64 / 1e6,
        all_meas.secs,
        out.display(),
        all_man.entries.len()
    );
    Ok(())
}


/// Decode two patch dirs' entries for one key and compare the reconstructions.
fn cmp(mut it: impl Iterator<Item = String>) -> Result<(), String> {
    let (mut game, mut hd, mut a, mut b, mut key) = (None, None, None, None, None);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--game-dir" => game = Some(PathBuf::from(take(&mut it, &k)?)),
            "--hd-cache" => hd = Some(PathBuf::from(take(&mut it, &k)?)),
            "--a" => a = Some(PathBuf::from(take(&mut it, &k)?)),
            "--b" => b = Some(PathBuf::from(take(&mut it, &k)?)),
            "--key" => key = Some(take(&mut it, &k)?),
            other => return Err(format!("unknown {other}")),
        }
    }
    let (game, hd, a, b, key) = (game.ok_or("--game-dir")?, hd.ok_or("--hd-cache")?, a.ok_or("--a")?, b.ok_or("--b")?, key.ok_or("--key")?);
    let vfs = Vfs::new(game, Vec::new());
    vfs.set_hd_enabled(false);
    let unit = collect_units(&hd, "all", None)?
        .into_iter()
        .find(|u| u.key == key)
        .ok_or("key not found in hd cache")?;
    let (_, orig, hd_img) = load_unit(&vfs, &unit)?;
    let t = Instant::now();
    let base = bicubic_4x(&orig);
    eprintln!("[cmp] baseline {:.1}s", t.elapsed().as_secs_f32());
    let mut recs = Vec::new();
    for dir in [&a, &b] {
        let man = load_resume(dir, "brotli-45")?;
        let e = man.entries.iter().find(|e| e.key == key).ok_or("key not in manifest")?;
        let bytes = std::fs::read(dir.join(&e.file)).map_err(|e| e.to_string())?;
        let ver = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let t = Instant::now();
        let rec = newpal_prepare::residual_pack::apply_packed(&base, &bytes)?;
        let ms = t.elapsed().as_millis();
        let p = residual::psnr(&hd_img, &rec).unwrap_or(0.0);
        eprintln!("[cmp] {} v{ver}: {} bytes, apply {ms} ms, PSNR vs HD {p:.3}", dir.display(), bytes.len());
        recs.push(rec);
    }
    eprintln!("[cmp] reconstructions identical: {}", recs[0].pixels == recs[1].pixels);
    Ok(())
}

fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(2))
        .unwrap_or(2)
        .max(1)
}

fn load_resume(out: &Path, encoding: &str) -> Result<PatchManifest, String> {
    let p = out.join("manifest.json");
    if !p.is_file() {
        return Ok(PatchManifest::new(encoding));
    }
    let man: PatchManifest = serde_json::from_slice(&std::fs::read(&p).map_err(|e| e.to_string())?)
        .map_err(|e| format!("manifest: {e}"))?;
    if man.encoding != encoding {
        return Err(format!(
            "existing manifest encoding '{}' != '{encoding}'",
            man.encoding
        ));
    }
    if man.baseline != residual::BASELINE || man.scale != residual::SCALE {
        return Err("existing manifest baseline/scale mismatch".into());
    }
    Ok(man)
}

fn write_manifest(out: &Path, man: &PatchManifest) -> Result<(), String> {
    let s = serde_json::to_vec_pretty(man).map_err(|e| e.to_string())?;
    let tmp = out.join("manifest.json.tmp");
    std::fs::write(&tmp, &s).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out.join("manifest.json")).map_err(|e| e.to_string())
}

/// Sprite batches stay at `chunk` and stop before the first map.
/// Map items are not batched here: the build loop loads and encodes one map at a
/// time. Compressed PNG size is a poor proxy for decoded working set
/// (a 10240×7680 map is 300 MiB RGBA and several GiB while encoding).
fn next_batch_end(units: &[Unit], i: usize, _workers: usize, chunk: usize) -> usize {
    if i >= units.len() {
        return i;
    }
    if units[i].is_map() {
        return (i + 1).min(units.len());
    }
    let end = (i + chunk.max(1)).min(units.len());
    units[i..end]
        .iter()
        .position(|u| u.is_map())
        .map(|k| i + k)
        .unwrap_or(end)
}

/// Read PNG width and height from the IHDR chunk only (no pixel decode).
fn png_ihdr_wh(path: &Path) -> Result<(u32, u32), String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hdr = [0u8; 24];
    f.read_exact(&mut hdr).map_err(|e| format!("{}: {e}", path.display()))?;
    const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if hdr[0..8] != PNG_SIG {
        return Err(format!("{}: not a PNG", path.display()));
    }
    if &hdr[12..16] != b"IHDR" {
        return Err(format!("{}: missing IHDR", path.display()));
    }
    let w = u32::from_be_bytes(hdr[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(hdr[20..24].try_into().unwrap());
    if w == 0 || h == 0 {
        return Err(format!("{}: bad IHDR {w}x{h}", path.display()));
    }
    Ok((w, h))
}

/// Conservative encode working set for one map, in bytes:
/// HD RGBA + orig (1/4 resolution) + bicubic base + i16 residual / predict /
/// packed planes + brotli buffers + slack.
fn map_workset_bytes(w: u32, h: u32) -> u64 {
    let hd = (w as u64).saturating_mul(h as u64).saturating_mul(4);
    let orig = (w as u64 / 4).saturating_mul(h as u64 / 4).saturating_mul(4);
    hd.saturating_mul(10).saturating_add(orig.saturating_mul(2)).max(32 * 1024 * 1024)
}

fn collect_units(hd: &Path, only: &str, limit: Option<usize>) -> Result<Vec<Unit>, String> {
    let want_sprites = only != "maps";
    let want_maps = only == "maps" || only == "all";
    let mut items = Vec::new();
    if want_sprites {
        collect_sprites(hd, only, limit, &mut items)?;
    }
    if want_maps && limit.is_none_or(|n| items.len() < n) {
        let remain = limit.map(|n| n.saturating_sub(items.len()));
        collect_maps(hd, remain, &mut items)?;
    }
    Ok(items)
}

fn collect_sprites(
    hd: &Path,
    only: &str,
    limit: Option<usize>,
    items: &mut Vec<Unit>,
) -> Result<(), String> {
    let tsws: Vec<&str> = if only == "sprites" || only == "all" || only == "maps" {
        residual::TSW_DEFAULT.to_vec()
    } else if only == "all_char" || only.starts_with("all_") {
        vec![only]
    } else {
        residual::TSW_DEFAULT.to_vec()
    };
    let root = hd.join("sprites");
    if !root.is_dir() {
        return Err(format!("no {} ", root.display()));
    }
    let mut tsw_dirs: Vec<_> = std::fs::read_dir(&root)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .collect();
    tsw_dirs.sort_by_key(|e| e.file_name());
    'outer: for td in tsw_dirs {
        let tsw = td.file_name().to_string_lossy().to_string();
        if !tsws.contains(&tsw.as_str()) {
            continue;
        }
        let mut ids: Vec<_> = std::fs::read_dir(td.path())
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .collect();
        ids.sort_by_key(|e| e.file_name());
        for idd in ids {
            let id: u32 = match idd.file_name().to_string_lossy().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let mut frames: Vec<_> = std::fs::read_dir(idd.path())
                .map_err(|e| e.to_string())?
                .filter_map(|e| e.ok())
                .collect();
            frames.sort_by_key(|e| e.file_name());
            for fe in frames {
                let name = fe.file_name().to_string_lossy().to_string();
                let Some(rest) = name.strip_prefix('f').and_then(|s| s.strip_suffix("@4x.png")) else {
                    continue;
                };
                let Ok(fi) = rest.parse::<usize>() else { continue };
                items.push(Unit {
                    key: format!("sprites/{tsw}/{id}/f{fi}"),
                    kind: UnitKind::Sprite {
                        tsw: tsw.clone(),
                        id,
                        fi,
                    },
                    hd_path: fe.path(),
                });
                if limit.is_some_and(|n| items.len() >= n) {
                    break 'outer;
                }
            }
        }
    }
    Ok(())
}

fn collect_maps(hd: &Path, limit: Option<usize>, items: &mut Vec<Unit>) -> Result<(), String> {
    let root = hd.join("maps");
    if !root.is_dir() {
        return Ok(());
    }
    let mut lmfs: Vec<_> = std::fs::read_dir(&root)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .collect();
    lmfs.sort_by_key(|e| e.file_name());
    'outer: for ld in lmfs {
        if !ld.path().is_dir() {
            continue;
        }
        let lmf = ld.file_name().to_string_lossy().to_string();
        let mut ids: Vec<_> = std::fs::read_dir(ld.path())
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .collect();
        ids.sort_by_key(|e| e.file_name());
        for idd in ids {
            let id: u32 = match idd.file_name().to_string_lossy().parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let bg = idd.path().join("bg@4x.png");
            if !bg.is_file() {
                continue;
            }
            items.push(Unit {
                key: format!("maps/{lmf}/{id}/bg"),
                kind: UnitKind::Map { lmf: lmf.clone(), id },
                hd_path: bg,
            });
            if limit.is_some_and(|n| items.len() >= n) {
                break 'outer;
            }
        }
    }
    Ok(())
}

fn load_unit(vfs: &Vfs, u: &Unit) -> Result<(String, RgbaImage, RgbaImage), String> {
    let bytes = std::fs::read(&u.hd_path).map_err(|e| e.to_string())?;
    let hd_img = RgbaImage::from_png(&bytes).map_err(|e| e.to_string())?;
    let orig = match &u.kind {
        UnitKind::Sprite { tsw, id, fi } => {
            let slot = id.saturating_sub(1) as usize;
            let raw = vfs
                .pak(&format!("{tsw}.tsw"))
                .map_err(|e| e.to_string())?
                .read(slot)
                .map_err(|e| e.to_string())?;
            let spr = SpriteEntry::parse(raw).map_err(|e| e.to_string())?;
            spr.frame_image(*fi).map_err(|e| e.to_string())?
        }
        UnitKind::Map { lmf, id } => vfs
            .map(lmf, *id)
            .map_err(|e| e.to_string())?
            .render_bg(),
    };
    Ok((u.key.clone(), orig, hd_img))
}

fn apply(mut it: impl Iterator<Item = String>) -> Result<(), String> {
    let mut game = None;
    let mut patch = None;
    let mut hd = None;
    let mut workers = 2usize;
    while let Some(k) = it.next() {
        match k.as_str() {
            "--game-dir" => game = Some(PathBuf::from(take(&mut it, &k)?)),
            "--patch" => patch = Some(PathBuf::from(take(&mut it, &k)?)),
            "--hd-cache" => hd = Some(PathBuf::from(take(&mut it, &k)?)),
            "--workers" => workers = take(&mut it, &k)?.parse().map_err(|e| format!("{e}"))?,
            other => return Err(format!("unknown {other}")),
        }
    }
    let game = game.ok_or("--game-dir")?;
    let patch = patch.ok_or("--patch")?;
    let hd = hd.ok_or("--hd-cache")?;
    let man: PatchManifest = serde_json::from_slice(
        &std::fs::read(patch.join("manifest.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if man.baseline != residual::BASELINE || man.scale != residual::SCALE {
        return Err(format!(
            "unsupported patch baseline/scale {} / {}",
            man.baseline, man.scale
        ));
    }
    let vfs = Vfs::new(game, Vec::new());
    vfs.set_hd_enabled(false);
    let t0 = Instant::now();
    let mut ok = 0usize;
    let mut fail = 0usize;
    let entries = man.entries;
    let n = entries.len();
    let chunk = (n / workers.max(1)).max(1);
    std::thread::scope(|scope| {
        let mut joins = Vec::new();
        for ch in entries.chunks(chunk) {
            let vfs = &vfs;
            let patch = &patch;
            let hd = &hd;
            let enc = man.encoding.clone();
            joins.push(scope.spawn(move || {
                let mut ok = 0usize;
                let mut fail = 0usize;
                for e in ch {
                    match apply_one(vfs, patch, hd, e, &enc) {
                        Ok(()) => ok += 1,
                        Err(err) => {
                            fail += 1;
                            eprintln!("[apply] {}: {err}", e.key);
                        }
                    }
                }
                (ok, fail)
            }));
        }
        for j in joins {
            if let Ok((a, b)) = j.join() {
                ok += a;
                fail += b;
            }
        }
    });
    eprintln!(
        "[patch] apply {} ok, {} failed, {:.1}s -> {}",
        ok,
        fail,
        t0.elapsed().as_secs_f32(),
        hd.display()
    );
    Ok(())
}

fn apply_one(
    vfs: &Vfs,
    patch: &PathBuf,
    hd: &PathBuf,
    e: &residual::PatchEntry,
    encoding: &str,
) -> Result<(), String> {
    let parts: Vec<&str> = e.key.split('/').collect();
    let orig = match parts.as_slice() {
        ["sprites", tsw, id, f] => {
            let id: u32 = id.parse().map_err(|_| "bad id")?;
            let fi: usize = f
                .strip_prefix('f')
                .ok_or("bad frame")?
                .parse()
                .map_err(|_| "bad frame")?;
            let slot = id.saturating_sub(1) as usize;
            let raw = vfs
                .pak(&format!("{tsw}.tsw"))
                .map_err(|e| e.to_string())?
                .read(slot)
                .map_err(|e| e.to_string())?;
            let spr = SpriteEntry::parse(raw).map_err(|e| e.to_string())?;
            spr.frame_image(fi).map_err(|e| e.to_string())?
        }
        ["maps", lmf, id, "bg"] => {
            let id: u32 = id.parse().map_err(|_| "bad id")?;
            vfs.map(lmf, id).map_err(|e| e.to_string())?.render_bg()
        }
        _ => return Err("unsupported key".into()),
    };
    let sha = source_hash(&orig);
    if sha != e.sha256 {
        return Err(format!("sha mismatch (got {sha})"));
    }
    let base = bicubic_4x(&orig);
    let bytes = std::fs::read(patch.join(&e.file)).map_err(|e| e.to_string())?;
    let rec = if encoding == "offset8" {
        let img = RgbaImage::from_png(&bytes).map_err(|e| e.to_string())?;
        residual::apply_residual_offset8(&base, &img)?
    } else if encoding.starts_with("zstd") || encoding.starts_with("brotli") {
        newpal_prepare::residual_pack::apply_packed(&base, &bytes)?
    } else {
        let (_, _, d) = decode_residual_lossless(&bytes)?;
        apply_residual(&base, &d)?
    };
    if rec.width != e.width || rec.height != e.height {
        return Err(format!(
            "size {}x{} vs entry {}x{}",
            rec.width, rec.height, e.width, e.height
        ));
    }
    let dest = match parts.as_slice() {
        ["sprites", tsw, id, f] => hd
            .join("sprites")
            .join(tsw)
            .join(*id)
            .join(format!("{f}@4x.png")),
        ["maps", lmf, id, "bg"] => hd
            .join("maps")
            .join(lmf)
            .join(*id)
            .join("bg@4x.png"),
        _ => unreachable!(),
    };
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    rec.save_png(&dest).map_err(|e| e.to_string())?;
    Ok(())
}