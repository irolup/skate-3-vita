//! The reference's game audio host (`skate-game/src/game_audio`, `b3c9679`) without Bevy: what
//! feeds `skate_audio`'s native runtime from the skater's audio publication
//! (`skate_sim::AudioRiding`). See PROVENANCE.md.
mod ambience;
mod crossfade_groups;
mod emitters;
mod prefetch;
pub use prefetch::set_thread_init as set_bank_worker_init;
pub mod frontend_marker;
mod frontend;
pub mod library;
mod mod_audio;
mod mod_rules;
pub mod player_audio;
pub mod grain_bed;
pub mod native;

/// skate_vita: the reference's mod system is not ported (no mods on the Vita); the library's
/// mod-file paths never occur.
pub(crate) mod skate_mods {
    pub mod audio_merge {
        /// `mod:<id>/<path>` → (id, path); no file is a mod file here.
        pub fn split_mod_ref(_file: &str) -> Option<(&str, &str)> {
            None
        }
    }
}

/// skate_vita: the reference's world sources (traffic, pedestrians) are not ported yet; their
/// manifest tuning section stays raw JSON.
pub(crate) mod world_sources {
    pub type WorldTuningJson = serde_json::Value;
}

#[cfg(test)]
mod pack_tests {
    use crate::library::Library;
    use std::path::Path;

    /// The library reads the baked `sound.pak` through
    /// `skate_data::vfs` and every board bank, grain, Splice bank, project and the MixMap load.
    /// `SKATE_SOUND_PAK=…/SKATEVITA/sound.pak cargo test -p game-audio -- --ignored --nocapture`
    #[test]
    #[ignore = "requires SKATE_SOUND_PAK (the user's baked sound.pak)"]
    fn the_baked_pack_loads() {
        let pak = std::env::var_os("SKATE_SOUND_PAK").expect("set SKATE_SOUND_PAK");
        let started = std::time::Instant::now();
        let bytes = std::fs::read(&pak).unwrap();
        let root = Path::new("vfs:/assets");
        for (path, data) in vita_pack::files::from_pack(&bytes).unwrap() {
            skate_data::vfs::mount(root.join(path), data);
        }
        let mounted = started.elapsed();
        let library = Library::load(root).unwrap();
        let aems = library.aems().clone();
        let mut samples = 0;
        let mut adpcm_bytes = 0usize;
        for stem in aems.banks.keys() {
            let pcm = library.bank_pcm(stem);
            for p in pcm.iter().flatten() {
                samples += 1;
                for c in &p.channels {
                    if let skate_audio::adpcm::Samples::Adpcm(a) = c {
                        adpcm_bytes += a.blocks().len();
                    }
                }
            }
            library.bank_source(stem).unwrap().load().unwrap();
        }
        for stem in aems.splice.keys() {
            let (_, pcm) = library.splice_bank(stem).unwrap_or_else(|| panic!("splice {stem}"));
            samples += pcm.iter().flatten().count();
        }
        for file in &aems.projects {
            let bytes = library.read(file).unwrap();
            skate_audio::formats::Project::parse(file, &bytes).unwrap();
        }
        let mixmap = library.read(aems.mixmap.as_ref().unwrap()).unwrap();
        skate_audio::mixmap::MixMapFile::parse(&mixmap).unwrap();
        assert!(library.grain_default_tuning().is_some() || !library.surface_map().is_empty());
        eprintln!(
            "sound.pak: mounted in {:.2} s; {} banks, {samples} samples ({:.1} MiB ADPCM), {} splice banks, {} projects; loaded in {:.2} s",
            mounted.as_secs_f32(),
            aems.banks.len(),
            adpcm_bytes as f64 / 1048576.0,
            aems.splice.len(),
            aems.projects.len(),
            started.elapsed().as_secs_f32()
        );
        assert!(samples > 2000);
    }

    /// The session marker's records through the real runtime from the baked pack: the Splice
    /// sounds they start are the reference's (its test `session_marker_events_start_retails_sounds`:
    /// activate 235, place 237, go to 236).
    #[test]
    #[ignore = "requires SKATE_SOUND_PAK (the user's baked sound.pak)"]
    fn the_marker_sounds_start_retails_splice_sounds() {
        use crate::frontend_marker::marker_record;
        let pak = std::env::var_os("SKATE_SOUND_PAK").expect("set SKATE_SOUND_PAK");
        let bytes = std::fs::read(&pak).unwrap();
        let root = Path::new("vfs:/assets");
        for (path, data) in vita_pack::files::from_pack(&bytes).unwrap() {
            skate_data::vfs::mount(root.join(path), data);
        }
        let library = Library::load(root).unwrap();
        let mut native = crate::native::Native::start(&library).unwrap();
        assert!(native.has_frontend(), "the pack has the front-end records and sk8_menu");
        let keys: Vec<u64> = [skate_sim::AUDIO_OPEN, skate_sim::AUDIO_SET, skate_sim::AUDIO_RETURN].into_iter().filter_map(marker_record).collect();
        let riding = skate_sim::AudioRiding::default();
        let mut block = [0.0f32; 1600];
        let mut peak = 0.0f32;
        for tick in 0..30 {
            let requests: &[u64] = if tick == 0 { &keys } else { &[] };
            native.frame(&library, crate::native::FrameInput {
                riding: &riding, steps: 1, listener: None, cuts: (0, 0), overstep: 0.0, game_dt: 1.0 / 60.0, real_dt: 1.0 / 60.0, silenced: false,
                frontend: requests, teleport: None, map: None,
            });
            native.render(&mut block);
            peak = block.iter().fold(peak, |m, x| m.max(x.abs()));
        }
        let started: Vec<u32> = native.frontend.as_ref().unwrap().started.iter().map(|&(_, id)| id).collect();
        eprintln!("front-end Splice sounds started: {started:?}, peak {peak:.3}");
        assert!(started.contains(&235) && started.contains(&237) && started.contains(&236), "{started:?}");
        assert!(peak > 0.01, "the sounds are audible (peak {peak})");
    }

    /// The zone beds from the baked packs: standing in University's mountain zone the bed fades in;
    /// walking into the campus the mountain bed fades out and the campus bed fades in (the zones'
    /// 2.5 s fades), the level following the MixMap's output, with the zone crossfade's voices
    /// playing during the change. `SKATE_AMBIENCE_PAK` = ambience.pak, `SKATE_WORLD_PAK` = world.pak.
    #[test]
    #[ignore = "requires SKATE_SOUND_PAK, SKATE_AMBIENCE_PAK and SKATE_WORLD_PAK (the user's baked packs)"]
    fn the_beds_follow_the_zones() {
        let mount = |var: &str| {
            let bytes = std::fs::read(std::env::var_os(var).unwrap_or_else(|| panic!("set {var}"))).unwrap();
            for (path, data) in vita_pack::files::from_pack(&bytes).unwrap() {
                skate_data::vfs::mount(Path::new("vfs:/assets").join(path), data);
            }
        };
        mount("SKATE_SOUND_PAK");
        mount("SKATE_AMBIENCE_PAK");
        mount("SKATE_WORLD_PAK");
        let library = Library::load(Path::new("vfs:/assets")).unwrap();
        let mut native = crate::native::Native::start(&library).unwrap();
        let mut block = [0.0f32; 1600];
        let mut history: Vec<(f32, Option<String>, f32)> = Vec::new();
        let mut crossfades: Vec<(f32, usize)> = Vec::new();
        // (x, z) of the skater by time: mountain zone for 4 s, then the campus.
        for tick in 0..720 {
            let t = tick as f32 / 60.0;
            let (x, z) = if t < 4.0 { (-350.0, -1058.0) } else { (162.0, 34.0) };
            let riding = skate_sim::AudioRiding { board: glam::Vec3::new(x, 0.0, z), ..Default::default() };
            native.frame(&library, crate::native::FrameInput {
                riding: &riding, steps: 1, listener: None, cuts: (0, 0), overstep: 0.0, game_dt: 1.0 / 60.0, real_dt: 1.0 / 60.0, silenced: false,
                frontend: &[], teleport: None, map: Some(("University", 1)),
            });
            native.render(&mut block);
            if tick % 60 == 59 {
                let rms = (block.iter().map(|x| x * x).sum::<f32>() / block.len() as f32).sqrt();
                history.push((t, native.bed_playing().map(str::to_owned), rms));
                crossfades.push((t, native.crossfade_playing()));
            }
        }
        for (t, bed, rms) in &history {
            eprintln!("t {t:4.1} s  bed {bed:?}  rms {rms:.4}");
        }
        eprintln!("crossfade voices by second: {:?}", crossfades.iter().map(|c| c.1).collect::<Vec<_>>());
        let at = |s: f32| history.iter().find(|(t, ..)| *t >= s).unwrap();
        assert_eq!(at(3.0).1.as_deref(), Some("07_univ_mt_high"));
        assert!(at(3.0).2 > 0.002, "the mountain bed is audible once faded in");
        assert_eq!(at(11.0).1.as_deref(), Some("09_univ_campus"), "the campus bed after the fade out and in");
        assert!(at(11.0).2 > 0.002, "the campus bed is audible");
        assert!(crossfades.iter().any(|c| c.1 == 4), "the zone crossfade plays its four voices: {crossfades:?}");
        assert_eq!(crossfades.last().map(|c| c.1), Some(0), "and stops when the new bed has faded in");
    }

    /// The sound emitters from the baked packs: standing inside University's `industrial_gen_loop_3`
    /// emitter the bank is read ahead on the worker, the emitter starts and sounds; walking far away
    /// stops it and the bank is unloaded again. `SKATE_WORLD_PAK` = world.pak.
    #[test]
    #[ignore = "requires SKATE_SOUND_PAK and SKATE_WORLD_PAK (the user's baked packs)"]
    fn the_emitters_load_by_proximity_and_sound() {
        let mount = |var: &str| {
            let bytes = std::fs::read(std::env::var_os(var).unwrap_or_else(|| panic!("set {var}"))).unwrap();
            for (path, data) in vita_pack::files::from_pack(&bytes).unwrap() {
                skate_data::vfs::mount(Path::new("vfs:/assets").join(path), data);
            }
        };
        mount("SKATE_SOUND_PAK");
        mount("SKATE_WORLD_PAK");
        let library = Library::load(Path::new("vfs:/assets")).unwrap();
        let mut native = crate::native::Native::start(&library).unwrap();
        let mut block = [0.0f32; 1600];
        let (inside, far) = (glam::Vec3::new(224.6, 82.7, -351.3), glam::Vec3::new(-600.0, 30.0, 600.0));
        let mut log: Vec<(f32, usize, bool, f32)> = Vec::new();
        for tick in 0..900 {
            let t = tick as f32 / 60.0;
            let at = if t < 8.0 { inside } else { far };
            let riding = skate_sim::AudioRiding { board: at, ..Default::default() };
            let listener = crate::native::Listener { position: at, forward: glam::Vec3::NEG_Z };
            native.frame(&library, crate::native::FrameInput {
                riding: &riding, steps: 1, listener: Some(listener), cuts: (0, 0), overstep: 0.0, game_dt: 1.0 / 60.0, real_dt: 1.0 / 60.0, silenced: false,
                frontend: &[], teleport: None, map: Some(("University", 1)),
            });
            native.render(&mut block);
            if tick % 60 == 59 {
                let rms = (block.iter().map(|x| x * x).sum::<f32>() / block.len() as f32).sqrt();
                log.push((t, native.emitters_playing(), native.bank_loaded("industrial_gen_loop_3"), rms));
            }
            // The worker reads the bank while the game runs.
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        for (t, n, loaded, rms) in &log {
            eprintln!("t {t:4.1} s  emitters {n}  bank loaded {loaded}  rms {rms:.4}");
        }
        let at = |s: f32| log.iter().find(|(t, ..)| *t >= s).unwrap();
        assert!(at(6.0).1 >= 1 && at(6.0).2, "an emitter started and its bank is loaded");
        assert!(at(6.0).3 > 0.0005, "the emitter is audible");
        assert_eq!(at(14.0).1, 0, "far away nothing plays");
        assert!(!at(14.0).2, "and the bank is unloaded again");
    }

    /// The reverb zones from the baked sound pack: standing inside University's large reverb zone the
    /// env network switches to a preset other than reverb01.
    #[test]
    #[ignore = "requires SKATE_SOUND_PAK (the user's baked sound.pak)"]
    fn a_reverb_zone_changes_the_preset() {
        let bytes = std::fs::read(std::env::var_os("SKATE_SOUND_PAK").expect("set SKATE_SOUND_PAK")).unwrap();
        for (path, data) in vita_pack::files::from_pack(&bytes).unwrap() {
            skate_data::vfs::mount(Path::new("vfs:/assets").join(path), data);
        }
        let library = Library::load(Path::new("vfs:/assets")).unwrap();
        let mut native = crate::native::Native::start(&library).unwrap();
        let mut block = [0.0f32; 1600];
        let (inside, outside) = (glam::Vec3::new(451.8, 66.7, -57.3), glam::Vec3::new(-900.0, 30.0, 900.0));
        let mut keys: Vec<(f32, Option<u64>)> = Vec::new();
        for tick in 0..600 {
            let t = tick as f32 / 60.0;
            let at = if t < 5.0 { inside } else { outside };
            let riding = skate_sim::AudioRiding { board: at, ..Default::default() };
            let listener = crate::native::Listener { position: at, forward: glam::Vec3::NEG_Z };
            native.frame(&library, crate::native::FrameInput {
                riding: &riding, steps: 1, listener: Some(listener), cuts: (0, 0), overstep: 0.0, game_dt: 1.0 / 60.0, real_dt: 1.0 / 60.0, silenced: false,
                frontend: &[], teleport: None, map: Some(("University", 1)),
            });
            native.render(&mut block);
            if tick % 60 == 59 {
                keys.push((t, native.shared.lock().unwrap().mixer.buses.env.key()));
            }
        }
        for (t, key) in &keys {
            eprintln!("t {t:4.1} s  reverb preset {key:016X?}");
        }
        let at = |s: f32| keys.iter().find(|(t, ..)| *t >= s).unwrap().1;
        assert_ne!(at(4.0), Some(skate_audio::bus::env::DEFAULT_PRESET), "inside the zone: its preset");
        assert_eq!(at(9.0), Some(skate_audio::bus::env::DEFAULT_PRESET), "outside every zone: reverb01 again");
    }
}
