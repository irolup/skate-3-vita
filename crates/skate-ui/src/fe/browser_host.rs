// skate_vita: ours (docs/skatepark-survey.md "My Parks", step S4).
//! The shared content browser on parks (skate.Park > My Parks; FE state 0x3B, vtable `0x82304260`, movie
//! `browser/browser`): the `CommonBrowser.*` natives (registrar `825B75C0`) answered from the game's saved parks, the
//! screen's C++ keys (`82621778`) and the screen modes it posts (`SetState(n)`, names `0x8302792C`).
//!
//! Retail's park provider (vtable `0x823041B0`): the title `ID_COMMON_BROWSER_SKATE_REEL_SKATEPARK`, the saved parks
//! under My Parks > Saved Parks, select (`8262A570`: the game loads it, asking first when the park has unsaved edits) and
//! delete (`8262ABA0`). The movie takes A (select), Y (delete) and X itself (`UpdateInput`); the C++ takes the d-pad (a
//! thumbnail, four for Up / Down), the shoulder buttons (LB / RB the top tabs, LT / RT the bottom ones) and B.
//!
//! NOT RETAIL YET: the Downloaded and Community tabs are online (empty here); the bottom tabs of those two, the item
//! fields retail fills for a park (the name shown on the creator line here) and the key sounds are not traced.
use crate::fe::apt_vm::{to_integer, Value, Vm};

/// The movie's modes in value order (`FECommonBrowserMode_<name>` = index).
pub const MODES: [&str; 11] = [
    "WaitForData",
    "OnlineDataReady",
    "LocalPhotoDataReady",
    "LocalVideoDataReady",
    "LocalSkateParkDataReady",
    "SetupFilterTabs",
    "UpdateSelectedTopTab",
    "UpdateSelectedBottomTab",
    "UpdateAvailableSpace",
    "UpdateSelectedMediaThumb",
    "CareerPhotoDataReady",
];
pub const MODE_PARKS_READY: i32 = 4;
pub const MODE_SETUP_TABS: i32 = 5;
pub const MODE_TOP_TAB: i32 = 6;
pub const MODE_BOTTOM_TAB: i32 = 7;
pub const MODE_SPACE: i32 = 8;
pub const MODE_THUMB: i32 = 9;

/// The tab table (`0x83027AB0`: label, icon) by tab id.
const TABS: [(&str, &str); 20] = [
    ("ID_COMMON_BROWSER_MY_FOOTAGE", ""),
    ("ID_COMMON_BROWSER_COMMUNITY", ""),
    ("ID_COMMON_BROWSER_DOWNLOADED", ""),
    ("ID_COMMON_BROWSER_SAVED_VIDEOS", ""),
    ("ID_COMMON_BROWSER_SHARED_VIDEOS", ""),
    ("ID_COMMON_BROWSER_FEATURED", "featured"),
    ("ID_COMMON_BROWSER_BOOKMARKED", "bookmarks"),
    ("ID_COMMON_BROWSER_MY_TEAM", "team"),
    ("ID_COMMON_BROWSER_MY_FRIENDS", "friends"),
    ("ID_COMMON_BROWSER_TOP_RATED", "toprated"),
    ("ID_COMMON_BROWSER_MOST_VIEWED", "viewed"),
    ("ID_COMMON_BROWSER_MOST_RECENT", "recent"),
    ("ID_COMMON_BROWSER_SAVED_PHOTOS", ""),
    ("ID_COMMON_BROWSER_SHARED_PHOTOS", ""),
    ("ID_COMMON_BROWSER_MOST_DOWNLOADED", "downloaded"),
    ("ID_COMMON_BROWSER_SAVED_PARKS", ""),
    ("ID_COMMON_BROWSER_SHARED_PARKS", ""),
    ("ID_COMMON_BROWSER_SAVED_CHALLENGES", ""),
    ("ID_COMMON_BROWSER_SHARED_CHALLENGES", ""),
    ("ID_COMMON_BROWSER_CAREER_PHOTOS", ""),
];
/// Tab 0's label on parks (`825B7E10`: the provider's kind 2).
const MY_PARKS: &str = "ID_COMMON_BROWSER_MY_PARKS";
/// The park browse's top tabs (My Parks, Downloaded, Community) and each one's bottom tabs.
const TOP: [u8; 3] = [0, 2, 1];
const BOTTOM: [&[u8]; 3] = [&[15, 16], &[], &[5]];
/// Saved Parks (the only tab with items here).
const SAVED_PARKS: u8 = 15;
/// The movie's page (`MAX_THUMBNAILS`) and Up / Down's step (`82621B50` / `82621D10` with 4).
pub const PAGE: usize = 8;
const ROW: usize = 4;
/// The park content type `GetCurrentContentType` gives (`B+0x14E4` = 3 for parks; the movie's photo 0 and video 1
/// are the others).
const CONTENT_PARKS: f64 = 3.0;
/// The memory card picture kind of a park (`LoadMemcardTexture(10, index)`).
pub const PARK_PICTURE: i32 = 10;

/// A saved park as the browser shows it: its name, its date as text, and its picture (the memory card index the movie
/// loads, `Game.ImageRenderer_MemcardLoad(10, index, slot)`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrowserItem {
    pub name: String,
    pub date: String,
    pub picture: Option<u32>,
}

/// What the player did: select or delete the item at an index of the saved parks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserEvent {
    Select(usize),
    Delete(usize),
}

/// The keys the screen's C++ handler takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserKey {
    Left,
    Right,
    Up,
    Down,
    TopNext,
    TopPrevious,
    BottomNext,
    BottomPrevious,
    Back,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrowserHost {
    pub open: bool,
    pub shown: bool,
    pub items: Vec<BrowserItem>,
    pub top: usize,
    pub bottom: usize,
    pub current: usize,
    /// Modes to post, in order.
    pub states: Vec<i32>,
    pub events: Vec<BrowserEvent>,
    /// B closed it.
    pub closed: bool,
}

impl BrowserHost {
    /// Opened on My Parks > Saved Parks (`82624790(B, 0, 15, 2)`), the first park.
    pub fn open(items: Vec<BrowserItem>) -> Self {
        Self { open: true, items, states: vec![MODE_SETUP_TABS, MODE_PARKS_READY, MODE_SPACE], ..Default::default() }
    }

    fn bottom_tabs(&self) -> &'static [u8] {
        BOTTOM.get(self.top).copied().unwrap_or(&[])
    }

    fn on_saved(&self) -> bool {
        self.bottom_tabs().get(self.bottom) == Some(&SAVED_PARKS)
    }

    /// The items of the tab shown.
    pub fn shown_items(&self) -> &[BrowserItem] {
        if self.on_saved() { &self.items } else { &[] }
    }

    /// New saved parks (a delete): the selection kept in range, the thumbnails filled again.
    pub fn set_items(&mut self, items: Vec<BrowserItem>) {
        self.items = items;
        self.current = self.current.min(self.items.len().saturating_sub(1));
        self.states.extend([MODE_PARKS_READY, MODE_SPACE]);
    }

    /// One key of the C++ handler (`82621778`); false when not its own (the movie gets it).
    pub fn key(&mut self, key: BrowserKey) -> bool {
        let count = self.shown_items().len();
        let page = self.current / PAGE;
        match key {
            BrowserKey::Back => self.closed = true,
            BrowserKey::Left | BrowserKey::Right | BrowserKey::Up | BrowserKey::Down => {
                if count == 0 {
                    return true;
                }
                let step = if matches!(key, BrowserKey::Up | BrowserKey::Down) { ROW } else { 1 };
                self.current = match key {
                    BrowserKey::Left | BrowserKey::Up => self.current.saturating_sub(step),
                    _ => (self.current + step).min(count - 1),
                };
                self.states.push(if self.current / PAGE == page { MODE_THUMB } else { MODE_PARKS_READY });
            }
            BrowserKey::TopNext | BrowserKey::TopPrevious => {
                let n = TOP.len();
                self.top = if key == BrowserKey::TopNext { (self.top + 1) % n } else { (self.top + n - 1) % n };
                (self.bottom, self.current) = (0, 0);
                self.states.extend([MODE_TOP_TAB, MODE_PARKS_READY]);
            }
            BrowserKey::BottomNext | BrowserKey::BottomPrevious => {
                let n = self.bottom_tabs().len();
                if n > 1 {
                    self.bottom = if key == BrowserKey::BottomNext { (self.bottom + 1) % n } else { (self.bottom + n - 1) % n };
                    self.current = 0;
                    self.states.extend([MODE_BOTTOM_TAB, MODE_PARKS_READY]);
                }
            }
        }
        true
    }

    /// One native call; None = not ours.
    pub fn call(&mut self, vm: &mut Vm, object: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        if !self.open || object != "CommonBrowser" {
            return Ok(None);
        }
        let index = |i: usize| args.get(i).map_or(0, to_integer).max(0) as usize;
        let text = |s: &str| Value::Text(s.to_owned());
        let shown = self.shown_items().len();
        let tab_names = |vm: &mut Vm, ids: &[u8], icons: bool| -> Result<Value, String> {
            let names = ids
                .iter()
                .map(|&id| {
                    let (label, icon) = TABS[usize::from(id)];
                    Value::Text(if icons { icon } else if id == 0 { MY_PARKS } else { label }.to_owned())
                })
                .collect();
            Ok(Value::Object(vm.array(names)?))
        };
        Ok(Some(match method {
            "GetTitle" => text("ID_COMMON_BROWSER_SKATE_REEL_SKATEPARK"),
            "GetNumItems" | "GetNumItemsForScrollBar" => Value::Number(shown as f64),
            // 825B7AF0: 21 fields (see the survey); the park's picture index as text in [2].
            "GetMediaThumbData" => {
                let Some(item) = self.shown_items().get(index(0)).cloned() else { return Ok(Some(Value::Undefined)) };
                let mut fields = vec![Value::Text(String::new()); 21];
                fields[1] = text("0.0");
                fields[2] = Value::Text(item.picture.map_or(String::new(), |p| p.to_string()));
                fields[3] = Value::Number(0.0);
                fields[4] = Value::Bool(false);
                fields[5] = Value::Text(item.name.clone());
                fields[6] = Value::Number(0.0);
                fields[8] = Value::Number(0.0);
                fields[10] = text("ID_TAG_NONE");
                fields[13] = Value::Text(item.date.clone());
                fields[14] = Value::Number(0.0);
                fields[15] = Value::Number(0.0);
                fields[16] = Value::Bool(false);
                fields[17] = Value::Number(0.0);
                fields[18] = Value::Text(item.name);
                fields[20] = Value::Bool(false);
                Value::Object(vm.array(fields)?)
            }
            "GetSelectedTopLevelTabIndex" => Value::Number(self.top as f64),
            "GetSelectedBottomLevelTabIndex" => Value::Number(self.bottom as f64),
            "GetTopLevelTabNames" => tab_names(vm, &TOP, false)?,
            "GetBottomLevelTabNames" => tab_names(vm, self.bottom_tabs(), false)?,
            "GetBottomLevelTabIconNames" => tab_names(vm, self.bottom_tabs(), true)?,
            // 825B8268 / 825B8350: the provider's select and delete (Saved Parks only).
            "OnMediaThumbSelected" | "OnMediaThumbDelete" => {
                let i = self.current;
                if self.on_saved() && i < self.items.len() {
                    self.events.push(if method == "OnMediaThumbSelected" { BrowserEvent::Select(i) } else { BrowserEvent::Delete(i) });
                }
                Value::Undefined
            }
            "OnMediaThumbUpload" | "OnMediaThumbBookmark" | "OnMediaThumbFeature" | "SignInToOnline" => Value::Undefined,
            "GetCurrentMediaThumbIndex" => Value::Number(self.current as f64),
            "GetCurrentWindowIndex" => Value::Number((self.current / PAGE) as f64),
            "GetSpaceAvailableText" => text("#"),
            "ShouldShowSpaceAvailableText" | "ShowDeleteBookmarkButton" | "ShowBookmarkButton" | "ShowUploadButton" | "ShowFeatureButton"
            | "ShowPlayerFeatureButton" | "IsOnline" | "GetIsFromWebKit" | "GetHasOnlineContentError" => Value::Bool(false),
            "ShowDeleteButton" => Value::Bool(self.on_saved() && shown > 0),
            "ShowSelectButton" => Value::Bool(true),
            "GetCurrentContentType" => Value::Number(CONTENT_PARKS),
            _ => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parks(n: usize) -> Vec<BrowserItem> {
        (0..n).map(|i| BrowserItem { name: format!("park {i}"), date: String::new(), picture: Some(i as u32) }).collect()
    }

    #[test]
    fn the_dpad_moves_within_the_saved_parks_and_pages_refill() {
        let mut b = BrowserHost::open(parks(10));
        b.states.clear();
        b.key(BrowserKey::Right);
        assert_eq!((b.current, b.states.pop()), (1, Some(MODE_THUMB)));
        b.key(BrowserKey::Down);
        b.key(BrowserKey::Down);
        assert_eq!((b.current, b.states.pop()), (9, Some(MODE_PARKS_READY)), "the second page");
        b.key(BrowserKey::Right);
        assert_eq!(b.current, 9, "the last park");
    }

    #[test]
    fn the_tabs_change_what_shows_and_b_closes() {
        let mut b = BrowserHost::open(parks(3));
        assert_eq!(b.shown_items().len(), 3);
        b.key(BrowserKey::BottomNext);
        assert_eq!((b.bottom, b.shown_items().len()), (1, 0), "Shared Parks: none offline");
        b.key(BrowserKey::TopNext);
        assert_eq!((b.top, b.bottom, b.shown_items().len()), (1, 0, 0), "Downloaded");
        b.key(BrowserKey::TopPrevious);
        assert_eq!(b.shown_items().len(), 3);
        b.key(BrowserKey::Back);
        assert!(b.closed);
    }
}
