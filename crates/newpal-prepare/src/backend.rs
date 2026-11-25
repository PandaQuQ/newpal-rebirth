//! Interpolation backends. Every backend turns one looping animation (frames on a shared canvas) into
//! `n * factor` frames: original i, then `factor - 1` in-betweens towards frame (i + 1) % n.
use newpal_assets::image::RgbaImage;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub type Result<T> = std::result::Result<T, String>;

pub trait Backend: Sync {
    /// stable id incl. model + version; part of the cache key
    fn id(&self) -> String;
    fn interpolate(&self, frames: &[RgbaImage], factor: u32) -> Result<Vec<RgbaImage>>;
}

/// Premultiplied-alpha lerp (no dark fringes).
pub fn blend(a: &RgbaImage, b: &RgbaImage, k: f32) -> RgbaImage {
    let mut o = RgbaImage::new(a.width, a.height);
    for i in 0..(a.width * a.height) as usize {
        let (pa, pb) = (&a.pixels[i * 4..i * 4 + 4], &b.pixels[i * 4..i * 4 + 4]);
        let (aa, ab) = (pa[3] as f32 / 255.0, pb[3] as f32 / 255.0);
        let al = aa * (1.0 - k) + ab * k;
        let mut px = [0u8; 4];
        if al > 0.0 {
            for c in 0..3 {
                px[c] = ((pa[c] as f32 * aa * (1.0 - k) + pb[c] as f32 * ab * k) / al).round().clamp(0.0, 255.0) as u8;
            }
        }
        px[3] = (al * 255.0).round() as u8;
        o.pixels[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    o
}

/// CPU fallback: cross-fade.
pub struct CpuCrossfade;

impl Backend for CpuCrossfade {
    fn id(&self) -> String {
        "cpu-crossfade/1".into()
    }
    fn interpolate(&self, frames: &[RgbaImage], factor: u32) -> Result<Vec<RgbaImage>> {
        let n = frames.len();
        let mut out = Vec::with_capacity(n * factor as usize);
        for i in 0..n {
            out.push(frames[i].clone());
            for j in 1..factor {
                out.push(blend(&frames[i], &frames[(i + 1) % n], j as f32 / factor as f32));
            }
        }
        Ok(out)
    }
}

/// Runs the interpolator executable (real process, or a mock in tests).
pub trait Runner: Sync {
    fn run(&self, exe: &Path, args: &[String]) -> Result<()>;
}

/// Real process at idle priority, no console window.
pub struct ProcessRunner;

impl Runner for ProcessRunner {
    fn run(&self, exe: &Path, args: &[String]) -> Result<()> {
        let mut c = std::process::Command::new(exe);
        c.args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // NORMAL priority, hidden console. Idle was for when the user is gaming.
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let o = c.output().map_err(|e| format!("{}: {e}", exe.display()))?;
        if !o.status.success() {
            return Err(format!("{} failed ({}): {}", exe.display(), o.status, String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")));
        }
        Ok(())
    }
}

/// Test double with rife-ncnn-vulkan's CLI contract: `-i in -o out -n T` writes T evenly spaced frames
/// `%08d.png` (1-based) where output k*(T-1)/(N-1) = input k. Interpolates by plain lerp (inputs are opaque).
pub struct MockRifeRunner;

fn arg<'a>(args: &'a [String], k: &str) -> Option<&'a str> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

impl Runner for MockRifeRunner {
    fn run(&self, _exe: &Path, args: &[String]) -> Result<()> {
        let (i, o) = (arg(args, "-i").ok_or("-i")?, arg(args, "-o").ok_or("-o")?);
        let t: usize = arg(args, "-n").ok_or("-n")?.parse().map_err(|_| "-n")?;
        let ins: Vec<RgbaImage> = pngs(Path::new(i)).iter().map(|p| load(p)).collect::<Result<_>>()?;
        let n = ins.len();
        if n < 2 || t < 2 {
            return Err("mock rife: need >= 2 frames".into());
        }
        for j in 0..t {
            let pos = j as f32 * (n - 1) as f32 / (t - 1) as f32;
            let (a, k) = (pos.floor() as usize, pos.fract());
            let img = if a + 1 >= n { ins[n - 1].clone() } else { lerp_rgb(&ins[a], &ins[a + 1], k) };
            save(&img, &Path::new(o).join(format!("{:08}.png", j + 1)))?;
        }
        Ok(())
    }
}

fn lerp_rgb(a: &RgbaImage, b: &RgbaImage, k: f32) -> RgbaImage {
    let mut o = a.clone();
    for (x, (pa, pb)) in o.pixels.iter_mut().zip(a.pixels.iter().zip(&b.pixels)) {
        *x = (*pa as f32 * (1.0 - k) + *pb as f32 * k).round() as u8;
    }
    o
}

pub fn pngs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn load(p: &Path) -> Result<RgbaImage> {
    RgbaImage::from_png(&std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| e.to_string())
}

fn save(img: &RgbaImage, p: &Path) -> Result<()> {
    img.save_png(p).map_err(|e| e.to_string())
}

fn save_rgb(img: &RgbaImage, p: &Path) -> Result<()> {
    img.save_rgb_png(p).map_err(|e| e.to_string())
}

/// RIFE (rife-ncnn-vulkan). RIFE has no alpha: each frame is split into premultiplied RGB-on-black and an
/// alpha mask, both are interpolated, then merged (un-premultiplied). Frame 0 is appended so the loop seam
/// (last -> first) gets in-betweens too; that duplicate is dropped afterwards.
pub struct Rife<R: Runner> {
    pub exe: PathBuf,
    pub model_dir: PathBuf,
    pub gpu: i32,
    pub work: PathBuf,
    pub runner: R,
    pub version: String,
}

static JOB: AtomicU64 = AtomicU64::new(0);

impl<R: Runner> Backend for Rife<R> {
    fn id(&self) -> String {
        let model = self.model_dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        format!("{}/{model}", self.version)
    }
    fn interpolate(&self, frames: &[RgbaImage], factor: u32) -> Result<Vec<RgbaImage>> {
        let n = frames.len();
        let w = self.work.join(format!("job{}-{}", std::process::id(), JOB.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&w);
        for d in ["rgb", "alpha", "rgb_out", "alpha_out"] {
            std::fs::create_dir_all(w.join(d)).map_err(|e| e.to_string())?;
        }
        for (k, img) in frames.iter().chain(std::iter::once(&frames[0])).enumerate() {
            let (mut rgb, mut al) = (RgbaImage::new(img.width, img.height), RgbaImage::new(img.width, img.height));
            for i in 0..(img.width * img.height) as usize {
                let p = &img.pixels[i * 4..i * 4 + 4];
                let m = p[3] as f32 / 255.0;
                rgb.pixels[i * 4..i * 4 + 4].copy_from_slice(&[(p[0] as f32 * m).round() as u8, (p[1] as f32 * m).round() as u8, (p[2] as f32 * m).round() as u8, 255]);
                al.pixels[i * 4..i * 4 + 4].copy_from_slice(&[p[3], p[3], p[3], 255]);
            }
            // 24-bit RGB: rife-ncnn-vulkan treats 32-bit RGBA as packed RGB (stride w*3 vs w*4).
            save_rgb(&rgb, &w.join("rgb").join(format!("{k:08}.png")))?;
            save_rgb(&al, &w.join("alpha").join(format!("{k:08}.png")))?;
        }
        let target = (n * factor as usize + 1).to_string();
        for c in ["rgb", "alpha"] {
            let args: Vec<String> = [
                "-i", &w.join(c).to_string_lossy(), "-o", &w.join(format!("{c}_out")).to_string_lossy(), "-n", &target,
                "-m", &self.model_dir.to_string_lossy(), "-g", &self.gpu.to_string(), "-j", "1:2:2",
            ].iter().map(|s| s.to_string()).collect();
            self.runner.run(&self.exe, &args)?;
        }
        let (rgb, al) = (pngs(&w.join("rgb_out")), pngs(&w.join("alpha_out")));
        if rgb.len() != n * factor as usize + 1 || al.len() != rgb.len() {
            return Err(format!("rife produced {} / {} frames, expected {}", rgb.len(), al.len(), n * factor as usize + 1));
        }
        let mut out = Vec::new();
        for k in 0..rgb.len() - 1 {
            let (c, m) = (load(&rgb[k])?, load(&al[k])?);
            let mut o = RgbaImage::new(c.width, c.height);
            for i in 0..(c.width * c.height) as usize {
                let a = m.pixels[i * 4];
                let f = if a == 0 { 0.0 } else { 255.0 / a as f32 };
                let px = [0, 1, 2].map(|j| (c.pixels[i * 4 + j] as f32 * f).round().clamp(0.0, 255.0) as u8);
                o.pixels[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], if a < 8 { 0 } else { a }]);
            }
            out.push(o);
        }
        let _ = std::fs::remove_dir_all(&w);
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn frames() -> Vec<RgbaImage> {
        // a 1-px dot moving right on a transparent 8x4 canvas
        (0..3).map(|k| {
            let mut i = RgbaImage::new(8, 4);
            i.put(k * 2, 1, [200, 100, 50, 255]);
            i
        }).collect()
    }

    #[test]
    fn crossfade_counts_and_keeps_originals() {
        let f = frames();
        let o = CpuCrossfade.interpolate(&f, 4).unwrap();
        assert_eq!(o.len(), 12);
        assert_eq!(o[4].pixels, f[1].pixels);
        // half-way blend has half alpha at both dots, colour un-darkened
        let mid = &o[2];
        let px = |x: u32| &mid.pixels[((1 * 8 + x) * 4) as usize..((1 * 8 + x) * 4 + 4) as usize];
        assert_eq!(px(0)[3], 128);
        assert_eq!(&px(0)[..3], &[200, 100, 50]);
    }

    #[test]
    fn rife_split_merge_with_mock() {
        let f = frames();
        let work = std::env::temp_dir().join(format!("np-rife-mock-{}", std::process::id()));
        let r = Rife { exe: "rife-ncnn-vulkan.exe".into(), model_dir: "x/rife-v4.6".into(), gpu: 0, work: work.clone(), runner: MockRifeRunner, version: "mock".into() };
        assert_eq!(r.id(), "mock/rife-v4.6");
        let o = r.interpolate(&f, 2).unwrap();
        assert_eq!(o.len(), 6);
        for k in 0..3 {
            assert_eq!(o[k * 2].pixels, f[k].pixels, "original {k} survives split/merge");
        }
        // loop seam: frame 5 is between last (dot at x=4) and first (x=0)
        let a = |img: &RgbaImage, x: u32| img.pixels[((8 + x) * 4 + 3) as usize];
        assert!(a(&o[5], 4) > 0 && a(&o[5], 0) > 0);
        assert!(std::fs::read_dir(&work).map(|d| d.count()).unwrap_or(0) == 0, "job dir cleaned");
        let _ = std::fs::remove_dir_all(&work);
    }
}