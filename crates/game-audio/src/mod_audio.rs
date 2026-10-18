//! The part of the reference's `game_audio/mod_audio.rs` (`b3c9679`) the player components use:
//! the event rows and the [`Observed`] Splice host. Without mods no site records (`EventBuf` stays
//! None) and no rule set exists, so `Observed` passes every call to the real host unchanged.
#![allow(dead_code)]
use crate::mod_rules;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventKind {
    Post,
    Release,
    Splice,
    EmitterStart,
    EmitterStop,
    Zone,
    Speech,
}

impl EventKind {
    fn name(self) -> &'static str {
        match self {
            Self::Post => "post",
            Self::Release => "release",
            Self::Splice => "splice",
            Self::EmitterStart => "emitter_start",
            Self::EmitterStop => "emitter_stop",
            Self::Zone => "zone",
            Self::Speech => "speech",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Player,
    World,
    Npc,
    Emitter,
    Ambience,
    Speech,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::World => "world",
            Self::Npc => "npc",
            Self::Emitter => "emitter",
            Self::Ambience => "ambience",
            Self::Speech => "speech",
        }
    }
}

/// One audio event: plain data, no allocation (`class`: a retail class / bank / slot name).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EventRow {
    pub kind: EventKind,
    pub source: Source,
    /// The class (posts), bank (Splice starts, emitters), or "".
    pub class: &'static str,
    /// The poster's slot (`grind`, `footstep`, `horn`, …) for posts and releases, or "".
    pub slot: &'static str,
    /// Splice sound id, emitter patch, slot index, speech event; 0 otherwise.
    pub id: i32,
    /// The world / NPC object, the zone key, the speaker; 0 for the local player.
    pub owner: u64,
}

/// A post site's buffer: Some while a mod subscribes (`AudioApi::events_on`).
pub(crate) type EventBuf = Option<Vec<EventRow>>;

pub(crate) const MAX_EVENTS: usize = 256;

/// Append a row when recording (one branch otherwise).
#[inline]
pub(crate) fn record(buf: &mut EventBuf, row: EventRow) {
    if let Some(rows) = buf {
        if rows.len() < MAX_EVENTS {
            rows.push(row);
        }
    }
}

/// A Splice host that records each start (`EventKind::Splice`) while the site records, and applies
/// the mods' rules (`mod_rules`: a muted start does not happen); every other call goes to the real
/// host unchanged.
pub(crate) struct Observed<'a, 'b> {
    inner: &'a mut dyn skate_audio::player::contacts::SpliceHost,
    rows: &'b mut EventBuf,
    source: Source,
    owner: u64,
    slot: &'static str,
    rules: Option<&'b mod_rules::RuleSet>,
}

impl<'a, 'b> Observed<'a, 'b> {
    pub(crate) fn new(inner: &'a mut dyn skate_audio::player::contacts::SpliceHost, rows: &'b mut EventBuf, source: Source, owner: u64) -> Self {
        Self { inner, rows, source, owner, slot: "", rules: None }
    }

    /// The site's rules (None: no rules, nothing is checked).
    pub(crate) fn rules(mut self, rules: Option<&'b mod_rules::RuleSet>) -> Self {
        self.rules = rules;
        self
    }

    /// The object's name for the recorded rows (`body_fall`, `ring`; "" by default).
    pub(crate) fn slot(mut self, slot: &'static str) -> Self {
        self.slot = slot;
        self
    }
}

impl skate_audio::player::contacts::SpliceHost for Observed<'_, '_> {
    fn set_route(&mut self, route: skate_audio::bus::Route) {
        self.inner.set_route(route);
    }
    fn set_submix(&mut self, submix: Option<skate_audio::player::footsteps::Submix>) {
        self.inner.set_submix(submix);
    }
    fn start(&mut self, bank: &str, id: u32, block: [f32; 6]) -> Option<skate_audio::splice::SoundId> {
        if let Some(rules) = self.rules {
            let row = EventRow { kind: EventKind::Splice, source: self.source, class: intern(bank), slot: self.slot, id: id as i32, owner: self.owner };
            if rules.mutes(&row) {
                // The request is still reported to subscribers; the sound does not start.
                record(self.rows, row);
                return None;
            }
        }
        let sound = self.inner.start(bank, id, block);
        if sound.is_some() && self.rows.is_some() {
            record(self.rows, EventRow { kind: EventKind::Splice, source: self.source, class: intern(bank), slot: self.slot, id: id as i32, owner: self.owner });
        }
        sound
    }
    fn update(&mut self, sound: skate_audio::splice::SoundId, block: [f32; 6]) {
        self.inner.update(sound, block);
    }
    fn alive(&self, sound: skate_audio::splice::SoundId) -> bool {
        self.inner.alive(sound)
    }
    fn release(&mut self, sound: skate_audio::splice::SoundId) {
        self.inner.release(sound);
    }
}

/// A bank / class name as `&'static str` for event rows (a few dozen distinct names; each is kept
/// once, the first time it is recorded).
pub(crate) fn intern(name: &str) -> &'static str {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut names) = NAMES.lock() else { return "" };
    if let Some(n) = names.iter().find(|n| **n == name) {
        return n;
    }
    if names.len() >= 4096 {
        return "";
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.push(leaked);
    leaked
}

/// A player component slot's name and index for event rows.
pub(crate) fn player_slot(slot: &skate_audio::player::components::Slot) -> (&'static str, i32) {
    use skate_audio::player::components::Slot;
    match *slot {
        Slot::Grind(n) => ("grind", i32::from(n)),
        Slot::Rattle => ("rattle", 0),
        Slot::Wind => ("wind", 0),
        Slot::FootDrag => ("foot_drag", 0),
        Slot::Skid => ("skid", 0),
        Slot::Squeaks => ("squeaks", 0),
        Slot::Seam(n) => ("seam", i32::from(n)),
        Slot::RollingSurface(n) => ("rolling_surface", i32::from(n)),
        Slot::RollingLayer(n) => ("rolling_layer", i32::from(n)),
        Slot::RollingRattle => ("rolling_rattle", 0),
        Slot::BoardSlide => ("board_slide", 0),
        Slot::Flips => ("flips", 0),
        Slot::Cloth(n) => ("cloth", i32::from(n)),
        Slot::Treatment => ("treatment", 0),
        Slot::HomSloMo => ("hom_slo_mo", 0),
        Slot::Footstep(n) => ("footstep", i32::from(n)),
        Slot::ClothFalls => ("cloth_falls", 0),
        Slot::BodySlide => ("body_slide", 0),
        #[allow(unreachable_patterns)]
        _ => ("other", 0),
    }
}
