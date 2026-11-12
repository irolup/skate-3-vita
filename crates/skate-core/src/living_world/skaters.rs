//! Ambient NPC skaters: the rules of the retail manager 8245BA28.
//! Kill switch, phases, cull and spawn: docs/hails-additions/26b-living-world-population.md#ambient-skater-manager

use super::config::{retail, SkaterConfig};
use super::population::{pick_highest, pick_lowest, KindState, Scorer};
use super::rng::Rng;
use super::{dist2, Decision, DespawnReason, SpawnChoice, TickInputs, Vec3};

/// One recorded AI line (`skate-data::aipath`), as the population needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct SkaterLine {
    /// Retail 16-byte `m_ID` (stable across machines and exports).
    pub id: [u8; 16],
    /// Node 0 position (path +48 → node 0).
    pub start: Vec3,
    /// Yaw at node 0 (radians about +y).
    pub heading: f32,
    /// `sub_82456970` validity (loaded, not blocked).
    pub valid: bool,
    /// `m_AllowedSkaters` (bit n = pro index n).
    pub allowed_skaters: u64,
    /// `m_BitFlags` (bits 0..2 compared with the profile's capability bytes).
    pub flags: u32,
}

/// One pool-eligible character (`characters_marquee` record with its profile).
#[derive(Clone, Debug, PartialEq)]
pub struct SkaterCharacter {
    pub key: String,
    /// Profile pro index (`Hash_A390BE5FC0DDA71C`), if the profile has one.
    pub pro_index: Option<u32>,
    /// The runtime profile's bytes +0x31 / +0x30 / +0x32 against line flag bits 0 / 1 / 2 (`sub_8245FFD8`, b675;
    /// names not decoded).
    pub capabilities: [bool; 3],
    pub community: bool,
}

/// What the game provides about lines and characters (the loaded tiles, the roster).
pub trait SkaterWorld {
    fn lines(&self) -> &[SkaterLine];
    fn characters(&self) -> &[SkaterCharacter];
    /// Whether a character may ride a line (`sub_82456970`, used by the spawn `sub_8245C018` / `sub_8245B068`, the
    /// branches and the line-end search; b159 / b675): the line's allowed-skater bit for the character's pro index
    /// AND a capability bit matching a line flag bit. NOT RETAIL YET: the controller exclusion (ctrl+0xA0) is not read.
    fn fits(&self, line: &SkaterLine, character: &SkaterCharacter) -> bool {
        character.pro_index.is_some_and(|p| p < 64 && line.allowed_skaters & (1u64 << p) != 0)
            && (0..3).any(|b| character.capabilities[b] && line.flags & (1 << b) != 0)
    }
}

/// A plain `SkaterWorld` (tests, the engine's loaded data).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkaterData {
    pub lines: Vec<SkaterLine>,
    pub characters: Vec<SkaterCharacter>,
}

impl SkaterWorld for SkaterData {
    fn lines(&self) -> &[SkaterLine] {
        &self.lines
    }
    fn characters(&self) -> &[SkaterCharacter] {
        &self.characters
    }
}

/// The near-skater term of `sub_8245C018` [code]: with the nearest skater closer than 10 m,
/// 600 below 0.5 m, else `(10 - d) x 300 x 600` (truncated).
pub fn near_skater_penalty(min_d2: f32, cfg: &SkaterConfig) -> i64 {
    if !(min_d2 < cfg.near_radius2) {
        return 0;
    }
    let d = min_d2.sqrt();
    if d < retail::SKATER_NEAR_TOUCH {
        retail::SKATER_NEAR_TOUCH_SCORE
    } else {
        ((retail::SKATER_NEAR_SPAN - d) * retail::SKATER_NEAR_K * retail::SKATER_NEAR_SCALE) as i64
    }
}

/// Line scorer: (character, line) pairs, lowest wins.
struct LineScorer<'a> {
    cfg: &'a SkaterConfig,
    world: &'a dyn SkaterWorld,
    skaters: &'a [Vec3],
    online: bool,
}

impl Scorer<(usize, usize)> for LineScorer<'_> {
    fn score(&mut self, &(c, l): &(usize, usize), rng: &mut Rng) -> Option<i64> {
        let line = &self.world.lines()[l];
        let character = &self.world.characters()[c];
        if !line.valid || !self.world.fits(line, character) {
            return None;
        }
        let min_d2 = self.skaters.iter().map(|p| dist2(*p, line.start)).fold(f32::MAX, f32::min);
        if min_d2 < self.cfg.reject_radius2 {
            return None;
        }
        // Online (mgr+608) both the near term and the random term are skipped.
        if self.online {
            return Some(0);
        }
        Some(near_skater_penalty(min_d2, self.cfg) + rng.modulo(self.cfg.score_rand) as i64)
    }
}

/// Character scorer `sub_8245B068` [code]: 500 if the character fits any nearby candidate line,
/// plus 5 per fitting line, plus `rand() % 160`; highest wins.
struct CharacterScorer<'a> {
    world: &'a dyn SkaterWorld,
    lines: &'a [usize],
}

impl Scorer<usize> for CharacterScorer<'_> {
    fn score(&mut self, &c: &usize, rng: &mut Rng) -> Option<i64> {
        let character = &self.world.characters()[c];
        let matches = self.lines.iter().filter(|&&l| self.world.fits(&self.world.lines()[l], character)).count() as i64;
        let fit = if matches > 0 { retail::CHARACTER_FIT } else { 0 };
        Some(fit + retail::CHARACTER_PER_LINE * matches + rng.modulo(retail::CHARACTER_RAND) as i64)
    }
}

fn line_of(choice: &SpawnChoice) -> Option<([u8; 16], &str, u8)> {
    match choice {
        SpawnChoice::Skater { line, character, slot } => Some((*line, character.as_str(), *slot)),
        _ => None,
    }
}

/// A called skater (FE 62 Call Skater; retail companion entries, research b544 / b563): requested until it spawns,
/// requested again when culled, freed only by a dismiss (the skater then rides on as an ambient one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Companion {
    pub key: String,
    /// The live skater serving it.
    pub serial: Option<u32>,
}

/// Candidate lines for a reference point: valid, unused, start node within [inner, outer], first N.
fn candidate_lines(cfg: &SkaterConfig, world: &dyn SkaterWorld, st: &KindState, reference: Vec3, inner: f32) -> Vec<usize> {
    let used: Vec<[u8; 16]> = st.live.values().filter_map(|l| line_of(&l.choice).map(|x| x.0)).collect();
    let (i2, o2) = (inner * inner, cfg.spawn_outer * cfg.spawn_outer);
    world
        .lines()
        .iter()
        .enumerate()
        .filter(|(_, l)| l.valid && !used.contains(&l.id))
        .filter(|(_, l)| {
            let d2 = dist2(l.start, reference);
            d2 >= i2 && d2 <= o2
        })
        .map(|(i, _)| i)
        .take(cfg.max_candidate_lines)
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn tick(cfg: &SkaterConfig, st: &mut KindState, pool: &mut Vec<String>, companions: &mut [Companion], inputs: &TickInputs, tick: u64, seed: u64, out: &mut Vec<Decision>) {
    // A despawned companion is requested again (8245DCE0 -> 827C6180).
    // A waiting one whose character already rides takes that skater (NOT RETAIL YET: inferred, the retail case is not
    // read; the character never rides twice).
    let requeue = |st: &KindState, companions: &mut [Companion]| {
        for c in companions.iter_mut() {
            if c.serial.is_some_and(|s| !st.live.contains_key(&s)) {
                c.serial = None;
            }
            if c.serial.is_none() {
                c.serial = st.live.values().find(|l| line_of(&l.choice).is_some_and(|x| x.1 == c.key)).map(|l| l.id.serial);
            }
        }
    };
    requeue(st, companions);
    // Kill switch.
    if !cfg.enabled || inputs.zombie || inputs.scripted_skaters > 0 {
        st.despawn_all(tick, DespawnReason::Disabled, out);
        requeue(st, companions);
        return;
    }
    let desired = if inputs.online { 0 } else { inputs.ambient_skater_override.unwrap_or(cfg.desired) };
    let option_on = inputs.free_play.is_none_or(|f| f.ai_skaters);
    let phase = (tick % cfg.cycle.max(1) as u64) as u32;
    let observers: Vec<Vec3> = inputs.observers.iter().map(|o| o.position).collect();
    // Reference for spawns: the local player (several observers: rotate per cycle).
    let reference = observers[((tick / cfg.cycle.max(1) as u64) % observers.len() as u64) as usize];

    if phase == cfg.phase_cull {
        let mut excess = st.len() as i64 - desired as i64;
        let serials: Vec<u32> = st.live.keys().copied().collect();
        for s in serials {
            let p = st.live[&s].position;
            let far = observers.iter().all(|o| dist2(p, *o) > cfg.cull * cfg.cull || (p[1] - o[1]).abs() > cfg.cull_height);
            // NOT RETAIL YET (inferred, b563 open): the excess cull leaves called skaters alone, else a call above the
            // desired count is culled and called again every cycle.
            let called = companions.iter().any(|c| c.serial == Some(s));
            let reason = if far {
                DespawnReason::Distance
            } else if excess > 0 && !called {
                DespawnReason::Excess
            } else {
                continue;
            };
            if let Some(r) = st.despawn(s, tick, reason) {
                out.push(Decision::Despawn(r));
                excess -= 1;
            }
        }
    }

    if cfg.is_check_phase(phase) && !option_on {
        st.despawn_all(tick, DespawnReason::FreePlayOff, out);
    }
    requeue(st, companions);

    let Some(world) = inputs.skater_world else { return };

    if phase == cfg.phase_pool {
        pool_update(cfg, st, pool, companions, world, inputs.online, reference);
    }

    if phase == cfg.phase_spawn && option_on {
        spawn(cfg, st, pool, companions, world, inputs, reference, tick, seed, desired, out);
    }
}

/// Character pool (8245B400, scorer 8245B068): one load per call; characters are ready at once here. A waiting
/// companion's character loads first (8245B7A0 takes a requested entry ahead of the random pick).
fn pool_update(cfg: &SkaterConfig, st: &mut KindState, pool: &mut Vec<String>, companions: &[Companion], world: &dyn SkaterWorld, online: bool, reference: Vec3) {
    let lines = candidate_lines(cfg, world, st, reference, cfg.spawn_inner);
    let in_use: Vec<String> = st.live.values().filter_map(|l| line_of(&l.choice).map(|x| x.1.to_string())).collect();
    pool.retain(|k| world.characters().iter().any(|c| &c.key == k));
    let requested = companions.iter().find(|c| c.serial.is_none() && !pool.contains(&c.key) && world.characters().iter().any(|w| w.key == c.key));
    if let Some(c) = requested {
        if pool.len() >= cfg.pool_size as usize {
            let Some(i) = pool.iter().position(|k| !in_use.contains(k) && !companions.iter().any(|c| &c.key == k)) else { return };
            pool.remove(i);
        }
        pool.push(c.key.clone());
        return;
    }
    if pool.len() >= cfg.pool_size as usize {
        let stale = pool.iter().position(|k| {
            !in_use.contains(k)
                && world.characters().iter().find(|c| &c.key == k).is_some_and(|c| !lines.iter().any(|&l| world.fits(&world.lines()[l], c)))
        });
        match stale {
            Some(i) => {
                pool.remove(i);
            }
            None => return,
        }
    }
    let allow_community = cfg.allow_community_offline && !online;
    let candidates: Vec<usize> = world
        .characters()
        .iter()
        .enumerate()
        .filter(|(_, c)| !pool.contains(&c.key) && (allow_community || !c.community))
        .map(|(i, _)| i)
        .collect();
    let mut scorer = CharacterScorer { world, lines: &lines };
    if let Some(best) = pick_highest(&candidates, &mut scorer, &mut st.rng) {
        pool.push(world.characters()[candidates[best]].key.clone());
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(cfg: &SkaterConfig, st: &mut KindState, pool: &[String], companions: &mut [Companion], world: &dyn SkaterWorld, inputs: &TickInputs, reference: Vec3, tick: u64, seed: u64, desired: u32, out: &mut Vec<Decision>) {
    let ambient = st.len() as u32;
    if ambient + inputs.scripted_skaters >= cfg.ai_cap {
        return;
    }
    let used: Vec<(&str, u8)> = st.live.values().filter_map(|l| line_of(&l.choice).map(|x| (x.1, x.2))).collect();
    // Free slot: slots [0, players) are players (retail slot 0 = local, online players after); the search starts at 1.
    let players = inputs.player_slots.max(1);
    let Some(slot) = (players.max(1)..cfg.slots).find(|s| !used.iter().any(|(_, u)| *u as u32 == *s)) else { return };
    // A loaded companion first (sub_8245C548): a fitting free line from the companion inner radius out, the nearest to
    // the player wins. NOT RETAIL YET (open in b563): whether the desired-count gate also holds a called skater back.
    let waiting = companions.iter().position(|c| c.serial.is_none() && pool.contains(&c.key) && !used.iter().any(|(k, _)| *k == c.key));
    if let Some(i) = waiting {
        let Some(character) = world.characters().iter().find(|c| c.key == companions[i].key) else { return };
        let skaters: Vec<Vec3> = inputs.observers.iter().map(|o| o.position).chain(st.live.values().map(|l| l.position)).collect();
        let best = candidate_lines(cfg, world, st, reference, cfg.companion_inner)
            .into_iter()
            .map(|l| &world.lines()[l])
            .filter(|line| world.fits(line, character) && skaters.iter().all(|p| dist2(*p, line.start) >= cfg.reject_radius2))
            .min_by(|a, b| dist2(a.start, reference).total_cmp(&dist2(b.start, reference)));
        if let Some(line) = best {
            let choice = SpawnChoice::Skater { line: line.id, character: character.key.clone(), slot: slot as u8 };
            let record = st.spawn(seed, tick, line.start, line.heading, false, choice);
            companions[i].serial = Some(record.id.serial);
            out.push(Decision::Spawn(record));
        }
        return;
    }
    if ambient >= desired {
        return;
    }
    let characters: Vec<usize> = pool
        .iter()
        .filter(|k| !used.iter().any(|(c, _)| c == k) && !companions.iter().any(|c| &c.key == *k))
        .filter_map(|k| world.characters().iter().position(|c| &c.key == k))
        .take(cfg.pool_size as usize)
        .collect();
    if characters.is_empty() {
        return;
    }
    let lines = candidate_lines(cfg, world, st, reference, cfg.spawn_inner);
    if lines.is_empty() {
        return;
    }
    let mut skaters: Vec<Vec3> = inputs.observers.iter().map(|o| o.position).collect();
    skaters.extend(st.live.values().map(|l| l.position));
    let pairs: Vec<(usize, usize)> = characters.iter().flat_map(|&c| lines.iter().map(move |&l| (c, l))).collect();
    let mut scorer = LineScorer { cfg, world, skaters: &skaters, online: inputs.online };
    let Some(best) = pick_lowest(&pairs, &mut scorer, &mut st.rng) else { return };
    let (c, l) = pairs[best];
    let line = &world.lines()[l];
    let choice = SpawnChoice::Skater { line: line.id, character: world.characters()[c].key.clone(), slot: slot as u8 };
    out.push(Decision::Spawn(st.spawn(seed, tick, line.start, line.heading, false, choice)));
}
