//! NPC skater natural stance (regular / goofy), set once at spawn from the character record.
//!
//! Retail [code] (doc 26, "NPC skater natural stance"): every NPC skater spawn refills its CAS
//! slot from the skater's character record (`GetCACSettings` `82590B50`, 168-byte entries at
//! `[0x83067060 + 8]`); the actor ctor (`82590DC0`) reads `goofy = (byte +120 == 0)` and passes it
//! to `Initialize82B97E38`, which stores it at character +228 (read back by virtual +128
//! `82B97168` and the Lua bindings `GetIfSkaterIsRegularStance` `8284B910` / `IsRegular`). For a
//! regular skater the same initialiser selects the orientation and mirror bits (`0xC000_0000` of the
//! animation flags), so every bind pose adds `BOARD_BACKWARDS` and mirrors (mode 2); a goofy skater
//! draws the stock rig as authored. The player's `SkaterAnimation::set_customisation` does the same.
//!
//! [data]: the record class (91BCA6693EFC9AA7) is not in our setup export, so the values come from
//! our own measurement table [`RETAIL_TABLE`] (`npc_natural_stance.tsv`: record name, record id,
//! stance, measured / inferred / unseen). A record the table does not know keeps the engine default,
//! goofy (the stock rig, what every NPC drew before this port).
//!
//! Mods: [`resolve`] takes overrides keyed by record id (16 hex digits) or record name; a mod's
//! value wins over the table. Deterministic: the stance is a pure function of the spawn record's
//! character and the overrides, so a client derives the host's value from the spawn record.

use std::collections::BTreeMap;

/// The measurement table (see the file's header).
pub const RETAIL_TABLE: &str = include_str!("npc_natural_stance.tsv");

/// A skater's natural stance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NaturalStance {
    Regular,
    Goofy,
}

impl NaturalStance {
    /// Engine default for a record without a value: the stock rig's stance (retail `82B975B8`
    /// constructs every skater goofy before the record is applied).
    pub const DEFAULT: Self = Self::Goofy;

    /// Retail character +228 / `SkaterPublicationState::natural_stance`: 0 = regular, 1 = goofy.
    pub fn natural_stance(self) -> u32 {
        match self {
            Self::Regular => 0,
            Self::Goofy => 1,
        }
    }

    /// Whether the animation mirror bit (`0x4000_0000`) and the board orientation bit
    /// (`0x8000_0000`) are set at construction (`Initialize82B97E38` toggles both for regular).
    pub fn mirrored(self) -> bool {
        self == Self::Regular
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::Goofy => "goofy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "regular" => Some(Self::Regular),
            "goofy" => Some(Self::Goofy),
            _ => None,
        }
    }
}

/// How a table value is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// Read from the record at NPC construction.
    Measured,
    /// The slot was refilled without a new read: the previous occupant's value (same in every
    /// spawn seen).
    Inferred,
    /// The record did not spawn in the runs: no value (engine default).
    Unseen,
}

/// One table row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StanceRow {
    /// Character record name (the profile's `recipe`, e.g. `josh_kalis`).
    pub record: String,
    /// The record's 64-bit id, if seen.
    pub record_id: Option<u64>,
    /// `None` = unknown (engine default).
    pub stance: Option<NaturalStance>,
    pub evidence: Evidence,
}

/// Parse a stance table (tab separated: record, record id hex or `-`, regular / goofy / unknown,
/// measured / inferred / unseen; `#` comments).
pub fn parse_table(text: &str) -> Result<Vec<StanceRow>, String> {
    let mut rows = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let [record, id, stance, evidence] = f[..] else { return Err(format!("line {}: expected 4 fields", n + 1)) };
        let record_id = match id {
            "-" => None,
            hex => Some(u64::from_str_radix(hex, 16).map_err(|e| format!("line {}: record id {hex}: {e}", n + 1))?),
        };
        let stance = match stance {
            "unknown" => None,
            s => Some(NaturalStance::parse(s).ok_or_else(|| format!("line {}: stance {s}", n + 1))?),
        };
        let evidence = match evidence {
            "measured" => Evidence::Measured,
            "inferred" => Evidence::Inferred,
            "unseen" => Evidence::Unseen,
            e => return Err(format!("line {}: evidence {e}", n + 1)),
        };
        if (stance.is_none()) != (evidence == Evidence::Unseen) {
            return Err(format!("line {}: unknown stance iff unseen", n + 1));
        }
        rows.push(StanceRow { record: record.to_owned(), record_id, stance, evidence });
    }
    Ok(rows)
}

/// The shipped table, parsed.
pub fn retail_table() -> Vec<StanceRow> {
    parse_table(RETAIL_TABLE).expect("the shipped stance table parses")
}

/// Normalise a mod override key: a record id (16 hex digits, any case, optional `0x`) becomes
/// upper-case hex; anything else is a record name.
pub fn override_key(key: &str) -> String {
    let hex = key.strip_prefix("0x").or_else(|| key.strip_prefix("0X")).unwrap_or(key);
    if hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        hex.to_ascii_uppercase()
    } else {
        key.to_owned()
    }
}

/// The natural stance of a character record: a mod override (by record id, then by record name),
/// else the table's value, else [`NaturalStance::DEFAULT`].
pub fn resolve(table: &[StanceRow], overrides: &BTreeMap<String, NaturalStance>, record: &str) -> NaturalStance {
    let row = table.iter().find(|r| r.record == record);
    let by_id = row.and_then(|r| r.record_id).and_then(|id| {
        overrides.iter().find(|(k, _)| override_key(k) == format!("{id:016X}")).map(|(_, v)| *v)
    });
    by_id
        .or_else(|| overrides.get(record).copied())
        .or_else(|| row.and_then(|r| r.stance))
        .unwrap_or(NaturalStance::DEFAULT)
}

/// The stance bits a skater's trick clips toggle (retail SkaterAnim flags `+15180` bit 31
/// "orientation" / board backwards, bit 30 mirror, and the relative stance `+15196`, 1 = switch).
///
/// Retail [code]: every actor's animation step (`82593230`, NPC skaters and the player alike, the
/// actor's SkaterAnim at actor `+1804 - 14960`) runs the tree, then `82B98980`, which queries the
/// current tree's attributes for `animboardbackward`, `mirrored` and `switch` (virtual +96, names
/// at `0x830BFAB4` / `0x830C0694` / `0x830C02A4`) and toggles bit 31, bit 30 and the relative stance
/// once per frame each is present. The natural stance sets the start value (`Initialize82B97E38`:
/// regular = bits 31 and 30 set). The bits bake into every tree built afterwards (the bind pose
/// tail, `MotionAnimation::add_bind_pose`): a tree keeps the bits it was built with.
///
/// Plain data (three bools; [`Self::to_bits`] / [`Self::from_bits`] for a snapshot): a pure
/// function of the natural stance and the trick clips played, so a client derives the host's value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StanceFlags {
    /// Bit 31: `BOARD_BACKWARDS` / `BOARD_BACKWARDS_IK` in the bind pose.
    pub board_backward: bool,
    /// Bit 30: mode 2 mirror in the bind pose.
    pub mirrored: bool,
    /// Relative stance 1 (switch); published to physics, no bind pose effect.
    pub switch: bool,
}

impl StanceFlags {
    /// The start value for a natural stance (`Initialize82B97E38`).
    pub fn natural(stance: NaturalStance) -> Self {
        Self { board_backward: stance.mirrored(), mirrored: stance.mirrored(), switch: false }
    }

    /// `82B98980`: toggle each bit whose event is present this frame.
    pub fn apply(&mut self, events: &StanceEvents, present: impl Fn(&str) -> bool) {
        let fires = |name: &str| !name.is_empty() && present(name);
        if fires(&events.board_backward) {
            self.board_backward = !self.board_backward;
        }
        if fires(&events.mirrored) {
            self.mirrored = !self.mirrored;
        }
        if fires(&events.switch) {
            self.switch = !self.switch;
        }
    }

    /// The board-flipped byte the actor publishes to physics: [code] `GetPhysUpdateData`
    /// `82B985E8` writes packet `+10368` = bit 31 xor bit 30 (the player's
    /// `physics_packet::publish_evaluated`). False for both natural stances; a shove-it (bit 31
    /// alone) sets it. Retail physics reads it as Processed `+2468` bit 20 (the deck's effective
    /// frame `82C01BF8`, push foot frame, deck angles), never in the riding-fakie rule.
    pub fn board_flipped(self) -> bool {
        self.board_backward ^ self.mirrored
    }

    /// The board axis retail's riding-fakie rule (`UpdateRidingFakie82BB2330`) reads, from the
    /// actor's root Z `axis`: [code] `Fill82BE1AE8` stores row 2 of `GetEffectiveRoot82BE3650`
    /// to PhysOutSkeleton `+0`, which copies Skeleton `+11920` and negates rows 0 and 2 iff
    /// Processed `+2476` bit 2 (= packet `+10370`, the mirror bit 30). Bit 31 is not read: a
    /// shove-it does not change the fakie rule's input.
    pub fn fakie_board_axis(self, axis: [f32; 4]) -> [f32; 4] {
        if self.mirrored { axis.map(|v| -v) } else { axis }
    }

    pub fn to_bits(self) -> u8 {
        u8::from(self.board_backward) | u8::from(self.mirrored) << 1 | u8::from(self.switch) << 2
    }

    pub fn from_bits(bits: u8) -> Self {
        Self { board_backward: bits & 1 != 0, mirrored: bits & 2 != 0, switch: bits & 4 != 0 }
    }
}

/// The clip attribute names that toggle each [`StanceFlags`] bit. Retail: the three names
/// `82B98980` queries ([`Self::RETAIL`]); a mod may rename one (its own clips' attribute) or turn a
/// toggle off with an empty name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StanceEvents {
    pub board_backward: String,
    pub mirrored: String,
    pub switch: String,
}

impl StanceEvents {
    /// Keys of the toggles (mod tuning `skater_stance_events`).
    pub const KEYS: [&'static str; 3] = ["board_backward", "mirrored", "switch"];

    pub fn retail() -> Self {
        Self { board_backward: "animboardbackward".into(), mirrored: "mirrored".into(), switch: "switch".into() }
    }

    /// The retail names with a mod's renames (key from [`Self::KEYS`] -> attribute name, empty =
    /// off); unknown keys are ignored.
    pub fn with_overrides(overrides: &BTreeMap<String, String>) -> Self {
        let mut out = Self::retail();
        for (k, v) in overrides {
            match k.as_str() {
                "board_backward" => out.board_backward = v.clone(),
                "mirrored" => out.mirrored = v.clone(),
                "switch" => out.switch = v.clone(),
                _ => {}
            }
        }
        out
    }
}

/// Whether a clip attribute (`begin` / `end` normalised to the clip, `-1` = untimed) is in the
/// collected list of a clip that advanced from `previous_time` to `time` s without wrapping
/// (`GetAttributes82D25E30` with the full mask, as the player's `refresh_tree_attributes`):
/// untimed records always, timed ones when their span meets the advanced window (`end >
/// previous_time && begin <= time`, [`crate::animation::clip_clock::ClipClock::attribute_status`]).
pub fn attribute_in_window(begin: f32, end: f32, length: f32, previous_time: f32, time: f32) -> bool {
    begin == -1.0 || (end * length > previous_time && begin * length <= time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stance_flags_toggle_like_82b98980() {
        let retail = StanceEvents::retail();
        let mut f = StanceFlags::natural(NaturalStance::Goofy);
        assert_eq!(f, StanceFlags::default());
        assert_eq!(StanceFlags::natural(NaturalStance::Regular), StanceFlags { board_backward: true, mirrored: true, switch: false });
        // One toggle per present name per frame; absent names leave their bit.
        f.apply(&retail, |n| n == "animboardbackward");
        assert_eq!(f, StanceFlags { board_backward: true, mirrored: false, switch: false });
        f.apply(&retail, |n| n == "mirrored" || n == "switch");
        assert_eq!(f, StanceFlags { board_backward: true, mirrored: true, switch: true });
        f.apply(&retail, |_| true);
        assert_eq!(f, StanceFlags::default());
        assert_eq!(StanceFlags::from_bits(StanceFlags { board_backward: true, mirrored: false, switch: true }.to_bits()), StanceFlags { board_backward: true, mirrored: false, switch: true });
        // Mod renames: another attribute name, or off.
        let mut o = BTreeMap::new();
        o.insert("mirrored".to_owned(), "my_mirror".to_owned());
        o.insert("switch".to_owned(), String::new());
        o.insert("bogus".to_owned(), "x".to_owned());
        let e = StanceEvents::with_overrides(&o);
        assert_eq!(e.board_backward, "animboardbackward");
        let mut g = StanceFlags::default();
        g.apply(&e, |_| true);
        assert_eq!(g, StanceFlags { board_backward: true, mirrored: true, switch: false }, "switch off, mirror on its new name");
        let mut h = StanceFlags::default();
        h.apply(&e, |n| n == "mirrored");
        assert_eq!(h, StanceFlags::default(), "the retail name no longer toggles");
    }

    /// Player reference: the board-flipped byte equals the player's publication
    /// (`physics_packet::publish_evaluated`, `82B985E8`) for every bit combination, and the
    /// riding-fakie rule (`riding_fakie::State`, `82BB2330`) on [`StanceFlags::fakie_board_axis`]
    /// gives the same answer before and after a shove-it (bit 31 toggled).
    #[test]
    fn board_flipped_and_fakie_axis_match_the_player_reference() {
        use crate::animation::output::physics_packet::{PhysicsPosePacket, SkaterPublicationState, publish_evaluated};
        use crate::animation::riding_fakie::{Physical, Settings, State};
        for bits in 0..8u8 {
            let f = StanceFlags::from_bits(bits);
            let mut state = SkaterPublicationState { orientation_bit31: f.board_backward, mirrored: f.mirrored, riding_fakie: false, weight_on_nose: false, relative_stance: i32::from(f.switch), natural_stance: 0, request_bit16: false, request_bit15: false, air_dismount_revert_requested: false, air_dismount_revert_frames: 0, signal: None };
            let mut packet = PhysicsPosePacket { bone_count: 0, hierarchy: Vec::new(), local: Vec::new(), timestep: 0.0, foot_surface_ids: [0; 2], flags: 0, board_flipped: false, mirrored: false, riding_switch: false, riding_fakie: false, weight_forwards: false, regular_stance: false, air_dismount_revert_frames: 0 };
            publish_evaluated(&mut state, &[], &[], &mut packet, 0.0, b"").unwrap();
            assert_eq!(f.board_flipped(), packet.board_flipped, "{f:?}");
            assert_eq!(f.mirrored, packet.mirrored, "{f:?}");
        }
        for stance in [NaturalStance::Goofy, NaturalStance::Regular] {
            let natural = StanceFlags::natural(stance);
            assert!(!natural.board_flipped(), "{stance:?}");
            let mut shoved = natural;
            shoved.apply(&StanceEvents::retail(), |n| n == "animboardbackward");
            assert!(shoved.board_flipped(), "{stance:?}: a shove-it flips the board");
            let settings = Settings { high_speed: 2.0, low_speed: 0.5, slowly_backwards_seconds: 0.5, after_teleport_seconds: 0.0 };
            for root_z in [[0.0, 0.0, 1.0, 0.0], [0.0, 0.0, -1.0, 0.0]] {
                let run = |flags: StanceFlags| {
                    let mut s = State::default();
                    let physical = Physical { category: 1, grind_state: 0, doing_trick: false, board_axis: flags.fakie_board_axis(root_z), deck_velocity: [0.0, 0.0, 3.0, 0.0], external_velocity: [0.0, 0.0, 3.0, 0.0], ground_projected_speed: 3.0 };
                    (0..3).map(|_| s.update(physical, 1.0 / 30.0, settings)).collect::<Vec<_>>()
                };
                assert_eq!(run(natural), run(shoved), "{stance:?} {root_z:?}: the board bit is not a fakie input");
            }
        }
    }

    #[test]
    fn attribute_window_matches_the_clip_clock() {
        use crate::animation::clip_clock::ClipClock;
        let clock = |previous_time: f32, time: f32| ClipClock { frames: 31.0, fps: 30.0, base_speed: 1.0, speed: 1.0, length: 1.0, time, previous_time, loops_since_evaluation: 0, looping: false, phase_controlled: false };
        for (b, e) in [(0.5f32, 0.5f32), (0.0, 0.0), (1.0, 1.0), (0.2, 0.6), (-1.0, -1.0)] {
            for (p, t) in [(0.0f32, 0.1f32), (0.45, 0.5), (0.5, 0.55), (0.9, 1.0), (0.3, 0.4), (0.0, 0.0)] {
                let status = clock(p, t).attribute_status(b, e);
                let collected = b == -1.0 || (status & 0x0c != 0 && status & 3 != 0);
                assert_eq!(attribute_in_window(b, e, 1.0, p, t), collected, "attribute {b}..{e}, window {p}..{t}");
            }
        }
        // A point event fires once over consecutive frames.
        let hits = (0..60).filter(|i| attribute_in_window(0.5, 0.5, 2.0, *i as f32 / 30.0, (*i + 1) as f32 / 30.0)).count();
        assert_eq!(hits, 1);
    }

    #[test]
    fn the_shipped_table_holds_the_measured_records() {
        let t = retail_table();
        let known: Vec<_> = t.iter().filter(|r| r.stance.is_some()).collect();
        assert_eq!(known.len(), 36);
        assert_eq!(known.iter().filter(|r| r.evidence == Evidence::Measured).count(), 35);
        assert_eq!(known.iter().filter(|r| r.evidence == Evidence::Inferred).count(), 1);
        assert!(known.iter().all(|r| r.record_id.is_some()));
        assert_eq!(t.iter().filter(|r| r.evidence == Evidence::Unseen).count(), 8);
        let mut names: Vec<_> = t.iter().map(|r| r.record.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), t.len(), "one row per record");
    }

    #[test]
    fn the_bit_per_record_matches_the_trace() {
        let t = retail_table();
        let none = BTreeMap::new();
        for r in ["attiba_jefferson", "chris_cole", "dan_drehobl", "john_rattray", "terry_kennedy", "teammate_01", "teammate_04", "teammate_02", "z_kook_1", "danny_way", "seb", "brayden_szafranski", "jason_dill", "pj_ladd", "andrew_reynolds", "darren_navarette", "ryan_smith"] {
            assert_eq!(resolve(&t, &none, r), NaturalStance::Regular, "{r}");
        }
        for r in ["cuz", "joey_brezinski", "john_cardiel", "josh_kalis", "pat_duffy", "ryan_gallant", "teammate_03", "deerman_of_darkwoods", "benny_fairfax", "chris_haslam", "mark_appleyard", "mike_carroll", "lizard_king", "rob_dyrdek", "dennis_busenitz", "eric_koston", "jerry_hsu", "ray_barbee", "slappy"] {
            assert_eq!(resolve(&t, &none, r), NaturalStance::Goofy, "{r}");
        }
        // Unseen and unlisted records keep the engine default.
        assert_eq!(resolve(&t, &none, "colin_mckay"), NaturalStance::DEFAULT);
        assert_eq!(resolve(&t, &none, "not_a_record"), NaturalStance::DEFAULT);
        let row = t.iter().find(|r| r.record == "josh_kalis").unwrap();
        assert_eq!(row.record_id, Some(0xCD56_C7FE_01EB_E665));
        // Retail encoding: +120 = 1 regular -> character +228 = 0.
        assert_eq!(NaturalStance::Regular.natural_stance(), 0);
        assert_eq!(NaturalStance::Goofy.natural_stance(), 1);
        assert!(NaturalStance::Regular.mirrored() && !NaturalStance::Goofy.mirrored());
    }

    #[test]
    fn overrides_by_record_id_or_name_win() {
        let t = retail_table();
        let mut o = BTreeMap::new();
        o.insert("0xcd56c7fe01ebe665".to_owned(), NaturalStance::Regular);
        assert_eq!(resolve(&t, &o, "josh_kalis"), NaturalStance::Regular);
        o.clear();
        o.insert("danny_way".to_owned(), NaturalStance::Goofy);
        assert_eq!(resolve(&t, &o, "danny_way"), NaturalStance::Goofy);
        assert_eq!(resolve(&t, &o, "cuz"), NaturalStance::Goofy, "others keep the table");
        // An id override beats a name override for the same record.
        o.insert("attiba_jefferson".to_owned(), NaturalStance::Regular);
        o.insert("88D72DF443D56751".to_owned(), NaturalStance::Goofy);
        assert_eq!(resolve(&t, &o, "attiba_jefferson"), NaturalStance::Goofy);
    }

    #[test]
    fn bad_tables_are_rejected() {
        assert!(parse_table("a\t-\tregular\tunseen").is_err());
        assert!(parse_table("a\t-\tunknown\tmeasured").is_err());
        assert!(parse_table("a\tXYZ\tgoofy\tmeasured").is_err());
        assert!(parse_table("a\t-\tsideways\tunseen").is_err());
        assert!(parse_table("a\t-\tgoofy").is_err());
        assert_eq!(parse_table("# c\n\na\t01\tgoofy\tmeasured\n").unwrap().len(), 1);
    }
}
