//! Pinned rife-ncnn-vulkan release (MIT, github.com/nihui/rife-ncnn-vulkan), downloaded on first prepare
//! into the per-user tools cache and verified by sha256. Uses the OS `curl.exe` + `tar.exe` (Windows 10+).
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const VERSION: &str = "rife-ncnn-vulkan-20221029";
pub const URL: &str = "https://github.com/nihui/rife-ncnn-vulkan/releases/download/20221029/rife-ncnn-vulkan-20221029-windows.zip";
pub const SHA256: &str = "d8e4d772d26cd8006ef0ad0bc82eb191b53c68677d1ae2f42506d74cbbbea606";
pub const ZIP_SIZE: u64 = 431_540_241;
pub const MODEL: &str = "rife-v4.6";
const ROOT: &str = "rife-ncnn-vulkan-20221029-windows";

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// (exe, model dir) if already installed.
pub fn installed(tools: &Path) -> Option<(PathBuf, PathBuf)> {
    let root = tools.join(ROOT);
    let (exe, model) = (root.join("rife-ncnn-vulkan.exe"), root.join(MODEL));
    (exe.is_file() && model.join("flownet.param").is_file()).then_some((exe, model))
}

/// Download (if needed), verify, extract only the exe, its runtime DLL, the license and the pinned model.
pub fn ensure(tools: &Path, log: &mut dyn FnMut(String)) -> Result<(PathBuf, PathBuf), String> {
    if let Some(x) = installed(tools) {
        return Ok(x);
    }
    std::fs::create_dir_all(tools).map_err(|e| e.to_string())?;
    let zip = tools.join(format!("{ROOT}.zip"));
    let ok = |z: &Path| sha256_file(z).map(|h| h == SHA256).unwrap_or(false);
    if !zip.is_file() || !ok(&zip) {
        log(format!("downloading {URL} ({} MB)", ZIP_SIZE / 1_000_000));
        let st = std::process::Command::new("curl.exe")
            .args(["-sSL", "--fail", "--retry", "3", "-o"])
            .arg(&zip)
            .arg(URL)
            .status()
            .map_err(|e| format!("curl.exe: {e}"))?;
        if !st.success() {
            return Err(format!("download failed: {st}"));
        }
        if !ok(&zip) {
            return Err(format!("sha256 mismatch for {} (expected {SHA256})", zip.display()));
        }
    }
    log("sha256 ok, extracting".into());
    let members = ["LICENSE", "README.md", "rife-ncnn-vulkan.exe", "vcomp140.dll", MODEL];
    let mut c = std::process::Command::new("tar.exe");
    c.arg("-xf").arg(&zip).arg("-C").arg(tools);
    for m in members {
        c.arg(format!("{ROOT}/{m}"));
    }
    let st = c.status().map_err(|e| format!("tar.exe: {e}"))?;
    if !st.success() {
        return Err(format!("extract failed: {st}"));
    }
    installed(tools).ok_or_else(|| "rife not found after extract".into())
}

/// Cheap check that a Vulkan loader exists (the GPU itself is probed by the caller, e.g. via wgpu).
pub fn vulkan_loader_present() -> bool {
    match std::env::var_os("SystemRoot") {
        Some(r) => Path::new(&r).join("System32").join("vulkan-1.dll").is_file(),
        None => false,
    }
}