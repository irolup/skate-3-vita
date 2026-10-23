//! The front-end screen stack (doc 31, "Screen stack"): which FE screens are open, in which order, and what
//! opening or closing one does to the game. A port of retail's stack (TU3 recomp addresses; research
//! `.local/research/fe-stack-port.md`, push / queue code main-checked).
//!
//! Two stacks: the main one (A, stack+16, queue +32) and the overlay one (B, +80, queue +96; the tips loader and
//! such, no blur, no input). Requests queue up and run in a batch on the next update (sub_82D0A560): the main
//! queue first, the overlay queue only when the main one ran nothing. A queue waits while its stack has more than
//! one screen and the top one is still loading or closing. The host owns the movies: this returns `Effect`s
//! (open / close / show a screen, gameplay suspend, chyron, HUD mode, focus) and is told when a screen's movie is
//! ready. Screens are indices into the per-build screen table (ids differ between builds); records come from
//! the setup-extracted table and mods override them by screen id.

use std::collections::VecDeque;

/// What a screen does while it is open (info table record, 24 bytes; retail defaults from setup data).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenInfo {
    /// +0: open pushes "context 3" (gameplay modules suspended, vt+32), close removes it (vt+36).
    pub suspends_gameplay: bool,
    /// +1: the chyron shows while the screen is open (vt+48).
    pub chyron: bool,
    /// +4: the world behind blurs while it is the top of the main stack (vt+28; ramp in `fe_blur`).
    pub blur: bool,
    /// +8: HUD while it is the top of the main stack (sub_82D098F0).
    pub hud: HudMode,
    /// +12: pad input its own handler did not use goes on to the movie's UpdateInput (sub_825EC0E8).
    pub input_to_movie: bool,
}

/// Info record +8 (sub_82D098F0 -> vt+44(hud, online)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudMode {
    /// 0: HUD on, online HUDs on.
    On,
    /// 1: both off.
    #[default]
    Off,
    /// 2: HUD off, online HUDs on.
    OnlineOnly,
    /// 3: leave as is.
    Keep,
}

impl HudMode {
    pub fn from_retail(v: i32) -> Self {
        match v {
            0 => Self::On,
            2 => Self::OnlineOnly,
            3 => Self::Keep,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Stack A: menus; blur, HUD mode and input follow its top.
    Main,
    /// Stack B: overlays.
    Overlay,
}

/// A queued request (20-byte queue entry: id, param, keep-open byte, keep-visible byte, op).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Op 1 (sub_82D09DE0).
    Pop,
    /// Op 2 (sub_82D09C58). `keep_open` = 0 closes the screen below (the plain push, sub_82D0ABE0, pause's);
    /// 1 keeps it open, shown or hidden by `keep_visible`.
    Push { screen: usize, param: i32, keep_open: bool, keep_visible: bool },
    /// Op 3 (sub_82D09B00).
    Replace { screen: usize, param: i32 },
    /// Op 4 (sub_82D0A078).
    PopTo { screen: usize },
    /// Op 5 (sub_82D09F90): pop it when on top, else unwind down to it and reopen it.
    Close { screen: usize },
}

impl Request {
    /// The plain push (sub_82D0ABE0): the screen below closes.
    pub fn push(screen: usize) -> Self {
        Self::Push { screen, param: -1, keep_open: false, keep_visible: false }
    }
}

/// What the host must do, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Open the screen's movie (screen init -> AS OpenScreen); it reports `ready` after its intro.
    Open(usize),
    /// Close the screen's movie (AS CloseScreen, its outro).
    Close(usize),
    /// Show or hide a screen that stays open (screen vt+28).
    Show(usize, bool),
    /// Screen vt+44: the request's param.
    Param(usize, i32),
    /// Gameplay suspended (context 3 pushed) or resumed (every context 3 removed).
    SuspendGameplay(bool),
    /// Chyron visible.
    Chyron(bool),
    /// World blur behind the main stack's top (vt+28).
    Blur(bool),
    /// HUD mode of the main stack's top.
    Hud(HudMode),
    /// Focus changed (vt+12): the top screen (overlay top first).
    Focus(usize),
}

/// Movie state of an open screen (screen +16): only 0 / 3 let the queue run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Loading,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    screen: usize,
    param: i32,
    /// Set by a push over it: stays open (else it was closed and reopens when on top again).
    kept_open: bool,
    state: State,
}

#[derive(Clone, Debug, Default)]
pub struct FeStack {
    main: Vec<Entry>,
    overlay: Vec<Entry>,
    main_queue: VecDeque<Request>,
    overlay_queue: VecDeque<Request>,
}

impl FeStack {
    pub fn request(&mut self, layer: Layer, request: Request) {
        match layer {
            Layer::Main => self.main_queue.push_back(request),
            Layer::Overlay => self.overlay_queue.push_back(request),
        }
    }

    /// The screen's movie finished its intro (OnScreenIntroComplete): state 3.
    pub fn ready(&mut self, screen: usize) {
        for e in self.main.iter_mut().chain(self.overlay.iter_mut()).filter(|e| e.screen == screen) {
            e.state = State::Ready;
        }
    }

    pub fn top(&self, layer: Layer) -> Option<usize> {
        self.stack(layer).last().map(|e| e.screen)
    }

    pub fn screens(&self, layer: Layer) -> impl Iterator<Item = usize> + '_ {
        self.stack(layer).iter().map(|e| e.screen)
    }

    /// Open, or waiting in a queue (sub_82D09A40, which gates op 5).
    pub fn active(&self, screen: usize) -> bool {
        self.main.iter().chain(&self.overlay).any(|e| e.screen == screen)
            || self.main_queue.iter().chain(&self.overlay_queue).any(|r| match r {
                Request::Push { screen: s, .. } | Request::Replace { screen: s, .. } => *s == screen,
                _ => false,
            })
    }

    /// Pad input goes nowhere while main requests are pending (sub_825EC0E8); else to the main top once ready.
    pub fn input_target(&self) -> Option<usize> {
        if !self.main_queue.is_empty() {
            return None;
        }
        self.main.last().filter(|e| e.state == State::Ready).map(|e| e.screen)
    }

    fn stack(&self, layer: Layer) -> &Vec<Entry> {
        match layer {
            Layer::Main => &self.main,
            Layer::Overlay => &self.overlay,
        }
    }

    /// One stack update (sub_82D0A560).
    pub fn update(&mut self, info: &dyn Fn(usize) -> ScreenInfo) -> Vec<Effect> {
        let mut out = Vec::new();
        let ran = self.run_queue(Layer::Main, info, &mut out) || self.run_queue(Layer::Overlay, info, &mut out);
        if ran {
            if let Some(top) = self.overlay.last().or(self.main.last()) {
                out.push(Effect::Focus(top.screen));
            }
            if let Some(top) = self.main.last() {
                let hud = info(top.screen).hud;
                if hud != HudMode::Keep {
                    out.push(Effect::Hud(hud));
                }
            }
        }
        out
    }

    /// sub_82D0A2C0: wait while the stack has more than one screen (or overlays are open) and its top is still
    /// loading; else run the whole batch in order.
    fn run_queue(&mut self, layer: Layer, info: &dyn Fn(usize) -> ScreenInfo, out: &mut Vec<Effect>) -> bool {
        let overlays_open = !self.overlay.is_empty();
        let (stack, queue) = match layer {
            Layer::Main => (&mut self.main, &mut self.main_queue),
            Layer::Overlay => (&mut self.overlay, &mut self.overlay_queue),
        };
        let busy = (stack.len() > 1 || overlays_open) && stack.last().is_some_and(|e| e.state != State::Ready);
        if busy || queue.is_empty() {
            return false;
        }
        let batch: Vec<Request> = queue.drain(..).collect();
        for request in batch {
            run(stack, layer, request, info, out);
        }
        true
    }
}

fn open(stack: &mut [Entry], screen: usize, info: &dyn Fn(usize) -> ScreenInfo, out: &mut Vec<Effect>) {
    // sub_82D09998: vt+32 when +0, vt+40 (a screen opened), vt+48(+1), then the screen's init opens its movie.
    let i = info(screen);
    if i.suspends_gameplay {
        out.push(Effect::SuspendGameplay(true));
    }
    out.push(Effect::Chyron(i.chyron));
    out.push(Effect::Open(screen));
    if let Some(e) = stack.iter_mut().rev().find(|e| e.screen == screen) {
        e.state = State::Loading;
        e.kept_open = false;
    }
}

fn close(screen: usize, info: &dyn Fn(usize) -> ScreenInfo, out: &mut Vec<Effect>) {
    // sub_82D09848: the screen's outro, then vt+36 when +0.
    out.push(Effect::Close(screen));
    if info(screen).suspends_gameplay {
        out.push(Effect::SuspendGameplay(false));
    }
}

fn blur(stack: &[Entry], layer: Layer, info: &dyn Fn(usize) -> ScreenInfo, out: &mut Vec<Effect>) {
    if layer == Layer::Main {
        out.push(Effect::Blur(stack.last().is_some_and(|e| info(e.screen).blur)));
    }
}

/// Unwind to `screen` (op 4 and op 5 below the top): close the top, pop until `screen` is on top (closing the
/// popped ones that were kept open), reopen it.
fn unwind(stack: &mut Vec<Entry>, layer: Layer, screen: usize, info: &dyn Fn(usize) -> ScreenInfo, out: &mut Vec<Effect>) {
    let Some(top) = stack.pop() else { return };
    close(top.screen, info, out);
    while let Some(e) = stack.last().copied() {
        if e.screen == screen {
            break;
        }
        stack.pop();
        if e.kept_open {
            close(e.screen, info, out);
        }
    }
    if !stack.is_empty() {
        open(stack, screen, info, out);
    }
    blur(stack, layer, info, out);
}

fn run(stack: &mut Vec<Entry>, layer: Layer, request: Request, info: &dyn Fn(usize) -> ScreenInfo, out: &mut Vec<Effect>) {
    match request {
        Request::Pop => {
            let Some(top) = stack.pop() else { return };
            blur(stack, layer, info, out);
            close(top.screen, info, out);
            if let Some(below) = stack.last().copied() {
                if below.kept_open {
                    // Shown again at once, no chyron call on this path.
                    if info(below.screen).suspends_gameplay {
                        out.push(Effect::SuspendGameplay(true));
                    }
                    out.push(Effect::Show(below.screen, true));
                    if let Some(e) = stack.last_mut() {
                        e.kept_open = false;
                    }
                } else {
                    open(stack, below.screen, info, out);
                }
            }
        }
        Request::Push { screen, param, keep_open, keep_visible } => {
            if let Some(old) = stack.last_mut() {
                old.kept_open = keep_open;
                let old = *old;
                if keep_open {
                    out.push(Effect::Show(old.screen, keep_visible));
                } else {
                    close(old.screen, info, out);
                }
            }
            stack.push(Entry { screen, param, kept_open: false, state: State::Loading });
            open(stack, screen, info, out);
            out.push(Effect::Param(screen, param));
            blur(stack, layer, info, out);
        }
        Request::Replace { screen, param } => {
            if let Some(top) = stack.last_mut().filter(|e| e.screen == screen) {
                top.param = param;
            } else {
                if let Some(top) = stack.pop() {
                    close(top.screen, info, out);
                }
                stack.push(Entry { screen, param, kept_open: false, state: State::Loading });
                open(stack, screen, info, out);
            }
            blur(stack, layer, info, out);
            out.push(Effect::Param(screen, param));
        }
        Request::PopTo { screen } => {
            if stack.last().is_some_and(|e| e.screen == screen) || !stack.iter().any(|e| e.screen == screen) {
                return;
            }
            unwind(stack, layer, screen, info, out);
        }
        Request::Close { screen } => {
            if !stack.iter().any(|e| e.screen == screen) {
                return;
            }
            if stack.last().is_some_and(|e| e.screen == screen) {
                run(stack, layer, Request::Pop, info, out);
            } else {
                unwind(stack, layer, screen, info, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: usize = 1;
    const PAUSE: usize = 56;
    const OPTIONS: usize = 45;

    fn info(screen: usize) -> ScreenInfo {
        match screen {
            PAUSE | OPTIONS => ScreenInfo { suspends_gameplay: true, blur: true, input_to_movie: true, ..Default::default() },
            _ => ScreenInfo { hud: HudMode::On, ..Default::default() },
        }
    }

    #[test]
    fn pause_push_and_pop() {
        let mut s = FeStack::default();
        s.request(Layer::Main, Request::push(PAUSE));
        let fx = s.update(&info);
        assert_eq!(
            fx,
            [
                Effect::SuspendGameplay(true),
                Effect::Chyron(false),
                Effect::Open(PAUSE),
                Effect::Param(PAUSE, -1),
                Effect::Blur(true),
                Effect::Focus(PAUSE),
                Effect::Hud(HudMode::Off),
            ]
        );
        assert_eq!(s.input_target(), None, "not ready before its intro");
        s.ready(PAUSE);
        assert_eq!(s.input_target(), Some(PAUSE));
        s.request(Layer::Main, Request::Close { screen: PAUSE });
        assert_eq!(s.input_target(), None, "input waits for pending requests");
        let fx = s.update(&info);
        assert_eq!(fx, [Effect::Blur(false), Effect::Close(PAUSE), Effect::SuspendGameplay(false)]);
        assert_eq!(s.top(Layer::Main), None);
    }

    #[test]
    fn plain_push_closes_the_screen_below_and_pop_reopens_it() {
        let mut s = FeStack::default();
        s.request(Layer::Main, Request::push(PAUSE));
        s.update(&info);
        s.ready(PAUSE);
        s.request(Layer::Main, Request::push(OPTIONS));
        let fx = s.update(&info);
        assert_eq!(&fx[..3], [Effect::Close(PAUSE), Effect::SuspendGameplay(false), Effect::SuspendGameplay(true)]);
        // The queue waits while the new top loads.
        s.request(Layer::Main, Request::Pop);
        assert!(s.update(&info).is_empty());
        s.ready(OPTIONS);
        let fx = s.update(&info);
        assert!(fx.starts_with(&[Effect::Blur(true), Effect::Close(OPTIONS), Effect::SuspendGameplay(false)]));
        assert!(fx.contains(&Effect::Open(PAUSE)));
    }

    #[test]
    fn kept_open_push_hides_and_pop_shows_again() {
        let mut s = FeStack::default();
        s.request(Layer::Main, Request::push(GAME));
        s.update(&info);
        s.ready(GAME);
        s.request(Layer::Main, Request::Push { screen: PAUSE, param: 2, keep_open: true, keep_visible: false });
        let fx = s.update(&info);
        assert_eq!(fx[0], Effect::Show(GAME, false));
        assert!(fx.contains(&Effect::Param(PAUSE, 2)));
        s.ready(PAUSE);
        s.request(Layer::Main, Request::Pop);
        let fx = s.update(&info);
        assert!(fx.contains(&Effect::Show(GAME, true)) && !fx.contains(&Effect::Open(GAME)));
        assert!(fx.contains(&Effect::Hud(HudMode::On)));
    }

    #[test]
    fn close_below_the_top_unwinds_and_reopens_it() {
        let mut s = FeStack::default();
        for screen in [GAME, PAUSE, OPTIONS] {
            s.request(Layer::Main, Request::push(screen));
            s.update(&info);
            s.ready(screen);
        }
        s.request(Layer::Main, Request::Close { screen: PAUSE });
        let fx = s.update(&info);
        assert_eq!(&fx[..2], [Effect::Close(OPTIONS), Effect::SuspendGameplay(false)]);
        assert!(fx.contains(&Effect::Open(PAUSE)));
        assert_eq!(s.screens(Layer::Main).collect::<Vec<_>>(), [GAME, PAUSE]);
        // Closing a screen that is not open does nothing.
        s.ready(PAUSE);
        s.request(Layer::Main, Request::Close { screen: 9 });
        assert!(!s.update(&info).iter().any(|e| matches!(e, Effect::Close(_))));
    }

    #[test]
    fn overlay_runs_only_when_main_is_idle_and_has_no_blur() {
        let mut s = FeStack::default();
        s.request(Layer::Main, Request::push(PAUSE));
        s.request(Layer::Overlay, Request::push(OPTIONS));
        let fx = s.update(&info);
        assert!(!fx.contains(&Effect::Open(OPTIONS)));
        let fx = s.update(&info);
        assert!(fx.contains(&Effect::Open(OPTIONS)) && !fx.iter().any(|e| matches!(e, Effect::Blur(_))));
        assert!(fx.contains(&Effect::Focus(OPTIONS)));
        assert_eq!(s.input_target(), None);
        s.ready(PAUSE);
        assert_eq!(s.input_target(), Some(PAUSE), "overlays never take input");
    }
}
