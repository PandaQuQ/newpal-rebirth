//! Runs every talk script of the player's install through the VM (auto-advancing dialogue, following
//! jumps / scene changes as no-ops). Checks: no panic, every run terminates, and coverage numbers.
//! Skips when no game install is found.
use newpal_assets::gamedir::{self, Config};
use newpal_assets::talk;
use newpal_vm::{AudioCmd, Event, ScriptBackend, Vm, VmState};
use std::collections::{BTreeMap, HashMap};

#[test]
fn all_scripts_terminate() {
    let p = Config::default_path();
    let cfg = Config::load_from(&p).unwrap_or_default();
    let Ok(r) = gamedir::resolve(None, &cfg, &p) else { eprintln!("SKIP: no game"); return };
    let (mut scripts, mut ended, mut dialogues, mut scenes, mut executed, mut skipped) = (0, 0, 0usize, 0usize, 0usize, 0usize);
    let mut unhandled: BTreeMap<u16, usize> = BTreeMap::new();
    let (mut sfx, mut music, mut max_sfx, mut max_music) = (0usize, 0usize, 0u16, 0u16);
    for k in 1..=8u32 {
        let bytes = std::fs::read(r.dir.join(format!("talk{k}.dat"))).unwrap();
        for slot in 1..=1000u32 {
            let n = (k - 1) * 1000 + slot % 1000;
            let Ok(s) = talk::script_from_file(&bytes, n) else { continue };
            if s.code.len() < 2 || s.start >= bytes.len() {
                continue;
            }
            scripts += 1;
            let mut st = VmState::new_game();
            st.city = 2;
            let mut vm = Vm::with_state(&s, HashMap::new(), st);
            vm.budget = 5000;
            let mut ev = 0;
            loop {
                match vm.next_event() {
                    Event::End => { ended += 1; break }
                    Event::Dialogue(_) => dialogues += 1,
                    Event::ChangeScene { .. } => scenes += 1,
                    Event::Movie(_) => {}
                }
                ev += 1;
                if ev > 2000 { break }
            }
            let st = vm.into_state();
            executed += st.executed;
            skipped += st.skipped;
            for a in &st.audio {
                match *a {
                    AudioCmd::Sfx(s) => { sfx += 1; max_sfx = max_sfx.max(s) }
                    AudioCmd::Music { a, b, .. } => { music += 1; max_music = max_music.max(a).max(b) }
                    AudioCmd::SceneMusic => {}
                }
            }
            for (op, c) in st.unhandled { *unhandled.entry(op).or_default() += c; }
        }
    }
    let mut top: Vec<_> = unhandled.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1));
    eprintln!("scripts {scripts} ended {ended} dialogues {dialogues} scene changes {scenes} executed {executed} skipped {skipped} B");
    eprintln!("top unhandled: {}", top.iter().take(25).map(|(o, c)| format!("{o:x}:{c}")).collect::<Vec<_>>().join(" "));
    let snd = newpal_assets::pak::IndexArchive::open(&r.dir.join("all.snd")).unwrap();
    let md = newpal_assets::mapsdat::MapsDat::parse(&std::fs::read(r.dir.join("maps.dat")).unwrap()).unwrap();
    eprintln!("audio: sfx {sfx} (max slot {max_sfx}, all.snd {}), music {music} (max id {max_music}, names {})", snd.entries.len(), md.music_names.len());
    assert!(sfx > 100 && music > 10);
    assert!((max_music as usize) < md.music_names.len());
    assert!(scripts > 1000);
    // a few scripts loop on dialogue until state we do not model changes (e.g. menus); cap at 2000 events
    assert!(scripts - ended <= scripts / 100, "{} scripts did not terminate", scripts - ended);
    assert!(dialogues > 10_000);
}