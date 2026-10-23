//! Saved settings: `ux0:data/SKATEVITA/settings.bin`, a small versioned
//! little-endian record. Unknown or damaged files fall back to the defaults.
use vita_platform::controls::{self, StickGain};

pub const PATH: &str = "ux0:data/SKATEVITA/settings.bin";
const MAGIC: &[u8; 8] = b"SKVSET09";
/// Version 8: the same record; its pedestrians byte read as the new default (Off), since version 8
/// defaulted to Low, which DownTown's memory cannot hold beside traffic and NPC skaters.
const MAGIC_V8: &[u8; 8] = b"SKVSET08";
/// Version 7 (before the pedestrians setting): the same record without its byte.
const MAGIC_V7: &[u8; 8] = b"SKVSET07";
/// Version 6 (before the AI skaters setting): the same record without its byte.
const MAGIC_V6: &[u8; 8] = b"SKVSET06";
/// Version 5 (before the traffic setting): the same record without the traffic byte.
const MAGIC_V5: &[u8; 8] = b"SKVSET05";
/// Version 4 (before the audio setting): the same record without the audio byte.
const MAGIC_V4: &[u8; 8] = b"SKVSET04";
const MAGIC_V3: &[u8; 8] = b"SKVSET03";
/// Version 2 (before the manual meter): the same record without the meter byte.
const MAGIC_V2: &[u8; 8] = b"SKVSET02";
/// Version 1 (before the retail menus): the same record without the menu-style byte.
const MAGIC_V1: &[u8; 8] = b"SKVSET01";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hardcore,
}
impl Difficulty {
    /// The stock physics mode key (skate-sim `difficulty.rs`).
    pub fn key(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Normal => "normal",
            Difficulty::Hardcore => "hardcore",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Normal => "Normal",
            Difficulty::Hardcore => "Hardcore",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Difficulty::Easy => Difficulty::Normal,
            Difficulty::Normal => Difficulty::Hardcore,
            Difficulty::Hardcore => Difficulty::Easy,
        }
    }
    fn code(self) -> u8 {
        self as u8
    }
    fn from_code(c: u8) -> Self {
        match c {
            0 => Difficulty::Easy,
            2 => Difficulty::Hardcore,
            _ => Difficulty::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub difficulty: Difficulty,
    pub controls: controls::Settings,
    /// Master volume 0..=10.
    pub volume: u8,
    /// FPS / tick-time overlay.
    pub debug_hud: bool,
    /// Last selected world (index into `packs::WORLDS`).
    pub world: u8,
    /// The retail crossbar for the main and pause menus (the plain menus when off or when
    /// the retail UI is unavailable); toggled with Select + Triangle on any menu.
    pub retail_menus: bool,
    /// The retail manual meter (the game's own crossbar has "Manual meter: On/Off"; off by default).
    pub manual_meter: bool,
    /// The retail trick analyzer (Learn > On Screen Help > Enable Trick Analyzer).
    pub trick_analyzer: bool,
    /// "Fast" physics: 25 solver iterations instead of retail's 50 (the Vita cannot run
    /// 50 within the 60 Hz tick everywhere; see `Session::set_solver_iterations`).
    pub fast_physics: bool,
    /// The retail board audio (sound.pak) instead of the procedural sounds. Without sound.pak
    /// the procedural sounds play whatever this says.
    pub retail_audio: bool,
    /// The traffic density, an index into `TRAFFIC` (Off, Low, Medium, Retail).
    pub traffic: u8,
    /// The ambient NPC skaters (retail three).
    pub ai_skaters: bool,
    /// The pedestrian density, an index into `PEDESTRIANS` (Off, Low, Medium, Retail).
    pub pedestrians: u8,
    /// The zombie cheat (Extras > Enter Cheat Code `zombie`): for the session only, never saved (retail keeps it on
    /// the simulation, not in the profile, b304).
    pub zombie: bool,
}

/// The Pedestrians setting's choices: (label, density), retail's Free Play scale semantics
/// (the census area's cap x density; retail allows 31 peds).
pub const PEDESTRIANS: [(&str, f32); 4] = [("Off", 0.0), ("Low", 0.25), ("Medium", 0.5), ("Retail", 1.0)];
/// Off on the Vita: peds cost about 0.5 ms a tick each and 25-30 MB of data, and DownTown with
/// traffic and NPC skaters ran out of heap with them on the device; switchable in WORLD.
const PEDESTRIANS_DEFAULT: u8 = 0;
/// The peds at most under the zombie cheat (retail's pool is 31). NOT RETAIL: the Vita's frame time with angry peds
/// (device-tuned).
const ZOMBIE_PEDS: u32 = 8;

/// The Traffic setting's choices: (label, density). The density scales each census area's
/// car cap as retail's Free Play traffic scale does (DownTown's is 30; at most 15 cars live).
pub const TRAFFIC: [(&str, f32); 4] = [("Off", 0.0), ("Low", 0.2), ("Medium", 0.4), ("Retail", 1.0)];
/// Medium: the Vita's tick is at its budget with 10-15 cars in DownTown.
const TRAFFIC_DEFAULT: u8 = 2;

impl Settings {
    /// The solve's constraint iterations for the physics setting.
    pub fn solver_iterations(&self) -> u32 {
        if self.fast_physics { 25 } else { 50 }
    }

    /// The living world's settings from the menu (traffic on with its density; peds and NPC
    /// skaters are not ported yet).
    pub fn living(&self) -> skate_sim::LivingWorldSettings {
        let (_, density) = TRAFFIC[usize::from(self.traffic).min(TRAFFIC.len() - 1)];
        let mut living = skate_sim::LivingWorldSettings::default();
        living.vehicles = skate_sim::KindSetting { enabled: density > 0.0, density };
        living.skaters.enabled = self.ai_skaters;
        let (_, peds) = PEDESTRIANS[usize::from(self.pedestrians).min(PEDESTRIANS.len() - 1)];
        living.pedestrians = skate_sim::KindSetting { enabled: peds > 0.0, density: peds };
        // The zombie cheat: peds whatever the setting (retail ignores the density then), bounded by the Vita's pool;
        // the sim drops the traffic and the NPC skaters itself.
        if self.zombie {
            living.zombie = true;
            living.pedestrians.enabled = true;
            living.ped_pool = Some(ZOMBIE_PEDS);
        }
        living
    }

    pub fn pedestrians_label(&self) -> &'static str {
        PEDESTRIANS[usize::from(self.pedestrians).min(PEDESTRIANS.len() - 1)].0
    }

    pub fn traffic_label(&self) -> &'static str {
        TRAFFIC[usize::from(self.traffic).min(TRAFFIC.len() - 1)].0
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            difficulty: Difficulty::Normal,
            controls: controls::Settings::default(),
            volume: 8,
            debug_hud: false,
            world: 3, // BlackBoxPark
            retail_menus: true,
            manual_meter: false,
            trick_analyzer: false,
            fast_physics: true,
            retail_audio: true,
            traffic: TRAFFIC_DEFAULT,
            ai_skaters: true,
            pedestrians: PEDESTRIANS_DEFAULT,
            zombie: false,
        }
    }
}

impl Settings {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        let c = &self.controls;
        b.extend_from_slice(&[
            self.difficulty.code(),
            u8::from(c.rear_touch_bumpers),
            u8::from(c.dpad_bumpers),
            u8::from(c.front_touch_thumbs),
            self.volume,
            u8::from(self.debug_hud),
            self.world,
            u8::from(self.retail_menus),
            u8::from(self.manual_meter),
            u8::from(self.trick_analyzer),
            u8::from(self.fast_physics),
            u8::from(self.retail_audio),
            self.traffic,
            u8::from(self.ai_skaters),
            self.pedestrians,
        ]);
        for g in [c.left_gain.x, c.left_gain.y, c.right_gain.x, c.right_gain.y] {
            b.extend_from_slice(&g.to_le_bytes());
        }
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        // Version 2 has one more byte (the menu style) after the seven of version 1, version 3
        // two more (the manual meter, the trick analyzer), version 4 another (fast physics),
        // version 5 another (retail audio), version 6 another (traffic); the gains follow. Older
        // files read as the defaults.
        let v1 = b.len() == 8 + 7 + 16 && &b[..8] == MAGIC_V1;
        let v2 = b.len() == 8 + 8 + 16 && &b[..8] == MAGIC_V2;
        let v3 = b.len() == 8 + 10 + 16 && &b[..8] == MAGIC_V3;
        let v4 = b.len() == 8 + 11 + 16 && &b[..8] == MAGIC_V4;
        let v5 = b.len() == 8 + 12 + 16 && &b[..8] == MAGIC_V5;
        let v6 = b.len() == 8 + 13 + 16 && &b[..8] == MAGIC_V6;
        let v7 = b.len() == 8 + 14 + 16 && &b[..8] == MAGIC_V7;
        let v8 = b.len() == 8 + 15 + 16 && &b[..8] == MAGIC_V8;
        if !v1 && !v2 && !v3 && !v4 && !v5 && !v6 && !v7 && !v8 && (b.len() != 8 + 15 + 16 || &b[..8] != MAGIC) {
            return None;
        }
        let shift = if v1 { 0 } else if v2 { 1 } else if v3 { 3 } else if v4 { 4 } else if v5 { 5 } else if v6 { 6 } else if v7 { 7 } else { 8 };
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let gain = |v: f32| if v.is_finite() && (0.5..=2.0).contains(&v) { v } else { 1.0 };
        Some(Self {
            difficulty: Difficulty::from_code(b[8]),
            controls: controls::Settings {
                rear_touch_bumpers: b[9] != 0,
                dpad_bumpers: b[10] != 0,
                front_touch_thumbs: b[11] != 0,
                left_gain: StickGain { x: gain(f(15 + shift)), y: gain(f(19 + shift)) },
                right_gain: StickGain { x: gain(f(23 + shift)), y: gain(f(27 + shift)) },
            },
            volume: b[12].min(10),
            debug_hud: b[13] != 0,
            world: b[14],
            // Version 1 files carry no menu style: the default (retail).
            retail_menus: if v1 { true } else { b[15] != 0 },
            manual_meter: !v1 && !v2 && b[16] != 0,
            trick_analyzer: !v1 && !v2 && b[17] != 0,
            fast_physics: v1 || v2 || v3 || b[18] != 0,
            // Files from before the setting existed read as the default (retail).
            retail_audio: v1 || v2 || v3 || v4 || b[19] != 0,
            traffic: if v1 || v2 || v3 || v4 || v5 { TRAFFIC_DEFAULT } else { b[20].min(TRAFFIC.len() as u8 - 1) },
            ai_skaters: v1 || v2 || v3 || v4 || v5 || v6 || b[21] != 0,
            pedestrians: if v1 || v2 || v3 || v4 || v5 || v6 || v7 || v8 { PEDESTRIANS_DEFAULT } else { b[22].min(PEDESTRIANS.len() as u8 - 1) },
            zombie: false,
        })
    }

    pub fn load() -> Self {
        std::fs::read(PATH).ok().and_then(|b| Self::decode(&b)).unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all("ux0:data/SKATEVITA");
        if let Err(e) = std::fs::write(PATH, self.encode()) {
            println!("settings not saved: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_reject_damage() {
        let mut s = Settings::default();
        s.difficulty = Difficulty::Hardcore;
        s.controls.dpad_bumpers = true;
        s.controls.left_gain.y = 1.1;
        s.volume = 3;
        s.world = 6;
        s.retail_menus = false;
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        // A version 1 file (no menu-style byte) still loads, with the default for it.
        let mut v1 = b"SKVSET01".to_vec();
        v1.extend_from_slice(&s.encode()[8..8 + 7]);
        v1.extend_from_slice(&s.encode()[8 + 15..]);
        let loaded = Settings::decode(&v1).unwrap();
        assert!(loaded.retail_menus && !loaded.manual_meter);
        assert_eq!((loaded.volume, loaded.world, loaded.controls.left_gain.y), (3, 6, 1.1));
        // A version 2 file (no meter byte) loads with the meter off.
        let mut v2 = b"SKVSET02".to_vec();
        v2.extend_from_slice(&s.encode()[8..8 + 8]);
        v2.extend_from_slice(&s.encode()[8 + 15..]);
        let loaded = Settings::decode(&v2).unwrap();
        assert!(!loaded.retail_menus && !loaded.manual_meter);
        assert_eq!(loaded.controls.left_gain.y, 1.1);
        s.manual_meter = true;
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        // A version 3 file (no physics byte) loads with fast physics.
        let mut v3 = b"SKVSET03".to_vec();
        v3.extend_from_slice(&s.encode()[8..8 + 10]);
        v3.extend_from_slice(&s.encode()[8 + 15..]);
        let loaded = Settings::decode(&v3).unwrap();
        assert!(loaded.fast_physics && loaded.manual_meter);
        assert_eq!(loaded.controls.left_gain.y, 1.1);
        s.fast_physics = false;
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        // A version 4 file (no audio byte) loads with the default, the retail audio.
        s.retail_audio = false;
        let mut v4 = b"SKVSET04".to_vec();
        v4.extend_from_slice(&s.encode()[8..8 + 11]);
        v4.extend_from_slice(&s.encode()[8 + 15..]);
        let loaded = Settings::decode(&v4).unwrap();
        assert!(loaded.retail_audio && !loaded.fast_physics);
        assert_eq!(loaded.controls.left_gain.y, 1.1);
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        s.retail_audio = true;
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        // A version 5 file (no traffic byte) loads with the default density.
        s.traffic = 3;
        let mut v5 = b"SKVSET05".to_vec();
        v5.extend_from_slice(&s.encode()[8..8 + 12]);
        v5.extend_from_slice(&s.encode()[8 + 15..]);
        let loaded = Settings::decode(&v5).unwrap();
        assert_eq!((loaded.traffic, loaded.controls.left_gain.y), (TRAFFIC_DEFAULT, 1.1));
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        // A version 6 file (no AI skaters byte) loads with them on.
        s.ai_skaters = false;
        let mut v6 = b"SKVSET06".to_vec();
        v6.extend_from_slice(&s.encode()[8..8 + 13]);
        v6.extend_from_slice(&s.encode()[8 + 15..]);
        let loaded = Settings::decode(&v6).unwrap();
        assert!(loaded.ai_skaters && loaded.traffic == 3);
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        // A version 7 file (no pedestrians byte) loads with the default density.
        s.pedestrians = 3;
        let mut v7 = b"SKVSET07".to_vec();
        v7.extend_from_slice(&s.encode()[8..8 + 14]);
        v7.extend_from_slice(&s.encode()[8 + 15..]);
        let loaded = Settings::decode(&v7).unwrap();
        assert_eq!(loaded.pedestrians, PEDESTRIANS_DEFAULT);
        assert_eq!(Settings::decode(&s.encode()), Some(s));
        assert_eq!(Settings::decode(b"garbage"), None);
        let mut bad = s.encode();
        bad[23..27].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(Settings::decode(&bad).unwrap().controls.left_gain.x, 1.0);
    }
}
