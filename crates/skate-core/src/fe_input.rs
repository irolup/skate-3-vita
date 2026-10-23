//! Front-end pad input (doc 31, "Pad input"): retail's `input.cfg` Apt actions (`AptUp = LStickU.repeat ||
//! DPadU.repeat;` ...) evaluated over the 24 native pad slots (`input::xbox::convert`), giving the key indices the
//! FE poll (sub_825DFDC8) passes to the screen stack's input handler, in the order of the Apt key table
//! (0x83027BF0, extracted at setup). Research `.local/research/fe-input.md`.
//!
//! The edges are retail's channel update sub_82966F30 (research `.local/research/input-repeat.md` pass 2; constants
//! main-checked): held when the value is above 0.5; after each change the next 3 ticks keep it frozen and report no
//! press or release; repeat fires with the press, 24 ticks later, then every 12. Suffixes (parser sub_829674F0):
//! held, pressed, released, repeat. NOT RETAIL YET: the tick rate is not proven; the console's 30 Hz cadence is
//! assumed (0.8 s to the first repeat, then every 0.4 s), normalised so any frame rate gives the same timing.

/// The 24 native pad slots in `input.cfg` / `xbox::convert` order (Button0..Button23).
pub const SLOTS: [&str; 24] = [
    "DPadU", "DPadD", "DPadL", "DPadR", "Start", "Back", "LStick", "RStick", "LBumper", "RBumper", "LTrigger",
    "RTrigger", "A", "B", "X", "Y", "LStickR", "LStickL", "LStickU", "LStickD", "RStickR", "RStickL", "RStickU",
    "RStickD",
];

/// A slot suffix in an Apt expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Held,
    Pressed,
    Released,
    Repeat,
}

/// One Apt key: fires when any of its terms does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptKey {
    pub name: String,
    pub terms: Vec<(usize, Edge)>,
}

/// Parse the Apt actions of `input.cfg` for the given key names (the Apt key table's order, so the index of a
/// key is its retail key index). A name without a definition gets no terms (never fires).
pub fn parse_apt_keys(cfg: &str, names: &[String]) -> Result<Vec<AptKey>, String> {
    let source: String = cfg.lines().map(|l| l.split("//").next().unwrap_or_default()).collect::<Vec<_>>().join("\n");
    let mut defined = std::collections::BTreeMap::new();
    for statement in source.split(';') {
        let Some((name, expression)) = statement.split_once('=') else { continue };
        let name = name.trim();
        if name.starts_with("Apt") {
            defined.insert(name.to_owned(), expression.trim().to_owned());
        }
    }
    names
        .iter()
        .map(|name| {
            let terms = match defined.get(name) {
                Some(expression) => expression.split("||").map(term).collect::<Result<Vec<_>, _>>()?,
                None => Vec::new(),
            };
            Ok(AptKey { name: name.clone(), terms })
        })
        .collect()
}

fn term(text: &str) -> Result<(usize, Edge), String> {
    let text = text.trim();
    let (slot, suffix) = text.split_once('.').ok_or_else(|| format!("Apt term {text:?} has no suffix"))?;
    let slot = SLOTS.iter().position(|s| *s == slot.trim()).ok_or_else(|| format!("unknown pad slot in {text:?}"))?;
    let edge = match suffix.trim() {
        "held" => Edge::Held,
        "pressed" => Edge::Pressed,
        "released" => Edge::Released,
        "repeat" => Edge::Repeat,
        other => return Err(format!("unsupported Apt suffix {other:?}")),
    };
    Ok((slot, edge))
}

/// Edge rule (data with retail defaults; mods may override).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepeatRule {
    /// Held when the slot value is above this (0x8209975C = 0.5).
    pub threshold: f32,
    /// Ticks from the press to the first repeat (24).
    pub first_delay: u32,
    /// Ticks between later repeats (12).
    pub period: u32,
    /// Ticks after a change during which the held state is frozen (3).
    pub settle: u32,
    /// Channel update ticks per second (assumed console cadence).
    pub tick_hz: f32,
}

impl RepeatRule {
    pub const RETAIL: Self = Self { threshold: 0.5, first_delay: 24, period: 12, settle: 3, tick_hz: 30.0 };
}

impl Default for RepeatRule {
    fn default() -> Self {
        Self::RETAIL
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    held: bool,
    settle: u32,
    countdown: u32,
}

/// Per-controller channels (sub_82966F30's 16-byte records).
#[derive(Clone, Debug, Default)]
pub struct FeInput {
    channels: [Channel; 24],
    pending: f32,
}

/// The edges of the ticks run this frame, per slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub held: [bool; 24],
    pub pressed: [bool; 24],
    pub released: [bool; 24],
    pub repeat: [bool; 24],
}

impl FeInput {
    /// Run the channel ticks due after `seconds` (none on a frame between ticks).
    pub fn update(&mut self, values: &[f32; 24], seconds: f32, rule: &RepeatRule) -> Edges {
        let period = 1.0 / rule.tick_hz.max(1.0);
        self.pending = (self.pending + seconds.max(0.0)).min(period * 8.0);
        let mut e = Edges::default();
        while self.pending >= period {
            self.pending -= period;
            self.tick(values, rule, &mut e);
        }
        for (i, c) in self.channels.iter().enumerate() {
            e.held[i] = c.held;
        }
        e
    }

    fn tick(&mut self, values: &[f32; 24], rule: &RepeatRule, e: &mut Edges) {
        for (i, c) in self.channels.iter_mut().enumerate() {
            let raw = values[i] > rule.threshold;
            if c.settle > 0 {
                c.settle -= 1;
            } else if raw != c.held {
                c.held = raw;
                c.settle = rule.settle;
                if raw {
                    e.pressed[i] = true;
                    e.repeat[i] = true;
                    c.countdown = rule.first_delay;
                } else {
                    e.released[i] = true;
                    c.countdown = 0;
                }
                continue;
            }
            if c.held && c.countdown > 0 {
                c.countdown -= 1;
                if c.countdown == 0 {
                    e.repeat[i] = true;
                    c.countdown = rule.period.max(1);
                }
            }
        }
    }
}

/// The key indices that fire this frame, in key order (retail polls the 56 keys in table order).
pub fn fired(keys: &[AptKey], e: &Edges) -> Vec<u32> {
    keys.iter()
        .enumerate()
        .filter(|(_, k)| {
            k.terms.iter().any(|(slot, edge)| match edge {
                Edge::Held => e.held[*slot],
                Edge::Pressed => e.pressed[*slot],
                Edge::Released => e.released[*slot],
                Edge::Repeat => e.repeat[*slot],
            })
        })
        .map(|(i, _)| i as u32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "//Apt\nAptStart = Start.pressed;\nAptSelect = Back.pressed;\nAptNext = A.pressed;\n\
        AptUp = LStickU.repeat || DPadU.repeat; // menu up\nAptUpReleased = LStickU.released || DPadU.released;\n\
        GP_A = A;";

    fn names() -> Vec<String> {
        ["AptStart", "AptSelect", "AptNext", "AptUp", "AptUpReleased", "AptMissing"].map(String::from).to_vec()
    }

    fn slot(name: &str) -> usize {
        SLOTS.iter().position(|s| *s == name).unwrap()
    }

    #[test]
    fn parses_retail_expressions_in_key_order() {
        let keys = parse_apt_keys(CFG, &names()).unwrap();
        assert_eq!(keys[3].terms, [(slot("LStickU"), Edge::Repeat), (slot("DPadU"), Edge::Repeat)]);
        assert!(keys[5].terms.is_empty());
        assert!(parse_apt_keys("AptUp = Nose.repeat;", &["AptUp".into()]).is_err());
    }

    #[test]
    fn press_repeat_release_in_ticks() {
        let keys = parse_apt_keys(CFG, &names()).unwrap();
        let rule = RepeatRule::RETAIL;
        let tick = 1.0 / rule.tick_hz + 1e-5;
        let mut input = FeInput::default();
        let mut up = [0.0; 24];
        up[slot("DPadU")] = 1.0;
        let mut fires = Vec::new();
        for t in 0..60 {
            if !fired(&keys, &input.update(&up, tick, &rule)).is_empty() {
                fires.push(t);
            }
        }
        assert_eq!(fires, [0, 24, 36, 48], "press, 24 ticks, then every 12");
        assert_eq!(fired(&keys, &input.update(&[0.0; 24], tick, &rule)), [4], "release");
        // A press right after the release is frozen by the settle ticks.
        assert!(fired(&keys, &input.update(&up, tick, &rule)).is_empty());
        let mut stick = [0.0; 24];
        stick[slot("LStickU")] = 0.5;
        let mut fresh = FeInput::default();
        assert!(fired(&keys, &fresh.update(&stick, tick, &rule)).is_empty(), "0.5 is not above the threshold");
        let mut start = [0.0; 24];
        start[slot("Start")] = 1.0;
        assert_eq!(fired(&keys, &fresh.update(&start, tick, &rule)), [0]);
        assert!(fired(&keys, &fresh.update(&start, 2.0, &rule)).is_empty(), "pressed does not repeat");
    }

    #[test]
    fn same_ticks_at_any_frame_rate() {
        let keys = parse_apt_keys(CFG, &names()).unwrap();
        let rule = RepeatRule::RETAIL;
        let mut up = [0.0; 24];
        up[slot("DPadU")] = 1.0;
        let count = |fps: f32, frames: usize| {
            let mut input = FeInput::default();
            (0..frames).filter(|_| !fired(&keys, &input.update(&up, 1.0 / fps, &rule)).is_empty()).count()
        };
        assert_eq!(count(30.0, 60), count(144.0, 288));
    }
}
