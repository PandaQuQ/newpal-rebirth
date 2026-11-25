//! Audio: background music (maps.dat music table -> `Music/*.mp3`, mod-overridable) and sound effects
//! (`all.snd`, 1-based ids). Playback uses rodio; without an output device (headless, `--mute`, tests)
//! every request is still resolved and decoded-checked, then logged instead of played.
use newpal_assets::vfs::Vfs;
use newpal_vm::AudioCmd;
use rodio::{Decoder, Source};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

#[derive(Clone)]
struct Bytes(Arc<Vec<u8>>);
impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

fn decoder(b: &Arc<Vec<u8>>) -> Result<Decoder<Cursor<Bytes>>, String> {
    Decoder::new(Cursor::new(Bytes(b.clone()))).map_err(|e| e.to_string())
}

#[allow(dead_code)] // used by tests / tools
/// Decode a sound fully without a device: (channels, sample rate, samples).
pub fn probe(bytes: Vec<u8>) -> Result<(u16, u32, usize), String> {
    let d = decoder(&Arc::new(bytes))?;
    let (ch, sr) = (d.channels().get(), d.sample_rate().get());
    Ok((ch, sr, d.count()))
}

struct Bgm {
    name: String,
    bytes: Arc<Vec<u8>>,
    looping: bool,
}

pub struct Audio {
    device: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
    bgm: Option<Bgm>,
    /// music that belongs to the current scene (op 0x89 returns to it)
    scene_music: Option<String>,
    music_names: Option<Arc<Vec<String>>>,
    sfx_cache: HashMap<u16, Option<Arc<Vec<u8>>>>,
    pub volume: f32,
    /// what was requested (newest last) - for headless evidence and debugging
    pub log: Vec<String>,
}

impl Audio {
    /// No device: requests are resolved and logged only.
    pub fn silent() -> Self {
        Self { device: None, player: None, bgm: None, scene_music: None, music_names: None, sfx_cache: HashMap::new(), volume: 0.8, log: Vec::new() }
    }

    /// Default output device; falls back to [Audio::silent] if none can be opened.
    pub fn open() -> Self {
        let mut a = Self::silent();
        match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut d) => {
                d.log_on_drop(false);
                a.device = Some(d);
            }
            Err(e) => a.log.push(format!("no audio device: {e}")),
        }
        a
    }

    #[allow(dead_code)]
    pub fn playing(&self) -> Option<&str> {
        self.bgm.as_ref().map(|b| b.name.as_str())
    }

    fn names(&mut self, vfs: &Vfs) -> Arc<Vec<String>> {
        self.music_names
            .get_or_insert_with(|| Arc::new(vfs.maps_dat().map(|m| m.music_names).unwrap_or_default()))
            .clone()
    }

    /// Scene music: maps.dat city -> track a. Same track keeps playing (like the original scene-music player).
    pub fn enter_scene(&mut self, vfs: &Vfs, city: Option<u16>) {
        let name = city.and_then(|c| vfs.maps_dat().ok()?.music_for(c).and_then(|(a, _, _)| a.map(str::to_string)));
        self.scene_music = name.clone();
        match name {
            Some(n) => self.play_music(vfs, &n, true),
            None => self.stop_music(),
        }
    }

    pub fn play_music(&mut self, vfs: &Vfs, name: &str, looping: bool) {
        if self.bgm.as_ref().is_some_and(|b| b.name.eq_ignore_ascii_case(name)) {
            return;
        }
        self.stop_music();
        match vfs.music(name) {
            Ok(Some((bytes, _ext))) => {
                let bytes = Arc::new(bytes);
                self.log.push(format!("bgm {name} ({} KB){}", bytes.len() / 1024, if looping { " loop" } else { "" }));
                if let Some(dev) = &self.device {
                    let p = rodio::Player::connect_new(dev.mixer());
                    p.set_volume(self.volume);
                    if let Ok(d) = decoder(&bytes) {
                        p.append(d);
                    }
                    self.player = Some(p);
                }
                self.bgm = Some(Bgm { name: name.to_string(), bytes, looping });
            }
            Ok(None) => self.log.push(format!("bgm {name}: not found")),
            Err(e) => self.log.push(format!("bgm {name}: {e}")),
        }
    }

    pub fn stop_music(&mut self) {
        if let Some(p) = self.player.take() {
            p.stop();
        }
        self.bgm = None;
    }

    pub fn play_sfx(&mut self, vfs: &Vfs, id: u16) {
        let entry = self.sfx_cache.entry(id).or_insert_with(|| vfs.sfx(id).ok().flatten().map(Arc::new)).clone();
        let Some(bytes) = entry else {
            self.log.push(format!("sfx {id}: none"));
            return;
        };
        self.log.push(format!("sfx {id} ({} B)", bytes.len()));
        if let Some(dev) = &self.device {
            if let Ok(d) = decoder(&bytes) {
                dev.mixer().add(d.amplify(self.volume));
            }
        }
    }

    /// Apply script requests.
    pub fn run(&mut self, vfs: &Vfs, cmds: Vec<AudioCmd>) {
        for c in cmds {
            match c {
                AudioCmd::Sfx(id) => self.play_sfx(vfs, id),
                AudioCmd::Music { a, mode, .. } => {
                    let names = self.names(vfs);
                    match names.get(a as usize) {
                        Some(n) => self.play_music(vfs, &n.clone(), mode != 2),
                        None => self.log.push(format!("music id {a}: unknown")),
                    }
                }
                AudioCmd::SceneMusic => match self.scene_music.clone() {
                    Some(n) => self.play_music(vfs, &n, true),
                    None => self.stop_music(),
                },
            }
        }
    }

    /// Per frame: restart looping music when it ran out.
    pub fn tick(&mut self) {
        let (Some(p), Some(b)) = (&self.player, &self.bgm) else { return };
        if b.looping && p.empty() {
            if let Ok(d) = decoder(&b.bytes) {
                p.append(d);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use newpal_assets::gamedir::{self, Config};

    fn vfs() -> Option<Vfs> {
        let p = Config::default_path();
        let cfg = Config::load_from(&p).unwrap_or_default();
        let v = gamedir::resolve(None, &cfg, &p).ok().map(|r| Vfs::new(r.dir, Vec::new()));
        if v.is_none() { eprintln!("SKIP: no game") }
        v
    }

    /// Decode only (no device): inn BGM and one all.snd effect.
    #[test]
    fn decode_scene_music_and_sfx() {
        let Some(v) = vfs() else { return };
        let md = v.maps_dat().unwrap();
        let (bytes, _) = v.music(md.music_for(2).unwrap().0.unwrap()).unwrap().unwrap();
        let (ch, sr, n) = probe(bytes).unwrap();
        eprintln!("bgm: {ch} ch {sr} Hz {n} samples");
        assert!(ch >= 1 && sr >= 8000 && n > sr as usize, "{ch} {sr} {n}");
        let (ch, sr, n) = probe(v.sfx(1).unwrap().unwrap()).unwrap();
        assert!(ch >= 1 && sr >= 8000 && n > 0, "{ch} {sr} {n}");
    }

    #[test]
    fn silent_audio_follows_scene_and_script() {
        let Some(v) = vfs() else { return };
        let mut a = Audio::silent();
        a.enter_scene(&v, Some(2));
        assert_eq!(a.playing(), Some("M027.mp3"));
        a.enter_scene(&v, Some(1)); // same track: keeps playing, no new log line
        assert_eq!(a.log.len(), 1);
        a.run(&v, vec![AudioCmd::Music { a: 3, b: 3, mode: 2 }, AudioCmd::Sfx(1)]);
        assert_eq!(a.playing(), Some("M005.wav"));
        a.run(&v, vec![AudioCmd::SceneMusic]);
        assert_eq!(a.playing(), Some("M027.mp3"));
    }
}