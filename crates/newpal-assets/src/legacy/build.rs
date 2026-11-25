//! Which build of the original game is installed? Identified by SHA-256 of the exe (and of each shipped
//! `StringDB.txt`). The text importers do not depend on the build: an unknown build is used as is, with a note
//! that the id-range categories of [`super::stringdb`] and the script layout were verified only on the known one.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct KnownBuild {
    pub label: &'static str,
    pub exe_sha256: &'static str,
    /// (folder, sha256 of StringDB.txt, number of entries)
    pub stringdb: &'static [(&'static str, &'static str, usize)],
}

pub const KNOWN: &[KnownBuild] = &[KnownBuild {
    label: "Steam 695630 NewPAL_Release.exe (1,429,504 bytes)",
    exe_sha256: "1301e72bf46008eb74a0f49ab81d9894ab92d037d63c63d86c906aa067c961dc",
    stringdb: &[
        ("zh-Hant", "7114b9e3c0096bb92fd7e1062a726579632c718ca80b3dd7407d8ceb1170cac1", 930),
        ("zh-Hans", "daad0985d26d97fdee4c01e8ef6292670a63af93e563b2e4fa3c485a9d3970d5", 930),
        ("en", "1ff32af47aadcc47839df5c968caf77a6901999f636a3d5c4dda553a04a3a859", 925),
    ],
}];

#[derive(Debug, Default)]
pub struct BuildInfo {
    pub exe: Option<PathBuf>,
    pub exe_sha256: Option<String>,
    pub known: Option<&'static KnownBuild>,
    pub notes: Vec<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn find_exe(game_dir: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(game_dir).ok()?;
    let mut exes: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("newpal")))
        .collect();
    exes.sort();
    exes.into_iter().next()
}

/// Read-only: hashes the exe and the shipped string tables.
pub fn detect(game_dir: &Path) -> BuildInfo {
    let mut b = BuildInfo::default();
    let Some(exe) = find_exe(game_dir) else {
        b.notes.push("no NewPAL*.exe in the game folder: build not identified".into());
        return b;
    };
    match std::fs::read(&exe) {
        Ok(bytes) => {
            let h = sha256_hex(&bytes);
            b.known = KNOWN.iter().find(|k| k.exe_sha256 == h);
            if b.known.is_none() {
                b.notes.push(format!("unknown exe build {h}: string categories / script layout unverified (text is still read as is)"));
            }
            b.exe_sha256 = Some(h);
        }
        Err(e) => b.notes.push(format!("cannot read {}: {e}", exe.display())),
    }
    b.exe = Some(exe);
    if let Some(k) = b.known {
        for (folder, sha, n) in k.stringdb {
            let p = game_dir.join(folder).join("StringDB.txt");
            match std::fs::read(&p) {
                Ok(bytes) if sha256_hex(&bytes) != *sha => b.notes.push(format!("{folder}/StringDB.txt differs from the known build (modified or patched; expected {n} entries)")),
                Ok(_) => {}
                Err(_) => b.notes.push(format!("{folder}/StringDB.txt missing")),
            }
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_folder_is_graceful() {
        let d = std::env::temp_dir().join(format!("newpal-build-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let b = detect(&d);
        assert!(b.known.is_none() && b.exe.is_none() && !b.notes.is_empty());
        std::fs::write(d.join("NewPAL_Release.exe"), b"not the real exe").unwrap();
        let b = detect(&d);
        assert!(b.known.is_none() && b.exe_sha256.is_some() && b.notes[0].contains("unknown exe build"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
