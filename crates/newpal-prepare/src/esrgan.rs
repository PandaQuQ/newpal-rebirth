//! Pinned Real-ESRGAN ncnn Vulkan (MIT, xinntao/Real-ESRGAN-ncnn-vulkan), downloaded into the per-user
//! tools cache and verified by sha256. Used to 4x-upscale the player's own sprites (never distributed).
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const VERSION: &str = "realesrgan-ncnn-vulkan-20220424";
pub const URL: &str = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip";
pub const SHA256: &str = "abc02804e17982a3be33675e4d471e91ea374e65b70167abc09e31acb412802d";
pub const ZIP_SIZE: u64 = 45_474_481;
pub const MODEL: &str = "realesrgan-x4plus-anime";
const ROOT: &str = "realesrgan-ncnn-vulkan-20220424-windows";

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn installed(tools: &Path) -> Option<(PathBuf, PathBuf)> {
    let root = tools.join(ROOT);
    let (exe, models) = (root.join("realesrgan-ncnn-vulkan.exe"), root.join("models"));
    (exe.is_file() && models.join(format!("{MODEL}.param")).is_file()).then_some((exe, models))
}

pub fn ensure(tools: &Path, log: &mut dyn FnMut(String)) -> Result<(PathBuf, PathBuf), String> {
    if let Some(x) = installed(tools) { return Ok(x); }
    std::fs::create_dir_all(tools).map_err(|e| e.to_string())?;
    let zip = tools.join(format!("{ROOT}.zip"));
    let ok = |z: &Path| sha256_file(z).map(|h| h == SHA256).unwrap_or(false);
    if !zip.is_file() || !ok(&zip) {
        log(format!("downloading {URL} ({} MB)", ZIP_SIZE / 1_000_000));
        let st = std::process::Command::new("curl.exe").args(["-sSL", "--fail", "--retry", "3", "-o"]).arg(&zip).arg(URL).status().map_err(|e| format!("curl.exe: {e}"))?;
        if !st.success() { return Err(format!("download failed: {st}")); }
        if !ok(&zip) { return Err(format!("sha256 mismatch for {} (expected {SHA256})", zip.display())); }
    }
    log("sha256 ok, extracting".into());
    std::fs::create_dir_all(tools.join(ROOT)).map_err(|e| e.to_string())?;
    let st = std::process::Command::new("tar.exe").arg("-xf").arg(&zip).arg("-C").arg(tools.join(ROOT)).status().map_err(|e| format!("tar.exe: {e}"))?;
    if !st.success() { return Err(format!("extract failed: {st}")); }
    installed(tools).ok_or_else(|| "realesrgan not found after extract".into())
}
