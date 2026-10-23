//! Replay: the sim thread records every tick's presentation snapshot and audio state into a
//! ring; the main thread plays them back (rewind, scrub, speed) without running the simulation.
//!
//! Nothing here re-simulates: `Session` cannot be cloned, and seeking back by re-running the
//! inputs costs half the time skipped. A frame is the tick's `current` snapshot; the previous
//! frame is the `previous` one, so playback interpolates between neighbours.
#![cfg_attr(not(target_os = "vita"), allow(dead_code))]
use std::sync::Mutex;

use glam::Mat4;
use skate_sim::live::{Session, Snapshot, Transform};

use crate::replay_camera::{self, BASE_FOV, Cam, ClipCamera, Kind};

/// Ticks kept: 60 s at 60 Hz. With 35 bones that is about 8 MB, allocated once.
pub const CAPACITY: usize = 3600;

#[derive(Clone, Copy)]
pub struct Frame {
    pub root: Transform,
    pub camera: Transform,
    pub fov: f32,
    /// The session's camera-cut counter at this tick: a change means no interpolation.
    pub cuts: u64,
    pub riding: skate_sim::AudioRiding,
    /// The session tick this frame is (the world replay's clock); `u64::MAX` for a loaded replay (no world).
    pub tick: u64,
}

pub struct Recorder {
    frames: Vec<Frame>,
    bones: Vec<Mat4>,
    nbones: usize,
    /// Ring: physical index of the oldest frame, and the number kept.
    head: usize,
    len: usize,
    /// Seconds of one tick.
    pub period: f32,
}

pub static RECORDER: Mutex<Recorder> = Mutex::new(Recorder::new());

impl Recorder {
    pub const fn new() -> Self {
        Self { frames: Vec::new(), bones: Vec::new(), nbones: 0, head: 0, len: 0, period: 1.0 / 60.0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Empties the recorder for frames of `nbones` bones (a loaded replay).
    fn reset(&mut self, nbones: usize, period: f32) {
        if self.nbones != nbones || self.bones.is_empty() {
            self.nbones = nbones;
            self.bones = vec![Mat4::IDENTITY; CAPACITY * nbones];
            self.frames = Vec::with_capacity(CAPACITY);
        }
        self.frames.clear();
        (self.head, self.len, self.period) = (0, 0, period);
    }

    /// Appends a whole frame (a loaded replay's).
    fn push_frame(&mut self, frame: Frame, bones: &[Mat4]) {
        if self.len >= CAPACITY {
            return;
        }
        let slot = self.physical(self.len);
        self.len += 1;
        self.frames.push(frame);
        self.bones[slot * self.nbones..(slot + 1) * self.nbones].copy_from_slice(bones);
    }

    /// Forgets the frames (a new session); the buffers stay.
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
        self.frames.clear();
    }

    fn physical(&self, logical: usize) -> usize {
        (self.head + logical) % CAPACITY
    }

    /// Appends a tick: `current` is the snapshot just produced.
    pub fn push(&mut self, current: &Snapshot, cuts: u64, period: f32, tick: u64) {
        if self.nbones != current.bones.len() || self.bones.is_empty() {
            self.nbones = current.bones.len();
            self.bones = vec![Mat4::IDENTITY; CAPACITY * self.nbones];
            self.frames = Vec::with_capacity(CAPACITY);
            self.head = 0;
            self.len = 0;
        }
        self.period = period;
        let riding = self.newest().map(|f| f.riding).unwrap_or_default();
        let frame = Frame { root: current.root, camera: current.camera, fov: current.fov, cuts, riding, tick };
        let slot = if self.len < CAPACITY {
            let slot = self.physical(self.len);
            self.len += 1;
            slot
        } else {
            let slot = self.head;
            self.head = (self.head + 1) % CAPACITY;
            slot
        };
        if slot < self.frames.len() {
            self.frames[slot] = frame;
        } else {
            self.frames.push(frame);
        }
        self.bones[slot * self.nbones..(slot + 1) * self.nbones].copy_from_slice(&current.bones);
    }

    fn newest(&self) -> Option<&Frame> {
        (self.len > 0).then(|| &self.frames[self.physical(self.len - 1)])
    }

    /// The newest frame's audio state, from the host pass that follows the tick.
    pub fn set_riding(&mut self, riding: skate_sim::AudioRiding) {
        if self.len > 0 {
            let slot = self.physical(self.len - 1);
            self.frames[slot].riding = riding;
        }
    }

    pub fn frame(&self, logical: usize) -> &Frame {
        &self.frames[self.physical(logical.min(self.len - 1))]
    }

    /// Writes frame `logical` into `to` (buffers reused).
    pub fn fill(&self, logical: usize, to: &mut Snapshot) {
        let logical = logical.min(self.len - 1);
        let slot = self.physical(logical);
        let f = &self.frames[slot];
        to.root = f.root;
        to.camera = f.camera;
        to.fov = f.fov;
        to.bones.clear();
        to.bones.extend_from_slice(&self.bones[slot * self.nbones..(slot + 1) * self.nbones]);
    }

    /// The world replay's time at position `pos` (in ticks): None for a loaded replay or a camera cut between the
    /// two frames (the newer one shows, as in [`Self::pair`]).
    pub fn world_time(&self, pos: f32) -> Option<f64> {
        let last = self.len.checked_sub(1)?;
        let i = (pos.max(0.0) as usize).min(last);
        let j = (i + 1).min(last);
        let (a, b) = (self.frame(i), self.frame(j));
        if a.tick == u64::MAX || b.tick == u64::MAX {
            return None;
        }
        let alpha = if a.cuts != b.cuts {
            1.0
        } else if i == j {
            0.0
        } else {
            (pos.max(0.0) - i as f32).clamp(0.0, 1.0)
        };
        Some((a.tick as f64 + f64::from(alpha) * (b.tick as f64 - a.tick as f64)) * f64::from(self.period))
    }

    /// The pair to draw at position `pos` (in ticks) and the blend between them.
    pub fn pair(&self, pos: f32, a: &mut Snapshot, b: &mut Snapshot) -> f32 {
        let last = self.len - 1;
        let i = (pos.max(0.0) as usize).min(last);
        let j = (i + 1).min(last);
        let alpha = if i == j { 1.0 } else { pos.max(0.0) - i as f32 };
        self.fill(i, a);
        self.fill(j, b);
        // A camera cut or teleport between them: show the new one.
        if self.frame(i).cuts != self.frame(j).cuts {
            self.fill(j, a);
            return 1.0;
        }
        alpha.clamp(0.0, 1.0)
    }
}

/// A saved clip speed: percent (10..=100), or P1's earlier setting 1..=5 (25 %, 50 %, then normal and the fast ones,
/// which retail does not have).
fn saved_speed(b: u8) -> u8 {
    match b {
        0 => SPEED_MAX,
        1 => 30,
        2 => 50,
        3..=5 => SPEED_MAX,
        v => (v.clamp(SPEED_MIN, SPEED_MAX) + SPEED_STEP / 2) / SPEED_STEP * SPEED_STEP,
    }
}

/// Called by the sim thread after each tick.
pub fn record(session: &Session) {
    if let Some((_, current)) = session.presentation() {
        RECORDER.lock().unwrap().push(current, session.presentation_cuts(), session.period().as_secs_f32(), session.ticks());
    }
}

/// Called by the sim thread with the audio state the host pass took after the ticks.
pub fn record_riding(riding: skate_sim::AudioRiding) {
    RECORDER.lock().unwrap().set_riding(riding);
}

/// A new session starts: the old frames belong to another world.
pub fn clear() {
    RECORDER.lock().unwrap().clear();
}

/// Held LT or RT: wind this fast (ticks per tick).
const WIND: f32 = 8.0;
/// A stick past this (of 0..1) is a deliberate push.
const DEAD: f32 = 0.25;
/// Retail's clip speed (research b398, b461): 10 % to 100 % in steps of 10 (slow motion only), 100 % by default.
const SPEED_STEP: u8 = 10;
const SPEED_MIN: u8 = 10;
const SPEED_MAX: u8 = 100;
/// Camera shake levels (off, low, medium, high): metres of offset.
const SHAKE: [f32; 4] = [0.0, 0.01, 0.025, 0.05];
/// The effects menu entries in the screen's order (`ReplayEditor.mAdvancedList`, `kEdit*`).
const EDIT_FOLLOW: usize = 0;
const EDIT_TRIPOD: usize = 1;
const EDIT_GAME: usize = 2;
const EDIT_FREE: usize = 3;
const EDIT_ADVANCED_FOLLOW: usize = 4;
const EDIT_ADVANCED_TRIPOD: usize = 5;
const EDIT_ADVANCED_FREE: usize = 6;
const EDIT_SPEED: usize = 7;
const EDIT_SHAKE: usize = 8;
const EDIT_ENTRIES: usize = 9;
/// The Finished menu's items (the retail popup's localization ids).
pub const FINISH_ITEMS: [&str; 3] = ["ID_REPLAYEDITOR_UPLOAD", "ID_REPLAYEDITOR_BASIC_SAVE_REPLAY", "ID_REPLAYEDITOR_EXIT"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Instant replay: watch and scrub the whole recording.
    Scrub,
    /// Edit timeline: the In and Out points trim the video, markers divide it into clips.
    Main,
    /// Edit clip: the effects menu (cameras, speed, shake).
    Effects,
    /// Editing a clip's camera (the kind and whether it is the advanced, keyed version).
    Camera(Kind, bool),
}

/// What one clip (the stretch between two markers) is set to.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipFx {
    pub camera: ClipCamera,
    /// The speed in percent (10..=100, steps of 10) and the shake level (0..=3).
    pub speed: u8,
    pub shake: u8,
}

impl Default for ClipFx {
    fn default() -> Self {
        Self { camera: ClipCamera::game(), speed: SPEED_MAX, shake: 0 }
    }
}

/// Markers and keys closer than this (ticks) to the playhead count as under it.
const OVER: f32 = 4.0;

/// What is under the playhead on the timeline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    Clip,
    Marker,
    Key,
}

/// A camera being edited: the setting, the clip's camera before (to cancel to), the aim point
/// the Tripod orbits, and whether the sticks changed it since the last key or scrub.
struct CameraEdit {
    cam: Cam,
    before: ClipCamera,
    anchor: glam::Vec3,
    dirty: bool,
}

pub struct Playback {
    /// Position in ticks from the oldest kept frame.
    pub pos: f32,
    pub mode: Mode,
    pub paused: bool,
    /// The In and Out points, in ticks.
    pub start: f32,
    pub end: f32,
    /// Clip markers between them, sorted; `clips` has one entry more.
    pub markers: Vec<f32>,
    pub clips: Vec<ClipFx>,
    /// The clip the effects menu edits, and its highlighted entry.
    pub edit_clip: usize,
    /// The effects menu's live speed and shake (retail `82508930` loads them when the menu opens; Left / Right step only
    /// these): written into the clip by the commit (B, or A leaving a camera edit, `82507438`).
    live: (u8, u8),
    pub selection: usize,
    /// The editor panel is hidden (the picture only).
    pub hidden: bool,
    /// The Finished menu (Upload, Save replay, Exit) when open: the highlighted item.
    pub finish: Option<usize>,
    /// X was pressed: the picture should be saved as a photo (the host takes it).
    pub photo: bool,
    /// Counts jumps, so the audio host resets.
    pub seeks: u64,
    winding: bool,
    editing: Option<CameraEdit>,
    /// The instant replay's orbit (yaw, pitch) of the camera of play around the skater: the right stick.
    orbit: [f32; 2],
    /// The instant replay's zoom (a factor on the field of view): the left stick.
    zoom: f32,
    /// The skater's position under the playhead (for a key edit's Tripod orbit).
    skater: glam::Vec3,
}

/// What a frame of playback asks of the audio host.
pub struct Step {
    pub steps: u32,
    pub silenced: bool,
}

/// One frame's controls in the Xbox 360 layout the game maps the Vita to (`controls::map`): the
/// buttons held and those that went down this frame (`controls::xbox` bits), LT and RT held, the
/// sticks as -1..1 with up positive.
pub struct Controls {
    pub held: u16,
    pub pressed: u16,
    pub lt: bool,
    pub rt: bool,
    pub left: [f32; 2],
    pub right: [f32; 2],
}

pub enum Outcome {
    Exit,
    /// The Save replay item of the Finished menu.
    Save,
    Frame(Step),
}

/// The skater's travel over half a second around a tick (for the Follow camera).
fn travel(rec: &Recorder, tick: usize) -> glam::Vec3 {
    let last = rec.len() - 1;
    let (a, b) = (tick.saturating_sub(15), (tick + 15).min(last));
    rec.frame(b).root.translation - rec.frame(a).root.translation
}

impl Playback {
    /// Starts 10 s before the end (or at the start).
    pub fn new(rec: &Recorder) -> Option<Self> {
        (rec.len() > 1).then(|| Self::with(rec.len().saturating_sub(600) as f32, (rec.len() - 1) as f32, Vec::new(), vec![ClipFx::default()]))
    }

    /// A loaded replay: the whole recording is the clip, with its markers and clips.
    pub fn loaded(rec: &Recorder, markers: Vec<f32>, clips: Vec<ClipFx>) -> Option<Self> {
        (rec.len() > 1 && clips.len() == markers.len() + 1).then(|| {
            let mut pb = Self::with(0.0, (rec.len() - 1) as f32, markers, clips);
            pb.mode = Mode::Main;
            pb
        })
    }

    fn with(pos: f32, end: f32, markers: Vec<f32>, clips: Vec<ClipFx>) -> Self {
        Self {
            pos,
            mode: Mode::Scrub,
            paused: false,
            start: 0.0,
            end,
            markers,
            clips,
            edit_clip: 0,
            live: (SPEED_MAX, 0),
            selection: EDIT_GAME,
            hidden: false,
            finish: None,
            photo: false,
            seeks: 0,
            winding: false,
            editing: None,
            orbit: [0.0; 2],
            zoom: 1.0,
            skater: glam::Vec3::ZERO,
        }
    }

    /// The position, start and end as fractions of the timeline.
    pub fn fractions(&self, rec: &Recorder) -> [f32; 3] {
        let last = (rec.len() - 1).max(1) as f32;
        [self.pos / last, self.start / last, self.end / last]
    }

    /// The clip markers and the camera keys as fractions of the timeline.
    pub fn marker_fractions(&self, rec: &Recorder) -> (Vec<f32>, Vec<f32>) {
        let last = (rec.len() - 1).max(1) as f32;
        let keys = self.clips.iter().filter(|c| c.camera.advanced).flat_map(|c| c.camera.keys.iter().map(|(t, _)| t / last)).collect();
        (self.markers.iter().map(|m| m / last).collect(), keys)
    }

    /// The trimmed video's length in seconds.
    pub fn clip_seconds(&self, rec: &Recorder) -> f32 {
        (self.end - self.start) * rec.period
    }

    /// The clip under a position.
    fn clip_at(&self, pos: f32) -> usize {
        self.markers.iter().filter(|&&m| m <= pos).count().min(self.clips.len() - 1)
    }

    /// What the playhead is over: a clip marker, a camera key of its clip, or the clip.
    pub fn over(&self) -> Over {
        if self.markers.iter().any(|m| (m - self.pos).abs() <= OVER) {
            return Over::Marker;
        }
        let clip = &self.clips[self.clip_at(self.pos)].camera;
        if clip.advanced && clip.keys.iter().any(|(t, _)| (t - self.pos).abs() <= OVER) {
            return Over::Key;
        }
        Over::Clip
    }

    /// The settings the effects menu shows and edits.
    pub fn edit_fx(&self) -> &ClipFx {
        &self.clips[self.edit_clip.min(self.clips.len() - 1)]
    }

    /// The settings that apply to the picture now: none while watching the instant replay.
    fn playing_fx(&self) -> Option<&ClipFx> {
        match self.mode {
            Mode::Scrub => None,
            Mode::Effects | Mode::Camera(..) => Some(self.edit_fx()),
            Mode::Main => Some(&self.clips[self.clip_at(self.pos)]),
        }
    }

    pub fn speed(&self) -> f32 {
        let speed = match self.mode {
            Mode::Effects | Mode::Camera(..) => Some(self.live.0),
            _ => self.playing_fx().map(|f| f.speed),
        };
        f32::from(speed.unwrap_or(SPEED_MAX).clamp(SPEED_MIN, SPEED_MAX)) / 100.0
    }

    /// The effects menu's values loaded from the clip it edits (the menu opens).
    fn load_live(&mut self) {
        let fx = self.edit_fx();
        self.live = (fx.speed, fx.shake);
    }

    /// The commit (`82507438`, base game): the live speed and shake into the clip under the playhead, unless a clip
    /// marker is under it. Ours: with no markers yet the clip is the whole replay and the commit lands there (retail
    /// loses a commit on a lane without keys, b465, medium-high confidence; user decision 2026-10-22).
    fn commit_live(&mut self) {
        if self.over() == Over::Marker {
            return;
        }
        let index = self.edit_clip.min(self.clips.len() - 1);
        (self.clips[index].speed, self.clips[index].shake) = self.live;
    }

    /// The encoded length (retail `8250B590`, b472): per clip, its 30 fps frames inside the In and Out points divided
    /// by its speed, truncated, summed; seconds = trunc(sum / 30 + 0.5).
    pub fn encoded_seconds(&self, rec: &Recorder) -> u32 {
        let b = self.boundaries();
        let frames: u32 = (0..self.clips.len())
            .map(|i| {
                let (from, to) = (b[i].max(self.start), b[i + 1].min(self.end));
                let speed = if Some(i) == self.editing_clip() { self.live.0 } else { self.clips[i].speed };
                let f = ((to - from).max(0.0) * rec.period * 30.0) / (f32::from(speed.clamp(SPEED_MIN, SPEED_MAX)) * 0.01);
                f as u32
            })
            .sum();
        (frames as f32 / 30.0 + 0.5) as u32
    }

    /// The clip whose live values show (the effects menu or a camera edit open).
    fn editing_clip(&self) -> Option<usize> {
        matches!(self.mode, Mode::Effects | Mode::Camera(..)).then(|| self.edit_clip.min(self.clips.len() - 1))
    }

    /// The zoom text of the camera edit screens.
    pub fn zoom_text(&self) -> String {
        replay_camera::zoom_text(self.editing.as_ref().map_or(BASE_FOV, |e| e.cam.fov))
    }

    /// The screen state (`ReplayEditor.k*Mode`) of the mode.
    pub fn screen_state(&self) -> f64 {
        match self.mode {
            Mode::Scrub => 0.0,
            Mode::Main => 9.0,
            Mode::Effects => 4.0,
            Mode::Camera(Kind::Game, _) => 10.0,
            Mode::Camera(Kind::Follow, false) => 1.0,
            Mode::Camera(Kind::Tripod, false) => 2.0,
            Mode::Camera(Kind::Free, false) => 7.0,
            Mode::Camera(Kind::Follow, true) => 12.0,
            Mode::Camera(Kind::Tripod, true) => 13.0,
            Mode::Camera(Kind::Free, true) => 14.0,
        }
    }

    /// What the retail replay screen on the menu stage shows this frame (skate-ui `replay_menu_host`): the
    /// playhead, the mode, the timeline's clips and keys, the effects values, and the help (`help` open, its page
    /// turns since the replay opened).
    pub fn screen_feed(&self, rec: &Recorder, help: bool, help_turns: u32) -> skate_ui::fe::replay_menu_host::ReplayScreenFeed {
        use skate_ui::fe::replay_menu_host::{mode, Over as ScreenOver, ReplayScreenFeed};
        let last = (rec.len() - 1).max(1) as f32;
        let (markers, camera_keys) = self.marker_fractions(rec);
        let mut keys = markers;
        keys.extend(camera_keys);
        keys.sort_by(f32::total_cmp);
        // The band's camera types: 0 Follow, 1 Tripod, 2 Free, 3 Game.
        let code = |kind: Kind| match kind {
            Kind::Follow => 0,
            Kind::Tripod => 1,
            Kind::Free => 2,
            Kind::Game => 3,
        };
        let fx = self.edit_fx();
        // 30 fps frames of the recording (the heads' unit, b472).
        let frames = |ticks: f32| (ticks * rec.period * 30.0).round().max(0.0) as u32;
        ReplayScreenFeed {
            active: true,
            instant: self.mode == Mode::Scrub,
            position: f64::from(self.pos / last),
            playing: !self.paused,
            panel: !self.hidden,
            length_seconds: if self.mode == Mode::Scrub { self.clip_seconds(rec).round().max(0.0) as u32 } else { self.encoded_seconds(rec) },
            length_valid: true,
            help,
            help_mode: self.help_mode(),
            help_turns,
            mode: match self.mode {
                Mode::Scrub => mode::INSTANT,
                Mode::Main => mode::TIMELINE,
                Mode::Effects => mode::EFFECTS,
                Mode::Camera(..) => mode::CAMERA,
            },
            camera_state: if matches!(self.mode, Mode::Camera(..)) { self.screen_state() as i32 } else { 0 },
            camera_row: self.selection as u32,
            keys,
            segments: self.zones(rec).into_iter().map(|(start, _, kind, _)| (start, code(kind))).collect(),
            heads: (frames(self.start), frames(self.end)),
            head_fractions: (f64::from(self.start / last), f64::from(self.end / last)),
            row: self.selection as u32,
            speed: i32::from(if self.editing_clip().is_some() { self.live.0 } else { fx.speed }),
            shake: i32::from(if self.editing_clip().is_some() { self.live.1 } else { fx.shake }),
            zoom: self.zoom_text(),
            over: match self.over() {
                Over::Clip => ScreenOver::Clip,
                Over::Marker => ScreenOver::Marker,
                Over::Key => ScreenOver::Key,
            },
        }
    }

    /// The kind of camera this frame shows when it is ours to keep out of walls (`replay_camera::constrain`): the clip's
    /// or the edited camera, or the instant replay's orbit (as a Follow); None for the camera of play as recorded (it
    /// collided when it was played).
    pub fn constrained_kind(&self) -> Option<Kind> {
        let Some(fx) = self.playing_fx() else {
            return (self.orbit != [0.0; 2]).then_some(Kind::Follow);
        };
        let kind = match (&self.editing, self.mode) {
            (Some(_), Mode::Camera(kind, _)) => kind,
            _ => fx.camera.kind,
        };
        (kind != Kind::Game).then_some(kind)
    }

    /// The help pages (`replay_help.k*Mode`) of the mode.
    pub fn help_mode(&self) -> i32 {
        match self.mode {
            Mode::Scrub => 0,
            Mode::Main => 1,
            Mode::Effects => 2,
            Mode::Camera(Kind::Follow, false) => 3,
            Mode::Camera(Kind::Game, _) => 4,
            Mode::Camera(Kind::Tripod, false) => 5,
            Mode::Camera(Kind::Free, false) => 6,
            Mode::Camera(Kind::Follow, true) => 7,
            Mode::Camera(Kind::Tripod, true) => 8,
            Mode::Camera(Kind::Free, true) => 9,
        }
    }

    /// The camera of one frame: the clip's (or the one being edited), then the shake.
    fn camera_at(&self, rec: &Recorder, tick: usize, snap: &mut Snapshot) {
        let Some(fx) = self.playing_fx() else {
            // The instant replay: the camera of play, turned about the skater by the orbit.
            let [yaw, pitch] = self.orbit;
            snap.fov *= self.zoom;
            if yaw != 0.0 || pitch != 0.0 {
                let target = snap.root.translation + glam::Vec3::Y;
                let right = snap.camera.rotation * glam::Vec3::X;
                let turn = glam::Quat::from_axis_angle(glam::Vec3::Y, yaw) * glam::Quat::from_axis_angle(right, pitch);
                snap.camera.translation = target + turn * (snap.camera.translation - target);
                snap.camera.rotation = (turn * snap.camera.rotation).normalize();
            }
            return;
        };
        let skater = snap.root.translation;
        let behind = replay_camera::behind(travel(rec, tick), skater, &snap.camera);
        let (kind, cam) = match (&self.editing, self.mode) {
            (Some(e), Mode::Camera(kind, _)) => (kind, e.cam),
            _ => (fx.camera.kind, fx.camera.at(tick as f32)),
        };
        (snap.camera, snap.fov) = replay_camera::eval(kind, &cam, skater, behind, (snap.camera, snap.fov));
        let shake = if self.editing_clip().is_some() { self.live.1 } else { fx.shake };
        let amount = SHAKE[usize::from(shake.min(3))];
        if amount > 0.0 {
            // Smooth, deterministic noise of the frame, so rewinding repeats it.
            let t = tick as f32 * 0.35;
            let n = |k: f32| ((t * k).sin() + (t * k * 2.3 + k).sin() * 0.5) / 1.5;
            snap.camera.translation += glam::Vec3::new(n(7.1), n(9.3), n(5.7)) * amount;
            snap.camera.rotation *= glam::Quat::from_euler(glam::EulerRot::YXZ, n(6.1) * amount * 0.4, n(8.3) * amount * 0.4, 0.0);
        }
    }

    /// Puts the clip's camera and shake over the recorded pair (the ticks either side of the playhead).
    pub fn apply_camera(&self, rec: &Recorder, a: &mut Snapshot, b: &mut Snapshot) {
        let last = rec.len() - 1;
        let i = (self.pos.max(0.0) as usize).min(last);
        self.camera_at(rec, i, a);
        self.camera_at(rec, (i + 1).min(last), b);
    }

    /// The boundaries of the clips: In point, markers, Out point.
    fn boundaries(&self) -> Vec<f32> {
        let mut b = vec![self.start];
        b.extend(&self.markers);
        b.push(self.end);
        b
    }

    fn jump(&mut self, pos: f32) {
        self.pos = pos;
        self.seeks += 1;
    }

    /// Edits the camera key under the playhead (A on a key in the timeline).
    fn edit_key(&mut self) {
        let index = self.clip_at(self.pos);
        let camera = self.clips[index].camera.clone();
        let Some(&(tick, cam)) = camera.keys.iter().find(|(t, _)| (t - self.pos).abs() <= OVER) else { return };
        self.edit_clip = index;
        self.pos = tick;
        self.paused = true;
        let anchor = glam::Vec3::Y * cam.aim + self.skater_near(tick);
        self.editing = Some(CameraEdit { cam, before: camera.clone(), anchor, dirty: false });
        self.mode = Mode::Camera(camera.kind, true);
    }

    /// The skater's recorded position at a tick (filled in by `update`, which has the recorder).
    fn skater_near(&self, _tick: f32) -> glam::Vec3 {
        self.skater
    }

    /// The timeline's clip zones in the timeline editor: (start, end) as fractions, the clip's
    /// camera, and whether the playhead is in it.
    pub fn zones(&self, rec: &Recorder) -> Vec<(f32, f32, Kind, bool)> {
        let last = (rec.len() - 1).max(1) as f32;
        let b = self.boundaries();
        let here = self.clip_at(self.pos);
        (0..self.clips.len()).map(|i| (b[i] / last, b[i + 1] / last, self.clips[i].camera.kind, i == here)).collect()
    }

    /// Starts editing the clip's camera of a kind from the effects menu.
    fn start_camera(&mut self, rec: &Recorder, kind: Kind, advanced: bool) {
        let index = self.edit_clip.min(self.clips.len() - 1);
        let before = self.clips[index].camera.clone();
        let tick = (self.pos as usize).min(rec.len() - 1);
        let frame = rec.frame(tick);
        let skater = frame.root.translation;
        let behind = replay_camera::behind(travel(rec, tick), skater, &frame.camera);
        // Editing the camera the clip has: from where it is; another kind starts at the camera of play.
        let cam = if before.kind == kind && before.advanced == advanced && (kind == Kind::Game || before != ClipCamera::game()) {
            before.at(self.pos)
        } else {
            Cam::starting(kind, &frame.camera, frame.fov, skater, behind)
        };
        if advanced {
            if !(before.kind == kind && before.advanced) {
                self.clips[index].camera = ClipCamera { kind, advanced: true, cam, keys: Vec::new() };
            }
            // Keys are set where the playhead stands: hold still to set the first one.
            self.paused = true;
        }
        self.editing = Some(CameraEdit { cam, before, anchor: skater + glam::Vec3::Y * cam.aim, dirty: false });
        self.mode = Mode::Camera(kind, advanced);
    }

    /// One main-loop frame of `dt` seconds. The controls are Skate 3's: RB plays and pauses, LT and
    /// RT wind, the D-pad skips and (in the timeline) places and removes markers, LB with the left or
    /// right stick drags the In or Out point, A goes in, B comes out (B on the timeline finishes), L3 hides.
    pub fn update(&mut self, rec: &Recorder, dt: f32, c: &Controls) -> Outcome {
        use vita_platform::controls::xbox;
        let last = (rec.len() - 1) as f32;
        let (pressed, held) = (c.pressed, c.held);
        self.skater = rec.frame((self.pos as usize).min(rec.len() - 1)).root.translation;
        let frozen = |silenced: bool| Outcome::Frame(Step { steps: 0, silenced });
        if let Some(item) = self.finish {
            // The Finished menu: Upload Video (there is no upload on the Vita), Save Replay, Exit.
            if pressed & xbox::DPAD_UP != 0 {
                self.finish = Some((item + FINISH_ITEMS.len() - 1) % FINISH_ITEMS.len());
            }
            if pressed & xbox::DPAD_DOWN != 0 {
                self.finish = Some((item + 1) % FINISH_ITEMS.len());
            }
            if pressed & xbox::B != 0 {
                self.finish = None;
            } else if pressed & xbox::A != 0 {
                match item {
                    0 => *SAVE_STATUS.lock().unwrap() = "Uploading is not available on the Vita.".into(),
                    1 => return Outcome::Save,
                    _ => return Outcome::Exit,
                }
            }
            return frozen(true);
        }
        let mut marker_moved = false;
        let mut camera_wind = false;
        match self.mode {
            Mode::Scrub => {
                if pressed & xbox::B != 0 {
                    self.finish = Some(0);
                } else if pressed & xbox::A != 0 {
                    self.mode = Mode::Main;
                    self.pos = self.pos.clamp(self.start, self.end);
                    self.orbit = [0.0; 2];
                    self.zoom = 1.0;
                }
                // The right stick orbits the camera of play around the skater (it still follows).
                let dead = |v: f32| if v.abs() > 0.15 { v } else { 0.0 };
                self.orbit[0] -= dead(c.right[0]) * 1.6 * dt;
                self.orbit[1] = (self.orbit[1] - dead(c.right[1]) * 1.2 * dt).clamp(-1.0, 0.9);
                // The left stick zooms: up in, down out.
                self.zoom = (self.zoom * (1.0 - dead(c.left[1]) * 0.9 * dt)).clamp(0.3, 1.6);
                // The D-pad skips to the start and the end of the recording.
                if pressed & xbox::DPAD_LEFT != 0 {
                    self.jump(0.0);
                }
                if pressed & xbox::DPAD_RIGHT != 0 {
                    self.jump(last);
                }
            }
            Mode::Main => {
                if pressed & xbox::B != 0 {
                    self.finish = Some(0);
                } else if pressed & xbox::A != 0 {
                    match self.over() {
                        Over::Marker => {}
                        Over::Key => self.edit_key(),
                        Over::Clip => {
                            self.edit_clip = self.clip_at(self.pos);
                            self.mode = Mode::Effects;
                            self.load_live();
                        }
                    }
                }
                // Down places a clip marker at the playhead, up removes the marker (or the camera
                // key) under it; the clips either side keep their settings (a split copies them,
                // a merge keeps the earlier).
                if pressed & xbox::DPAD_DOWN != 0 && self.pos > self.start + OVER && self.pos < self.end - OVER && self.over() != Over::Marker {
                    let at = self.clip_at(self.pos);
                    self.markers.insert(at, self.pos);
                    let fx = self.clips[at].clone();
                    self.clips.insert(at, fx);
                }
                if pressed & xbox::DPAD_UP != 0 {
                    match self.over() {
                        Over::Marker => {
                            if let Some(i) = self.markers.iter().position(|m| (m - self.pos).abs() <= OVER) {
                                self.markers.remove(i);
                                self.clips.remove(i + 1);
                            }
                        }
                        Over::Key => {
                            let index = self.clip_at(self.pos);
                            let pos = self.pos;
                            self.clips[index].camera.keys.retain(|(t, _)| (t - pos).abs() > OVER);
                        }
                        Over::Clip => {}
                    }
                }
                // Left and right skip to the previous and next marker (the In and Out points when
                // there is none).
                let b = self.boundaries();
                if pressed & xbox::DPAD_LEFT != 0 {
                    let to = b.iter().rev().copied().find(|&m| m < self.pos - OVER / 2.0).unwrap_or(self.start);
                    self.jump(to);
                }
                if pressed & xbox::DPAD_RIGHT != 0 {
                    let to = b.iter().copied().find(|&m| m > self.pos + OVER / 2.0).unwrap_or(self.end);
                    self.jump(to);
                }
                // LB held: the left stick drags the In point, the right stick the Out point.
                if held & xbox::LEFT_SHOULDER != 0 {
                    let rate = |x: f32| x.signum() * (x.abs() - DEAD).max(0.0).powi(2) * 1.5 * last * dt;
                    // Each head stays OVER from its neighbour as it is now: the Out point reads the In point
                    // already moved, so both sticks together cannot cross them (the device crashed on In
                    // past Out).
                    if c.left[0].abs() > DEAD {
                        let first = self.markers.first().copied().unwrap_or(self.end);
                        self.start = (self.start + rate(c.left[0])).clamp(0.0, (first - OVER).max(0.0));
                        marker_moved = true;
                    }
                    if c.right[0].abs() > DEAD {
                        let lastm = self.markers.last().copied().unwrap_or(self.start);
                        self.end = (self.end + rate(c.right[0])).clamp((lastm + OVER).min(last), last);
                        marker_moved = true;
                    }
                }
            }
            Mode::Effects => {
                if pressed & xbox::DPAD_UP != 0 {
                    self.selection = (self.selection + EDIT_ENTRIES - 1) % EDIT_ENTRIES;
                }
                if pressed & xbox::DPAD_DOWN != 0 {
                    self.selection = (self.selection + 1) % EDIT_ENTRIES;
                }
                // Left / Right step only the live values (`8250ADF8` / `8250AB28`); B commits them.
                let step = i16::from(pressed & xbox::DPAD_RIGHT != 0) - i16::from(pressed & xbox::DPAD_LEFT != 0);
                let live = &mut self.live;
                match self.selection {
                    EDIT_SPEED => live.0 = (i16::from(live.0) + step * i16::from(SPEED_STEP)).clamp(i16::from(SPEED_MIN), i16::from(SPEED_MAX)) as u8,
                    EDIT_SHAKE => live.1 = (i16::from(live.1) + step).clamp(0, 3) as u8,
                    _ => {}
                }
                if pressed & xbox::A != 0 {
                    match self.selection {
                        EDIT_SPEED => live.0 = if live.0 >= SPEED_MAX { SPEED_MIN } else { live.0 + SPEED_STEP },
                        EDIT_SHAKE => live.1 = (live.1 + 1) % 4,
                        EDIT_FOLLOW => self.start_camera(rec, Kind::Follow, false),
                        EDIT_TRIPOD => self.start_camera(rec, Kind::Tripod, false),
                        EDIT_GAME => self.start_camera(rec, Kind::Game, false),
                        EDIT_FREE => self.start_camera(rec, Kind::Free, false),
                        EDIT_ADVANCED_FOLLOW => self.start_camera(rec, Kind::Follow, true),
                        EDIT_ADVANCED_TRIPOD => self.start_camera(rec, Kind::Tripod, true),
                        _ => self.start_camera(rec, Kind::Free, true),
                    }
                } else if pressed & xbox::B != 0 {
                    self.commit_live();
                    self.mode = Mode::Main;
                }
            }
            Mode::Camera(kind, advanced) => {
                let index = self.edit_clip.min(self.clips.len() - 1);
                let input = replay_camera::Input {
                    left: c.left,
                    right: c.right,
                    up: held & xbox::DPAD_UP != 0,
                    down: held & xbox::DPAD_DOWN != 0,
                    zoom_in: held & xbox::DPAD_RIGHT != 0,
                    zoom_out: held & xbox::DPAD_LEFT != 0,
                    fast: held & xbox::LEFT_SHOULDER != 0,
                };
                let pos = self.pos;
                let edit = self.editing.as_mut().expect("a camera edit");
                if replay_camera::edit(kind, &mut edit.cam, dt, &input, edit.anchor) {
                    edit.dirty = true;
                }
                if advanced {
                    // Commit (A) sets a key marker where the playhead stands and goes back to the
                    // timeline; LT and RT move along the clip first (the camera shows the keys
                    // there until the sticks change it); B cancels.
                    camera_wind = true;
                    let clip = &mut self.clips[index].camera;
                    if !edit.dirty && !clip.keys.is_empty() && (c.lt || c.rt || !self.paused) {
                        edit.cam = clip.at(pos);
                    }
                    if pressed & xbox::A != 0 {
                        clip.set_key(pos, edit.cam, OVER);
                        self.editing = None;
                        self.mode = Mode::Main;
                        self.commit_live();
                    } else if pressed & xbox::B != 0 {
                        *clip = edit.before.clone();
                        self.editing = None;
                        self.mode = Mode::Effects;
                    }
                } else if pressed & xbox::A != 0 {
                    self.clips[index].camera = ClipCamera { kind, advanced: false, cam: edit.cam, keys: Vec::new() };
                    self.editing = None;
                    self.mode = Mode::Main;
                    self.commit_live();
                } else if pressed & xbox::B != 0 {
                    self.clips[index].camera = edit.before.clone();
                    self.editing = None;
                    self.mode = Mode::Effects;
                }
            }
        }
        if pressed & xbox::X != 0 && matches!(self.mode, Mode::Scrub | Mode::Main) {
            self.photo = true;
        }
        if pressed & xbox::RIGHT_SHOULDER != 0 {
            self.paused = !self.paused;
        }
        if pressed & xbox::LEFT_THUMB != 0 {
            self.hidden = !self.hidden;
        }
        // Editing a clip stays inside it; the timeline plays between the In and Out points (the
        // instant replay over everything).
        let (lo, hi) = match self.mode {
            Mode::Scrub => (0.0, last),
            Mode::Main => (self.start, self.end),
            Mode::Effects | Mode::Camera(..) => {
                let b = self.boundaries();
                let i = self.edit_clip.min(b.len() - 2);
                (b[i], b[i + 1])
            }
        };
        // LT winds back, RT forward (not while the sticks drag the markers or move a basic camera).
        let wind_allowed = !marker_moved && (camera_wind || !matches!(self.mode, Mode::Camera(..)));
        let wind = if !wind_allowed {
            None
        } else if c.rt {
            Some(WIND)
        } else if c.lt {
            Some(-WIND)
        } else {
            None
        };
        if wind.is_some() != self.winding {
            self.seeks += 1;
            self.winding = wind.is_some();
        }
        let before = self.pos.clamp(lo, hi);
        let rate = match wind {
            Some(r) => r,
            None if self.paused => 0.0,
            None => self.speed(),
        };
        self.pos = (before + rate * dt / rec.period).clamp(lo, hi);
        if wind.is_none() && !self.paused && self.pos >= hi && rate > 0.0 {
            if matches!(self.mode, Mode::Effects | Mode::Camera(..)) {
                // Editing a clip: loop it so the change can be seen.
                self.pos = lo;
                self.seeks += 1;
            } else if self.mode == Mode::Main {
                self.paused = true;
            }
        }
        let moved = self.pos - before;
        Outcome::Frame(Step {
            steps: if rate > 0.0 && wind.is_none() { self.pos.floor() as u32 - before.floor() as u32 } else { 0 },
            silenced: moved <= 0.0 || wind.is_some() || (self.speed() - 1.0).abs() > 1e-3,
        })
    }
}

/// Where replays are saved.
pub const DIR: &str = "ux0:data/SKATEVITA/replays";
const MAGIC: &[u8; 4] = b"SKRP";
const VERSION: u32 = 2;

/// Where photos are saved.
pub const PHOTO_DIR: &str = "ux0:data/SKATEVITA/photos";

/// Writes a screenshot as `photos/photo-N.bmp` on a thread; returns its name.
pub fn save_photo(bmp: Vec<u8>) -> Result<String, String> {
    std::fs::create_dir_all(PHOTO_DIR).map_err(|e| format!("{PHOTO_DIR}: {e}"))?;
    let mut n = 1;
    let path = loop {
        let candidate = format!("{PHOTO_DIR}/photo-{n}.bmp");
        if !std::path::Path::new(&candidate).exists() {
            break candidate;
        }
        n += 1;
    };
    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
    std::thread::Builder::new()
        .name("photo-save".into())
        .spawn(move || {
            if let Err(e) = std::fs::write(&path, &bmp) {
                println!("photo: {path}: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(name)
}

/// The last save's outcome, shown on the Finished menu.
pub static SAVE_STATUS: Mutex<String> = Mutex::new(String::new());

fn put_f32s(out: &mut Vec<u8>, values: impl IntoIterator<Item = f32>) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn put_transform(out: &mut Vec<u8>, t: &Transform) {
    put_f32s(out, t.translation.to_array().into_iter().chain(t.rotation.to_array()).chain(t.scale.to_array()));
}

/// The audio state of a frame as the bytes it is in memory. A file belongs to builds with the
/// same `Riding` layout: the header carries its size and [`layout_tag`], and a file whose tag
/// differs is refused.
fn riding_bytes(r: &skate_sim::AudioRiding) -> &[u8] {
    // SAFETY: `Riding` is plain old data (floats, integers, bools, arrays of them).
    unsafe { std::slice::from_raw_parts((r as *const skate_sim::AudioRiding).cast::<u8>(), std::mem::size_of::<skate_sim::AudioRiding>()) }
}

/// A fingerprint of `Riding`'s fields (their names, order and kinds, from its `Debug` form).
fn layout_tag() -> u64 {
    let text = format!("{:?}", skate_sim::AudioRiding::default());
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn put_clip(out: &mut Vec<u8>, fx: &ClipFx, shift: f32) {
    let c = &fx.camera;
    out.extend_from_slice(&[c.kind.code(), u8::from(c.advanced), fx.speed, fx.shake]);
    put_f32s(out, c.cam.to_array());
    out.extend_from_slice(&(c.keys.len() as u32).to_le_bytes());
    for (t, cam) in &c.keys {
        put_f32s(out, std::iter::once(t - shift).chain(cam.to_array()));
    }
}

/// Saves the video between the In and Out points with its markers and clips as
/// `replays/replay-N.rpl`; the file is built here and written by a thread. Returns the name.
pub fn save(rec: &Recorder, pb: &Playback, world: &str) -> Result<String, String> {
    let (from, to) = (pb.start.ceil() as usize, (pb.end.floor() as usize).min(rec.len() - 1));
    if to <= from {
        return Err("the clip is empty".into());
    }
    std::fs::create_dir_all(DIR).map_err(|e| format!("{DIR}: {e}"))?;
    let mut n = 1;
    let path = loop {
        let candidate = format!("{DIR}/replay-{n}.rpl");
        if !std::path::Path::new(&candidate).exists() {
            break candidate;
        }
        n += 1;
    };
    let frames = to - from + 1;
    let riding = std::mem::size_of::<skate_sim::AudioRiding>();
    let mut out = Vec::with_capacity(frames * (rec.nbones * 64 + 128 + riding) + 4096);
    out.extend_from_slice(MAGIC);
    for v in [VERSION, riding as u32, rec.nbones as u32, frames as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&layout_tag().to_le_bytes());
    out.extend_from_slice(&rec.period.to_le_bytes());
    out.extend_from_slice(&(world.len() as u32).to_le_bytes());
    out.extend_from_slice(world.as_bytes());
    let shift = from as f32;
    out.extend_from_slice(&(pb.markers.len() as u32).to_le_bytes());
    put_f32s(&mut out, pb.markers.iter().map(|m| m - shift));
    out.extend_from_slice(&(pb.clips.len() as u32).to_le_bytes());
    for fx in &pb.clips {
        put_clip(&mut out, fx, shift);
    }
    for i in from..=to {
        let f = rec.frame(i);
        put_transform(&mut out, &f.root);
        put_transform(&mut out, &f.camera);
        out.extend_from_slice(&f.fov.to_le_bytes());
        out.extend_from_slice(&f.cuts.to_le_bytes());
        out.extend_from_slice(riding_bytes(&f.riding));
        let slot = rec.physical(i);
        for m in &rec.bones[slot * rec.nbones..(slot + 1) * rec.nbones] {
            put_f32s(&mut out, m.to_cols_array());
        }
    }
    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
    *SAVE_STATUS.lock().unwrap() = format!("Saving {name} ({:.1} MB)...", out.len() as f32 / 1_048_576.0);
    let shown = name.clone();
    std::thread::Builder::new()
        .name("replay-save".into())
        .spawn(move || {
            let result = std::fs::write(&path, &out);
            *SAVE_STATUS.lock().unwrap() = match result {
                Ok(()) => format!("Saved {shown}"),
                Err(e) => format!("Save failed: {e}"),
            };
            refresh_list();
        })
        .map_err(|e| e.to_string())?;
    Ok(name)
}

/// A little-endian reader over a replay file.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len()).ok_or("the file is cut short")?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if v.is_finite() { Ok(v) } else { Err("a value is not finite".into()) }
    }
    fn f32s<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
    fn transform(&mut self) -> Result<Transform, String> {
        let v: [f32; 10] = self.f32s()?;
        Ok(Transform {
            translation: glam::Vec3::new(v[0], v[1], v[2]),
            rotation: glam::Quat::from_xyzw(v[3], v[4], v[5], v[6]),
            scale: glam::Vec3::new(v[7], v[8], v[9]),
        })
    }
}

/// The saved replays as the pause menu lists them (read when the menu opens and after a save).
pub static SAVED: Mutex<Vec<Saved>> = Mutex::new(Vec::new());

pub fn refresh_list() {
    *SAVED.lock().unwrap() = list();
}

/// The Replay Editor choices of the pause menu: the instant replay, then each saved replay.
pub fn choices() -> Vec<String> {
    let mut out = vec!["Instant replay".to_owned()];
    out.extend(SAVED.lock().unwrap().iter().map(|s| format!("{}  {}  {:.0} s", s.name.trim_end_matches(".rpl"), s.world, s.seconds)));
    out
}

/// A saved replay: its file name, world (retail map name, as in `packs::WORLDS`) and length.
pub struct Saved {
    pub name: String,
    pub world: String,
    pub seconds: f32,
}

/// The replay header: (riding size, bones, frames, layout tag, period, world).
fn header(r: &mut Reader) -> Result<(usize, usize, usize, u64, f32, String), String> {
    if r.take(4)? != MAGIC {
        return Err("not a replay".into());
    }
    if r.u32()? != VERSION {
        return Err("a replay of another version".into());
    }
    let (riding, bones, frames) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
    let tag = r.u64()?;
    let period = r.f32()?;
    let len = r.u32()? as usize;
    let world = String::from_utf8(r.take(len.min(256))?.to_vec()).map_err(|_| "a bad world name")?;
    Ok((riding, bones, frames, tag, period, world))
}

/// The saved replays, newest number first (headers only).
pub fn list() -> Vec<Saved> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(DIR) else { return out };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".rpl") {
            continue;
        }
        let Ok(mut file) = std::fs::File::open(entry.path()) else { continue };
        let mut head = vec![0u8; 512];
        let n = std::io::Read::read(&mut file, &mut head).unwrap_or(0);
        head.truncate(n);
        if let Ok((_, _, frames, _, period, world)) = header(&mut Reader { data: &head, at: 0 }) {
            out.push(Saved { name, world, seconds: frames as f32 * period });
        }
    }
    let number = |n: &str| n.trim_start_matches("replay-").trim_end_matches(".rpl").parse::<u32>().unwrap_or(0);
    out.sort_by_key(|s| std::cmp::Reverse(number(&s.name)));
    out
}

/// Loads a saved replay into the recorder (replacing what it held) and returns its world, its
/// clip markers and its clips, to edit it as it was saved.
pub fn load(name: &str) -> Result<(String, Vec<f32>, Vec<ClipFx>), String> {
    let path = format!("{DIR}/{name}");
    let data = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut r = Reader { data: &data, at: 0 };
    let (riding, nbones, frames, tag, period, world) = header(&mut r)?;
    if riding != std::mem::size_of::<skate_sim::AudioRiding>() || tag != layout_tag() {
        return Err("a replay saved by another version of the game".into());
    }
    if frames < 2 || frames > CAPACITY || nbones == 0 || nbones > 256 {
        return Err("a replay of a bad size".into());
    }
    let markers = (0..r.u32()?.min(256)).map(|_| r.f32()).collect::<Result<Vec<_>, _>>()?;
    let count = r.u32()? as usize;
    if count != markers.len() + 1 {
        return Err("the clips do not match the markers".into());
    }
    let mut clips = Vec::with_capacity(count);
    for _ in 0..count {
        let b = r.take(4)?;
        let kind = Kind::from_code(b[0]).ok_or("an unknown camera")?;
        let cam = Cam::from_array(r.f32s()?);
        let keys = (0..r.u32()?.min(1024))
            .map(|_| -> Result<(f32, Cam), String> { Ok((r.f32()?, Cam::from_array(r.f32s()?))) })
            .collect::<Result<Vec<_>, _>>()?;
        clips.push(ClipFx { camera: ClipCamera { kind, advanced: b[1] != 0, cam, keys }, speed: saved_speed(b[2]), shake: b[3].min(3) });
    }
    let mut rec = RECORDER.lock().unwrap();
    rec.reset(nbones, period);
    for _ in 0..frames {
        let root = r.transform()?;
        let camera = r.transform()?;
        let fov = r.f32()?;
        let cuts = r.u64()?;
        let bytes = r.take(riding)?;
        // SAFETY: the size and the field layout (`layout_tag`) are those of this build's `Riding`,
        // and the bytes are what the same layout wrote.
        let riding: skate_sim::AudioRiding = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };
        let mut bones = Vec::with_capacity(nbones);
        for _ in 0..nbones {
            bones.push(Mat4::from_cols_array(&r.f32s()?));
        }
        rec.push_frame(Frame { root, camera, fov, cuts, riding, tick: u64::MAX }, &bones);
    }
    Ok((world, markers, clips))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    fn snap(bones: usize, x: f32) -> Snapshot {
        let t = Transform { translation: Vec3::new(x, 0.0, 0.0), rotation: Quat::IDENTITY, scale: Vec3::ONE };
        Snapshot { root: t, bones: vec![Mat4::from_translation(Vec3::splat(x)); bones], camera: t, fov: 1.0 }
    }

    /// LB with both sticks pushing the In and Out points together: they stop OVER apart, never cross (the device
    /// crashed on the playhead's clamp with In past Out).
    #[test]
    fn the_heads_never_cross() {
        let mut r = Recorder::new();
        for i in 0..601 {
            r.push(&snap(1, i as f32), 0, 1.0 / 60.0, i as u64);
        }
        let mut pb = Playback::with(0.0, 600.0, Vec::new(), vec![ClipFx::default()]);
        pb.mode = Mode::Main;
        (pb.start, pb.end) = (295.0, 305.0);
        let lb = vita_platform::controls::xbox::LEFT_SHOULDER;
        for _ in 0..30 {
            pb.update(&r, 1.0 / 60.0, &Controls { held: lb, pressed: 0, lt: false, rt: false, left: [1.0, 0.0], right: [-1.0, 0.0] });
            assert!(pb.end - pb.start >= OVER - 1e-3, "{} .. {}", pb.start, pb.end);
        }
    }

    /// Retail's effects rules (b461 to b472): Left / Right change only the live values, B commits them into the clip
    /// under the playhead (not over a marker; with no marker the whole replay, ours), and the encoded length counts
    /// each clip's frames at its speed.
    #[test]
    fn effects_land_on_commit_and_the_length_counts_slow_motion() {
        let mut r = Recorder::new();
        for i in 0..601 {
            r.push(&snap(1, i as f32), 0, 1.0 / 60.0, i as u64);
        }
        let mut pb = Playback::with(0.0, 600.0, Vec::new(), vec![ClipFx::default()]);
        pb.mode = Mode::Main;
        pb.paused = true;
        let press = |pb: &mut Playback, button: u16| {
            pb.update(&r, 1.0 / 60.0, &Controls { held: button, pressed: button, lt: false, rt: false, left: [0.0; 2], right: [0.0; 2] });
        };
        assert_eq!(pb.encoded_seconds(&r), 10);
        // A: the effects menu; to the speed row, Left five times: 50 % live, the clip untouched until B.
        press(&mut pb, vita_platform::controls::xbox::A);
        assert_eq!(pb.mode, Mode::Effects);
        pb.selection = EDIT_SPEED;
        for _ in 0..5 {
            press(&mut pb, vita_platform::controls::xbox::DPAD_LEFT);
        }
        assert_eq!((pb.live.0, pb.clips[0].speed), (50, 100));
        assert_eq!(pb.encoded_seconds(&r), 20, "the live speed counts while the menu is open");
        press(&mut pb, vita_platform::controls::xbox::B);
        assert_eq!((pb.mode, pb.clips[0].speed), (Mode::Main, 50), "no marker yet: the whole replay");
        assert_eq!(pb.encoded_seconds(&r), 20);
        // Over a marker the commit does nothing.
        pb.markers = vec![300.0];
        pb.clips.push(ClipFx::default());
        pb.pos = 300.0;
        pb.edit_clip = 1;
        pb.mode = Mode::Effects;
        pb.load_live();
        pb.live.0 = 10;
        pb.commit_live();
        assert_eq!(pb.clips[1].speed, 100);
        // Halves of 5 s: 50 % counts 10 s, 100 % 5 s.
        pb.mode = Mode::Main;
        assert_eq!(pb.encoded_seconds(&r), 15);
        // Saved replays from before keep a sensible speed.
        assert_eq!([1, 2, 3, 5, 0, 40, 47, 200].map(saved_speed), [30, 50, 100, 100, 100, 40, 50, 100]);
    }

    #[test]
    fn ring_keeps_the_newest_frames_in_order() {
        let mut r = Recorder::new();
        for i in 0..CAPACITY + 10 {
            r.push(&snap(3, i as f32), 0, 1.0 / 60.0, i as u64);
        }
        assert_eq!(r.len(), CAPACITY);
        let (mut a, mut b) = (snap(3, 0.0), snap(3, 0.0));
        let alpha = r.pair(0.5, &mut a, &mut b);
        assert_eq!(a.root.translation.x, 10.0);
        assert_eq!(b.root.translation.x, 11.0);
        assert_eq!(a.bones.len(), 3);
        assert!((alpha - 0.5).abs() < 1e-6);
        r.pair(1e9, &mut a, &mut b);
        assert_eq!(b.root.translation.x, (CAPACITY + 9) as f32);
    }

    #[test]
    fn a_cut_between_frames_does_not_blend() {
        let mut r = Recorder::new();
        r.push(&snap(2, 0.0), 0, 1.0 / 60.0, 0);
        r.push(&snap(2, 100.0), 1, 1.0 / 60.0, 1);
        let (mut a, mut b) = (snap(2, 0.0), snap(2, 0.0));
        let alpha = r.pair(0.3, &mut a, &mut b);
        assert_eq!(alpha, 1.0);
        assert_eq!(a.root.translation.x, 100.0);
    }
}
