//! The instant replay screen's natives (FE 12, template Replay2, `replay2/replay_editor2`; research b385 / b390 /
//! b391; skate-game `replay_menu_host.rs`, PR #52 `b110bab`): `FEReplayMenu.*` answered from a per-frame snapshot of
//! the replay ([`ReplayScreenFeed`], published by the game), and the screen states the C++ posts back, as the Trick
//! Guide host does.
//!
//! skate_vita: the Create pack's screen (`replay_editor3`, its edit channels and tints) is not taken. Ours from the
//! P1 editor (device-tuned on the same movie): the timeline's over-marker states (1006 / 1007 / 1009), the camera
//! zoom text and its refresh (11), the help pages per camera mode, and a camera edit returning to the effects menu.
use crate::fe::apt_vm::{Value, Vm};

/// Movie states the C++ posts (b386 / b390; names from the movie's SetState).
pub mod movie_state {
    /// kMovePlayHead, kStartPlaybackReplay / kStopPlaybackReplay.
    pub const CURSOR: i32 = 999;
    pub const PLAY: i32 = 18;
    pub const STOP: i32 = 19;
    /// kShowEditor / kHideEditor: the whole panel; the movie's own Hide / Show prompt follows.
    pub const SHOW: i32 = 1002;
    pub const HIDE: i32 = 1003;
    /// kShowToggle / kHideToggle: only the Hide / Show prompt itself (`ForceShowHideToggle`).
    pub const SHOW_TOGGLE: i32 = 21;
    pub const HIDE_TOGGLE: i32 = 22;
    /// kEffectsMode, kRefreshEffectsMenu (the row moved), kUpdateEffectValues (a value changed), kMainEditMode.
    pub const EFFECTS: i32 = 4;
    pub const REFRESH_EFFECTS: i32 = 5;
    pub const UPDATE_EFFECTS: i32 = 6;
    pub const MAIN_EDIT: i32 = 9;
    /// kMoveStartMarker / kMoveEndMarker (the in / out head moved), and the LB move state (b386: 1005).
    pub const MOVE_START: i32 = 1000;
    pub const MOVE_END: i32 = 1001;
    pub const LB_MOVE: i32 = 1005;
    /// skate_vita (P1): the camera zoom text changed (kUpdateZoom), and what the playhead is over in the timeline:
    /// a clip marker, a clip, a camera key.
    pub const UPDATE_ZOOM: i32 = 11;
    pub const OVER_CLIP_MARKER: i32 = 1006;
    pub const OVER_CLIP: i32 = 1007;
    pub const OVER_KEY_MARKER: i32 = 1009;
}

/// Editor modes (retail `+0x30`, b398): instant replay, the timeline, the LB move, the effects menu, playback.
pub mod mode {
    pub const INSTANT: u8 = 5;
    pub const TIMELINE: u8 = 6;
    pub const MOVE: u8 = 2;
    pub const EFFECTS: u8 = 11;
    pub const PLAYBACK: u8 = 1;
    /// A camera edit (7..=10 in upstream's numbering; ours: 7 + the camera's row); `camera_state` says which.
    pub const CAMERA: u8 = 7;
}

/// What the playhead is over in the timeline (P1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Over {
    #[default]
    Clip,
    Marker,
    Key,
}

/// What the screen reads from the replay each frame (a serialisable snapshot).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplayScreenFeed {
    /// The replay is open: the screen shows.
    pub active: bool,
    /// Instant mode (retail FE 12 byte +0x82, `IsInstantReplayMode` 825ADFC0).
    pub instant: bool,
    /// Cursor and playhead as a fraction of the recording, 0..1.
    pub position: f64,
    pub playing: bool,
    /// The panel (prompts and timeline) is shown.
    pub panel: bool,
    /// The encoded length, s (`GetEncodedLength` 825AE488 reads it as a whole number) and whether it is within the
    /// share limit (`IsEncodedLengthValid` 825ADF88).
    pub length_seconds: u32,
    pub length_valid: bool,
    /// The help screen (FE 13) is open, its page set (`GetHelpMode`), and how many page turns were asked.
    pub help: bool,
    pub help_mode: i32,
    pub help_turns: u32,
    /// The editor mode ([`mode`]) and, in a camera edit, its state (Follow 1, Tripod 2, Game 10, Free 7, the advanced
    /// cameras 12 / 13 / 14, b405).
    pub mode: u8,
    pub camera_state: i32,
    /// The clip camera's menu row (`GetCurrentCameraEditIndex`; b401).
    pub camera_row: u32,
    /// The timeline's keys (clip markers and camera keys) as fractions of the recording (b403), and the band's
    /// segments (start fraction, camera type 0 Follow / 1 Tripod / 2 Free / 3 Game).
    pub keys: Vec<f32>,
    pub segments: Vec<(f32, u8)>,
    /// The in / out points in 30 fps frames, and as fractions of the recording (where the timeline draws them).
    pub heads: (u32, u32),
    pub head_fractions: (f64, f64),
    /// The effects row and the speed / shake values (b398).
    pub row: u32,
    pub speed: i32,
    pub shake: i32,
    /// P1: the camera zoom text ("Zoom 35 mm") and what the playhead is over.
    pub zoom: String,
    pub over: Over,
}

/// The help movie's SetState that turns the page (`NextHelpPage`).
pub const HELP_NEXT_PAGE: i32 = 0;

#[derive(Clone, Debug, Default)]
pub struct ReplayMenuHost {
    /// The screen is open (its natives are ours).
    pub open: bool,
    pub feed: ReplayScreenFeed,
    /// The feed last posted to the movie (edges become states).
    posted: Option<ReplayScreenFeed>,
    /// Screen states to post to the movie, in order.
    pub states: Vec<i32>,
    /// The movie called `FEReplayMenu.Ready`.
    pub ready: bool,
    /// The natives the movie called since the screen opened (diagnostics, tests).
    pub seen: Vec<String>,
    /// The help screen is open / shown (after its IntroComplete), and the page turns already posted to it.
    pub help_open: bool,
    pub help_shown: bool,
    pub help_turns_posted: u32,
    /// The state being posted (the stage sets it around each SetState): the movie's In / Out placement reads the
    /// playhead (`GetCursorOffset` ignores its argument), so during MOVE_START / MOVE_END the playhead answers the
    /// head's fraction (P1's bug: the markers followed the playhead).
    pub posting: Option<i32>,
}

impl ReplayMenuHost {
    pub fn reset(&mut self) {
        let feed = std::mem::take(&mut self.feed);
        *self = Self { feed, ..Default::default() };
    }

    /// Queue the states for what changed since the last post: play / stop and panel edges (never on the first post:
    /// the movie opens paused with its panel shown), the mode, then the heads and the cursor.
    pub fn post_changes(&mut self) {
        use movie_state::*;
        let last = self.posted.clone();
        let feed = &self.feed;
        if last.as_ref().is_some_and(|l| l.playing != feed.playing) || (last.is_none() && feed.playing) {
            self.states.push(if feed.playing { PLAY } else { STOP });
        }
        if last.as_ref().is_some_and(|l| l.panel != feed.panel) {
            self.states.push(if feed.panel { SHOW } else { HIDE });
        }
        // Editor mode changes (b386): A from instant -> 21, 9; LB -> 1005, released -> 21; A -> effects 22, 4; B out of
        // the effects -> 21, 9; a camera edit's state from its camera; a commit back to the timeline 21, 9 (ours: or
        // to the effects menu, 4).
        if let Some(l) = last.as_ref().filter(|l| (l.mode, l.camera_state) != (feed.mode, feed.camera_state)) {
            use mode::*;
            match (l.mode, feed.mode) {
                (INSTANT, TIMELINE) | (EFFECTS, TIMELINE) => self.states.extend([SHOW_TOGGLE, MAIN_EDIT]),
                (TIMELINE, MOVE) => self.states.push(LB_MOVE),
                (MOVE, TIMELINE) => self.states.push(SHOW_TOGGLE),
                (TIMELINE, EFFECTS) => self.states.extend([HIDE_TOGGLE, movie_state::EFFECTS]),
                (_, CAMERA) => self.states.push(feed.camera_state),
                (CAMERA, TIMELINE) => self.states.extend([SHOW_TOGGLE, MAIN_EDIT]),
                (CAMERA, EFFECTS) => self.states.push(movie_state::EFFECTS),
                _ => {}
            }
        }
        if feed.mode == mode::TIMELINE && last.as_ref().is_none_or(|l| l.mode != mode::TIMELINE || l.over != feed.over) {
            self.states.push(match feed.over {
                Over::Marker => OVER_CLIP_MARKER,
                Over::Key => OVER_KEY_MARKER,
                Over::Clip => OVER_CLIP,
            });
        }
        if let Some(l) = last.as_ref().filter(|l| l.mode == mode::EFFECTS && feed.mode == mode::EFFECTS) {
            if l.row != feed.row {
                self.states.push(REFRESH_EFFECTS);
            }
            if l.speed != feed.speed || l.shake != feed.shake {
                self.states.push(UPDATE_EFFECTS);
            }
        }
        if feed.mode == mode::CAMERA && last.as_ref().is_some_and(|l| l.mode == mode::CAMERA && l.zoom != feed.zoom) {
            self.states.push(UPDATE_ZOOM);
        }
        if last.as_ref().is_none_or(|l| l.head_fractions.0 != feed.head_fractions.0) {
            self.states.push(MOVE_START);
        }
        if last.as_ref().is_none_or(|l| l.head_fractions.1 != feed.head_fractions.1) {
            self.states.push(MOVE_END);
        }
        if last.as_ref().is_none_or(|l| l.position != feed.position || l.length_seconds != feed.length_seconds) {
            self.states.push(CURSOR);
        }
        self.posted = Some(self.feed.clone());
    }

    /// The length text: `ID_REPLAY_EDITOR_LENGTH` ("Estimated Video Length: {0:time}") with the length. Retail's `time`
    /// placeholder (handler 824FE900, b397) is the number (grouped by 3, `8290E370`) + " " + `ID_COMMON_METRIC_TIME`
    /// ("s"), not a clock. NOT RETAIL YET: the default decimals are not proven (0 assumed: "42 s").
    fn length_text(&self, language: &std::collections::BTreeMap<String, String>) -> String {
        let digits = self.feed.length_seconds.to_string();
        let mut grouped = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        let unit = crate::fe::fe_language::template(language, "ID_COMMON_METRIC_TIME");
        let template = crate::fe::fe_language::template(language, "ID_REPLAY_EDITOR_LENGTH");
        template.replacen("{0:time}", &format!("{grouped} {unit}"), 1)
    }

    /// One native call; None = not ours.
    pub fn call(&mut self, _vm: &mut Vm, language: &std::collections::BTreeMap<String, String>, object: &str, method: &str) -> Option<Value> {
        if object != "FEReplayMenu" || !self.open {
            return None;
        }
        if !self.seen.iter().any(|m| m == method) {
            self.seen.push(method.to_owned());
        }
        let feed = &self.feed;
        Some(match method {
            "Ready" | "UpdateReplayEditorReady" => {
                self.ready = true;
                Value::Undefined
            }
            "IsInstantReplayMode" => Value::Bool(feed.instant),
            "RefreshCursor" => Value::Undefined,
            "GetCursorPosition" | "GetPlayheadPosition" => Value::Number(match self.posting {
                Some(movie_state::MOVE_START) => feed.head_fractions.0,
                Some(movie_state::MOVE_END) => feed.head_fractions.1,
                _ => feed.position,
            }.clamp(0.0, 1.0)),
            "GetEncodedLength" => Value::Text(self.length_text(language)),
            "IsEncodedLengthValid" => Value::Bool(feed.length_valid),
            // Ours on the Vita: the photo item shows (Take Photo saves a screenshot to the card).
            "CanSaveMedia" => Value::Bool(true),
            // Retail 825ADF00: in editor modes 5 (instant), 6 (timeline) and 1 (playback) (b470).
            "CanDisplayShowHide" => Value::Bool(matches!(feed.mode, mode::INSTANT | mode::TIMELINE | mode::PLAYBACK)),
            "GetHelpMode" => Value::Number(f64::from(feed.help_mode)),
            // The editor's natives (b398; AS integers): effects row, speed, shake 0..3, the camera row, the in / out
            // points in 30 fps frames, the zoom text.
            "GetEditIndex" => Value::Number(f64::from(feed.row)),
            "GetSimpleSpeedSetting" => Value::Number(f64::from(feed.speed)),
            "GetShakeCamType" => Value::Number(f64::from(feed.shake)),
            "GetCurrentCameraEditIndex" => Value::Number(f64::from(feed.camera_row)),
            "GetStartHeadPosition" => Value::Number(f64::from(feed.heads.0)),
            "GetEndHeadPosition" => Value::Number(f64::from(feed.heads.1)),
            "GetCameraZoomString" => Value::Text(feed.zoom.clone()),
            _ => return None,
        })
    }
}

/// The replay editor's timeline strip (retail 82509B80 -> 82508E08 bands, 82509298 keys; b403), in the strip clip's
/// space (x = fraction x 817): one band from y 3 to 30 coloured by the segment's camera type, brighter while
/// "selected" (no key under the playhead, not instant mode); one 13 x 51 key quad per key, `key_glow` under the
/// playhead, `key_default` else; in a camera edit the key under the playhead is not drawn. Values are retail's
/// ([`TimelineLook`]). `placeholder`: the CustomTimeline draw (the clip's origin, x and y unit points).
pub fn timeline_draws(placeholder: &crate::fe::apt_scene::Draw, feed: &ReplayScreenFeed) -> Vec<crate::fe::apt_scene::Draw> {
    use crate::fe::apt_scene::{Draw, Vertex};
    let look = TimelineLook::default();
    let mut out = Vec::new();
    if feed.instant || placeholder.vertices.len() < 3 {
        return out;
    }
    let [o, px, py] = [0, 1, 2].map(|i| placeholder.vertices[i].position);
    let at = |x: f32, y: f32, u: f32, v: f32| Vertex {
        position: [o[0] + x * (px[0] - o[0]) + y * (py[0] - o[0]), o[1] + x * (px[1] - o[1]) + y * (py[1] - o[1])],
        uv: [u, v],
    };
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32, texture: String, multiply: [f32; 4]| Draw {
        texture,
        fill: placeholder.fill,
        vertices: vec![at(x0, y0, 0., 0.), at(x1, y0, 1., 0.), at(x1, y1, 1., 1.), at(x0, y0, 0., 0.), at(x1, y1, 1., 1.), at(x0, y1, 0., 1.)],
        multiply,
        add: [0.0; 4],
    };
    let position = feed.position as f32;
    let under = feed.keys.iter().position(|k| (k - position).abs() <= look.snap);
    let a = placeholder.multiply[3];
    // The bands run from the In point to the Out point.
    let (in_point, out_point) = (feed.head_fractions.0 as f32, feed.head_fractions.1 as f32);
    let segments = if feed.segments.is_empty() { vec![(in_point, 3)] } else { feed.segments.clone() };
    for (i, &(start, kind)) in segments.iter().enumerate() {
        let end = segments.get(i + 1).map_or(out_point, |s| s.0);
        let current = (start..end).contains(&position) || (i + 1 == segments.len() && position >= end);
        let kind = usize::from(kind.min(3));
        let (rgb, alpha) = if current && under.is_none() { (look.selected[kind], look.alpha_selected) } else { (look.normal[kind], look.alpha) };
        // An untextured quad is a solid fill (the stage's convention).
        let mut band = quad(start * look.width, look.band_top, end * look.width, look.band_bottom, String::new(), [rgb[0], rgb[1], rgb[2], alpha * a]);
        band.fill = crate::fe::apt_scene::Fill::Solid;
        out.push(band);
    }
    let camera_edit = feed.mode == mode::CAMERA;
    for (i, k) in feed.keys.iter().enumerate() {
        let highlighted = under == Some(i);
        if highlighted && camera_edit {
            continue;
        }
        let x = k * look.width - 0.5;
        let name = if highlighted { "key_glow" } else { "key_default" };
        out.push(quad(
            x - look.key_size[0] / 2.0,
            0.0,
            x + look.key_size[0] / 2.0,
            look.key_size[1],
            format!("{}texture/{name}", crate::fe::retail_menu_movies::NAMED_IMAGE_PREFIX),
            [1.0, 1.0, 1.0, a],
        ));
    }
    out
}

/// The timeline strip's retail look (b403, 82508E08 / 82509298).
struct TimelineLook {
    width: f32,
    band_top: f32,
    band_bottom: f32,
    /// Band colours per camera type (0 Follow, 1 Tripod, 2 Free, 3 Game), normal / selected, and their alphas.
    normal: [[f32; 3]; 4],
    selected: [[f32; 3]; 4],
    alpha: f32,
    alpha_selected: f32,
    key_size: [f32; 2],
    /// A key within this fraction of the playhead is under it (`825084C8`).
    snap: f32,
}

impl Default for TimelineLook {
    fn default() -> Self {
        Self {
            width: 817.0,
            band_top: 3.0,
            band_bottom: 30.0,
            normal: [[0.41, 0.41, 0.19], [0.12, 0.28, 0.36], [0.27, 0.40, 0.15], [0.30, 0.30, 0.35]],
            selected: [[0.74, 0.74, 0.19], [0.29, 0.54, 0.65], [0.52, 0.72, 0.36], [0.53, 0.53, 0.53]],
            alpha: 0.65,
            alpha_selected: 0.85,
            key_size: [13.0, 51.0],
            snap: 0.00889,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natives_answer_from_the_feed_and_edges_become_states() {
        let mut vm = Vm::new();
        let language = std::collections::BTreeMap::from([
            ("ID_REPLAY_EDITOR_LENGTH".to_owned(), "Estimated Video Length: {0:time}".to_owned()),
            ("ID_COMMON_METRIC_TIME".to_owned(), "s".to_owned()),
        ]);
        let mut host = ReplayMenuHost { open: true, ..Default::default() };
        host.feed = ReplayScreenFeed {
            instant: true,
            position: 0.5,
            panel: true,
            length_seconds: 61,
            mode: mode::INSTANT,
            head_fractions: (0.0, 1.0),
            ..Default::default()
        };
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "CanDisplayShowHide"), Some(Value::Bool(true)));
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "IsInstantReplayMode"), Some(Value::Bool(true)));
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "GetCursorPosition"), Some(Value::Number(0.5)));
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "GetEncodedLength"), Some(Value::Text("Estimated Video Length: 61 s".into())));
        assert_eq!(host.call(&mut vm, &language, "Game", "IsInstantReplayMode"), None);
        host.post_changes();
        assert_eq!(
            std::mem::take(&mut host.states),
            vec![movie_state::MOVE_START, movie_state::MOVE_END, movie_state::CURSOR],
            "opens paused with its panel shown, the in / out markers placed"
        );
        host.post_changes();
        assert!(host.states.is_empty(), "nothing changed");
        host.feed.playing = true;
        host.feed.position = 0.6;
        host.post_changes();
        assert_eq!(std::mem::take(&mut host.states), vec![movie_state::PLAY, movie_state::CURSOR]);
        host.feed.panel = false;
        host.post_changes();
        assert_eq!(std::mem::take(&mut host.states), vec![movie_state::HIDE]);
        host.feed.panel = true;
        host.post_changes();
        host.states.clear();
        // The editor: A -> timeline (21, 9, over a clip), effects (22, 4), a row move (5), a value change (6), a camera
        // edit (its state), a zoom change (11), back to the effects (4), B back to the timeline (21, 9).
        for (mode, camera_state, row, speed, zoom, expect) in [
            (mode::TIMELINE, 0, 0u32, 3i32, "", vec![movie_state::SHOW_TOGGLE, movie_state::MAIN_EDIT, movie_state::OVER_CLIP]),
            (mode::EFFECTS, 0, 0, 3, "", vec![movie_state::HIDE_TOGGLE, movie_state::EFFECTS]),
            (mode::EFFECTS, 0, 7, 3, "", vec![movie_state::REFRESH_EFFECTS]),
            (mode::EFFECTS, 0, 7, 4, "", vec![movie_state::UPDATE_EFFECTS]),
            (mode::CAMERA, 7, 7, 4, "Zoom 35 mm", vec![7]),
            (mode::CAMERA, 7, 7, 4, "Zoom 50 mm", vec![movie_state::UPDATE_ZOOM]),
            (mode::EFFECTS, 0, 7, 4, "", vec![movie_state::EFFECTS]),
            (mode::TIMELINE, 0, 7, 4, "", vec![movie_state::SHOW_TOGGLE, movie_state::MAIN_EDIT, movie_state::OVER_CLIP]),
        ] {
            (host.feed.mode, host.feed.camera_state, host.feed.row, host.feed.speed) = (mode, camera_state, row, speed);
            host.feed.zoom = zoom.into();
            host.post_changes();
            assert_eq!(std::mem::take(&mut host.states), expect, "mode {mode} state {camera_state} row {row} speed {speed}");
        }
        host.feed.over = Over::Marker;
        host.post_changes();
        assert_eq!(std::mem::take(&mut host.states), vec![movie_state::OVER_CLIP_MARKER]);
        // Moving the in point: its state, the playhead answering the head (the movie places the marker from it).
        host.feed.head_fractions = (0.25, 1.0);
        host.post_changes();
        assert_eq!(std::mem::take(&mut host.states), vec![movie_state::MOVE_START]);
        host.posting = Some(movie_state::MOVE_START);
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "GetPlayheadPosition"), Some(Value::Number(0.25)));
        host.posting = Some(movie_state::MOVE_END);
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "GetPlayheadPosition"), Some(Value::Number(1.0)));
        host.posting = None;
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "GetPlayheadPosition"), Some(Value::Number(0.6)));
        host.open = false;
        assert_eq!(host.call(&mut vm, &language, "FEReplayMenu", "Ready"), None, "closed: not ours");
    }

    #[test]
    fn the_timeline_strip_draws_bands_and_keys_in_the_clip_space() {
        use crate::fe::apt_scene::{Draw, Vertex};
        let v = |x: f32, y: f32| Vertex { position: [x, y], uv: [0.0, 0.0] };
        // The clip at (100, 500), unit x and y (scale 1).
        let placeholder = Draw { texture: crate::fe::apt_scene::REPLAY_TIMELINE.into(), vertices: vec![v(100., 500.), v(101., 500.), v(100., 501.)], multiply: [1.0; 4], add: [0.0; 4], fill: crate::fe::apt_scene::Fill::TextureClamped };
        let mut feed = ReplayScreenFeed {
            mode: mode::TIMELINE,
            position: 0.25,
            keys: vec![0.5],
            segments: vec![(0.0, 0), (0.5, 3)],
            head_fractions: (0.0, 0.75),
            ..Default::default()
        };
        let draws = timeline_draws(&placeholder, &feed);
        assert_eq!(draws.len(), 3, "two bands and a key");
        assert_eq!(draws[1].vertices[0].position, [100.0 + 0.5 * 817.0, 503.0]);
        assert_eq!(draws[1].vertices[1].position[0], 100.0 + 0.75 * 817.0, "the last band ends at the Out point");
        assert!(draws[2].texture.ends_with("texture/key_default"));
        feed.instant = true;
        assert!(timeline_draws(&placeholder, &feed).is_empty(), "nothing in instant mode");
    }
}
