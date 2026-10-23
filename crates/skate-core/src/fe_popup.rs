//! The front end's popups (doc 31 "Popups"): retail's popup list on the FE stack object (TU3, research
//! `.local/research/menus-m6/fe-popup.md`). A caller fills an sPopupInfo (defaults `sub_82D0B0B0`) and pushes it
//! (`sub_82D0A8D0`): it gets the next id and joins a list sorted by priority, ascending; ties keep arrival order, so
//! the newest of the highest priority is last, and the last one is shown (FE vt+24 `sub_825EC530`). The player's
//! choice goes to the popup's callback as (id, choice, user) (host vt+64 `sub_825EF698`), then, with auto-remove on
//! (the default), the popup leaves the list (`sub_82D0A628`) and the next last one shows. Clear-all
//! (`sub_82D0A738`) calls back every popup not marked to survive it with choice -2 and drops it.
//!
//! Callbacks are stable string ids (never code pointers), so requests and events are plain data a mod can send and
//! receive, and a multiplayer build could serialise them.

/// Which retail movie shows it (sPopupInfo +1436: 0 Sk8Popup, 1 Sk8AutoSavePopup, 2 / 3 Sk8SmallPopup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopupKind {
    #[default]
    Standard,
    AutoSave,
    Small,
}

impl PopupKind {
    /// Retail's type value.
    pub fn from_retail(kind: i32) -> Self {
        match kind {
            1 => Self::AutoSave,
            2 | 3 => Self::Small,
            _ => Self::Standard,
        }
    }

    /// The template the host shows for it (template table names).
    pub fn template(self) -> &'static str {
        match self {
            Self::Standard => "Sk8Popup",
            Self::AutoSave => "Sk8AutoSavePopup",
            Self::Small => "Sk8SmallPopup",
        }
    }
}

/// The choice for B on a cancelable popup (sk8popup / small_popup `OnMenuSelect(0, 255, 'CANCEL!')`).
pub const CHOICE_CANCEL: i32 = 255;
/// The choice clear-all reports (`sub_82D0A738`).
pub const CHOICE_CLEARED: i32 = -2;
/// Options slots in sPopupInfo (+388, 8 x 128 bytes).
pub const MAX_OPTIONS: usize = 8;

/// sPopupInfo as data. Defaults are retail's (`sub_82D0B0B0`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupRequest {
    /// +0, string id or text (only the small popup shows it).
    pub title: String,
    /// +128.
    pub description: String,
    /// +384 count and +388 texts (at most `MAX_OPTIONS`).
    pub options: Vec<String>,
    pub kind: PopupKind,
    /// +1440: the list's sort key; the highest is shown.
    pub priority: i32,
    /// +1444 (not read by the retail movies checked).
    pub default_selection: i32,
    /// +1448: B answers `CHOICE_CANCEL`; otherwise B does nothing.
    pub cancelable: bool,
    /// +1449: leave the list after the choice callback.
    pub auto_remove: bool,
    /// +1451: survives clear-all.
    pub keep_on_clear: bool,
    /// +1412 as a stable id: who gets the choice.
    pub callback: String,
    /// +1432: passed back with the choice.
    pub user: u32,
}

impl Default for PopupRequest {
    fn default() -> Self {
        Self {
            title: "#TODO".into(),
            description: "#DESCRIPTION".into(),
            options: Vec::new(),
            kind: PopupKind::Standard,
            priority: 0,
            default_selection: 0,
            cancelable: false,
            auto_remove: true,
            keep_on_clear: false,
            callback: String::new(),
            user: 0,
        }
    }
}

/// A choice for a popup's callback: (id, choice, user) as retail passes them, with the callback id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupEvent {
    pub id: i32,
    pub callback: String,
    pub choice: i32,
    pub user: u32,
}

/// The list (FE stack object +112..+120) and the id counter (`[0x830853F8]`, post-increment from 0).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopupList {
    next_id: i32,
    entries: Vec<(i32, PopupRequest)>,
}

impl PopupList {
    /// `sub_82D0A8D0`: returns the new popup's id. At most `MAX_OPTIONS` options are kept.
    pub fn push(&mut self, mut request: PopupRequest) -> i32 {
        request.options.truncate(MAX_OPTIONS);
        let id = self.next_id;
        self.next_id += 1;
        // After the last entry whose priority is <= the new one (ascending; ties keep arrival order).
        let at = self.entries.iter().rposition(|(_, r)| r.priority <= request.priority).map_or(0, |i| i + 1);
        self.entries.insert(at, (id, request));
        id
    }

    /// The popup on screen: the last entry.
    pub fn shown(&self) -> Option<(i32, &PopupRequest)> {
        self.entries.last().map(|(id, r)| (*id, r))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The player chose on the shown popup (host vt+64): its callback event, and the popup leaves the list when it
    /// auto-removes.
    pub fn choose(&mut self, choice: i32) -> Option<PopupEvent> {
        let (id, request) = self.entries.last()?;
        let event = PopupEvent { id: *id, callback: request.callback.clone(), choice, user: request.user };
        if request.auto_remove {
            self.entries.pop();
        }
        Some(event)
    }

    /// `sub_82D0A628`: drop a popup by id; true when it was the shown one (the next shows).
    pub fn remove(&mut self, id: i32) -> bool {
        let Some(i) = self.entries.iter().position(|(e, _)| *e == id) else { return false };
        let shown = i + 1 == self.entries.len();
        self.entries.remove(i);
        shown
    }

    /// Drop the popup with this callback id (a mod replacing or taking down its popup); true when it was shown.
    pub fn remove_callback(&mut self, callback: &str) -> bool {
        match self.entries.iter().position(|(_, r)| r.callback == callback) {
            Some(i) => {
                let id = self.entries[i].0;
                self.remove(id)
            }
            None => false,
        }
    }

    /// `sub_82D0A738`: every popup not marked `keep_on_clear` gets `CHOICE_CLEARED` and leaves.
    pub fn clear_all(&mut self) -> Vec<PopupEvent> {
        let mut events = Vec::new();
        self.entries.retain(|(id, r)| {
            if r.keep_on_clear {
                return true;
            }
            events.push(PopupEvent { id: *id, callback: r.callback.clone(), choice: CHOICE_CLEARED, user: r.user });
            false
        });
        events
    }

    /// Drop the popups whose callback id starts with `prefix` (a mod's popups when it stops), as clear-all does.
    pub fn clear_owned(&mut self, prefix: &str) -> Vec<PopupEvent> {
        let mut events = Vec::new();
        self.entries.retain(|(id, r)| {
            if !r.callback.starts_with(prefix) {
                return true;
            }
            events.push(PopupEvent { id: *id, callback: r.callback.clone(), choice: CHOICE_CLEARED, user: r.user });
            false
        });
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(callback: &str, priority: i32) -> PopupRequest {
        PopupRequest { callback: callback.into(), priority, ..Default::default() }
    }

    #[test]
    fn highest_priority_shows_and_ties_show_the_newest() {
        let mut list = PopupList::default();
        let a = list.push(req("a", 0));
        assert_eq!(list.shown().unwrap().0, a);
        let b = list.push(req("b", 5));
        let c = list.push(req("c", 0));
        assert_eq!(list.shown().unwrap().0, b, "a lower priority goes below");
        let d = list.push(req("d", 5));
        assert_eq!(list.shown().unwrap().0, d, "equal priority: the newest shows");
        assert_eq!((a, b, c, d), (0, 1, 2, 3), "ids count up from 0");
        assert!(list.remove(d));
        assert_eq!(list.shown().unwrap().0, b);
        assert!(!list.remove(a), "not the shown one");
        assert_eq!(list.choose(1).unwrap(), PopupEvent { id: b, callback: "b".into(), choice: 1, user: 0 });
        assert_eq!(list.shown().unwrap().0, c, "auto-remove shows the next");
    }

    #[test]
    fn auto_remove_off_keeps_the_popup_and_clear_all_reports_minus_two() {
        let mut list = PopupList::default();
        list.push(PopupRequest { auto_remove: false, ..req("stay", 0) });
        list.choose(0).unwrap();
        assert!(!list.is_empty());
        list.push(PopupRequest { keep_on_clear: true, ..req("keep", 0) });
        let events = list.clear_all();
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].callback.as_str(), events[0].choice), ("stay", CHOICE_CLEARED));
        assert_eq!(list.shown().unwrap().1.callback, "keep");
    }

    #[test]
    fn defaults_are_retail_and_options_cap_at_eight() {
        let r = PopupRequest::default();
        assert_eq!((r.title.as_str(), r.description.as_str()), ("#TODO", "#DESCRIPTION"));
        assert!(r.auto_remove && !r.cancelable && !r.keep_on_clear);
        let mut list = PopupList::default();
        list.push(PopupRequest { options: vec!["x".into(); 10], ..Default::default() });
        assert_eq!(list.shown().unwrap().1.options.len(), MAX_OPTIONS);
        assert_eq!(PopupKind::from_retail(3), PopupKind::Small);
        assert_eq!(PopupKind::from_retail(0).template(), "Sk8Popup");
    }

    #[test]
    fn a_mods_popups_leave_when_it_stops() {
        let mut list = PopupList::default();
        list.push(req("engine.x", 0));
        list.push(req("mod:m.ask", 0));
        let events = list.clear_owned("mod:m.");
        assert_eq!(events.len(), 1);
        assert_eq!(list.shown().unwrap().1.callback, "engine.x");
    }
}
