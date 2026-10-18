//! The gameplay HUD: trick name, sequence score and multiplier, the line
//! and its timer, the session total. The reference shows these through its Flash
//! scoring movie; here they are bitmap-font text over the 3D view.
use std::collections::HashMap;

use skate_sim::live::Score;

/// English trick names (`private/hud/trick-names.txt` in sim.pak, `ID<TAB>Text`).
#[derive(Default)]
pub struct Names {
    map: HashMap<String, String>,
}
impl Names {
    pub fn parse(text: &str) -> Self {
        Self {
            map: text
                .lines()
                .filter_map(|l| l.split_once('\t'))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        }
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    /// The English name, or a readable form of the id.
    pub fn name(&self, id: &str) -> String {
        if let Some(n) = self.map.get(id) {
            return n.clone();
        }
        let stripped = id
            .trim_start_matches("ID_TRICK_")
            .trim_start_matches("FLIP_")
            .trim_start_matches("GROUND_TRICK_")
            .trim_start_matches("GRAB_");
        stripped
            .split('_')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) if w.chars().all(|x| x.is_ascii_uppercase()) && w.len() > 2 => {
                        f.to_string() + &c.as_str().to_ascii_lowercase()
                    }
                    _ => w.to_owned(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// `12345.6` → `12,345`.
pub fn points(v: f32) -> String {
    let n = v.max(0.0).round() as u64;
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The trick line as shown: stance prefix and name, e.g. `Fakie 360 Flip`.
pub fn trick_line(names: &Names, score: &Score) -> Option<String> {
    if score.trick.is_empty() {
        return None;
    }
    let name = names.name(&score.trick);
    let [switch, fakie, nollie] = score.stance;
    // Names that already carry their stance ("Nollie Kickflip") keep it once.
    let prefix = if fakie {
        "Fakie "
    } else if switch {
        "Switch "
    } else if nollie && !name.starts_with("Nollie") {
        "Nollie "
    } else {
        ""
    };
    Some(format!("{prefix}{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_points_and_stances() {
        let names = Names::parse("ID_TRICK_FLIP_KICKFLIP\tKickflip\nGRIND_BS_50_50\tBS 50-50\n");
        assert_eq!(names.name("GRIND_BS_50_50"), "BS 50-50");
        assert_eq!(names.name("ID_TRICK_FLIP_360_HARDFLIP"), "360 Hardflip");
        assert_eq!(points(1234567.4), "1,234,567");
        assert_eq!(points(999.0), "999");
        let mut s = Score { trick: "ID_TRICK_FLIP_KICKFLIP".into(), ..Score::default() };
        assert_eq!(trick_line(&names, &s).unwrap(), "Kickflip");
        s.stance = [false, true, false];
        assert_eq!(trick_line(&names, &s).unwrap(), "Fakie Kickflip");
        assert_eq!(trick_line(&names, &Score::default()), None);
    }
}
