//! Merge one hd-work batch into cache/hd (salvage a failed esrgan child).
//!
//! Usage:
//!   merge_job <batch_id> sprite|map
//!            [--data-root <path>] [--work <hd-work>] [--out <hd-cache>]
//!
//! Paths default to `<data-root>/cache/hd-work` and `<data-root>/cache/hd`.
//! `data-root` comes from `--data-root`, else `NEWPAL_DATA_ROOT`, else auto-detect.
fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut id: Option<u32> = None;
    let mut kind: Option<newpal_prepare::hd_batch::Kind> = None;
    let mut data_root: Option<std::path::PathBuf> = None;
    let mut work: Option<std::path::PathBuf> = None;
    let mut out: Option<std::path::PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-root" => {
                data_root = Some(std::path::PathBuf::from(
                    args.next().expect("--data-root needs a path"),
                ));
            }
            "--work" => {
                work = Some(std::path::PathBuf::from(
                    args.next().expect("--work needs a path"),
                ));
            }
            "--out" => {
                out = Some(std::path::PathBuf::from(
                    args.next().expect("--out needs a path"),
                ));
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage: merge_job <batch_id> sprite|map [--data-root PATH] [--work DIR] [--out DIR]"
                );
                return;
            }
            s if s.starts_with('-') => panic!("unknown flag {s}"),
            s => {
                if id.is_none() {
                    id = Some(s.parse().expect("usage: merge_job <batch_id> sprite|map"));
                } else if kind.is_none() {
                    kind = Some(match s {
                        "sprite" => newpal_prepare::hd_batch::Kind::Sprite,
                        "map" => newpal_prepare::hd_batch::Kind::Map,
                        _ => panic!("usage: merge_job <batch_id> sprite|map"),
                    });
                } else {
                    panic!("unexpected argument {s}");
                }
            }
        }
    }
    let id = id.expect("usage: merge_job <batch_id> sprite|map");
    let kind = kind.expect("usage: merge_job <batch_id> sprite|map");
    if let Some(root) = data_root {
        std::env::set_var(newpal_assets::gamedir::ENV_DATA_ROOT, &root);
    }
    let cache = newpal_assets::gamedir::cache_dir();
    let work = work.unwrap_or_else(|| cache.join("hd-work"));
    let out = out.unwrap_or_else(|| cache.join("hd"));
    let n = newpal_prepare::hd_batch::merge_job(&work, &out, id, kind).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(2);
    });
    eprintln!("merged {n} files from batch {id}\n  work {}\n  out  {}", work.display(), out.display());
}
