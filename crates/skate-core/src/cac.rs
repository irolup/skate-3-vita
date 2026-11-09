//! Retail's character customisation screen (FE 15, `CreateASkater`, template 92 `cas/createskater.swf`; TU3, research
//! of 3 Nov 2026, docs/customization-survey.md). The movie moves the selection itself and reports it
//! (`CAC_UpdateMenuIndices`); it asks the natives for titles, rows and values, and `SetState` picks its layout.
//! Left, Right and Back are the C++ handler's (825FC7E8), Select comes back through `Game.OnMenuSelect` (825FE3C0).
//!
//! The tables come from the executable: the items from the state's constructor (825F4B98), the menus from 0x8302FA10
//! and their static lists from 0x83026778. Runtime lists (gestures, thumbnails) are built by 82600AF8.
//!
//! Edit Skater (menu 15), Edit Style (16), Back Alley Pete's (17 to 29: gender, skin, hair, body shape, presets, the
//! face, facial hair) and Slappy's (1 to 3 and the thumbnails, 10: lists, trying on, wearing, locks; no cash in TU3).
//! TU3 never opens Slappy's filters (31, 32) and never fills its brand list, so they are not ported (a sponsor logo is
//! always `no_image`, as retail's). NOT RETAIL YET: the tattoos are closed (their rows do nothing); Wear Styles'
//! graphic rows do nothing until the Graphics menus; the first-visit tips (825FAA48); the gesture
//! preview and the camera moves of `82539688` / `82536778`; the fade around the screen (825E6B30); the row sounds are
//! stand-ins for retail's hashed fe records.
use crate::cas_rules::{Camera, Catalog, Field, Record, Vault, THUMB_REMOVE};
use crate::skater_record::{ColourLayer, Gender, Graphic, Item as Worn, SkaterRecord, Slot};
use crate::stamp::{self, Placement};
use crate::unlocks::{UnlockKind, UnlockStore, FRESH_GESTURES};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: &'static str,
    pub description: &'static str,
    /// The menu it opens, -1 none.
    pub target: i32,
    /// `option`, `slider`, `selector`, `category`, `morph`, `color`, or empty (a thumbnail list row).
    pub kind: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    pub name: &'static str,
    pub title: &'static str,
    /// The movie's layout: 0 list, 1 thumbnails, 2 style, 3 morph, 4 scale, 5 tattoo, 6 custom thumbnails.
    pub state: i32,
    /// The kinds of the A and B button texts.
    pub help_a: u32,
    pub help_b: u32,
    pub items: &'static [usize],
}

#[rustfmt::skip]
pub const ITEMS: [Item; 115] = [
    Item { label: "ID_CAC_BACKALLEY_PETES_TITLE", description: "ID_CAC_BACKALLEY_PETES_DESC", target: 17, kind: "option" },
    Item { label: "ID_CAC_SLAPPYS_MERCH_TITLE", description: "ID_CAC_SLAPPYS_MERCH_DESC", target: 1, kind: "option" },
    Item { label: "ID_CAC_ACCESSORIES_TITLE", description: "", target: 2, kind: "" },
    Item { label: "ID_CAC_SKATEBOARDS_TITLE", description: "", target: 3, kind: "" },
    Item { label: "ID_CAC_HATS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_TSHIRTS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_BUTTONSHIRTS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_HOODIES_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_JACKETS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_SWEATERS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_LEGS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_SHOES_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_SOCKS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_GLASSES_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_WRIST_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_BOARDS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_TRUCKS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_WHEELS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_BOARDGRAPHICS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_UPPER_BODY_TATTOO", description: "", target: 4, kind: "option" },
    Item { label: "ID_CAC_LOWER_BODY_TATTOO", description: "", target: 5, kind: "option" },
    Item { label: "ID_CAC_TATTOO_CATEGORY_01", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_TATTOO_CATEGORY_02", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_TATTOO_CATEGORY_03", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_TATTOO_CATEGORY_04", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_TATTOO_CATEGORY_05", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_TATTOO_CATEGORY_06", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_UPPERTATTOO_FRONT", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_UPPERTATTOO_BACK", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_UPPERTATTOO_LEFTARM", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_UPPERTATTOO_RIGHTARM", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_LOWERATTOO_LEFTLEG", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_LOWERATTOO_RIGHTLEG", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_PIERCINGS_TITLE", description: "", target: 9, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_LEFTEAR_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_RIGHTEAR_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_LEFTBROW_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_RIGHTBROW_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_LEFTNOSE_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_MIDDLENOSE_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_RIGHTNOSE_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_PIERCINGS_LIP_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_NECKLACES_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_RINGS_TITLE", description: "", target: 10, kind: "" },
    Item { label: "ID_CAC_FINISHED_TITLE", description: "", target: -1, kind: "" },
    Item { label: "ID_CAC_SKATE_STYLE_TITLE", description: "ID_CAC_SKATE_STYLE_DESC", target: 16, kind: "option" },
    Item { label: "ID_CAC_LABEL_TRUCK_TIGHTNESS", description: "ID_CAC_LABEL_TRUCK_DESC", target: -1, kind: "slider" },
    Item { label: "ID_CAC_LABEL_WHEEL_STIFFNESS", description: "ID_CAC_LABEL_WHEEL_DESC", target: -1, kind: "slider" },
    Item { label: "", description: "ID_CAC_SKATE_GESTURE_DESC", target: -1, kind: "category" },
    Item { label: "", description: "ID_CAC_SKATE_GESTURE_DESC", target: -1, kind: "category" },
    Item { label: "", description: "ID_CAC_SKATE_GESTURE_DESC", target: -1, kind: "category" },
    Item { label: "", description: "ID_CAC_SKATE_GESTURE_DESC", target: -1, kind: "category" },
    Item { label: "ID_CAC_EDIT_POSTURE", description: "ID_CAC_EDIT_POSTURE_DESC", target: -1, kind: "selector" },
    Item { label: "ID_CAC_EDIT_STYLE", description: "ID_CAC_EDIT_STYLE_DESC", target: -1, kind: "selector" },
    Item { label: "ID_CAC_EDIT_STANCE", description: "ID_CAC_EDIT_STANCE_DESC", target: -1, kind: "selector" },
    Item { label: "ID_CAC_CHOOSE_GENDER_TITLE", description: "", target: -1, kind: "" },
    Item { label: "ID_CAC_SKIN_TITLE", description: "", target: 18, kind: "" },
    Item { label: "ID_CAC_BODY_SHAPE_TITLE", description: "", target: 19, kind: "" },
    Item { label: "ID_CAC_HAIR_TITLE", description: "", target: 20, kind: "" },
    Item { label: "ID_CAC_FACIAL_HAIR_TITLE", description: "", target: 21, kind: "" },
    Item { label: "ID_CAC_FACE_MODIFICATION_TITLE", description: "", target: 22, kind: "" },
    Item { label: "ID_CAC_SKIN_COLOUR_TITLE", description: "", target: -1, kind: "color" },
    Item { label: "ID_CAC_SKIN_TONE", description: "", target: -1, kind: "selector" },
    Item { label: "ID_CAC_BODY_SHAPE_SIZE", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_HAIR_STYLE", description: "", target: -1, kind: "selector" },
    Item { label: "ID_CAC_HAIR_COLOUR", description: "", target: -1, kind: "color" },
    Item { label: "ID_CAC_FACIAL_HAIR_STYLE", description: "", target: -1, kind: "selector" },
    Item { label: "ID_CAC_FACIAL_HAIR_COLOUR", description: "", target: -1, kind: "color" },
    Item { label: "ID_CAC_EYES_TITLE", description: "", target: 23, kind: "" },
    Item { label: "ID_CAC_BROW_TITLE", description: "", target: 24, kind: "" },
    Item { label: "ID_CAC_NOSE_TITLE", description: "", target: 25, kind: "" },
    Item { label: "ID_CAC_JAW_TITLE", description: "", target: 26, kind: "" },
    Item { label: "ID_CAC_MOUTH_TITLE", description: "", target: 27, kind: "" },
    Item { label: "ID_CAC_CHIN_TITLE", description: "", target: 28, kind: "" },
    Item { label: "ID_CAC_PRESETS_TITLE", description: "", target: 29, kind: "" },
    Item { label: "ID_CAC_EYES_COLOUR", description: "", target: -1, kind: "color" },
    Item { label: "ID_CAC_EYES_ROTATION", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_EYES_WIDTH", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_EYES_HEIGHT", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_BROW_STYLE", description: "", target: -1, kind: "selector" },
    Item { label: "ID_CAC_BROW_PROFILE", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_BROW_ROTATION", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_BROW_HEIGHT", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_NOSE_LENGTH", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_NOSE_WIDTH", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_NOSE_HEIGHT", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_NOSE_CURVE", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_JAW_DEFINITION", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_JAW_ROUNDNESS", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_MOUTH_WIDTH", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_MOUTH_HEIGHT", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_MOUTH_FULLNESS", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_MOUTH_SMILE", description: "", target: -1, kind: "morph" },
    Item { label: "ID_CAC_CHIN_LENGTH", description: "", target: -1, kind: "morph" },
    Item { label: "", description: "", target: -1, kind: "category" },
    Item { label: "ID_CAC_MORPHING_UNDO", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_MORPHING_UNDO", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_WEARSTYLES_COLOUR", description: "", target: -1, kind: "color" },
    Item { label: "ID_CAC_WEARSTYLES_STYLE", description: "", target: -1, kind: "category" },
    Item { label: "ID_CAC_ADD_GRAPHIC", description: "", target: 33, kind: "option" },
    Item { label: "ID_CAC_BRANDGRAPHICS_TITLE", description: "", target: 13, kind: "option" },
    Item { label: "ID_CAC_TEAMGRAPHICS_TITLE", description: "", target: 14, kind: "option" },
    Item { label: "ID_CAC_ADD_GRAPHIC", description: "", target: 12, kind: "option" },
    Item { label: "ID_CAC_REMOVE_GRAPHIC", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_FILTER_ALL", description: "", target: -1, kind: "" },
    Item { label: "ID_CAC_FILTER_NEW", description: "", target: -1, kind: "" },
    Item { label: "ID_CAC_FILTER_OWNED", description: "", target: -1, kind: "" },
    Item { label: "ID_CAC_FILTER_BRANDS", description: "", target: 32, kind: "" },
    Item { label: "ID_CAC_FILTER_LOCKED", description: "", target: -1, kind: "" },
    Item { label: "ID_CAC_FILTER_DLC", description: "", target: -1, kind: "" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_SHIRT_FRONT", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_SHIRT_BACK", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_SHIRT_LEFTARM", description: "", target: -1, kind: "option" },
    Item { label: "ID_CAC_CUSTOMGRAPHIC_SHIRT_RIGHTARM", description: "", target: -1, kind: "option" },
];

#[rustfmt::skip]
pub const MENUS: [Menu; 37] = [
    Menu { name: "Shops", title: "ID_CAC_SHOPS_TITLE", state: 0, help_a: 1, help_b: 2, items: &[1, 0] },
    Menu { name: "Slappys", title: "ID_CAC_MAINMENU_TITLE", state: 0, help_a: 1, help_b: 2, items: &[4, 5, 6, 7, 8, 9, 10, 11, 12, 2, 3] },
    Menu { name: "Accessories", title: "ID_CAC_ACCESSORIES_TITLE", state: 0, help_a: 1, help_b: 2, items: &[13, 14, 43, 44] },
    Menu { name: "Skateboards", title: "ID_CAC_SKATEBOARDS_TITLE", state: 0, help_a: 1, help_b: 2, items: &[15, 16, 17] },
    Menu { name: "TattooUpperRegions", title: "ID_CAC_UPPER_BODY_TATTOO", state: 0, help_a: 1, help_b: 2, items: &[27, 28, 29, 30] },
    Menu { name: "TattooLowerRegions", title: "ID_CAC_LOWER_BODY_TATTOO", state: 0, help_a: 1, help_b: 2, items: &[31, 32] },
    Menu { name: "TattooCategories", title: "ID_CAC_TATTOOS_TITLE", state: 0, help_a: 1, help_b: 0, items: &[] },
    Menu { name: "TattooUpperAdjust", title: "ID_CAC_GRAPHIC_ADJUST", state: 5, help_a: 2, help_b: 3, items: &[] },
    Menu { name: "TattooLowerAdjust", title: "ID_CAC_GRAPHIC_ADJUST", state: 5, help_a: 2, help_b: 3, items: &[] },
    Menu { name: "Piercings", title: "ID_CAC_PIERCINGS_TITLE", state: 0, help_a: 1, help_b: 2, items: &[35, 36, 37, 38, 39, 40, 41, 42] },
    Menu { name: "PartThumbnails", title: "ID_CAC_THUMBNAILS_TITLE", state: 1, help_a: 2, help_b: 2, items: &[] },
    Menu { name: "TattooThumbnails", title: "ID_CAC_THUMBNAILS_TITLE", state: 6, help_a: 1, help_b: 2, items: &[] },
    Menu { name: "GraphicCategories", title: "ID_CAC_GRAPHICS_TITLE", state: 0, help_a: 1, help_b: 0, items: &[101, 102] },
    Menu { name: "BrandGraphicThumbnails", title: "ID_CAC_THUMBNAILS_TITLE", state: 6, help_a: 1, help_b: 2, items: &[] },
    Menu { name: "TeamGraphicThumbnails", title: "ID_CAC_THUMBNAILS_TITLE", state: 6, help_a: 1, help_b: 2, items: &[] },
    Menu { name: "EditSkaterMain", title: "ID_CAC_EDITSKATER_TITLE", state: 3, help_a: 1, help_b: 1, items: &[0, 1, 46, 47, 48] },
    Menu { name: "EditSkaterStyle", title: "ID_CAC_SKATE_STYLE_TITLE", state: 2, help_a: 2, help_b: 0, items: &[49, 50, 51, 52, 55, 54, 53] },
    Menu { name: "BackAlleyPete", title: "ID_CAC_BODY_MODIFICATION_TITLE", state: 0, help_a: 1, help_b: 2, items: &[56, 57, 59, 58, 75, 61, 60, 19, 20] },
    Menu { name: "Skin", title: "ID_CAC_SKIN_TITLE", state: 3, help_a: 2, help_b: 0, items: &[62, 96] },
    Menu { name: "BodyShape", title: "ID_CAC_BODY_SHAPE_TITLE", state: 3, help_a: 2, help_b: 0, items: &[64, 97] },
    Menu { name: "Hair", title: "ID_CAC_HAIR_TITLE", state: 3, help_a: 2, help_b: 0, items: &[65, 66, 96] },
    Menu { name: "FacialHair", title: "ID_CAC_FACIAL_HAIR_TITLE", state: 3, help_a: 2, help_b: 0, items: &[67, 96] },
    Menu { name: "FaceMod", title: "ID_CAC_FACE_MODIFICATION_TITLE", state: 0, help_a: 2, help_b: 2, items: &[69, 70, 71, 72, 73, 74] },
    Menu { name: "Eyes", title: "ID_CAC_EYES_TITLE", state: 3, help_a: 2, help_b: 0, items: &[76, 77, 78, 79, 96] },
    Menu { name: "Brow", title: "ID_CAC_BROW_TITLE", state: 3, help_a: 2, help_b: 0, items: &[80, 81, 82, 83, 96] },
    Menu { name: "Nose", title: "ID_CAC_NOSE_TITLE", state: 3, help_a: 2, help_b: 0, items: &[84, 85, 86, 87, 96] },
    Menu { name: "Jaw", title: "ID_CAC_JAW_TITLE", state: 3, help_a: 2, help_b: 0, items: &[88, 89, 96] },
    Menu { name: "Mouth", title: "ID_CAC_MOUTH_TITLE", state: 3, help_a: 2, help_b: 0, items: &[90, 92, 93, 96] },
    Menu { name: "Chin", title: "ID_CAC_CHIN_TITLE", state: 3, help_a: 2, help_b: 0, items: &[94, 96] },
    Menu { name: "Presets", title: "ID_CAC_PRESETS_TITLE", state: 3, help_a: 2, help_b: 0, items: &[95] },
    Menu { name: "WearStyles", title: "ID_CAC_WEAR_STYLES_TITLE", state: 3, help_a: 2, help_b: 0, items: &[98, 99] },
    Menu { name: "Filters", title: "ID_CAC_FILTERS_TITLE", state: 0, help_a: 1, help_b: 2, items: &[] },
    Menu { name: "BrandFilters", title: "ID_CAC_BRAND_FILTERS_TITLE", state: 0, help_a: 1, help_b: 2, items: &[] },
    Menu { name: "ShirtGraphicLocation", title: "ID_CAC_SHIRT_GRAPHIC_LOCATION", state: 0, help_a: 1, help_b: 0, items: &[111, 112, 113, 114] },
    Menu { name: "ShirtGraphicAdjust", title: "ID_CAC_GRAPHIC_ADJUST", state: 4, help_a: 2, help_b: 3, items: &[] },
    Menu { name: "BoardGraphicAdjust", title: "ID_CAC_GRAPHIC_ADJUST", state: 4, help_a: 2, help_b: 3, items: &[] },
    Menu { name: "HatGraphicAdjust", title: "ID_CAC_GRAPHIC_ADJUST", state: 4, help_a: 2, help_b: 3, items: &[] },
];

pub const MENU_EDIT_SKATER: usize = 15;
pub const MENU_EDIT_STYLE: usize = 16;
/// The rows of Edit Skater and Edit Style that this slice edits (retail's item ids).
const ITEM_EDIT_STYLE: usize = 0x2E;
const ITEM_TRUCKS: usize = 0x2F;
const ITEM_WHEELS: usize = 0x30;
const ITEM_GESTURES: [usize; 4] = [0x31, 0x32, 0x33, 0x34];
const ITEM_POSTURE: usize = 0x35;
const ITEM_STYLE: usize = 0x36;
const ITEM_STANCE: usize = 0x37;
const ITEM_GENDER: usize = 0x38;
const ITEM_BODY_SIZE: usize = 0x40;
const ITEM_HAIR_STYLE: usize = 0x41;
const ITEM_FACIAL_HAIR: usize = 0x43;
const ITEM_BROW_STYLE: usize = 0x50;
const ITEM_FACIAL_HAIR_PAGE: usize = 0x3C;
const ITEM_PRESET: usize = 0x5F;
const ITEM_UNDO: usize = 0x60;
const ITEM_BODY_UNDO: usize = 0x61;

pub const MENU_SLAPPYS: usize = 1;
pub const MENU_BACK_ALLEY: usize = 0x11;
const MENU_HAIR: usize = 0x14;
const MENU_FACIAL_HAIR: usize = 0x15;
const MENU_FACE: usize = 0x16;
const MENU_BROW: usize = 0x18;
pub const MENU_THUMBNAILS: usize = 10;
pub const MENU_WEAR_STYLES: usize = 0x1E;
/// Wear Styles' rows (825F4B98): the colour, the style, the graphic rows (Add Graphic on a top, on the others, Remove).
const ITEM_WEAR_COLOUR: usize = 0x62;
const ITEM_WEAR_STYLE: usize = 0x63;
const ITEM_ADD_SHIRT_GRAPHIC: usize = 0x64;
const ITEM_ADD_GRAPHIC: usize = 0x67;
const ITEM_REMOVE_GRAPHIC: usize = 0x68;

// The tattoo pages (Back Alley Pete's rows 19 and 20): the regions (4 upper, 5 lower), the categories (6), the stamps
// (0xB) and the placement pages (7 upper, 8 lower).
pub const MENU_TATTOO_UPPER: usize = 4;
pub const MENU_TATTOO_LOWER: usize = 5;
pub const MENU_TATTOO_CATEGORIES: usize = 6;
pub const MENU_TATTOO_ADJUST_UPPER: usize = 7;
pub const MENU_TATTOO_ADJUST_LOWER: usize = 8;
pub const MENU_TATTOO_STAMPS: usize = 0xB;
// The graphics pages (Wear Styles' Add Graphic): the categories (0xC), the brand and team logos (0xD, 0xE), a shirt's
// location (0x21) and the placement pages of a shirt, a board and a hat (0x22 to 0x24).
pub const MENU_GRAPHIC_CATEGORIES: usize = 12;
pub const MENU_GRAPHIC_BRAND: usize = 13;
pub const MENU_GRAPHIC_TEAM: usize = 14;
pub const MENU_SHIRT_LOCATION: usize = 0x21;
pub const MENU_ADJUST_SHIRT: usize = 0x22;
pub const MENU_ADJUST_BOARD: usize = 0x23;
pub const MENU_ADJUST_HAT: usize = 0x24;
const ITEM_BRAND_GRAPHICS: usize = 0x65;
const ITEM_TEAM_GRAPHICS: usize = 0x66;
const ITEM_SHIRT_REGIONS: usize = 0x6F;
/// Per shirt location the camera and the pose (0x83044528 and 0x830445C8 resolved to their vault keys).
const SHIRT_CAMERAS: [&str; 4] = ["custom_shirt_front", "custom_shirt_back", "custom_shirt_left_arm", "custom_shirt_right_arm"];
const SHIRT_BODIES: [&str; 4] = ["front", "back", "left_arm", "right_arm"];
const ITEM_UPPER_TATTOO: usize = 19;
const ITEM_LOWER_TATTOO: usize = 20;
/// The first category row (`0x15 + i`, 82600AF8 menu 6) and the region rows.
const ITEM_TATTOO_CATEGORY: usize = 0x15;
const ITEM_UPPER_REGIONS: usize = 0x1B;
const ITEM_LOWER_REGIONS: usize = 0x1F;
/// Per region the camera and the pose (0x83044548, 0x830445E8 and 0x83044568, 0x83044608 resolved to their vault keys).
const UPPER_CAMERAS: [&str; 4] = ["upper_tattoo_front", "upper_tattoo_back", "upper_tattoo_left_arm", "upper_tattoo_right_arm"];
const UPPER_BODIES: [&str; 4] = ["front", "back", "left_arm", "right_arm"];
const LOWER_CAMERAS: [&str; 2] = ["lower_tattoo_left_leg", "lower_tattoo_right_leg"];
const LOWER_BODIES: [&str; 2] = ["left_leg", "right_leg"];
/// The stamp zones' keys (0x48A4C0A51F96FF17 and 0xB1FDC7DC52BFA174 are their key hashes).
const ZONE_UPPER: &str = "tattooupper";
const ZONE_LOWER: &str = "tattoolower";

/// How a Slappy's page keeps a row of its slot (the class objects' row test, vtable +8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowTest {
    Any,
    /// T-shirts (823044A0, filled by 825F3AC8 with the texts the catalog holds).
    TopTypeIn(&'static [&'static str]),
    /// Button shirts, hoodies, jackets, sweaters (825F3A58: a text the catalog lacks keeps nothing).
    TopType(&'static str),
    /// Socks (825F3D78 with 825F3D18: not `tiny`).
    SockStyleNot(&'static str),
}

/// A Slappy's row's class (its +0x14, 825F4B98): the slot it lists (vtable +0x14) and its row test. [code]
fn class_of(item: usize) -> Option<(Slot, RowTest)> {
    Some(match item {
        4 => (Slot::Hat, RowTest::Any),
        5 => (Slot::OuterTorso, RowTest::TopTypeIn(&["tshirt", "ls_tshirt"])),
        6 => (Slot::OuterTorso, RowTest::TopType("buttonshirt")),
        7 => (Slot::OuterTorso, RowTest::TopType("hoody")),
        8 => (Slot::OuterTorso, RowTest::TopType("jacket")),
        9 => (Slot::OuterTorso, RowTest::TopType("sweater")),
        0xA => (Slot::Pants, RowTest::Any),
        0xB => (Slot::Feet, RowTest::Any),
        0xC => (Slot::Sock, RowTest::SockStyleNot("tiny")),
        0xD => (Slot::Glasses, RowTest::Any),
        0xE => (Slot::WristItem, RowTest::Any),
        0xF => (Slot::SkateBoard, RowTest::Any),
        0x10 => (Slot::SkateTruck, RowTest::Any),
        0x11 => (Slot::SkateWheel, RowTest::Any),
        0x2B => (Slot::Jewellery, RowTest::Any),
        0x2C => (Slot::Accessory, RowTest::Any),
        _ => return None,
    })
}

/// The skater's pose a row sets when it opens its page (its +0x30, `cac_body`); the others keep `standing`. [code]
fn body_of(item: usize) -> Option<&'static str> {
    Some(match item {
        0xE => "wrist",
        0x11 => "wheels",
        0x2C => "rings",
        _ => return None,
    })
}

/// The sponsor inventories of the content packs (table 0x8302676C, "Special Filter" 825F33A0): their items are hidden
/// until a pack grants them. [code]
const PACK_UNLOCKS: [&str; 2] = ["cac_throwback_pack_inventory", "cac_dlc_first_gear_pack_inventory"];
/// The thumbnails page tries a row on after it is highlighted this long (825FBAE8: 500 ms). [code]
const TRY_ON_DELAY: f32 = 0.5;

/// The camera a row moves the preview to when it opens its page (its +0x28, 825F4B98; 825FE3C0 sets it).
fn camera_of(item: usize) -> Option<&'static str> {
    Some(match item {
        0x39 | 0x3A => "body",
        0x3B | 0x3C | 0x3D | 0x4B => "head",
        0x45 => "eyes",
        0x46 => "brow",
        0x47 => "nose",
        0x48 => "jaw",
        0x49 => "mouth",
        0x4A => "chin",
        4 => "hat",
        5..=9 => "chest",
        0xA => "legs",
        0xB | 0xC => "feet",
        0xD => "face",
        0xE => "wrist_left",
        0xF => "skateboard",
        0x10 => "skatetruck",
        0x11 => "skatewheel",
        0x2B => "necklace",
        0x2C => "rings",
        _ => return None,
    })
}

/// A morph row's `cac_morph_params` row (its +0x58, 825F4B98).
fn morph_of(item: usize) -> Option<&'static str> {
    Some(match item {
        ITEM_BODY_SIZE => "global_fat",
        0x4D => "eye_rotation",
        0x4E => "eye_width",
        0x4F => "eye_height",
        0x51 => "brows_depth",
        0x52 => "brows_rotation",
        0x53 => "brows_height",
        0x54 => "nose_length",
        0x55 => "nose_width",
        0x56 => "nose_height",
        0x57 => "nose_curve",
        0x58 => "jaw_chiseled",
        0x59 => "jaw_depth",
        0x5A => "mouth_width",
        0x5B => "mouth_height",
        0x5C => "mouth_lipsize",
        0x5D => "mouth_corner",
        0x5E => "chin_length",
        _ => return None,
    })
}

/// A colour row's zone (82603E98: the zone's colourised slot).
fn zone_of(item: usize) -> Option<u32> {
    Some(match item {
        0x3E => 2,
        0x42 => 1,
        0x44 => 8,
        0x4C => 6,
        _ => return None,
    })
}

/// The zones whose colour is part of a preset: picking one leaves the preset (825FC7E8; their keys are in the presets).
const ZONES_OF_PRESETS: [u32; 2] = [2, 6];
/// `cac_fe_morph_settings`: the stick's dead zone and a slider's step per tick. [data]
const MORPH_DEAD_ZONE: f32 = 0.1;
const MORPH_STEP: f32 = 0.01;
/// Retail's ticks a second: its step is per tick (825FBAE8), the Vita's frames are not. Survey note.
const TICKS: f32 = 30.0;
/// The triggers' dead zone for turning the skater (`0x821E9580`). [code]
const TURN_DEAD_ZONE: f32 = 0.02;
/// `CAC_GetIntegerValue` of a slider (DAT_822F95E4).
const MORPH_BARS: f32 = 122.0;
/// The preset that is none (`custom`, 825FAFF0).
const PRESET_CUSTOM: &str = "custom";

/// Movie state: refresh the current row's value (`SetState(50)`).
pub const STATE_REFRESH_ROW: i32 = 50;
/// Movie state: the page group changed (`SetState(51)`, 82600AF8).
pub const STATE_GROUP: i32 = 51;
/// Movie state: refresh the current thumbnail's icon (`SetState(53)`, 825FD9F0).
pub const STATE_PART_ICON: i32 = 53;

/// The labels of retail's gestures, by gesture index (word 0x60C of the state).
#[rustfmt::skip]
pub const GESTURE_LABELS: [&str; 37] = [
    "ID_GESTURE_AIRGUITAR", "ID_GESTURE_AIRPLANE", "ID_GESTURE_BOXING", "ID_GESTURE_BRUCE", "ID_GESTURE_CHECKTIME",
    "ID_GESTURE_DEVIL", "ID_GESTURE_DOUBLE_GUN", "ID_GESTURE_DUNNO", "ID_GESTURE_FINGERWAG", "ID_GESTURE_FISTS",
    "ID_GESTURE_BICEPFLEX", "ID_GESTURE_FLIPTABLE", "ID_GESTURE_FONZ", "ID_GESTURE_FREEDOM", "ID_GESTURE_FU",
    "ID_GESTURE_GETAWAY", "ID_GESTURE_GETOUTTAHERE", "ID_GESTURE_HANDCUFFS", "ID_GESTURE_PUMP_HI", "ID_GESTURE_PUMP_LO",
    "ID_GESTURE_PREWIND", "ID_GESTURE_PEACE", "ID_GESTURE_FINGERPOINT", "ID_GESTURE_RAISEROOF", "ID_GESTURE_SHAKA",
    "ID_GESTURE_SHRUG", "ID_GESTURE_SKY", "ID_GESTURE_SNAP", "ID_GESTURE_SOULARCH", "ID_GESTURE_SPOCK",
    "ID_GESTURE_SURFSUP", "ID_GESTURE_SWINGHIGH", "ID_GESTURE_SWINGLOW", "ID_GESTURE_THROWARMS", "ID_GESTURE_THMB_DWN",
    "ID_GESTURE_WINGS", "ID_GESTURE_YARDSALE",
];

/// The keys the C++ handler takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Back,
    /// A (`AptNext`): taken only by the placement pages, which go back to Back Alley Pete's.
    Accept,
    /// X (`AptOption`): the placement page starts over (825FC7E8 key 4).
    Reset,
    /// Y (`AptMisc`): the placement page puts the top or the pants on and off (825FC7E8 key 5).
    Toggle,
}

/// What the pages read: the catalog, the vault rows and the disc's fallback characters by gender.
#[derive(Clone, Debug, PartialEq)]
pub struct CacData {
    pub catalog: Catalog,
    pub vault: Vault,
    pub fallback: [SkaterRecord; 2],
}

/// The preview: retail's camera row, the skater's spot on the set and heading (radians about up), its stand.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub camera: Camera,
    pub position: [f32; 3],
    pub heading: f32,
    pub stand: String,
    pub gender: Gender,
}

/// What the player's mesh needs after an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookChange {
    /// The same parts, other morph weights.
    Morphs,
    /// Other parts or colours: built anew, the keys wait for it as retail's (+0x1800, +0x1804).
    Rebuild,
}

/// The two shader rows per placement slot (shirt, upper and lower tattoo, board, hat) of a record: the placement's matrix columns while the slot has a
/// tattoo, zeros otherwise.
pub fn decal_rows_of(record: &SkaterRecord) -> [[[f32; 3]; 2]; 5] {
    std::array::from_fn(|at| {
        if record.adjust.words[at] == u32::MAX {
            [[0.0; 3]; 2]
        } else {
            Placement::from_values(record.adjust.values[at]).custom_graphic()
        }
    })
}

/// What the tattoo pages keep between them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Tattoo {
    /// The region row chosen (+0x1C3C) and the category page (+0x17D0).
    item: usize,
    category: usize,
    /// The top and the pants as they were before a page took them off (+0x1738 and +0x1748; +0x16E4 and +0x16E5 say so).
    top: Option<Worn>,
    pants: Option<Worn>,
    /// The shirt's graphic kept while the top is off (825FB718 saves its adjust, +0x1720 to +0x1730).
    shirt_graphic: Option<(Graphic, u32, [f32; 4])>,
    /// The stamp being placed, and the row the stamps page was opened on.
    stamp: Option<Worn>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cac {
    pub record: SkaterRecord,
    data: Option<Arc<CacData>>,
    /// Which page group this is (+0x177C): 0 Slappy's, 1 Back Alley Pete's, 2 Edit Skater.
    group: u32,
    /// The row list of a selector page (+0x1798, [`Worn::NONE`] is the row without hair, +0x1A84) and where it is (+0x1808).
    list: Vec<Worn>,
    list_at: usize,
    /// The preset shown (+0x1C20).
    preset: String,
    /// The record as the page group opened with it, for the undo rows (82538E78 with +0x5DF0).
    snapshot: SkaterRecord,
    /// Each gender's character from before a switch (82DDF3A0).
    genders: [Option<SkaterRecord>; 2],
    /// The preview's `cac_camera` row (82536778).
    camera: String,
    /// The skater's `cac_body` pose (the preview's +0x10, 82539688).
    body: &'static str,
    /// The unlock store as the screen opened (its locks and new marks; 82503AE0, 82503B60).
    unlocks: UnlockStore,
    /// The Slappy's row whose page is open (+0x178C, its class) and its title (+0x1788).
    category: Option<usize>,
    /// The highlighted thumbnail (+0x17A8) and how long since (+0x1C40 armed, +0x1C44 ms).
    highlight: Option<usize>,
    hover: Option<f32>,
    /// The item tried on, shown on the skater but not in the record (825370A8; 82537268 shows the record again).
    pub trying: Option<(Slot, Worn)>,
    /// Back on Edit Skater asks first (825FC768 > 82603290): the host's popup answers [`Cac::exit_answer`].
    pub ask_exit: bool,
    /// Wear Styles: the item it shows (+0x17A8), its rows (built by 82600AF8), the style row's place among the
    /// variants (+0x17AC; the variants are `list`, +0x1798).
    wear_item: Option<(Slot, Worn)>,
    wear_rows: Vec<usize>,
    wear_at: usize,
    /// The tattoo pages.
    tattoo: Tattoo,
    /// A region of a body that has a tattoo already asks first (825FE3C0 menus 4 and 5, 82604618 and 82604658): the
    /// host's popup answers [`Cac::tattoo_answer`]; this is whether it is the lower body's.
    pub ask_tattoo: Option<bool>,
    /// How far the triggers turned the skater from its heading, radians (8253B618's +100; 82536818 zeroes it).
    turn: f32,
    /// The mesh is out of date.
    pub look: Option<LookChange>,
    /// The keys wait for the mesh (+0x1800).
    pub busy: bool,
    /// The gender row asks first (825FE3C0): the host's popup answers [`Cac::switch_gender`].
    pub ask_gender: bool,
    /// The open menus (+0x16D0).
    stack: Vec<usize>,
    /// Per menu: the current row and the window's first row (0x8302FA28, 0x8302FA2C).
    cursor: [(i32, i32); 37],
    /// Edit Style's choices, by gesture row (+0x180C, +0x1818, +0x1814, +0x1810), written to the record on leaving.
    gesture_rows: [u32; 4],
    /// The gestures that can be chosen (+0x181C): the unlocked ones, in retail's order.
    gestures: Vec<u32>,
    /// Movie states to post, in order.
    pub states: Vec<i32>,
    pub sounds: Vec<&'static str>,
    /// Back on the first menu: the screen closes (82603290).
    pub closed: bool,
}

/// The placement slot of a graphic on a slot's item (the adjust record's index: shirt 0, upper tattoo 1, lower tattoo 2,
/// board 3, hat 4).
fn graphic_slot(slot: Slot) -> Option<usize> {
    match slot {
        Slot::OuterTorso => Some(0),
        Slot::SkateBoard => Some(3),
        Slot::Hat => Some(4),
        _ => None,
    }
}

/// The stamp zone of a graphic's item (8253E228).
fn graphic_zone(slot: Slot) -> &'static str {
    match slot {
        Slot::OuterTorso => "shirt",
        Slot::SkateBoard => "skateboard",
        _ => "hat",
    }
}

/// The adjust index of a placement page (the tattoos' 7 and 8, the graphics' 0x22 to 0x24).
fn adjust_index(menu: usize) -> Option<usize> {
    match menu {
        MENU_TATTOO_ADJUST_UPPER => Some(1),
        MENU_TATTOO_ADJUST_LOWER => Some(2),
        MENU_ADJUST_SHIRT => Some(0),
        MENU_ADJUST_BOARD => Some(3),
        MENU_ADJUST_HAT => Some(4),
        _ => None,
    }
}

/// The record's gesture slot of a gesture row (825FFE08 reads the rows, 826008C0 writes them).
const GESTURE_SLOT: [usize; 4] = [0, 3, 2, 1];

impl Cac {
    /// Opened on a skater (825FB0F8), on Edit Skater (825FAFF0, mode 0). `unlocks` limits the gestures (82600AF8).
    pub fn open(record: SkaterRecord, unlocks: &UnlockStore, data: Option<Arc<CacData>>) -> Self {
        let gestures = (0..FRESH_GESTURES.len() as u32).filter(|&i| unlocks.has(UnlockKind::Gestures, FRESH_GESTURES[i as usize], true)).collect();
        Self {
            data,
            group: 2,
            list: Vec::new(),
            list_at: 0,
            preset: PRESET_CUSTOM.to_owned(),
            snapshot: record.clone(),
            genders: [None, None],
            camera: "standing".into(),
            body: "standing",
            unlocks: unlocks.clone(),
            category: None,
            highlight: None,
            hover: None,
            trying: None,
            ask_exit: false,
            wear_item: None,
            wear_rows: Vec::new(),
            wear_at: 0,
            tattoo: Tattoo::default(),
            ask_tattoo: None,
            turn: 0.0,
            look: None,
            busy: false,
            ask_gender: false,
            record,
            stack: vec![MENU_EDIT_SKATER],
            cursor: [(0, 0); 37],
            gesture_rows: [0; 4],
            gestures,
            states: vec![MENUS[MENU_EDIT_SKATER].state],
            sounds: Vec::new(),
            closed: false,
        }
    }

    pub fn menu(&self) -> usize {
        *self.stack.last().expect("the stack starts with a menu and never empties")
    }

    fn table(&self) -> &'static Menu {
        &MENUS[self.menu()]
    }

    /// The row's item.
    fn item(&self, row: i32) -> Option<usize> {
        let r = usize::try_from(row).ok()?;
        if self.menu() == MENU_WEAR_STYLES {
            return self.wear_rows.get(r).copied();
        }
        if self.menu() == MENU_TATTOO_CATEGORIES {
            return (r < self.categories()).then_some(ITEM_TATTOO_CATEGORY + r);
        }
        self.table().items.iter().copied().filter(|&i| self.shown(i)).nth(r)
    }

    /// Back Alley Pete's lists facial hair for a male skater only (82600AF8 menu 0x11).
    fn shown(&self, item: usize) -> bool {
        item != ITEM_FACIAL_HAIR_PAGE || self.record.gender == Gender::Male
    }

    /// The preview's camera.
    pub fn camera(&self) -> Option<&Camera> {
        self.data.as_ref()?.vault.camera(&self.camera)
    }

    /// The per-frame turn of 825FBAE8: LT turns one way, RT the other (`axis` LT minus RT, 0 to 1 each), past the
    /// triggers' dead zone, by the body row's rate a tick (8253B618).
    pub fn turn_by(&mut self, axis: f32, seconds: f32) {
        let Some(rate) = self.data.as_ref().and_then(|d| d.vault.body(self.body)).map(|b| b.turn) else { return };
        if axis.abs() >= TURN_DEAD_ZONE {
            self.turn += rate * axis * seconds * TICKS;
        }
    }

    /// What the preview draws now: the camera row and the skater on the set (its `cac_body`, turned) with the stand
    /// of its posture.
    pub fn preview(&self) -> Option<Preview> {
        let vault = &self.data.as_ref()?.vault;
        let camera = vault.camera(&self.camera)?.clone();
        let body = vault.body(self.body).or_else(|| vault.body("standing"))?;
        let stand = vault.stand(self.record.posture).unwrap_or(&body.anim).to_owned();
        Some(Preview { camera, position: body.position, heading: body.heading.to_radians() + self.turn, stand, gender: self.record.gender })
    }

    /// The mesh is built: the keys go on.
    pub fn look_done(&mut self) {
        self.busy = false;
    }

    fn changed(&mut self) {
        self.look = Some(LookChange::Rebuild);
        self.busy = true;
    }

    /// `CAC_GetTitleText` (825A7190): the thumbnails page shows its category's label (+0x1788).
    pub fn title(&self) -> &'static str {
        match self.category {
            Some(item) if self.menu() == MENU_THUMBNAILS => ITEMS[item].label,
            // A deck's Wear Styles is "Edit Deck".
            _ if self.menu() == MENU_WEAR_STYLES && matches!(self.wear_item, Some((Slot::SkateBoard, _))) => "ID_CUSTOM_EDITDECK",
            _ => self.table().title,
        }
    }

    /// `CAC_GetNumItems` (825A7308): the thumbnails page counts its list (+0x1798).
    pub fn row_count(&self) -> usize {
        match self.menu() {
            MENU_THUMBNAILS | MENU_TATTOO_STAMPS | MENU_GRAPHIC_BRAND | MENU_GRAPHIC_TEAM => return self.list.len(),
            MENU_WEAR_STYLES => return self.wear_rows.len(),
            MENU_TATTOO_CATEGORIES => return self.categories(),
            _ => {}
        }
        self.table().items.iter().filter(|&&i| self.shown(i)).count()
    }

    /// `CAC_GetItemText`: the gender row names the skater's (825FF5xx).
    pub fn row_label(&self, row: i32) -> &'static str {
        if matches!(self.menu(), MENU_THUMBNAILS | MENU_TATTOO_STAMPS | MENU_GRAPHIC_BRAND | MENU_GRAPHIC_TEAM) {
            return "";
        }
        match self.item(row) {
            Some(ITEM_GENDER) if self.record.gender == Gender::Male => "ID_CAC_CHOOSE_GENDER_MALE",
            Some(ITEM_GENDER) => "ID_CAC_CHOOSE_GENDER_FEMALE",
            Some(i) => ITEMS[i].label,
            None => "",
        }
    }

    /// `CAC_GetDescriptionText` (825FF160). Empty texts read as `#`, the movie's nothing. A thumbnail is named by
    /// the first of its part names the string table holds (`has`, 82605528); the remove tile is `ID_CAC_REMOVE_PART`.
    pub fn row_description(&self, row: i32, has: &dyn Fn(&str) -> bool) -> String {
        if matches!(self.menu(), MENU_TATTOO_STAMPS | MENU_GRAPHIC_BRAND | MENU_GRAPHIC_TEAM) {
            // The remove tile is `ID_CAC_REMOVE_TATTOO`, a stamp its `ID_CAC_PART_` name (8253DEA0, +0xC).
            return match (self.list.get(usize::try_from(row).unwrap_or(usize::MAX)), &self.data) {
                (Some(item), _) if item.is_none() => "ID_CAC_REMOVE_TATTOO".into(),
                (Some(&item), Some(data)) => data.catalog.stamp_label(item),
                _ => "#".into(),
            };
        }
        if self.menu() == MENU_THUMBNAILS {
            let Some((slot, item)) = self.thumb(row) else { return "#".into() };
            if item.is_none() {
                return "ID_CAC_REMOVE_PART".into();
            }
            // Two materials are named by hand (825FF160). [code]
            if item.material == 0x2C7F_3817_0017_3587 || item.material == 0x2C7F_3817_0002_0626 {
                return "#Bryant".into();
            }
            let Some(data) = &self.data else { return "#".into() };
            let names = data.catalog.part_names(slot, item);
            let last = names.last().cloned().unwrap_or_default();
            return names.into_iter().find(|n| has(n)).unwrap_or(last);
        }
        self.item(row).map(|i| ITEMS[i].description).filter(|d| !d.is_empty()).unwrap_or("#").to_owned()
    }

    /// `CAC_GetOptionType` (825FF4C8): a thumbnail's icon is its state (the remove tile none).
    pub fn row_kind(&self, row: i32) -> &'static str {
        if matches!(self.menu(), MENU_TATTOO_STAMPS | MENU_GRAPHIC_BRAND | MENU_GRAPHIC_TEAM) {
            return "none";
        }
        if self.menu() == MENU_THUMBNAILS {
            return match self.thumb(row) {
                Some((_, item)) if item.is_none() => "none",
                Some((slot, item)) => self.part_state(slot, item),
                None => "none",
            };
        }
        self.item(row).map_or("", |i| ITEMS[i].kind)
    }

    /// `CAC_GetInfoPanelType` (825A74F0): from which page group this is (+0x177C, set by 82600AF8: Edit Skater's own
    /// pages 2); on Slappy's thumbnails the row's state, `blurb` for the remove tile.
    pub fn info_panel(&self, row: i32) -> &'static str {
        match self.group {
            1 => "petes",
            2 => "blurb",
            _ if self.menu() == MENU_THUMBNAILS => match self.thumb(row) {
                Some((_, item)) if item.is_none() => "blurb",
                Some((slot, item)) => self.part_state(slot, item),
                None => "noblurb",
            },
            _ => "noblurb",
        }
    }

    /// A thumbnail's slot and item ([`Worn::NONE`]: the remove tile, +0x1A84).
    fn thumb(&self, row: i32) -> Option<(Slot, Worn)> {
        if self.menu() != MENU_THUMBNAILS {
            return None;
        }
        let slot = class_of(self.category?)?.0;
        Some((slot, *self.list.get(usize::try_from(row).ok()?)?))
    }

    fn row_of(&self, slot: Slot, item: Worn) -> Option<&Record> {
        self.data.as_ref()?.catalog.get(slot, item)
    }

    /// 825F46C8: an item is locked while its clothing unlock (kind 5) or its sponsor inventory (kind 6) is not granted
    /// (both tables).
    fn locked(&self, slot: Slot, item: Worn) -> bool {
        let (Some(data), Some(r)) = (&self.data, self.row_of(slot, item)) else { return false };
        let need = |field: Field, kind: UnlockKind| {
            let name = data.catalog.text_of(r, field);
            !name.is_empty() && !self.unlocks.has(kind, name, true)
        };
        need(Field::Unlock, UnlockKind::ClothingUnlock) || need(Field::SponsorUnlock, UnlockKind::ClothingSponsor)
    }

    /// A thumbnail's state (825FF5E8): `owned` when it or one of its variants is worn, `locked`, `dlc` for an installed
    /// DLC item (825F4500: no DLC here, so never), else `none`.
    fn part_state(&self, slot: Slot, item: Worn) -> &'static str {
        let Some(data) = &self.data else { return "none" };
        let worn = self.record.worn[slot as usize];
        if data.catalog.get(slot, worn).is_some() && (worn == item || data.catalog.variants(slot, worn).contains(&item)) {
            return "owned";
        }
        if self.locked(slot, item) {
            return "locked";
        }
        "none"
    }

    /// `CAC_GetItemUnlockHALID` (825FF3E0): `ID_<unlock>_UNLOCK` of its clothing unlock, else of its sponsor
    /// inventory, upper case; `#` without either.
    pub fn unlock_text(&self, row: i32) -> String {
        let Some((slot, item)) = self.thumb(row) else { return "#".into() };
        let (Some(data), Some(r)) = (&self.data, self.row_of(slot, item)) else { return "#".into() };
        let name = [Field::Unlock, Field::SponsorUnlock].into_iter().map(|f| data.catalog.text_of(r, f)).find(|n| !n.is_empty());
        name.map_or_else(|| "#".into(), |n| format!("ID_{n}_UNLOCK").to_ascii_uppercase())
    }

    /// `CAC_IsPartNew` (825A7600 with 825F4160): its clothing unlock or sponsor inventory is new in the store.
    pub fn is_new(&self, row: i32) -> bool {
        let Some((slot, item)) = self.thumb(row) else { return false };
        let (Some(data), Some(r)) = (&self.data, self.row_of(slot, item)) else { return false };
        let new = |field: Field, kind: UnlockKind| {
            let name = data.catalog.text_of(r, field);
            !name.is_empty() && self.unlocks.is_new(kind, name)
        };
        new(Field::Unlock, UnlockKind::ClothingUnlock) | new(Field::SponsorUnlock, UnlockKind::ClothingSponsor)
    }

    /// `CAC_GetThumbnailFileName` (825FF7D0): the picture the movie loads into the tile.
    pub fn thumbnail(&self, row: i32) -> String {
        if matches!(self.menu(), MENU_TATTOO_STAMPS | MENU_GRAPHIC_BRAND | MENU_GRAPHIC_TEAM) {
            return match (self.list.get(usize::try_from(row).unwrap_or(usize::MAX)), &self.data) {
                (Some(item), _) if item.is_none() => THUMB_REMOVE.to_owned(),
                (Some(&item), Some(data)) => data.catalog.stamp_thumbnail(item),
                _ => crate::cas_rules::THUMB_NONE.to_owned(),
            };
        }
        match (self.thumb(row), &self.data) {
            (Some((_, item)), _) if item.is_none() => THUMB_REMOVE.to_owned(),
            (Some((slot, item)), Some(data)) => data.catalog.thumbnail(slot, item, self.record.gender),
            _ => crate::cas_rules::THUMB_NONE.to_owned(),
        }
    }

    /// What the preview wears: the record, with the item tried on in its slot.
    pub fn shown_record(&self) -> std::borrow::Cow<'_, SkaterRecord> {
        match self.trying {
            Some((slot, item)) => {
                let mut r = self.record.clone();
                r.wear_new(slot, item, false);
                std::borrow::Cow::Owned(r)
            }
            None => std::borrow::Cow::Borrowed(&self.record),
        }
    }

    /// `CAC_GetBannerType` (the same word).
    pub fn banner(&self) -> &'static str {
        match self.group {
            0 => "slappys",
            1 => "petes",
            _ => "none",
        }
    }

    /// 825A7A08: A's text; none on Edit Skater's sliders.
    pub fn help_a(&self) -> &'static str {
        let on_slider = matches!(self.item(self.cursor().0), Some(ITEM_TRUCKS | ITEM_WHEELS));
        match self.table().help_a {
            1 if !(self.menu() == MENU_EDIT_SKATER && on_slider) => "ID_COMMON_SELECT",
            2 => "ID_COMMON_ACCEPT",
            _ => "#",
        }
    }

    /// 825A7B28: B's text.
    pub fn help_b(&self) -> &'static str {
        match self.table().help_b {
            1 => "ID_CAC_EXIT",
            2 => "ID_COMMON_BACK",
            3 => "ID_CAC_CHANGE_GRAPHIC",
            _ => "#",
        }
    }

    pub fn cursor(&self) -> (i32, i32) {
        self.cursor[self.menu()]
    }

    /// `CAC_UpdateMenuIndices`.
    pub fn set_cursor(&mut self, row: i32, window: i32) {
        let menu = self.menu();
        self.cursor[menu] = (row, window);
    }

    fn gesture_row(item: usize) -> Option<usize> {
        ITEM_GESTURES.iter().position(|&g| g == item)
    }

    /// `CAC_GetIntegerValue` (825FFB18): a slider's bars, 0 to 10, rounded.
    pub fn row_integer(&self, row: i32) -> i32 {
        match self.item(row) {
            Some(ITEM_TRUCKS) => (self.record.trucks * 10.0 + 0.5) as i32,
            Some(ITEM_WHEELS) => (self.record.wheels * 10.0 + 0.5) as i32,
            Some(i) if morph_of(i).is_some() => (self.morph_value(i) * MORPH_BARS + 0.5) as i32,
            Some(i) => self.row_zone(i).map_or(0, |zone| self.colour_at(zone) as i32),
            None => 0,
        }
    }

    /// A zone's current colour among its list (82603F38): 0 when the skater's is not in it.
    fn colour_at(&self, zone: u32) -> usize {
        let Some(z) = self.data.as_ref().and_then(|d| d.vault.zone(zone)) else { return 0 };
        z.colours.iter().position(|c| c.key == self.record.colour_keys[zone as usize]).unwrap_or(0)
    }

    /// `CAC_GetColours` (825A77D8): the swatches of a colour row, three numbers each.
    /// A colour row's zone (82603E98): Pete's fixed rows, Wear Styles' by the shown item's slot (825416A8).
    fn row_zone(&self, item: usize) -> Option<u32> {
        if item == ITEM_WEAR_COLOUR {
            return self.wear_item.and_then(|(slot, _)| Catalog::colour_zone(slot));
        }
        zone_of(item)
    }

    pub fn row_colours(&self, row: i32) -> Vec<f32> {
        let zone = self.item(row).and_then(|i| self.row_zone(i)).and_then(|z| self.data.as_ref()?.vault.zone(z));
        zone.map_or_else(Vec::new, |z| z.colours.iter().flat_map(|c| c.swatch).collect())
    }

    /// A slider's place, 0 to 1 (8253CD18; body size is fat and thin as one, 825FB490).
    fn morph_value(&self, item: usize) -> f32 {
        let Some(key) = morph_of(item) else { return 0.0 };
        if item == ITEM_BODY_SIZE {
            let (fat, thin) = (self.weight("global_fat"), self.weight("global_thin"));
            return ((1.0 - thin) * 0.5 + fat * 0.5).clamp(0.0, 1.0);
        }
        self.weight(key)
    }

    fn weight(&self, key: &str) -> f32 {
        let Some(m) = self.data.as_ref().and_then(|d| d.vault.morph(key).cloned()) else { return 0.0 };
        let v = self.record.morphs.get(m.index).copied().unwrap_or(0.0);
        if v > m.max {
            return 1.0;
        }
        (v.max(m.min) - m.min) / (m.max - m.min)
    }

    /// 82536B10: a slider's place into the record's weight.
    fn set_weight(&mut self, key: &str, value: f32) {
        let Some(m) = self.data.as_ref().and_then(|d| d.vault.morph(key).cloned()) else { return };
        if let Some(w) = self.record.morphs.get_mut(m.index) {
            *w = (m.max - m.min) * value.clamp(0.0, 1.0) + m.min;
        }
    }

    /// 825FB5B8: above the middle fat, below it thin.
    fn set_morph(&mut self, item: usize, value: f32) {
        let Some(key) = morph_of(item) else { return };
        if item == ITEM_BODY_SIZE {
            let (fat, thin) = if value >= 0.5 { ((value - 0.5) * 2.0, 0.0) } else { (0.0, 1.0 - value * 2.0) };
            self.set_weight("global_thin", thin);
            self.set_weight("global_fat", fat);
        } else {
            self.set_weight(key, value);
        }
    }

    /// The per-frame part of 825FBAE8: a slider moves while the stick or d-pad is held (`axis`, -1 to 1).
    pub fn hold(&mut self, axis: f32, seconds: f32) {
        if self.busy || self.table().state != 3 || axis.abs() <= MORPH_DEAD_ZONE {
            return;
        }
        let Some(item) = self.item(self.cursor().0).filter(|&i| morph_of(i).is_some()) else { return };
        let step = MORPH_STEP * seconds * TICKS * axis.signum();
        self.set_morph(item, (self.morph_value(item) + step).clamp(0.0, 1.0));
        self.preset = PRESET_CUSTOM.to_owned();
        self.look.get_or_insert(LookChange::Morphs);
        self.states.push(STATE_REFRESH_ROW);
    }

    /// `CAC_GetStringValue` (825FFE08).
    pub fn row_text(&self, row: i32) -> &'static str {
        let band = |v: f32, low: &'static str, mid: &'static str, high: &'static str| {
            if v < 0.35 {
                low
            } else if v > 0.75 {
                high
            } else {
                mid
            }
        };
        let r = &self.record;
        match self.item(row) {
            Some(ITEM_TRUCKS) => band(r.trucks, "ID_CAC_EDIT_TRUCKS_LOOSE", "ID_CAC_EDIT_TRUCKS_MEDIUM", "ID_CAC_EDIT_TRUCKS_TIGHT"),
            Some(ITEM_WHEELS) => band(r.wheels, "ID_CAC_EDIT_WHEELS_SOFT", "ID_CAC_EDIT_WHEELS_MEDIUM", "ID_CAC_EDIT_WHEELS_HARD"),
            Some(ITEM_STANCE) => {
                if r.stance == 0 {
                    "ID_CAC_STANCE_GOOFY"
                } else {
                    "ID_CAC_STANCE_REGULAR"
                }
            }
            Some(ITEM_STYLE) => ["ID_CAC_LABEL_STYLE_NORMAL", "ID_CAC_LABEL_STYLE_LOOSE", "ID_CAC_LABEL_STYLE_GONZO", "ID_CAC_LABEL_STYLE_AGGRESSIVE"].get(r.style as usize).copied().unwrap_or("#"),
            Some(ITEM_POSTURE) => ["ID_CAC_LABEL_POSTURE_1", "ID_CAC_LABEL_POSTURE_2", "ID_CAC_LABEL_POSTURE_3", "ID_CAC_LABEL_POSTURE_4"].get(r.posture as usize).copied().unwrap_or("#"),
            Some(i) => Self::gesture_row(i).and_then(|g| GESTURE_LABELS.get(self.gesture_rows[g] as usize)).copied().unwrap_or("#"),
            None => "#",
        }
    }

    /// `CAC_GetStringValue`: the styles and presets come from the data, the rest from [`Cac::row_text`].
    pub fn row_string(&self, row: i32) -> String {
        let id = |prefix: &str, style: &str| format!("{prefix}{}", style.to_ascii_uppercase().replace(' ', "_"));
        let Some(data) = self.data.as_ref() else { return self.row_text(row).to_owned() };
        let at = self.list.get(self.list_at).copied();
        match (self.item(row), at) {
            (Some(ITEM_HAIR_STYLE), Some(Worn::NONE)) => "ID_CAC_HAIRSTYLE_BALD".to_owned(),
            (Some(ITEM_HAIR_STYLE), Some(i)) => id("ID_CAC_HAIRSTYLE_", data.catalog.row_text(Slot::Hair, i, Field::HairStyle)),
            (Some(ITEM_FACIAL_HAIR), Some(i)) => id("ID_CAC_FACIAL_HAIR_", data.catalog.row_text(Slot::Rostral, i, Field::FacialHairStyle)),
            (Some(ITEM_BROW_STYLE), Some(i)) => id("ID_CAC_EYEBROW_", data.catalog.row_text(Slot::Rostral, i, Field::EyebrowStyle)),
            (Some(ITEM_WEAR_STYLE), _) => format!("ID_CAC_WEAR_STYLE_OPTION_{}", self.wear_at + 1),
            (Some(ITEM_PRESET), _) => data.vault.presets.iter().find(|p| p.key == self.preset).map_or_else(|| "#".to_owned(), |p| p.label.clone()),
            _ => self.row_text(row).to_owned(),
        }
    }

    /// 82600AF8 for a menu: its group (a change takes a new undo point and tells the movie), its camera, its list.
    fn build(&mut self, menu: usize) {
        let group = match menu {
            MENU_SLAPPYS => Some(0),
            MENU_BACK_ALLEY => Some(1),
            MENU_EDIT_SKATER => Some(2),
            _ => None,
        };
        if let Some(group) = group {
            self.camera = "standing".into();
            self.body = "standing";
            self.turn = 0.0;
            if group != self.group {
                self.group = group;
                self.snapshot = self.record.clone();
                self.states.push(STATE_GROUP);
            }
        }
        if menu == MENU_EDIT_STYLE {
            self.gesture_rows = std::array::from_fn(|row| self.record.gestures[GESTURE_SLOT[row]]);
        }
        self.build_list(menu);
        self.states.push(MENUS[menu].state);
    }

    /// The row list of a selector page and the worn row's place in it (82604188: the first when it is not there).
    fn build_list(&mut self, menu: usize) {
        self.list.clear();
        self.list_at = 0;
        let Some(data) = self.data.clone() else { return };
        let (catalog, gender) = (&data.catalog, self.record.gender);
        let head = self.record.worn[Slot::Rostral as usize];
        let current = match menu {
            MENU_HAIR => {
                self.list = catalog.hair_rows(gender);
                let worn = self.record.worn[Slot::Hair as usize];
                if gender == Gender::Male {
                    self.list.insert(0, Worn::NONE);
                }
                if worn.is_none() {
                    Worn::NONE
                } else {
                    catalog.full_hair_of(worn).unwrap_or(worn)
                }
            }
            MENU_FACIAL_HAIR => {
                self.list = catalog.facial_hair_rows(head, gender);
                head
            }
            MENU_BROW => {
                self.list = catalog.brow_rows(head, gender);
                head
            }
            MENU_THUMBNAILS => {
                self.thumbnails_list();
                return;
            }
            MENU_WEAR_STYLES => {
                self.wear_styles_list();
                return;
            }
            MENU_GRAPHIC_BRAND | MENU_GRAPHIC_TEAM => {
                // The logos of the page (82600AF8 menus 0xD, 0xE) and the item's stamp zone's camera.
                self.list = data.catalog.logo_rows(menu == MENU_GRAPHIC_TEAM);
                if let Some((slot, _)) = self.wear_item {
                    if let Some(camera) = data.vault.stamp_camera(graphic_zone(slot)) {
                        self.camera = camera.to_owned();
                    }
                }
                return;
            }
            MENU_TATTOO_STAMPS => {
                // The remove tile in front of the category's stamps (82600AF8 menu 0xB), and the stamp zone's camera.
                let lower = self.tattoo.item >= ITEM_LOWER_REGIONS;
                if let Some(category) = data.vault.tattoo_categories.get(self.tattoo.category) {
                    self.list = std::iter::once(Worn::NONE).chain(data.catalog.tattoo_rows(category)).collect();
                }
                if let Some(camera) = data.vault.stamp_camera(if lower { ZONE_LOWER } else { ZONE_UPPER }) {
                    self.camera = camera.to_owned();
                }
                return;
            }
            _ => return,
        };
        self.list_at = self.list.iter().position(|&i| i == current).unwrap_or(0);
    }

    /// The thumbnails page's list (82600AF8 menu 10 with 82603480): the category's rows, the worn row moved to the
    /// front, and the remove tile before it on a slot that can be empty (82541008) or the outer torso.
    fn thumbnails_list(&mut self) {
        let (Some(data), Some((slot, test))) = (self.data.clone(), self.category.and_then(class_of)) else { return };
        let catalog = &data.catalog;
        // The packs' inventories and the vault's hidden unlocks, unless granted (825F33A0, 825F35A8 and 825F3960); a
        // name the catalog does not hold is left out (82DDD7D8 gives -1).
        let hidden_packs: Vec<&str> = PACK_UNLOCKS
            .into_iter()
            .filter(|n| catalog.holds(n))
            .filter(|n| !self.unlocks.has(UnlockKind::ClothingSponsor, n, true) && !self.unlocks.has(UnlockKind::ClothingUnlock, n, true))
            .collect();
        let hidden: Vec<&str> = data
            .vault
            .hidden_clothing
            .iter()
            .filter(|n| !self.unlocks.has(UnlockKind::ClothingUnlock, n, true))
            .chain(data.vault.hidden_sponsors.iter().filter(|n| !self.unlocks.has(UnlockKind::ClothingSponsor, n, true)))
            .map(String::as_str)
            .filter(|n| catalog.holds(n))
            .collect();
        let shown = |r: &Record, names: &[&str]| !names.iter().any(|n| catalog.is(r, Field::SponsorUnlock, n) || catalog.is(r, Field::Unlock, n));
        let keep = |r: &Record| {
            let class = match test {
                RowTest::Any => true,
                RowTest::TopTypeIn(types) => types.iter().any(|t| catalog.holds(t) && catalog.is(r, Field::TopType, t)),
                RowTest::TopType(t) => catalog.holds(t) && catalog.is(r, Field::TopType, t),
                RowTest::SockStyleNot(t) => !catalog.is(r, Field::SockStyle, t),
            };
            class && shown(r, &hidden_packs) && shown(r, &hidden)
        };
        self.list = catalog.shop_rows(slot, self.record.gender, keep);
        let worn = self.record.worn[slot as usize];
        if catalog.get(slot, worn).is_some() {
            if let Some(at) = self.list.iter().position(|&i| i == worn) {
                self.list.remove(at);
                self.list.insert(0, worn);
            }
        }
        if Catalog::removable(slot) || slot == Slot::OuterTorso {
            self.list.insert(0, Worn::NONE);
        }
        self.highlight = None;
        self.hover = None;
    }

    /// `Game.OnMenuHilight` on the thumbnails (825FEDB8): the tile becomes the highlighted one and the try-on waits.
    pub fn highlight(&mut self, row: i32) {
        if self.menu() != MENU_THUMBNAILS {
            return;
        }
        self.highlight = usize::try_from(row).ok().filter(|&r| r < self.list.len());
        self.hover = Some(0.0);
    }

    /// The per-frame part of 825FBAE8 on the thumbnails: 500 ms after a highlight the tile is tried on (825FEEA0) —
    /// unless the worn item is one of its variants — and the remove tile shows the record (82537268).
    pub fn tick(&mut self, seconds: f32) {
        if self.menu() != MENU_THUMBNAILS {
            return;
        }
        let Some(t) = self.hover.as_mut() else { return };
        *t += seconds;
        if *t <= TRY_ON_DELAY {
            return;
        }
        self.hover = None;
        let tried = self.highlight.and_then(|r| self.thumb(r as i32)).filter(|(_, item)| !item.is_none()).filter(|&(slot, item)| {
            let worn = self.record.worn[slot as usize];
            self.data.as_ref().is_some_and(|d| d.catalog.get(slot, worn).is_none() || !d.catalog.same_style(slot, item).contains(&worn))
        });
        if tried != self.trying {
            self.trying = tried;
            self.look = Some(LookChange::Rebuild);
        }
    }

    /// `CAC_OnThumbnailSelect` (825FD9F0). The remove tile takes the slot's item off (825377E0) and goes back to
    /// Slappy's; a locked item only sounds. An item with colours, a logo or several variants opens Wear Styles (menu
    /// 30) and is worn there (a deck of the worn deck's style keeps the worn one); any other is worn (825375C0, kept in
    /// the closet), its tile refreshed (state 0x35) and the page goes back to Slappy's. Either way nothing is worn when
    /// it or one of its variants is worn already.
    pub fn select_thumb(&mut self, row: i32) {
        if self.busy {
            return;
        }
        if self.menu() == MENU_TATTOO_STAMPS {
            self.select_stamp(row);
            return;
        }
        if matches!(self.menu(), MENU_GRAPHIC_BRAND | MENU_GRAPHIC_TEAM) {
            self.select_logo(row);
            return;
        }
        if self.menu() != MENU_THUMBNAILS {
            return;
        }
        let Some((slot, item)) = self.thumb(row) else { return };
        let Some(data) = self.data.clone() else { return };
        if item.is_none() {
            self.sounds.push("core_a_button");
            self.end_try_on();
            self.back_to(MENU_SLAPPYS);
            self.take_off(&data.catalog, slot);
            return;
        }
        if self.locked(slot, item) {
            self.sounds.push("core_error");
            return;
        }
        self.sounds.push("core_a_button");
        self.end_try_on();
        let worn = self.record.worn[slot as usize];
        let variants = data.catalog.variants_by_model(slot, item);
        let flag = |f: Field| data.catalog.get(slot, item).is_some_and(|r| data.catalog.flag_of(r, f));
        if flag(Field::IsColourizable) || variants.len() > 1 || flag(Field::IsLogoable) {
            self.wear_item = Some((slot, item));
            self.enter(MENU_WEAR_STYLES);
            let style = |i: Worn| data.catalog.row_text(slot, i, Field::StyleSubcategory);
            let target = if slot == Slot::SkateBoard && !style(item).is_empty() {
                match data.catalog.get(slot, worn) {
                    Some(_) if style(worn) == style(item) => (worn != item).then_some(worn),
                    _ => Some(item),
                }
            } else {
                (!variants.contains(&worn)).then_some(item)
            };
            if let Some(i) = target {
                self.wear_part(slot, i);
            }
            return;
        }
        let wear = !variants.contains(&worn);
        self.states.push(STATE_PART_ICON);
        self.back_to(MENU_SLAPPYS);
        if wear {
            self.wear_part(slot, item);
        }
    }

    /// 825375C0: worn and kept in the closet (82DDEDB0); a top clears the shirt graphic's adjust flag.
    fn wear_part(&mut self, slot: Slot, item: Worn) {
        self.record.wear_new(slot, item, true);
        if slot == Slot::OuterTorso {
            self.record.adjust.flag = 0;
        }
        self.changed();
    }

    /// Wear Styles' rows (82600AF8 menu 0x1E): the colour row for a colourisable item; the style row when it has more
    /// than one variant (not on decks), on the worn one (82604188, the shown item when the worn is not a variant); the
    /// graphic rows for a logoable item (Add Graphic, the top's own on a top, then Remove Graphic).
    fn wear_styles_list(&mut self) {
        self.wear_rows.clear();
        self.list.clear();
        self.wear_at = 0;
        let (Some(data), Some((slot, item))) = (self.data.clone(), self.wear_item) else { return };
        let catalog = &data.catalog;
        let flag = |f: Field| catalog.get(slot, item).is_some_and(|r| catalog.flag_of(r, f));
        if flag(Field::IsColourizable) {
            self.wear_rows.push(ITEM_WEAR_COLOUR);
        }
        if slot != Slot::SkateBoard {
            self.list = catalog.variants_by_model(slot, item);
            if self.list.len() > 1 {
                let worn = self.record.worn[slot as usize];
                let on = if self.list.contains(&worn) { worn } else { item };
                self.wear_at = self.list.iter().position(|&i| i == on).unwrap_or(0);
                self.wear_rows.push(ITEM_WEAR_STYLE);
            }
        }
        if flag(Field::IsLogoable) {
            self.wear_rows.push(if slot == Slot::OuterTorso { ITEM_ADD_SHIRT_GRAPHIC } else { ITEM_ADD_GRAPHIC });
            self.wear_rows.push(ITEM_REMOVE_GRAPHIC);
        }
    }

    /// The style row (825FC7E8 item 0x63): Left and Right wrap over the variants and wear the one reached.
    fn step_wear_style(&mut self, forward: bool) -> bool {
        let n = self.list.len();
        let Some((slot, _)) = self.wear_item.filter(|_| n > 0) else { return false };
        self.wear_at = (self.wear_at as isize + if forward { 1 } else { -1 }).rem_euclid(n as isize) as usize;
        let item = self.list[self.wear_at];
        self.wear_item = Some((slot, item));
        self.wear_part(slot, item);
        true
    }

    /// Wearing or taking off ends the try-on (825375C0 and 825377E0 stop the preview task, +0x5DDC).
    fn end_try_on(&mut self) {
        self.hover = None;
        if self.trying.take().is_some() {
            self.look = Some(LookChange::Rebuild);
        }
    }

    /// 825377E0 with no new item: the slot is emptied; a slot that cannot be empty wears its default instead — the
    /// outer torso the bare top of the arms' skin (with the shirt graphic's adjust reset), else the gender's default.
    fn take_off(&mut self, catalog: &Catalog, slot: Slot) {
        if catalog.get(slot, self.record.worn[slot as usize]).is_none() {
            return;
        }
        self.record.wear(slot, Worn::NONE);
        if !Catalog::removable(slot) {
            let item = if slot == Slot::OuterTorso {
                let a = &mut self.record.adjust;
                a.flag = 1;
                a.words[0] = u32::MAX;
                a.values[0] = [0.0, 1.0, -1.0, -1.0];
                self.record.graphics.retain(|g| g.flags[1] != 1);
                catalog.bare_top(&self.record)
            } else {
                None
            };
            let item = item.or_else(|| catalog.default_item(slot, self.record.gender)).unwrap_or(Worn::NONE);
            self.record.wear(slot, item);
        }
        self.changed();
    }

    /// The exit question's answer (82604748): Yes closes (the host saves), No rebuilds Edit Skater from its root.
    pub fn exit_answer(&mut self, yes: bool) {
        if yes {
            self.closed = true;
        } else {
            self.stack = vec![MENU_EDIT_SKATER];
            self.build(MENU_EDIT_SKATER);
        }
    }

    /// The first row of a menu: its cursor starts over (82600780).
    fn enter(&mut self, menu: usize) {
        self.stack.push(menu);
        self.cursor[menu] = (0, 0);
        self.build(menu);
    }

    /// 82600820: back to a menu on the stack, nothing else closes.
    fn back_to(&mut self, menu: usize) {
        while self.stack.len() > 1 && self.menu() != menu {
            self.stack.pop();
        }
        self.build(menu);
    }

    /// One zone's colour (82538060): the layer of the slot it colours, the skin all four layers on it.
    fn set_colour(&mut self, slot: u32, key: u64) {
        let Some(data) = self.data.clone() else { return };
        let Some(zone) = data.vault.zone(slot) else { return };
        let Some(colour) = zone.colours.iter().find(|c| c.key == key) else { return };
        let worn = |r: &SkaterRecord, s: Slot| r.worn[s as usize];
        let mut layers: Vec<(usize, Worn)> = match slot {
            1 => vec![(1, worn(&self.record, Slot::Hair)), (7, Worn::NONE), (8, Worn::NONE)],
            2..=5 => {
                let r = &self.record;
                let top = [Slot::OuterTorso, Slot::InnerTorso].into_iter().map(|s| (s, worn(r, s))).find(|&(s, i)| data.catalog.is_skinned(s, i));
                vec![(2, worn(r, Slot::Arm)), (3, worn(r, Slot::Leg)), (4, worn(r, Slot::Rostral)), (5, top.map_or(Worn::NONE, |(_, i)| i))]
            }
            6 => {
                let eyes = data.catalog.eyes_of_colour(worn(&self.record, Slot::Organ), &colour.tag);
                self.record.worn[Slot::Organ as usize] = eyes;
                vec![(6, eyes)]
            }
            // The clothes' zones colour the worn item of their slot.
            0 => vec![(0, worn(&self.record, Slot::Hat))],
            9 => vec![(9, worn(&self.record, Slot::OuterTorso))],
            10 => vec![(10, worn(&self.record, Slot::Sock))],
            12 => vec![(12, worn(&self.record, Slot::Pants))],
            // A deck colour is a deck: the one of that colour is worn (82DDC160 then 82DDEDB0), the layer names none.
            11 => {
                if let Some(deck) = data.catalog.deck_of_colour(&colour.tag) {
                    self.record.wear_new(Slot::SkateBoard, deck, true);
                }
                vec![(11, Worn::NONE)]
            }
            s => vec![(s as usize, Worn::NONE)],
        };
        for (index, part) in layers.drain(..) {
            self.record.colour_keys[index] = key;
            let layer = ColourLayer { index: index as u8, primary: colour.primary, secondary: colour.secondary, part };
            match self.record.colours.iter_mut().find(|c| usize::from(c.index) == index) {
                Some(c) => *c = layer,
                None => self.record.colours.push(layer),
            }
        }
    }

    /// The presets of the skater's gender with `custom` first (825FC7E8's walk): a step past the ends goes through it.
    fn step_preset(&mut self, forward: bool) {
        let Some(data) = self.data.clone() else { return };
        let male = self.record.gender == Gender::Male;
        let last = data.vault.presets.iter().filter(|p| p.male == male).map(|p| p.number).max().unwrap_or(0) as i32;
        let now = data.vault.presets.iter().find(|p| p.key == self.preset).map_or(0, |p| p.number as i32);
        let next = now + if forward { 1 } else { -1 };
        let next = if next < 0 { last } else { next };
        if next != 0 && next <= last {
            let Some(preset) = data.vault.presets.iter().find(|p| p.male == male && p.number == next as u32) else { return };
            self.preset = preset.key.clone();
            // 82604208: the sliders but the body's, then the colours.
            for (morph, value) in &preset.morphs {
                if morph != "global_fat" && morph != "global_thin" {
                    self.set_weight(morph, *value);
                }
            }
            for (zone, colour) in &preset.colours {
                if let Some(z) = data.vault.zones.iter().find(|z| z.key == *zone) {
                    self.set_colour(z.slot, *colour);
                }
            }
        } else {
            // Custom: the skater as the group opened.
            self.preset = PRESET_CUSTOM.to_owned();
            self.record = self.snapshot.clone();
        }
        self.changed();
    }

    /// The gender row answered Yes (82604698, 825461A0): the other gender's character from before, else the disc's
    /// fallback with the morph defaults. The board and style stay. NOT RETAIL YET: 8253D470 builds the new character
    /// part by part from the defaults; the fallback recipe stands in for it, and the snapshots are not saved.
    pub fn switch_gender(&mut self) {
        let Some(data) = self.data.clone() else { return };
        let (now, other) = if self.record.gender == Gender::Male { (Gender::Male, Gender::Female) } else { (Gender::Female, Gender::Male) };
        self.genders[now as usize] = Some(self.record.clone());
        let mut next = self.genders[other as usize].take().unwrap_or_else(|| data.fallback[other as usize].clone());
        next.gender = other;
        next.trucks = self.record.trucks;
        next.wheels = self.record.wheels;
        next.stance = self.record.stance;
        next.style = self.record.style;
        next.posture = self.record.posture;
        next.gestures = self.record.gestures;
        next.unknown = self.record.unknown;
        self.record = next;
        self.preset = PRESET_CUSTOM.to_owned();
        self.changed();
        self.build(self.menu());
    }

    /// `Game.OnMenuSelect` (825FE3C0): a row that opens a menu, or accepts a page. Slappy's and the tattoos are not ported yet.
    pub fn select(&mut self, row: i32) {
        if self.busy {
            return;
        }
        let Some(item) = self.item(row) else { return };
        if self.table().state == 2 || (self.table().state == 3 && item != ITEM_TRUCKS && item != ITEM_WHEELS) {
            self.sounds.push("core_a_button");
        }
        let has_data = self.data.is_some();
        if matches!(item, ITEM_UPPER_TATTOO | ITEM_LOWER_TATTOO) {
            // Back Alley Pete's body modification rows: the region pages (4 and 5).
            if has_data {
                self.enter(if item == ITEM_UPPER_TATTOO { MENU_TATTOO_UPPER } else { MENU_TATTOO_LOWER });
            }
            return;
        }
        if matches!(self.menu(), MENU_TATTOO_UPPER | MENU_TATTOO_LOWER) {
            self.select_region(item);
            return;
        }
        match (self.menu(), item) {
            // Wear Styles' graphic rows (825FE3C0): a top picks its location first, the other items the category.
            (MENU_WEAR_STYLES, ITEM_ADD_SHIRT_GRAPHIC) if has_data => {
                self.enter(MENU_SHIRT_LOCATION);
                return;
            }
            (MENU_WEAR_STYLES, ITEM_ADD_GRAPHIC) if has_data => {
                self.enter(MENU_GRAPHIC_CATEGORIES);
                return;
            }
            (MENU_WEAR_STYLES, ITEM_REMOVE_GRAPHIC) => {
                self.remove_graphic();
                self.back_to(MENU_SLAPPYS);
                return;
            }
            (MENU_SHIRT_LOCATION, ITEM_SHIRT_REGIONS..=0x72) => {
                self.select_shirt_location(item - ITEM_SHIRT_REGIONS);
                return;
            }
            (MENU_GRAPHIC_CATEGORIES, ITEM_BRAND_GRAPHICS | ITEM_TEAM_GRAPHICS) => {
                self.enter(if item == ITEM_BRAND_GRAPHICS { MENU_GRAPHIC_BRAND } else { MENU_GRAPHIC_TEAM });
                return;
            }
            _ => {}
        }
        if self.menu() == MENU_TATTOO_CATEGORIES {
            // 825FE3C0 menu 6: the category (+0x17D0), then its stamps.
            self.tattoo.category = item - ITEM_TATTOO_CATEGORY;
            self.enter(MENU_TATTOO_STAMPS);
            return;
        }
        if item == ITEM_GENDER {
            self.ask_gender = true;
        } else if item == ITEM_UNDO {
            self.preset = PRESET_CUSTOM.to_owned();
            self.record = self.snapshot.clone();
            self.changed();
        } else if item == ITEM_BODY_UNDO {
            self.record.morphs = self.snapshot.morphs.clone();
            self.look = Some(LookChange::Morphs);
        } else if item == ITEM_EDIT_STYLE {
            self.enter(ITEMS[item].target as usize);
            return;
        } else if let Some(menu) = usize::try_from(ITEMS[item].target).ok().filter(|&m| has_data && (MENU_BACK_ALLEY..=0x1D).contains(&m)) {
            if let Some(camera) = camera_of(item) {
                self.camera = camera.into();
            }
            self.enter(menu);
            return;
        } else if let Some(menu) = usize::try_from(ITEMS[item].target).ok().filter(|&m| has_data && ((MENU_SLAPPYS..=3).contains(&m) || (m == MENU_THUMBNAILS && class_of(item).is_some()))) {
            // A shop row (825FE3C0's generic path): a category keeps its class and title (+0x178C, +0x1788), then the
            // row's camera and pose, then its menu.
            if menu == MENU_THUMBNAILS {
                self.category = Some(item);
            }
            if let Some(camera) = camera_of(item) {
                self.camera = camera.into();
            }
            if let Some(body) = body_of(item) {
                self.body = body;
                self.turn = 0.0;
            } else if menu == MENU_THUMBNAILS {
                self.body = "standing";
                self.turn = 0.0;
            }
            self.enter(menu);
            return;
        }
        // Any row of a page accepts it (825FE3C0's last switch): these pages have no B.
        match self.menu() {
            MENU_EDIT_STYLE => self.leave(),
            // Wear Styles: any row accepts and goes back to Slappy's (825FE3C0 case 0x1E), but the graphic rows that
            // open the Graphics menus. NOT RETAIL YET: those menus are not ported, so Add Graphic does nothing.
            MENU_WEAR_STYLES => self.back_to(MENU_SLAPPYS),
            0x12..=0x15 | 0x1D => self.back_to(MENU_BACK_ALLEY),
            0x17..=0x1C => self.back_to(MENU_FACE),
            _ => {}
        }
    }

    /// A style row (825FC7E8 items 0x41, 0x43, 0x50): the list wraps, the row is worn at once; the row without hair
    /// takes it off (825377E0).
    fn step_list(&mut self, slot: Slot, forward: bool) -> bool {
        let n = self.list.len() as isize;
        if n == 0 {
            return false;
        }
        self.list_at = (self.list_at as isize + if forward { 1 } else { -1 }).rem_euclid(n) as usize;
        let item = self.list[self.list_at];
        if item.is_none() {
            let worn = self.record.worn[slot as usize];
            self.record.take(slot, worn);
        } else {
            self.record.wear_new(slot, item, true);
        }
        self.changed();
        true
    }

    /// A colour row (825FC7E8): the swatches do not wrap.
    fn step_colour(&mut self, zone: u32, forward: bool) -> bool {
        let Some(data) = self.data.clone() else { return false };
        let Some(z) = data.vault.zone(zone) else { return false };
        let at = (self.colour_at(zone) as isize + if forward { 1 } else { -1 }).clamp(0, z.colours.len() as isize - 1) as usize;
        self.set_colour(zone, z.colours[at].key);
        if ZONES_OF_PRESETS.contains(&zone) {
            self.preset = PRESET_CUSTOM.to_owned();
        }
        self.changed();
        true
    }

    fn step(&self, row_value: u32, forward: bool) -> u32 {
        let n = self.gestures.len();
        if n == 0 {
            return row_value;
        }
        let at = self.gestures.iter().position(|&g| g == row_value).unwrap_or(0) as isize + if forward { 1 } else { -1 };
        self.gestures[at.rem_euclid(n as isize) as usize]
    }

    /// One key of the C++ handler (825FC7E8).
    /// One key of the C++ handler (825FC7E8). True when it took the key (retail's 1): always while the mesh is built
    /// (+0x1800), B on a menu whose table gives it an action, Left and Right on a row with a value. Any other key goes
    /// on to the movie (retail's 0), which moves the thumbnails with them.
    /// The categories on the vault (`cac_tattoo_category_order`).
    fn categories(&self) -> usize {
        self.data.as_ref().map_or(0, |d| d.vault.tattoo_categories.len())
    }

    /// The region (0 to 3 upper, 0 and 1 lower) of the row chosen.
    fn region(&self, lower: bool) -> usize {
        self.tattoo.item.saturating_sub(if lower { ITEM_LOWER_REGIONS } else { ITEM_UPPER_REGIONS })
    }

    /// A region row of menu 4 or 5 (825FE3C0): a body without a tattoo opens the region (825FE0E0 or 825FE280), one
    /// with a tattoo asks first (82604618 and 82604658 do the same on Yes).
    fn select_region(&mut self, item: usize) {
        let lower = self.menu() == MENU_TATTOO_LOWER;
        if item < if lower { ITEM_LOWER_REGIONS } else { ITEM_UPPER_REGIONS } {
            return;
        }
        self.tattoo.item = item;
        if self.region(lower) >= if lower { 2 } else { 4 } {
            return;
        }
        if self.record.adjust.words[usize::from(lower) + 1] == u32::MAX {
            self.open_region(lower);
        } else {
            self.ask_tattoo = Some(lower);
        }
    }

    /// The question's answer (the host took `ask_tattoo`: `lower` is which): Yes opens the region as an empty body would.
    pub fn tattoo_answer(&mut self, lower: bool, yes: bool) {
        if yes && matches!(self.menu(), MENU_TATTOO_UPPER | MENU_TATTOO_LOWER) {
            self.open_region(lower);
        }
    }

    /// 825FE0E0 / 825FE280: the region is the body's tattoo region now with nothing placed (82536FA0 with 0, 1, -1,
    /// -1), the camera and the pose of the region, the top (or the pants) off, then the categories.
    fn open_region(&mut self, lower: bool) {
        let region = self.region(lower);
        let at = usize::from(lower) + 1;
        self.record.adjust.words[at] = region as u32;
        self.record.adjust.values[at] = stamp::NONE.values();
        let (camera, body) = if lower { (LOWER_CAMERAS[region], LOWER_BODIES[region]) } else { (UPPER_CAMERAS[region], UPPER_BODIES[region]) };
        self.camera = camera.into();
        self.body = body;
        self.turn = 0.0;
        if lower {
            if self.tattoo.pants.is_none() {
                self.swap_pants();
            }
        } else if self.tattoo.top.is_none() {
            self.swap_top();
        }
        self.enter(MENU_TATTOO_CATEGORIES);
    }

    /// 825FB718: the top comes off (the bare top of the arms' skin) and is kept; again, it is put on. A body that is
    /// topless already is left alone.
    fn swap_top(&mut self) {
        let Some(data) = self.data.clone() else { return };
        match self.tattoo.top.take() {
            None if self.record.adjust.flag != 0 => {}
            None => {
                self.tattoo.top = Some(self.record.worn[Slot::OuterTorso as usize]);
                let a = &self.record.adjust;
                self.tattoo.shirt_graphic = self.record.graphics.iter().find(|g| g.flags[1] == 1).map(|g| (g.clone(), a.words[0], a.values[0]));
                self.take_off(&data.catalog, Slot::OuterTorso);
            }
            Some(top) => {
                self.wear_part(Slot::OuterTorso, top);
                if let Some((graphic, word, values)) = self.tattoo.shirt_graphic.take() {
                    self.record.graphics.push(graphic);
                    self.record.adjust.words[0] = word;
                    self.record.adjust.values[0] = values;
                }
            }
        }
    }

    /// 825FB8C8: the pants worn while the lower body is placed (the first the tattoo filter passes, else the gender's
    /// default) and kept; again, put back.
    fn swap_pants(&mut self) {
        let Some(data) = self.data.clone() else { return };
        let slot = Slot::Pants as usize;
        match self.tattoo.pants.take() {
            None => {
                self.tattoo.pants = Some(self.record.worn[slot]);
                let gender = self.record.gender;
                if let Some(pants) = data.catalog.tattoo_pants(gender).or_else(|| data.catalog.default_item(Slot::Pants, gender)) {
                    self.record.worn[slot] = pants;
                    self.changed();
                }
            }
            Some(worn) => {
                self.record.worn[slot] = worn;
                self.changed();
            }
        }
    }

    /// The top and the pants put back when they were taken off.
    fn restore_clothes(&mut self) {
        if self.tattoo.top.is_some() {
            self.swap_top();
        }
        if self.tattoo.pants.is_some() {
            self.swap_pants();
        }
    }

    /// 825FD9F0 on the stamps page: the remove tile clears the body's tattoo (825315E8) and goes back to Pete's, a
    /// stamp becomes the body's tattoo at its start (825FAAF0) and opens the placement page. NOT RETAIL YET: the
    /// record's graphic entry (its three 64-bit values) is ours; retail's layout is not traced.
    fn select_stamp(&mut self, row: i32) {
        let Some(&stamp) = usize::try_from(row).ok().and_then(|r| self.list.get(r)) else { return };
        let Some(data) = self.data.clone() else { return };
        let lower = self.tattoo.item >= ITEM_LOWER_REGIONS;
        let at = usize::from(lower) + 1;
        self.sounds.push("core_a_button");
        self.record.graphics.retain(|g| usize::from(g.flags[1]) != at + 1);
        if stamp.is_none() {
            self.record.adjust.words[at] = u32::MAX;
            self.record.adjust.values[at] = stamp::NONE.values();
            self.tattoo.stamp = None;
            self.back_to(MENU_BACK_ALLEY);
            self.restore_clothes();
            self.changed();
            return;
        }
        self.record.graphics.push(Graphic { name: data.catalog.stamp_name(stamp).to_owned(), values: [stamp.model, stamp.material, 0], flags: [0, at as u8 + 1] });
        self.tattoo.stamp = Some(stamp);
        let start = self.start_placement(at);
        self.record.adjust.values[at] = start.values();
        self.enter(if lower { MENU_TATTOO_ADJUST_LOWER } else { MENU_TATTOO_ADJUST_UPPER });
        self.changed();
    }

    /// Where a placement starts (the table at 0x830442B8): the location of the slot's region (a shirt's, a tattoo's)
    /// or the board's and the hat's only one.
    fn start_placement(&self, at: usize) -> Placement {
        let region = match at {
            1 | 2 => self.region(at == 2),
            0 => self.record.adjust.words[0] as usize,
            _ => 0,
        };
        let location = stamp::LOCATIONS[at].get(region).copied().unwrap_or(0);
        stamp::start(location, self.record.gender == Gender::Female)
    }

    /// 825FE3C0 menu 0x21: the shirt's location (the adjust word +0xC), its start (82536FA0), the location's pose and
    /// camera, then the categories.
    fn select_shirt_location(&mut self, region: usize) {
        if region >= 4 {
            return;
        }
        self.record.adjust.words[0] = region as u32;
        self.record.adjust.values[0] = self.start_placement(0).values();
        self.body = SHIRT_BODIES[region];
        self.turn = 0.0;
        self.camera = SHIRT_CAMERAS[region].into();
        self.enter(MENU_GRAPHIC_CATEGORIES);
    }

    /// 825FD9F0 on the logo pages: the logo becomes the item's graphic at the placement's start (a board's and a hat's
    /// only location), and the item's placement page opens (825FAAF0). NOT RETAIL YET: the graphic entry is ours.
    fn select_logo(&mut self, row: i32) {
        let Some(&logo) = usize::try_from(row).ok().and_then(|r| self.list.get(r)) else { return };
        let (Some(data), Some((slot, _))) = (self.data.clone(), self.wear_item) else { return };
        let Some(at) = graphic_slot(slot) else { return };
        self.sounds.push("core_a_button");
        self.record.graphics.retain(|g| usize::from(g.flags[1]) != at + 1);
        self.record.graphics.push(Graphic { name: data.catalog.stamp_name(logo).to_owned(), values: [logo.model, logo.material, 0], flags: [0, at as u8 + 1] });
        if at != 0 {
            self.record.adjust.words[at] = 0;
        }
        self.record.adjust.values[at] = self.start_placement(at).values();
        self.tattoo.stamp = Some(logo);
        self.enter(match at {
            0 => MENU_ADJUST_SHIRT,
            3 => MENU_ADJUST_BOARD,
            _ => MENU_ADJUST_HAT,
        });
        self.changed();
    }

    /// Wear Styles' Remove Graphic (825FE3C0 item 0x68): the item's graphic and placement are cleared.
    fn remove_graphic(&mut self) {
        let Some(at) = self.wear_item.and_then(|(slot, _)| graphic_slot(slot)) else { return };
        let had = self.record.graphics.len();
        self.record.adjust.words[at] = u32::MAX;
        self.record.adjust.values[at] = stamp::NONE.values();
        self.record.graphics.retain(|g| usize::from(g.flags[1]) != at + 1);
        if self.record.graphics.len() != had {
            self.changed();
        }
    }

    /// The tattoos' and graphics' two shader rows per slot (the adjust index) for the stamps on the skater (82784B88),
    /// while the slot has a placement.
    pub fn decal_rows(&self) -> [[[f32; 3]; 2]; 5] {
        decal_rows_of(&self.record)
    }

    /// The placement pages' inputs each frame (825FBAE8): `input` is the turn (left bumper minus right), the scale (the
    /// left stick up minus down), the offset across (right stick right minus left) and up; each past the dead zone moves
    /// its number by its rate (`stamp::RATES`), and the placement is kept on the body part (82536C40).
    pub fn adjust_by(&mut self, input: [f32; 4], seconds: f32) {
        let Some(at) = adjust_index(self.menu()) else { return };
        if self.busy {
            return;
        }
        let (Some(data), Some(stamp)) = (self.data.clone(), self.tattoo.stamp) else { return };
        let (location, part) = match at {
            1 | 2 => {
                let lower = at == 2;
                let region = self.region(lower);
                // The part the stamp lies on: the arms or the bare top (the top, when it is off), the legs below; the
                // front and the back of a body that is not topless have none (the quarter's whole).
                let part = if lower {
                    Some(Slot::Leg)
                } else if self.record.adjust.flag != 0 {
                    Some(Slot::OuterTorso)
                } else if region >= 2 {
                    Some(Slot::Arm)
                } else {
                    None
                };
                (stamp::LOCATIONS[at].get(region).copied().unwrap_or(0), part)
            }
            // A shirt's graphic lies on the worn top at its location; a board's and a hat's on theirs.
            0 => (stamp::LOCATIONS[0].get(self.record.adjust.words[0] as usize).copied().unwrap_or(0), Some(Slot::OuterTorso)),
            3 => (stamp::LOCATIONS[3][0], Some(Slot::SkateBoard)),
            _ => (stamp::LOCATIONS[4][0], Some(Slot::Hat)),
        };
        let rect = data.catalog.stamp_rect(part.map(|s| (s, self.record.worn[s as usize])), location);
        let bounds = data.catalog.stamp_bounds(stamp);
        for (value, &x) in input.iter().enumerate() {
            if let Some(delta) = stamp::change(value, x) {
                let now = Placement::from_values(self.record.adjust.values[at]);
                self.record.adjust.values[at] = stamp::step(now, value, delta * seconds * TICKS, rect, bounds).values();
            }
        }
    }

    pub fn key(&mut self, key: Key) -> bool {
        if self.busy {
            return true;
        }
        match key {
            Key::Back => {
                if matches!(self.table().help_b, 1..=3) {
                    self.leave();
                    return true;
                }
                false
            }
            Key::Accept | Key::Reset | Key::Toggle if adjust_index(self.menu()).is_none() => false,
            Key::Accept => {
                // 825FC7E8 key 2 on the placement pages: back to Pete's, the top and the pants put on again.
                self.sounds.push("core_a_button");
                if matches!(self.menu(), MENU_ADJUST_SHIRT | MENU_ADJUST_BOARD | MENU_ADJUST_HAT) {
                    // A graphic's page goes back to Slappy's (825FC7E8, 82600820 with 1).
                    self.back_to(MENU_SLAPPYS);
                } else {
                    self.back_to(MENU_BACK_ALLEY);
                    self.restore_clothes();
                }
                true
            }
            Key::Reset => {
                // Key 4: the placement starts over from the table.
                if let Some(at) = adjust_index(self.menu()) {
                    self.record.adjust.values[at] = self.start_placement(at).values();
                }
                true
            }
            Key::Toggle if !matches!(self.menu(), MENU_TATTOO_ADJUST_UPPER | MENU_TATTOO_ADJUST_LOWER) => false,
            Key::Toggle => {
                // Key 5: the top (upper) or the pants (lower) on and off. Retail answers "not taken".
                if self.menu() == MENU_TATTOO_ADJUST_UPPER {
                    self.swap_top();
                } else {
                    self.swap_pants();
                }
                false
            }
            Key::Left | Key::Right => {
                let forward = key == Key::Right;
                let Some(item) = self.item(self.cursor().0) else { return false };
                let stepped = match item {
                    ITEM_HAIR_STYLE => Some(self.step_list(Slot::Hair, forward)),
                    ITEM_FACIAL_HAIR | ITEM_BROW_STYLE => Some(self.step_list(Slot::Rostral, forward)),
                    ITEM_PRESET => {
                        self.step_preset(forward);
                        Some(true)
                    }
                    ITEM_WEAR_STYLE => Some(self.step_wear_style(forward)),
                    i => self.row_zone(i).map(|zone| self.step_colour(zone, forward)),
                };
                if let Some(stepped) = stepped {
                    if stepped {
                        self.sounds.push(if forward { "core_nav_right" } else { "core_nav_left" });
                        self.states.push(STATE_REFRESH_ROW);
                    }
                    return true;
                }
                let r = &mut self.record;
                let slider = |v: f32| (v + if forward { 0.1 } else { -0.1 }).clamp(0.0, 1.0);
                let wrap = |v: u32, n: i64| (i64::from(v) + if forward { 1 } else { -1 }).rem_euclid(n) as u32;
                match item {
                    ITEM_TRUCKS => r.trucks = slider(r.trucks),
                    ITEM_WHEELS => r.wheels = slider(r.wheels),
                    ITEM_STANCE => r.stance = u32::from(r.stance == 0),
                    ITEM_STYLE => r.style = wrap(r.style, 4),
                    ITEM_POSTURE => r.posture = wrap(r.posture, 4),
                    i => match Self::gesture_row(i) {
                        Some(g) => self.gesture_rows[g] = self.step(self.gesture_rows[g], forward),
                        None => return false,
                    },
                }
                self.sounds.push(if forward { "core_nav_right" } else { "core_nav_left" });
                self.states.push(STATE_REFRESH_ROW);
                true
            }
        }
    }

    /// 825FC768: back a menu (826008C0: leaving Edit Style writes its gestures, leaving the thumbnails shows the record
    /// again), or ask before closing on the first (82603290).
    fn leave(&mut self) {
        self.sounds.push("core_b_button");
        if self.stack.len() <= 1 {
            self.ask_exit = true;
            return;
        }
        match self.stack.pop() {
            Some(MENU_EDIT_STYLE) => {
                for row in 0..4 {
                    self.record.gestures[GESTURE_SLOT[row]] = self.gesture_rows[row];
                }
            }
            Some(MENU_THUMBNAILS) => {
                self.camera = "slappys".into();
                self.body = "standing";
                self.turn = 0.0;
                self.hover = None;
                if self.trying.take().is_some() {
                    self.look = Some(LookChange::Rebuild);
                }
            }
            _ => {}
        }
        self.build(self.menu());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cac() -> Cac {
        let mut r = SkaterRecord::cleared();
        (r.trucks, r.wheels) = (0.7, 0.7);
        Cac::open(r, &UnlockStore::fresh_profile(), None)
    }

    #[test]
    fn the_tables_read_as_retail() {
        assert_eq!(MENUS[MENU_EDIT_SKATER].name, "EditSkaterMain");
        assert_eq!(MENUS[MENU_EDIT_SKATER].items, [0x0, 0x1, 0x2E, 0x2F, 0x30]);
        assert_eq!(ITEMS[0x2E].target, MENU_EDIT_STYLE as i32);
        assert_eq!(ITEMS[0x2F].kind, "slider");
        assert_eq!(ITEMS[7].label, "ID_CAC_HOODIES_TITLE");
        for m in &MENUS {
            assert!(m.items.iter().all(|&i| i < ITEMS.len()), "{}", m.name);
        }
    }

    #[test]
    fn edit_skater_opens_with_its_layout_and_rows() {
        let c = cac();
        assert_eq!((c.states.as_slice(), c.title(), c.row_count()), ([3].as_slice(), "ID_CAC_EDITSKATER_TITLE", 5));
        assert_eq!((c.row_label(3), c.row_kind(3), c.row_description(3, &|_| true)), ("ID_CAC_LABEL_TRUCK_TIGHTNESS", "slider", "ID_CAC_LABEL_TRUCK_DESC".to_owned()));
        assert_eq!(c.row_description(2, &|_| true), "ID_CAC_SKATE_STYLE_DESC");
        assert_eq!((c.row_integer(3), c.row_text(3)), (7, "ID_CAC_EDIT_TRUCKS_MEDIUM"));
        assert_eq!(c.help_b(), "ID_CAC_EXIT");
    }

    #[test]
    fn sliders_step_a_tenth_within_their_ends_and_name_their_band() {
        let mut c = cac();
        c.set_cursor(3, 0);
        c.key(Key::Right);
        assert_eq!((c.row_integer(3), c.row_text(3), c.states.last()), (8, "ID_CAC_EDIT_TRUCKS_TIGHT", Some(&STATE_REFRESH_ROW)));
        for _ in 0..5 {
            c.key(Key::Right);
        }
        assert_eq!(c.record.trucks, 1.0);
        for _ in 0..11 {
            c.key(Key::Left);
        }
        assert_eq!((c.record.trucks, c.row_text(3)), (0.0, "ID_CAC_EDIT_TRUCKS_LOOSE"));
        // The wheels are their own row; A's help is hidden on sliders.
        c.set_cursor(4, 0);
        c.key(Key::Left);
        assert_eq!(c.row_integer(4), 6);
        assert_eq!(c.help_a(), "#");
    }

    #[test]
    fn edit_style_wraps_its_selectors_and_writes_gestures_on_leaving() {
        let mut c = cac();
        c.select(2);
        assert_eq!((c.menu(), c.states.last(), c.row_count()), (MENU_EDIT_STYLE, Some(&2), 7));
        assert_eq!(c.title(), "ID_CAC_SKATE_STYLE_TITLE");
        // Stance, style and posture rows, as retail lists them.
        c.set_cursor(4, 0);
        assert_eq!(c.row_text(4), "ID_CAC_STANCE_GOOFY");
        c.key(Key::Right);
        assert_eq!((c.record.stance, c.row_text(4)), (1, "ID_CAC_STANCE_REGULAR"));
        c.key(Key::Left);
        assert_eq!(c.record.stance, 0);
        c.set_cursor(5, 0);
        assert_eq!(c.row_text(5), "ID_CAC_LABEL_STYLE_AGGRESSIVE");
        c.key(Key::Right);
        assert_eq!((c.record.style, c.row_text(5)), (0, "ID_CAC_LABEL_STYLE_NORMAL"));
        c.set_cursor(6, 0);
        c.key(Key::Left);
        assert_eq!((c.record.posture, c.row_text(6)), (3, "ID_CAC_LABEL_POSTURE_4"));
        // The first gesture row steps through the unlocked gestures, the record changes only when leaving.
        c.set_cursor(0, 0);
        assert_eq!(c.row_text(0), "ID_GESTURE_AIRGUITAR");
        c.key(Key::Right);
        assert_eq!(c.row_text(0), "ID_GESTURE_AIRPLANE");
        assert_eq!(c.record.gestures, [0, 1, 2, 3]);
        // B does nothing here (the table gives it no action); A accepts, on any row.
        c.key(Key::Back);
        assert_eq!(c.menu(), MENU_EDIT_STYLE);
        c.select(0);
        assert_eq!((c.menu(), c.record.gestures, c.closed), (MENU_EDIT_SKATER, [1, 1, 2, 3], false));
        // On Edit Skater, B asks first (825FC768 > 82603290): No rebuilds Edit Skater, Yes closes.
        c.key(Key::Back);
        assert!(c.ask_exit && !c.closed);
        c.ask_exit = false;
        c.exit_answer(false);
        assert_eq!((c.menu(), c.closed), (MENU_EDIT_SKATER, false));
        c.key(Key::Back);
        c.exit_answer(true);
        assert!(c.closed);
    }

    #[test]
    fn gesture_rows_are_the_records_slots_in_retails_order() {
        let mut c = cac();
        c.record.gestures = [10, 11, 12, 13];
        c.select(2);
        assert_eq!([c.row_text(0), c.row_text(1), c.row_text(2), c.row_text(3)], [GESTURE_LABELS[10], GESTURE_LABELS[13], GESTURE_LABELS[12], GESTURE_LABELS[11]]);
    }

    #[test]
    fn locked_gestures_are_not_offered_and_the_shops_stay_shut() {
        let mut c = Cac::open(SkaterRecord::cleared(), &UnlockStore::default(), None);
        assert!(c.gestures.is_empty());
        c.select(0);
        c.select(1);
        assert_eq!(c.menu(), MENU_EDIT_SKATER);
    }

    use crate::cas_rules::{Body, Colour, MorphParam, Preset, Zone};
    use crate::stamp::{self, Placement};
    use std::collections::BTreeMap;

    const HAIR_A: Worn = Worn { model: 0x20, material: 1 };
    const HAIR_B: Worn = Worn { model: 0x10, material: 1 };
    const HAT_CUT: Worn = Worn { model: 0x30, material: 1 };
    const FEMALE_HAIR: Worn = Worn { model: 0x05, material: 1 };
    const HEAD_PLAIN: Worn = Worn { model: 0x40, material: 3 };
    const HEAD_BEARD: Worn = Worn { model: 0x40, material: 2 };
    const HEAD_BROW: Worn = Worn { model: 0x40, material: 1 };
    const EYES_BLUE: Worn = Worn { model: 0x50, material: 2 };
    const EYES_GREEN: Worn = Worn { model: 0x50, material: 1 };
    const ARM: Worn = Worn { model: 0x60, material: 1 };
    const TEE_A: Worn = Worn { model: 0x70, material: 1 };
    const TEE_B: Worn = Worn { model: 0x71, material: 1 };
    const TEE_C: Worn = Worn { model: 0x72, material: 1 };
    const TEE_CUSTOM: Worn = Worn { model: 0x73, material: 1 };
    const TEE_FEMALE: Worn = Worn { model: 0x74, material: 1 };
    const TEE_PROMO: Worn = Worn { model: 0x75, material: 1 };
    const TEE_WORN_IN: Worn = Worn { model: 0x77, material: 1 };
    const HOODY_UP: Worn = Worn { model: 0x76, material: 1 };
    const HOODY_DOWN: Worn = Worn { model: 0x76, material: 2 };
    const BARE: Worn = Worn { model: 0x7F, material: 1 };
    const SHOE: Worn = Worn { model: 0x90, material: 1 };
    const SOCK_TINY: Worn = Worn { model: 0xA0, material: 1 };
    const SOCK_CREW: Worn = Worn { model: 0xA1, material: 1 };
    const DECK_RED: Worn = Worn { model: 0xB2, material: 1 };
    const DECK_BLUE: Worn = Worn { model: 0xB1, material: 1 };
    const DECK_BLUE_WORN: Worn = Worn { model: 0xB0, material: 1 };
    const TEE_LOGOABLE: Worn = Worn { model: 0x78, material: 1 };
    const STAMP_A: Worn = Worn { model: 0xC0, material: 1 };
    const STAMP_B: Worn = Worn { model: 0xC0, material: 2 };
    const STAMP_UNTAGGED: Worn = Worn { model: 0xC0, material: 3 };
    const STAMP_LOGO: Worn = Worn { model: 0xC0, material: 4 };
    const STAMP_TEAM: Worn = Worn { model: 0xC0, material: 5 };
    const STAMP_EMPTY: Worn = Worn { model: 0xC0, material: 6 };
    const PANTS_PLAIN: Worn = Worn { model: 0xD0, material: 1 };
    const PANTS_VIEW: Worn = Worn { model: 0xD0, material: 2 };

    fn colour(key: u64, tag: &str, swatch: f32) -> Colour {
        Colour { key, primary: [swatch; 3], swatch: [swatch; 3], secondary: [0.0; 3], tag: tag.to_owned() }
    }

    fn data() -> Arc<CacData> {
        let row = |slot, item, fields: &[(&str, &str)]| (slot, item, fields.iter().map(|&(k, v)| (k.to_owned(), v.to_owned())).collect::<BTreeMap<_, _>>());
        let head = |fh: &str, brow: &str| [("Gender", "male"), ("SkinTone", "light"), ("HairlineShadowStyle", "on"), ("FacialHairStyle", fh), ("EyebrowStyle", brow)].map(|(k, v)| (k, v.to_owned()));
        let owned = |f: [(&'static str, String); 5]| f.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<BTreeMap<_, _>>();
        let rows = vec![
            row(Slot::Hair, HAIR_A, &[("Gender", "male"), ("HairModelType", "full"), ("HairStyle", "moosed up")]),
            row(Slot::Hair, HAIR_B, &[("Gender", "male"), ("HairModelType", "full"), ("HairStyle", "short")]),
            row(Slot::Hair, HAT_CUT, &[("Gender", "male"), ("HairModelType", "hat"), ("HairStyle", "short")]),
            row(Slot::Hair, FEMALE_HAIR, &[("Gender", "female"), ("HairModelType", "full"), ("HairStyle", "long")]),
            (Slot::Rostral, HEAD_PLAIN, owned(head("clean", "style1"))),
            (Slot::Rostral, HEAD_BEARD, owned(head("goatee", "style1"))),
            (Slot::Rostral, HEAD_BROW, owned(head("clean", "style2"))),
            row(Slot::Organ, EYES_BLUE, &[("Gender", "male"), ("EyeColour", "blue")]),
            row(Slot::Organ, EYES_GREEN, &[("Gender", "male"), ("EyeColour", "green")]),
            row(Slot::Arm, ARM, &[("Gender", "male"), ("SkinTone", "light"), ("SkinNormal", "average")]),
            row(Slot::OuterTorso, TEE_A, &[("Gender", "male"), ("TopType", "tshirt"), ("ManufacturerID", "zoo"), ("ModelName", "tee_a"), ("MaterialVariantName", "tee_a_Mat")]),
            row(Slot::OuterTorso, TEE_B, &[("Gender", "male"), ("TopType", "tshirt"), ("ManufacturerID", "alien"), ("Unlock", "cac_unlock_bs_group1")]),
            row(Slot::OuterTorso, TEE_C, &[("Gender", "male"), ("TopType", "ls_tshirt"), ("ManufacturerID", "skate")]),
            row(Slot::OuterTorso, TEE_CUSTOM, &[("Gender", "male"), ("TopType", "tshirt"), ("ManufacturerID", "zoo"), ("IsColourizable", "1")]),
            row(Slot::OuterTorso, TEE_FEMALE, &[("Gender", "female"), ("TopType", "tshirt")]),
            row(Slot::OuterTorso, TEE_PROMO, &[("Gender", "male"), ("TopType", "tshirt"), ("SponsorUnlock", "cac_promo_drpepper1")]),
            row(Slot::OuterTorso, TEE_WORN_IN, &[("Gender", "male"), ("TopType", "tshirt"), ("IsUsedInGame", "1")]),
            row(Slot::OuterTorso, HOODY_UP, &[("Gender", "male"), ("TopType", "hoody"), ("StyleSubcategory", "hoody1")]),
            row(Slot::OuterTorso, HOODY_DOWN, &[("Gender", "male"), ("TopType", "hoody"), ("StyleSubcategory", "hoody1"), ("IsDefaultWearStyle", "1")]),
            row(Slot::OuterTorso, TEE_LOGOABLE, &[("Gender", "male"), ("TopType", "jacket"), ("IsLogoable", "1")]),
            row(Slot::OuterTorso, BARE, &[("Gender", "male"), ("IsSkinned", "1"), ("SkinTone", "light"), ("SkinNormal", "average"), ("StampUVConstraintQ1", "0.9,0.495,0.045,0.455"), ("StampUVConstraintQ3", "0.48,0.01,0.16,0.38")]),
            row(Slot::Feet, SHOE, &[("Gender", "unisex"), ("ModelName", "half_cab"), ("MaterialVariantName", "half_cab_material")]),
            row(Slot::Sock, SOCK_TINY, &[("Gender", "male"), ("SockStyle", "tiny")]),
            row(Slot::Sock, SOCK_CREW, &[("Gender", "male"), ("SockStyle", "crew")]),
            row(Slot::SkateBoard, DECK_RED, &[("IsLogoable", "1"), ("IsColourizable", "1"), ("DeckColour", "red")]),
            row(Slot::SkateBoard, DECK_BLUE, &[("IsLogoable", "1"), ("DeckColour", "blue")]),
            row(Slot::SkateBoard, DECK_BLUE_WORN, &[("IsLogoable", "1"), ("DeckColour", "blue"), ("IsUsedInGame", "1")]),
            row(Slot::Misc, STAMP_A, &[("ModelName", "logo_object"), ("MaterialVariantName", "tattoo_a_mat"), ("TattooCategory", "1"), ("StampBorderConstraint", "1,0,0.25,0.75")]),
            row(Slot::Misc, STAMP_B, &[("ModelName", "logo_object"), ("MaterialVariantName", "tattoo_b_mat"), ("TattooCategory", "2")]),
            row(Slot::Misc, STAMP_UNTAGGED, &[("ModelName", "logo_object"), ("MaterialVariantName", "tattoo_c_mat")]),
            row(Slot::Misc, STAMP_LOGO, &[("ModelName", "logo_object"), ("MaterialVariantName", "logo_thing")]),
            row(Slot::Misc, STAMP_TEAM, &[("ModelName", "logo_object"), ("MaterialVariantName", "team_logo_x")]),
            row(Slot::Misc, STAMP_EMPTY, &[("ModelName", "logo_object"), ("MaterialVariantName", "logoEmpty_diffuse_1_Mat")]),
            row(Slot::Pants, PANTS_PLAIN, &[("Gender", "male")]),
            row(Slot::Pants, PANTS_VIEW, &[("Gender", "male"), ("IsTattooViewingChoice", "true")]),
        ];
        let mut female = SkaterRecord::cleared();
        female.gender = Gender::Female;
        female.worn[Slot::Hair as usize] = FEMALE_HAIR;
        Arc::new(CacData {
            catalog: Catalog::build(&rows, &[]),
            fallback: [female, SkaterRecord::cleared()],
            vault: Vault {
                zones: vec![
                    Zone { key: 100, slot: 1, colours: vec![colour(1, "blond", 0.9), colour(2, "black", 0.1)] },
                    Zone { key: 200, slot: 2, colours: vec![colour(3, "light", 0.8), colour(4, "dark", 0.2)] },
                    Zone { key: 600, slot: 6, colours: vec![colour(5, "blue", 0.5), colour(6, "green", 0.6)] },
                    Zone { key: 900, slot: 9, colours: vec![colour(90, "red", 0.7), colour(91, "blue", 0.2)] },
                    Zone { key: 1100, slot: 11, colours: vec![colour(110, "red", 0.5), colour(111, "blue", 0.2)] },
                ],
                morphs: vec![
                    MorphParam { key: "global_fat".into(), index: 0, min: 0.0, max: 0.5 },
                    MorphParam { key: "global_thin".into(), index: 1, min: 0.0, max: 0.5 },
                    MorphParam { key: "nose_length".into(), index: 7, min: 0.0, max: 0.5 },
                ],
                presets: vec![
                    Preset { key: "custom".into(), male: false, number: 0, label: "ID_CAC_PRESET_CUSTOM".into(), morphs: vec![], colours: vec![] },
                    Preset { key: "male_01".into(), male: true, number: 1, label: "ID_CAC_PRESET_01".into(), morphs: vec![("nose_length".into(), 1.0), ("global_fat".into(), 1.0)], colours: vec![(200, 4)] },
                    Preset { key: "male_02".into(), male: true, number: 2, label: "ID_CAC_PRESET_02".into(), morphs: vec![], colours: vec![] },
                ],
                cameras: vec![
                    Camera { key: "chest".into(), eye: [-2.0, 1.2, -7.0], target: [3.0, 1.2, -7.5], fov: 35.0 },
                    Camera { key: "wrist_left".into(), eye: [-2.0, 1.0, -7.0], target: [3.0, 1.0, -7.5], fov: 35.0 },
                    Camera { key: "head".into(), eye: [-1.7, 1.42, -7.15], target: [3.0, 2.4, -7.5], fov: 35.0 },
                    Camera { key: "standing".into(), eye: [-3.8, 0.88, -7.6], target: [49.0, 0.7, -9.0], fov: 35.0 },
                ],
                bodies: vec![
                    Body { key: "standing".into(), position: [-1.0, 0.0, -7.05], heading: 260.0, turn: 0.1, anim: "CAS_Stand".into() },
                    Body { key: "wrist".into(), position: [-1.0, 0.0, -7.05], heading: 149.2, turn: 0.1, anim: "CAS_Stand".into() },
                ],
                postures: vec![("posture_normal".into(), "CAS_Stand".into()), ("posture_stiff".into(), "CAS_Stand_Stiff".into())],
                hidden_clothing: Vec::new(),
                hidden_sponsors: vec!["cac_promo_drpepper1".into()],
                tattoo_categories: vec![
                    crate::cas_rules::TattooCategory { text: "1".into(), untagged: true },
                    crate::cas_rules::TattooCategory { text: "2".into(), untagged: false },
                ],
                stamp_zones: vec![("tattooupper".into(), "upper_tattoo".into()), ("tattoolower".into(), "lower_tattoo".into())],
            },
        })
    }

    fn pete() -> Cac {
        let mut r = SkaterRecord::cleared();
        r.worn[Slot::Hair as usize] = HAT_CUT;
        r.worn[Slot::Rostral as usize] = HEAD_PLAIN;
        r.worn[Slot::Organ as usize] = EYES_BLUE;
        r.colour_keys[1] = 2;
        r.colour_keys[2] = 3;
        r.colour_keys[6] = 5;
        let mut c = Cac::open(r, &UnlockStore::fresh_profile(), Some(data()));
        c.select(0);
        c
    }

    /// Opens a row of Back Alley Pete's by its item.
    fn open(c: &mut Cac, item: usize) {
        let row = (0..c.row_count() as i32).find(|&r| c.item(r) == Some(item)).unwrap();
        c.select(row);
    }

    #[test]
    fn back_alley_is_its_own_group_with_an_undo_point() {
        let mut c = pete();
        assert_eq!((c.menu(), c.info_panel(0), c.banner()), (MENU_BACK_ALLEY, "petes", "petes"));
        assert!(c.states.contains(&STATE_GROUP));
        assert_eq!(c.row_label(0), "ID_CAC_CHOOSE_GENDER_MALE");
        assert_eq!(c.row_count(), 9);
        c.key(Key::Back);
        assert_eq!((c.menu(), c.info_panel(0), c.banner()), (MENU_EDIT_SKATER, "blurb", "none"));
    }

    #[test]
    fn the_hair_page_wraps_through_no_hair_and_waits_for_the_mesh() {
        let mut c = pete();
        open(&mut c, 0x3B);
        assert_eq!((c.menu(), c.camera().map(|c| c.key.as_str())), (MENU_HAIR, Some("head")));
        // The list is the row without hair, then the full cuts highest id first; the hat cut shows as its full cut.
        assert_eq!(c.list, [Worn::NONE, HAIR_A, HAIR_B]);
        assert_eq!(c.row_string(0), "ID_CAC_HAIRSTYLE_SHORT");
        c.key(Key::Left);
        assert_eq!((c.record.worn[Slot::Hair as usize], c.row_string(0).as_str()), (HAIR_A, "ID_CAC_HAIRSTYLE_MOOSED_UP"));
        assert!(c.busy && c.look == Some(LookChange::Rebuild));
        // The keys wait.
        c.key(Key::Left);
        assert_eq!(c.record.worn[Slot::Hair as usize], HAIR_A);
        c.look_done();
        c.key(Key::Left);
        assert_eq!((c.record.worn[Slot::Hair as usize], c.row_string(0).as_str()), (Worn::NONE, "ID_CAC_HAIRSTYLE_BALD"));
        c.look_done();
        // Any row accepts, back on Pete's.
        c.select(0);
        assert_eq!(c.menu(), MENU_BACK_ALLEY);
    }

    #[test]
    fn colours_stop_at_their_ends_and_skin_covers_its_four_layers() {
        let mut c = pete();
        open(&mut c, 0x39);
        assert_eq!(c.row_colours(0), [0.8, 0.8, 0.8, 0.2, 0.2, 0.2]);
        assert_eq!(c.row_integer(0), 0);
        c.key(Key::Left);
        assert_eq!(c.row_integer(0), 0);
        c.look_done();
        c.key(Key::Right);
        assert_eq!(c.row_integer(0), 1);
        for layer in 2..=4 {
            assert_eq!(c.record.colour_keys[layer], 4);
        }
        let head = c.record.colours.iter().find(|l| l.index == 4).unwrap();
        assert_eq!((head.part, head.primary), (HEAD_PLAIN, [0.2; 3]));
    }

    #[test]
    fn eye_colour_wears_the_eyes_of_its_tag() {
        let mut c = pete();
        open(&mut c, 0x3D);
        open(&mut c, 0x45);
        assert_eq!(c.camera().map(|c| c.key.as_str()), None);
        c.key(Key::Right);
        assert_eq!(c.record.worn[Slot::Organ as usize], EYES_GREEN);
        c.look_done();
        c.select(0);
        assert_eq!(c.menu(), MENU_FACE);
    }

    #[test]
    fn a_slider_moves_while_held_as_its_weight_within_the_limits() {
        let mut c = pete();
        open(&mut c, 0x3D);
        open(&mut c, 0x47);
        // Nose length is the first row: 0 to 1 on the slider is 0 to 0.5 of the record's weight.
        c.hold(0.05, 1.0 / 30.0);
        assert_eq!(c.record.morphs[7], 0.0, "inside the dead zone");
        c.hold(1.0, 1.0);
        assert!((c.record.morphs[7] - 0.15).abs() < 1e-5);
        assert_eq!(c.row_integer(0), 37);
        assert_eq!((c.look, c.busy), (Some(LookChange::Morphs), false));
        for _ in 0..200 {
            c.hold(1.0, 1.0);
        }
        assert_eq!(c.record.morphs[7], 0.5);
    }

    #[test]
    fn body_size_is_fat_above_the_middle_and_thin_below() {
        let mut c = pete();
        open(&mut c, 0x3A);
        assert_eq!(c.row_integer(0), 61);
        c.set_morph(ITEM_BODY_SIZE, 1.0);
        assert_eq!((c.record.morphs[0], c.record.morphs[1]), (0.5, 0.0));
        c.set_morph(ITEM_BODY_SIZE, 0.0);
        assert_eq!((c.record.morphs[0], c.record.morphs[1]), (0.0, 0.5));
        assert_eq!(c.row_integer(0), 0);
        // Its undo row takes the morphs back, on Pete's.
        c.select(1);
        assert_eq!((c.record.morphs[1], c.menu()), (0.0, MENU_BACK_ALLEY));
    }

    #[test]
    fn presets_walk_through_custom_which_is_the_skater_as_opened() {
        let mut c = pete();
        open(&mut c, 0x4B);
        assert_eq!(c.row_string(0), "ID_CAC_PRESET_CUSTOM");
        c.key(Key::Right);
        assert_eq!((c.row_string(0).as_str(), c.record.morphs[7], c.record.morphs[0], c.record.colour_keys[2]), ("ID_CAC_PRESET_01", 0.5, 0.0, 4));
        c.look_done();
        c.key(Key::Right);
        assert_eq!(c.row_string(0), "ID_CAC_PRESET_02");
        c.look_done();
        c.key(Key::Right);
        assert_eq!((c.row_string(0).as_str(), c.record.morphs[7], c.record.colour_keys[2]), ("ID_CAC_PRESET_CUSTOM", 0.0, 3));
        c.look_done();
        // Before the first, the last.
        c.key(Key::Left);
        assert_eq!(c.row_string(0), "ID_CAC_PRESET_02");
    }

    #[test]
    fn facial_hair_and_brows_are_heads_that_differ_only_there() {
        let mut c = pete();
        open(&mut c, 0x3C);
        assert_eq!(c.list, [HEAD_PLAIN, HEAD_BEARD]);
        c.key(Key::Right);
        assert_eq!((c.record.worn[Slot::Rostral as usize], c.row_string(0).as_str()), (HEAD_BEARD, "ID_CAC_FACIAL_HAIR_GOATEE"));
        c.look_done();
        c.select(0);
        open(&mut c, 0x3D);
        open(&mut c, 0x46);
        // The worn head has a goatee, so only that head remains here.
        assert_eq!((c.list.as_slice(), c.row_string(0).as_str()), ([HEAD_BEARD].as_slice(), "ID_CAC_EYEBROW_STYLE1"));
    }

    #[test]
    fn the_gender_row_asks_first_and_each_gender_keeps_its_character() {
        let mut c = pete();
        c.select(0);
        assert!(c.ask_gender && c.record.gender == Gender::Male);
        c.record.style = 1;
        c.switch_gender();
        assert_eq!((c.record.gender, c.record.style, c.record.worn[Slot::Hair as usize]), (Gender::Female, 1, FEMALE_HAIR));
        assert_eq!((c.row_label(0), c.row_count()), ("ID_CAC_CHOOSE_GENDER_FEMALE", 8), "no facial hair row");
        c.look_done();
        c.switch_gender();
        assert_eq!((c.record.gender, c.record.worn[Slot::Hair as usize], c.row_count()), (Gender::Male, HAT_CUT, 9));
    }

    #[test]
    fn undo_takes_the_skater_back_to_the_groups_start() {
        let mut c = pete();
        open(&mut c, 0x3B);
        c.key(Key::Left);
        c.look_done();
        c.select(2);
        // Undo counted as Accept: back on Pete's, the old hair on.
        assert_eq!((c.record.worn[Slot::Hair as usize], c.menu()), (HAT_CUT, MENU_BACK_ALLEY));
    }

    #[test]
    fn the_preview_stands_on_the_body_row_and_the_triggers_turn_it() {
        let mut c = pete();
        let p = c.preview().unwrap();
        assert_eq!((p.position, p.stand.as_str(), p.camera.key.as_str()), ([-1.0, 0.0, -7.05], "CAS_Stand", "standing"));
        c.turn_by(0.01, 1.0);
        assert_eq!(c.preview().unwrap().heading, 260f32.to_radians(), "inside the dead zone");
        c.turn_by(1.0, 1.0 / 30.0);
        assert!((c.preview().unwrap().heading - (260f32.to_radians() + 0.1)).abs() < 1e-5);
        c.record.posture = 1;
        assert_eq!(c.preview().unwrap().stand, "CAS_Stand_Stiff");
        // Back on Edit Skater the turn starts over.
        c.key(Key::Back);
        assert_eq!(c.preview().unwrap().heading, 260f32.to_radians());
    }

    fn slappys(unlocks: &UnlockStore) -> Cac {
        let mut r = SkaterRecord::cleared();
        r.worn[Slot::Arm as usize] = ARM;
        r.worn[Slot::OuterTorso as usize] = TEE_A;
        r.worn[Slot::Feet as usize] = SHOE;
        let mut c = Cac::open(r, unlocks, Some(data()));
        c.select(1);
        assert_eq!((c.menu(), c.banner(), c.info_panel(0)), (MENU_SLAPPYS, "slappys", "noblurb"));
        c.states.clear();
        c
    }

    /// The T-shirts page of [`slappys`]: the remove tile, the worn tee, the custom tee first, then by brand, `skate`
    /// last; the female, worn-in, hidden promo and other top types are out.
    fn tees(c: &mut Cac) {
        c.set_cursor(1, 0);
        c.select(1);
        assert_eq!((c.menu(), c.title(), c.camera.as_str()), (MENU_THUMBNAILS, "ID_CAC_TSHIRTS_TITLE", "chest"));
        assert_eq!(c.states, [MENUS[MENU_THUMBNAILS].state]);
        c.states.clear();
    }

    #[test]
    fn a_shop_page_lists_its_class_in_retails_brand_order_with_the_worn_first() {
        let mut c = slappys(&UnlockStore::default());
        tees(&mut c);
        assert_eq!(c.list, [Worn::NONE, TEE_A, TEE_CUSTOM, TEE_B, TEE_C]);
        assert_eq!(c.row_count(), 5);
        // The tiles' states, icons and texts.
        assert_eq!((c.info_panel(0), c.row_kind(0), c.row_description(0, &|_| true)), ("blurb", "none", "ID_CAC_REMOVE_PART".to_owned()));
        assert_eq!((c.info_panel(1), c.row_kind(1)), ("owned", "owned"));
        assert_eq!((c.info_panel(3), c.row_kind(3), c.unlock_text(3)), ("locked", "locked", "ID_CAC_UNLOCK_BS_GROUP1_UNLOCK".to_owned()));
        assert_eq!((c.info_panel(2), c.unlock_text(2), c.is_new(3)), ("none", "#".to_owned(), false));
        assert_eq!(c.row_description(1, &|n| n == "ID_CAC_PART_TEE_A_TEE_A_MAT"), "ID_CAC_PART_TEE_A_TEE_A_MAT");
        assert_eq!(c.row_description(1, &|_| false), "ID_CAC_COMPONENT_TYPE_OUTERTORSO");
        assert_eq!(c.thumbnail(0), "fe\\cas\\general\\remove");
        assert_eq!(c.thumbnail(1), "fe\\cas\\male\\outertorso\\tee_a_tee_a_mat");
        // Hoodies: one row per style, its default wear style.
        c.key(Key::Back);
        assert_eq!((c.menu(), c.camera.as_str()), (MENU_SLAPPYS, "standing"));
        c.select(3);
        assert_eq!(c.list, [Worn::NONE, HOODY_DOWN]);
    }

    #[test]
    fn granted_unlocks_open_the_lock_and_show_hidden_items_new() {
        let mut store = UnlockStore::default();
        store.grant(UnlockKind::ClothingUnlock, "cac_unlock_bs_group1");
        store.grant(UnlockKind::ClothingSponsor, "cac_promo_drpepper1");
        let mut c = slappys(&store);
        tees(&mut c);
        // The promo tee has no brand: it goes with `no_brand`, last.
        assert_eq!(c.list, [Worn::NONE, TEE_A, TEE_CUSTOM, TEE_B, TEE_C, TEE_PROMO]);
        assert_eq!((c.info_panel(3), c.is_new(3), c.is_new(5)), ("none", true, true));
        store.unlock_all = true;
        let mut c = slappys(&store);
        tees(&mut c);
        assert!(!c.is_new(3), "nothing is new with unlock-all");
    }

    #[test]
    fn a_highlight_tries_the_item_on_after_half_a_second_and_leaving_shows_the_record() {
        let mut c = slappys(&UnlockStore::default());
        tees(&mut c);
        c.highlight(4);
        c.tick(0.3);
        assert_eq!((c.trying, c.look), (None, None));
        c.tick(0.3);
        assert_eq!((c.trying, c.look), (Some((Slot::OuterTorso, TEE_C)), Some(LookChange::Rebuild)));
        assert_eq!(c.shown_record().worn[Slot::OuterTorso as usize], TEE_C);
        assert_eq!(c.record.worn[Slot::OuterTorso as usize], TEE_A, "the record is not touched");
        // The worn tee and the remove tile show the record.
        c.look = None;
        c.highlight(1);
        c.tick(0.6);
        assert_eq!(c.trying, None);
        c.highlight(4);
        c.tick(0.6);
        c.look = None;
        c.key(Key::Back);
        assert_eq!((c.menu(), c.trying, c.look), (MENU_SLAPPYS, None, Some(LookChange::Rebuild)));
    }

    #[test]
    fn picking_wears_the_item_or_sounds_a_lock_and_the_remove_tile_bares_the_torso() {
        let mut c = slappys(&UnlockStore::default());
        tees(&mut c);
        c.select_thumb(3);
        assert_eq!((c.menu(), c.sounds.last().copied(), c.record.worn[Slot::OuterTorso as usize]), (MENU_THUMBNAILS, Some("core_error"), TEE_A));
        c.select_thumb(4);
        assert_eq!((c.menu(), c.record.worn[Slot::OuterTorso as usize], c.busy), (MENU_SLAPPYS, TEE_C, true));
        assert!(c.record.closet[Slot::OuterTorso as usize].contains(&TEE_C), "kept in the closet");
        assert_eq!(c.states, [STATE_PART_ICON, MENUS[MENU_SLAPPYS].state]);
        c.states.clear();
        c.look_done();
        tees(&mut c);
        assert_eq!(c.list[1], TEE_C, "the worn tee comes first");
        c.select_thumb(0);
        assert_eq!((c.menu(), c.record.worn[Slot::OuterTorso as usize], c.record.adjust.flag), (MENU_SLAPPYS, BARE, 1));
    }

    #[test]
    fn socks_hide_tiny_and_shoes_take_the_skaters_gender_folder() {
        let mut c = slappys(&UnlockStore::default());
        c.select(8);
        assert_eq!(c.list, [Worn::NONE, SOCK_CREW], "socks can be taken off; tiny is not offered");
        c.key(Key::Back);
        c.select(7);
        assert_eq!(c.list, [SHOE], "shoes cannot be taken off");
        assert_eq!(c.thumbnail(0), "fe\\cas\\male\\feet\\half_cab_half_cab_material");
        c.key(Key::Back);
        // Accessories > Wrist: its camera and its pose.
        c.select(9);
        c.select(1);
        assert_eq!((c.menu(), c.camera.as_str(), c.body), (MENU_THUMBNAILS, "wrist_left", "wrist"));
        assert_eq!(c.preview().map(|p| p.heading), Some(149.2f32.to_radians()));
    }

    #[test]
    fn a_colourisable_item_opens_wear_styles_whose_colour_row_tints_the_worn_item() {
        let mut c = slappys(&UnlockStore::default());
        tees(&mut c);
        assert_eq!(c.list[2], TEE_CUSTOM);
        c.select_thumb(2);
        assert_eq!((c.menu(), c.row_count(), c.row_kind(0)), (MENU_WEAR_STYLES, 1, "color"));
        assert_eq!(c.record.worn[Slot::OuterTorso as usize], TEE_CUSTOM, "worn as Wear Styles opens");
        assert_eq!(c.row_colours(0).len(), 6, "the outer torso's zone");
        c.look_done();
        c.set_cursor(0, 0);
        assert!(c.key(Key::Right));
        let layer = c.record.colours.iter().find(|l| l.index == 9).copied().unwrap();
        assert_eq!((layer.part, c.record.colour_keys[9], c.row_integer(0)), (TEE_CUSTOM, 91, 1));
        c.look_done();
        c.select(0);
        assert_eq!(c.menu(), MENU_SLAPPYS, "any row accepts");
    }

    #[test]
    fn the_style_row_wraps_over_the_variants_and_wears_each() {
        let mut c = slappys(&UnlockStore::default());
        c.select(3);
        assert_eq!(c.list, [Worn::NONE, HOODY_DOWN]);
        c.select_thumb(1);
        assert_eq!((c.menu(), c.wear_rows.clone(), c.list.clone()), (MENU_WEAR_STYLES, vec![ITEM_WEAR_STYLE], vec![HOODY_DOWN, HOODY_UP]));
        assert_eq!((c.row_string(0), c.record.worn[Slot::OuterTorso as usize]), ("ID_CAC_WEAR_STYLE_OPTION_1".to_owned(), HOODY_DOWN));
        c.look_done();
        c.set_cursor(0, 0);
        c.key(Key::Right);
        assert_eq!((c.row_string(0), c.record.worn[Slot::OuterTorso as usize]), ("ID_CAC_WEAR_STYLE_OPTION_2".to_owned(), HOODY_UP));
        c.look_done();
        c.key(Key::Right);
        assert_eq!(c.record.worn[Slot::OuterTorso as usize], HOODY_DOWN, "it wraps");
    }

    #[test]
    fn a_deck_is_edit_deck_and_its_colour_wears_the_deck_of_that_colour() {
        let mut c = slappys(&UnlockStore::default());
        c.select(10);
        c.select(0);
        // Both decks are logoable: the custom group, reversed. No remove tile, no worn-in copy.
        assert_eq!((c.menu(), c.list.clone()), (MENU_THUMBNAILS, vec![DECK_BLUE, DECK_RED]));
        c.select_thumb(0);
        assert_eq!((c.menu(), c.title(), c.wear_rows.clone()), (MENU_WEAR_STYLES, "ID_CUSTOM_EDITDECK", vec![ITEM_ADD_GRAPHIC, ITEM_REMOVE_GRAPHIC]));
        c.look_done();
        c.select(1);
        assert_eq!(c.menu(), MENU_SLAPPYS, "Remove Graphic accepts back to Slappy's");
        c.select(10);
        c.select(0);
        c.select_thumb(1);
        assert_eq!((c.wear_rows.clone(), c.record.worn[Slot::SkateBoard as usize]), (vec![ITEM_WEAR_COLOUR, ITEM_ADD_GRAPHIC, ITEM_REMOVE_GRAPHIC], DECK_RED));
        c.look_done();
        c.set_cursor(0, 0);
        c.key(Key::Right);
        assert_eq!((c.record.worn[Slot::SkateBoard as usize], c.record.colour_keys[11]), (DECK_BLUE, 111), "the clean blue deck");
    }

    #[test]
    fn a_decks_graphic_goes_categories_brand_logos_then_its_placement_page() {
        let mut c = slappys(&UnlockStore::default());
        c.select(10);
        c.select(0);
        c.select_thumb(0);
        c.look_done();
        // Add Graphic: the categories (no Back), Brand: the logos (not the team's, the tattoos' or logoEmpty).
        c.select(0);
        assert_eq!((c.menu(), c.row_count(), c.table().help_b), (MENU_GRAPHIC_CATEGORIES, 2, 0));
        c.select(0);
        assert_eq!((c.menu(), c.list.clone()), (MENU_GRAPHIC_BRAND, vec![STAMP_LOGO]));
        assert_eq!((c.thumbnail(0), c.row_kind(0)), ("fe/cas/logos/logo_object_logo_thing".to_owned(), "none"));
        c.select_thumb(0);
        c.look_done();
        assert_eq!(c.menu(), MENU_ADJUST_BOARD);
        assert_eq!((c.record.adjust.words[3], c.record.adjust.values[3]), (0, stamp::start(2, false).values()));
        assert_eq!(c.record.graphics.iter().map(|g| (g.name.as_str(), g.flags[1])).collect::<Vec<_>>(), vec![("logo_thing", 4)]);
        // The sticks and X; A goes back to Slappy's.
        c.adjust_by([0.0, 0.0, 1.0, 0.0], 1.0 / 30.0);
        assert_ne!(c.record.adjust.values[3], stamp::start(2, false).values());
        assert!(c.key(Key::Reset));
        assert_eq!(c.record.adjust.values[3], stamp::start(2, false).values());
        assert!(!c.key(Key::Toggle) || c.menu() == MENU_ADJUST_BOARD);
        assert!(c.key(Key::Accept));
        assert_eq!(c.menu(), MENU_SLAPPYS);
        // The deck's Remove Graphic clears it.
        c.select(10);
        c.select(0);
        c.select_thumb(0);
        c.look_done();
        c.select(1);
        assert_eq!((c.record.graphics.len(), c.record.adjust.words[3]), (0, u32::MAX));
        assert_eq!(c.menu(), MENU_SLAPPYS);
    }

    #[test]
    fn a_shirts_graphic_picks_its_location_first_and_the_team_page_lists_team_logos() {
        let mut c = slappys(&UnlockStore::default());
        c.record.worn[Slot::OuterTorso as usize] = TEE_LOGOABLE;
        c.select(8 - 4);
        c.select_thumb(1);
        assert_eq!(c.menu(), MENU_WEAR_STYLES);
        c.look_done();
        assert_eq!(c.wear_rows, vec![ITEM_ADD_SHIRT_GRAPHIC, ITEM_REMOVE_GRAPHIC]);
        c.select(0);
        assert_eq!((c.menu(), c.row_count()), (MENU_SHIRT_LOCATION, 4));
        // The back: its camera and pose, the placement slot's word and start.
        c.select(1);
        assert_eq!((c.menu(), c.camera.as_str(), c.body, c.record.adjust.words[0]), (MENU_GRAPHIC_CATEGORIES, "custom_shirt_back", "back", 1));
        assert_eq!(c.record.adjust.values[0], stamp::start(4, false).values());
        c.select(1);
        assert_eq!((c.menu(), c.list.clone()), (MENU_GRAPHIC_TEAM, vec![STAMP_TEAM]));
        c.select_thumb(0);
        c.look_done();
        assert_eq!(c.menu(), MENU_ADJUST_SHIRT);
        assert_eq!(c.record.graphics.iter().map(|g| g.flags[1]).collect::<Vec<_>>(), vec![1]);
        // B goes back a page (the logos), A to Slappy's.
        assert!(c.key(Key::Back));
        assert_eq!(c.menu(), MENU_GRAPHIC_TEAM);
    }

    /// Pete's tattoo rows (19, 20), a body in a tee and plain pants.
    fn tattooed() -> Cac {
        let mut c = pete();
        c.record.worn[Slot::OuterTorso as usize] = TEE_A;
        c.record.worn[Slot::Arm as usize] = ARM;
        c.record.worn[Slot::Pants as usize] = PANTS_PLAIN;
        c
    }

    #[test]
    fn the_stamps_a_category_lists_follow_retails_name_filter() {
        let c = tattooed();
        let data = c.data.clone().unwrap();
        let cats = &data.vault.tattoo_categories;
        // The first category takes the untagged stamps too; logos, team logos and logoEmpty are never listed.
        assert_eq!(data.catalog.tattoo_rows(&cats[0]), vec![STAMP_UNTAGGED, STAMP_A]);
        assert_eq!(data.catalog.tattoo_rows(&cats[1]), vec![STAMP_B]);
        assert_eq!(data.catalog.stamp_thumbnail(STAMP_A), "fe/cas/logos/logo_object_tattoo_a_mat");
        assert_eq!(data.catalog.stamp_label(STAMP_A), "ID_CAC_PART_LOGO_OBJECT_TATTOO_A_MAT");
        assert_eq!(data.catalog.stamp_bounds(STAMP_B), [1.0, 0.0, 0.0, 1.0], "a stamp without bounds takes the whole picture");
        assert_eq!(data.catalog.stamp_bounds(STAMP_A), [1.0, 0.0, 0.25, 0.75]);
        // The quarter a part gives, or the whole quarter.
        assert_eq!(data.catalog.stamp_rect(Some((Slot::OuterTorso, BARE)), 7), [0.99, 0.52, 0.16, 0.38]);
        assert_eq!(data.catalog.stamp_rect(None, 7), [1.0, 0.5, 0.0, 0.5]);
        assert_eq!(data.catalog.stamp_rect(Some((Slot::OuterTorso, TEE_A)), 10), [1.0, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn an_upper_tattoo_goes_region_category_stamp_then_is_placed_on_the_bare_body() {
        let mut c = tattooed();
        open(&mut c, ITEM_UPPER_TATTOO);
        assert_eq!((c.menu(), c.row_count()), (MENU_TATTOO_UPPER, 4));
        assert_eq!(c.row_label(2), "ID_CAC_CUSTOMGRAPHIC_UPPERTATTOO_LEFTARM");
        // The left arm: no tattoo yet, so no question; the top is off, the camera and the pose are the arm's.
        c.select(2);
        assert_eq!(c.ask_tattoo, None);
        assert_eq!((c.menu(), c.record.adjust.words[1], c.record.adjust.flag), (MENU_TATTOO_CATEGORIES, 2, 1));
        assert_eq!((c.camera.as_str(), c.body), ("upper_tattoo_left_arm", "left_arm"));
        assert_eq!(c.tattoo.top, Some(TEE_A));
        assert!(c.look.is_some());
        c.look_done();
        // The categories, then the stamps with the remove tile first.
        assert_eq!((c.row_count(), c.row_label(1), c.table().help_b), (2, "ID_CAC_TATTOO_CATEGORY_02", 0));
        assert!(!c.key(Key::Back), "the categories have no Back");
        c.select(0);
        assert_eq!(c.menu(), MENU_TATTOO_STAMPS);
        assert_eq!(c.camera, "upper_tattoo", "the stamp zone's camera");
        assert_eq!((c.row_count(), c.thumbnail(0), c.row_description(0, &|_| true)), (3, THUMB_REMOVE.to_owned(), "ID_CAC_REMOVE_TATTOO".to_owned()));
        assert_eq!(c.thumbnail(2), "fe/cas/logos/logo_object_tattoo_a_mat");
        // Picking a stamp opens the placement page at the start of its location.
        c.select_thumb(2);
        assert!(c.look.is_some(), "the stamp is drawn: the mesh is built again");
        c.look_done();
        assert_eq!(c.menu(), MENU_TATTOO_ADJUST_UPPER);
        assert_eq!(c.record.adjust.values[1], stamp::start(7, false).values());
        assert_eq!(c.record.graphics.len(), 1);
        assert_eq!((c.record.graphics[0].name.as_str(), c.record.graphics[0].flags), ("tattoo_a_mat", [0, 2]));
        // The sticks move it a little a frame and keep it on the part.
        let before = c.record.adjust.values[1];
        c.adjust_by([0.0, 0.0, 1.0, 0.0], 1.0 / 30.0);
        assert_ne!(c.record.adjust.values[1], before);
        for _ in 0..300 {
            c.adjust_by([1.0, -1.0, 1.0, 1.0], 1.0 / 30.0);
        }
        let rect = c.data.as_ref().unwrap().catalog.stamp_rect(Some((Slot::OuterTorso, c.record.worn[Slot::OuterTorso as usize])), 7);
        let bounds = c.data.as_ref().unwrap().catalog.stamp_bounds(STAMP_A);
        assert!(stamp::fits(Placement::from_values(c.record.adjust.values[1]), rect, bounds));
        // X starts over; Y puts the top on and off; A goes back to Pete's with the top on.
        assert!(c.key(Key::Reset));
        assert_eq!(c.record.adjust.values[1], stamp::start(7, false).values());
        assert!(!c.key(Key::Toggle));
        assert_eq!((c.record.worn[Slot::OuterTorso as usize], c.record.adjust.flag), (TEE_A, 0));
        c.look_done();
        assert!(!c.key(Key::Toggle));
        assert_eq!(c.record.adjust.flag, 1);
        c.look_done();
        assert!(c.key(Key::Accept));
        assert_eq!((c.menu(), c.record.worn[Slot::OuterTorso as usize], c.record.adjust.flag), (MENU_BACK_ALLEY, TEE_A, 0));
        assert_eq!((c.record.adjust.words[1], c.record.graphics.len()), (2, 1), "the tattoo stays");
    }

    #[test]
    fn a_body_with_a_tattoo_asks_before_a_region_and_the_remove_tile_clears_it() {
        let mut c = tattooed();
        open(&mut c, ITEM_UPPER_TATTOO);
        c.select(0);
        c.look_done();
        c.select(0);
        c.select_thumb(1);
        c.look_done();
        c.key(Key::Accept);
        c.look_done();
        assert_eq!(c.menu(), MENU_BACK_ALLEY);
        open(&mut c, ITEM_UPPER_TATTOO);
        c.select(1);
        assert_eq!((c.ask_tattoo.take(), c.menu()), (Some(false), MENU_TATTOO_UPPER));
        c.tattoo_answer(false, false);
        assert_eq!((c.menu(), c.record.adjust.words[1]), (MENU_TATTOO_UPPER, 0));
        c.select(1);
        assert_eq!(c.ask_tattoo.take(), Some(false));
        c.tattoo_answer(false, true);
        assert_eq!((c.menu(), c.record.adjust.words[1]), (MENU_TATTOO_CATEGORIES, 1));
        c.look_done();
        // The remove tile: no tattoo, the top on, back at Pete's.
        c.select(0);
        c.select_thumb(0);
        c.look_done();
        assert_eq!((c.menu(), c.record.adjust.words[1], c.record.graphics.len()), (MENU_BACK_ALLEY, u32::MAX, 0));
        assert_eq!((c.record.worn[Slot::OuterTorso as usize], c.record.adjust.flag), (TEE_A, 0));
        assert_eq!(c.record.adjust.values[1], stamp::NONE.values());
    }

    #[test]
    fn a_lower_tattoo_wears_the_viewing_pants_and_gives_them_back() {
        let mut c = tattooed();
        open(&mut c, ITEM_LOWER_TATTOO);
        assert_eq!((c.menu(), c.row_count()), (MENU_TATTOO_LOWER, 2));
        c.select(1);
        assert_eq!((c.menu(), c.record.adjust.words[2], c.camera.as_str(), c.body), (MENU_TATTOO_CATEGORIES, 1, "lower_tattoo_right_leg", "right_leg"));
        assert_eq!(c.record.worn[Slot::Pants as usize], PANTS_VIEW);
        c.look_done();
        c.select(1);
        c.select_thumb(1);
        c.look_done();
        assert_eq!(c.menu(), MENU_TATTOO_ADJUST_LOWER);
        assert_eq!(c.record.adjust.values[2], stamp::start(11, false).values());
        assert_eq!(c.camera, "lower_tattoo");
        c.key(Key::Accept);
        assert_eq!((c.menu(), c.record.worn[Slot::Pants as usize]), (MENU_BACK_ALLEY, PANTS_PLAIN));
    }
}
