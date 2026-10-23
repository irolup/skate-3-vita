//! Retail teleporters (skate-game `teleporters.rs`, PR #52 `b110bab`; research b420 / b421 / b423): the disc's
//! teleport challenges, from `private/teleporters.json` in ui.pak (setup `teleporters.py`, every volume inline:
//! `vita-bake ui`).
//!
//! Retail (stategraph `challenge_stategraph/teleport`, b421): every teleporter of the loaded map is armed; the skater
//! entering its volume shows the prompt (the action overlay's Teleport state with the challenge's title, D-pad Up),
//! leaving hides it; D-pad Up asks `Popup.ShowYesNoPopup(Title, "ID_PAUSEMENU_CONFIRM")` (No / Yes) for one exit, a
//! list of the exits for several; Yes takes exit 0, No goes back to waiting (a skater still inside is asked again: no
//! cooldown). The move: an exit on the loaded map is placed at once, another map loads first.
//!
//! Ours: the skater's query cylinder (retail's feet, head and hips points, b423) stands on the root with the
//! recomp's heights (head 1.63 m, hips 0.98 m) instead of the posed bones (NOT RETAIL YET while crouching or
//! falling). Without the action overlay movie (older sets) there is no prompt and no teleporter.
use serde::Deserialize;
use skate_core::fe_popup::PopupRequest;
use skate_core::triggers::{overlaps, OrientedBox, QueryShape};

/// The teleporters' file in ui.pak (`vita-bake ui`).
const PATH: &str = "private/teleporters.json";
/// The confirm popup's callback.
pub const CONFIRM: &str = "engine:teleporter";
/// The recomp's head and hips heights above the feet (b423: 1.61-1.65 m, 0.97-1.00 m).
const HEAD: f32 = 1.63;
const HIPS: f32 = 0.98;

#[derive(Deserialize)]
struct File {
    format: String,
    version: u32,
    teleporters: Vec<Record>,
}

#[derive(Deserialize)]
struct Record {
    id: String,
    map: String,
    volume: Option<skate_data::trigger_volumes::TriggerVolumeRecord>,
    #[serde(default)]
    title_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    exits: Vec<Exit>,
    /// A skate.Park layout's teleporter: not armed (the Vita loads no layouts yet).
    #[serde(default)]
    layout: Option<String>,
    /// Its floating icon (`teleporter_icons`): the `ribbon_indicator` row and the anchor locator's matrix.
    #[serde(default)]
    indicator: Option<Placement>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Placement {
    pub key: String,
    #[serde(default)]
    pub locator: String,
    pub matrix: [[f32; 4]; 4],
}

#[derive(Clone, Debug, Deserialize)]
pub struct Exit {
    pub map: String,
    pub locator: String,
    pub matrix: [[f32; 4]; 4],
}

#[derive(Clone, Debug)]
pub struct Teleporter {
    pub id: String,
    pub map: String,
    pub title: String,
    pub volume: OrientedBox,
    pub exits: Vec<Exit>,
    pub indicator: Option<Placement>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Waiting,
    /// The confirm popup is up.
    Confirm,
}

#[derive(Default)]
pub struct Teleporters {
    list: Vec<Teleporter>,
    /// The teleporter whose volume the skater is inside (index in `list`).
    inside: Option<usize>,
    phase: Phase,
}

impl Teleporters {
    /// The baked teleporters (none when ui.pak has no file).
    pub fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Self {
        let parsed = read(PATH).and_then(|b| serde_json::from_slice::<File>(&b).map_err(|e| e.to_string()));
        let list = match parsed {
            Ok(f) if f.format == "skate3rust.teleporters" && f.version == 1 => f
                .teleporters
                .into_iter()
                .filter(|r| r.layout.is_none() && !r.exits.is_empty())
                .filter(|r| r.exits.iter().all(|e| e.matrix.iter().flatten().all(|v| v.is_finite())))
                .filter_map(|r| {
                    Some(Teleporter {
                        title: r.title.or(r.title_id).unwrap_or_else(|| r.id.clone()),
                        id: r.id,
                        map: r.map,
                        volume: r.volume?.shape.oriented_box(),
                        exits: r.exits,
                        indicator: r.indicator,
                    })
                })
                .collect(),
            Ok(f) => {
                println!("teleporters: {} v{} not supported", f.format, f.version);
                Vec::new()
            }
            Err(e) => {
                println!("teleporters: none ({e})");
                Vec::new()
            }
        };
        println!("teleporters: {} loaded", list.len());
        Self { list, ..Self::default() }
    }

    /// Whether the skater on `world` with its feet at `feet` is inside a teleporter (None while a world loads);
    /// leaving it, or another world, forgets it.
    pub fn track(&mut self, world: Option<&str>, feet: [f32; 3]) {
        let Some(world) = world else {
            self.inside = None;
            return;
        };
        let up = |h: f32| [feet[0], feet[1] + h, feet[2]];
        let cylinder = QueryShape::default().cylinder(feet, up(HEAD), up(HIPS));
        self.inside = cylinder.and_then(|c| self.list.iter().position(|t| t.map.eq_ignore_ascii_case(world) && overlaps(&c, &t.volume)));
    }

    /// The icon anchors of the armed teleporters on `world` with indicator row `key`, one per locator (retail keeps
    /// the first entry per locator, b533).
    pub fn icon_anchors(&self, world: &str, key: &str, out: &mut Vec<[[f32; 4]; 4]>) {
        out.clear();
        let mut seen: Vec<&str> = Vec::new();
        for t in self.list.iter().filter(|t| t.map.eq_ignore_ascii_case(world)) {
            let Some(p) = t.indicator.as_ref().filter(|p| p.key == key) else { continue };
            if !p.locator.is_empty() && seen.contains(&p.locator.as_str()) {
                continue;
            }
            seen.push(&p.locator);
            out.push(p.matrix);
        }
    }

    /// The indicator row the armed teleporters name (the disc's all name `teleport`).
    pub fn icon_key(&self) -> Option<&str> {
        self.list.iter().find_map(|t| t.indicator.as_ref().map(|p| p.key.as_str()))
    }

    /// The prompt's title while the skater is inside and not asked yet.
    pub fn prompt(&self) -> Option<&str> {
        self.inside.filter(|_| self.phase == Phase::Waiting).map(|i| self.list[i].title.as_str())
    }

    /// D-pad Up while the prompt shows: the confirm popup to raise (b421 / `828556D8`: one exit asks No / Yes,
    /// several list the exits).
    pub fn ask(&mut self) -> Option<PopupRequest> {
        let t = &self.list[self.inside.filter(|_| self.phase == Phase::Waiting)?];
        self.phase = Phase::Confirm;
        let options = if t.exits.len() == 1 {
            vec!["ID_COMMON_NO".into(), "ID_COMMON_YES".into()]
        } else {
            t.exits.iter().map(|e| e.locator.clone()).collect()
        };
        Some(PopupRequest {
            title: t.title.clone(),
            description: if t.exits.len() == 1 { "ID_PAUSEMENU_CONFIRM" } else { "ID_TELEPORT_STADIUM_RAMPS_DESC" }.into(),
            options,
            callback: CONFIRM.into(),
            cancelable: true,
            ..PopupRequest::default()
        })
    }

    /// The confirm popup is up (the game waits for it).
    pub fn asking(&self) -> bool {
        self.phase == Phase::Confirm
    }

    /// The popup's choice: the exit to take (Yes of No / Yes takes exit 0, a list option its exit), or none; back to
    /// waiting either way.
    pub fn choose(&mut self, choice: i32) -> Option<Exit> {
        self.phase = Phase::Waiting;
        let t = &self.list[self.inside?];
        let exit = if t.exits.len() == 1 { (choice == 1).then_some(0) } else { usize::try_from(choice).ok().filter(|&i| i < t.exits.len()) };
        exit.map(|i| t.exits[i].clone())
    }

    /// The popup went without a choice (the menus closed it).
    pub fn cancel(&mut self) {
        self.phase = Phase::Waiting;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(json: &'static str) -> impl Fn(&str) -> Result<Vec<u8>, String> {
        move |_| Ok(json.as_bytes().to_vec())
    }

    const FILE: &str = r#"{"format": "skate3rust.teleporters", "version": 1, "teleporters": [
        {"id": "tele_a", "map": "DownTown", "volume_id": "1", "title": "Enter the park",
         "volume": {"id": "1", "name": "v", "shape": {"kind": "box", "center": [10, 0, 0], "half_extents": [1, 2, 1]}},
         "exits": [{"map": "DownTownSkatePark", "locator": "l", "matrix": [[1,0,0,0],[0,1,0,0],[0,0,1,0],[5,6,7,1]]}]},
        {"id": "tele_layout", "map": "DownTown", "volume_id": "2", "layout": "MegaPark_Fullpark",
         "volume": {"id": "2", "name": "w", "shape": {"kind": "box", "center": [0, 0, 0], "half_extents": [1, 1, 1]}},
         "exits": [{"map": "DownTown", "locator": "l", "matrix": [[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]]}]}
    ]}"#;

    #[test]
    fn inside_asks_yes_takes_the_exit_and_no_asks_again() {
        let mut t = Teleporters::load(&read(FILE));
        assert_eq!(t.list.len(), 1, "layout teleporters are not armed");
        t.track(Some("DownTown"), [0.0, 0.0, 0.0]);
        assert_eq!(t.prompt(), None);
        t.track(Some("University"), [10.0, 0.0, 0.0]);
        assert_eq!(t.prompt(), None, "another map's teleporter");
        t.track(Some("DownTown"), [10.0, -1.0, 0.0]);
        assert_eq!(t.prompt(), Some("Enter the park"));
        let popup = t.ask().unwrap();
        assert_eq!((popup.options.len(), popup.description.as_str()), (2, "ID_PAUSEMENU_CONFIRM"));
        assert_eq!(t.prompt(), None, "no prompt while asking");
        assert!(t.choose(0).is_none(), "No");
        assert_eq!(t.prompt(), Some("Enter the park"), "still inside: asked again");
        t.ask();
        let exit = t.choose(1).unwrap();
        assert_eq!((exit.map.as_str(), exit.matrix[3]), ("DownTownSkatePark", [5.0, 6.0, 7.0, 1.0]));
        t.track(None, [10.0, 0.0, 0.0]);
        assert_eq!(t.prompt(), None, "a world loading");
    }
}
