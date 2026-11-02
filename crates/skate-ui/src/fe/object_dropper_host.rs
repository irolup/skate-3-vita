// skate_vita: ours (docs/object-dropper-survey.md "The screen", step D5).
//! The Object Dropper screen's natives: `SkatePark.*` (registrar `825B3CE0`) answered from the engine's feed
//! ([`ObjectDropperFeed`], filled each frame from the editor), the screen modes the engine posts (`SetState(n)`, the
//! editor's `screen` numbers) and the Feng Shui bars the C++ draws over the movie's `FungShui` / `FungShuiDynamic`
//! placeholder clips (`825D4828` -> `825119C8`).
//!
//! The movie takes no keys (its class has no UpdateInput): the editor reads the pad and posts what changed. Two FE
//! states open the movie (template `ObjectDropper`): 0x35 in a skate.Park and 0x37 in the open world, which the movie
//! tells apart with `IsWorldSpawner` (no terrain bar, no Properties, "Reset all objects" on Back).
//!
//! NOT RETAIL YET: unlocks (every item unlocked and none new until the career), the park brand, the spawner (`IsSpawnerPresent`), the size text's distance format (metres,
//! one decimal) and the bars' colours past 0.75 / 0.9 (the drawer's vector inserts are not readable; yellow and red
//! inferred).
use crate::fe::apt_scene::{Draw, Fill, Vertex};
use crate::fe::apt_vm::{to_integer, Value, Vm};

/// The screen modes' names, in value order (the C++ table `0x8302C440`); the stage registers each as
/// `FEObjectDropperScreenMode_<name>` = its index, which the movie's `SetState` compares against.
pub const SCREEN_MODES: [&str; 30] = [
    "FreeCam",
    "Catalog",
    "Manipulate",
    "Refresh",
    "NextCategory",
    "PreviousCategory",
    "NextSubCategory",
    "PreviousSubCategory",
    "NextItem",
    "PreviousItem",
    "NextType",
    "PreviousType",
    "SnapObject",
    "MoveOnLockedAxis",
    "RotateObject",
    "GroupSelect",
    "HideSelection",
    "ShowSelection",
    "Delete",
    "RefreshFengShui",
    "EnableUI",
    "DisableUI",
    "RefreshDuplicate",
    "RefreshFineTune",
    "RefreshSize",
    "ShowLockAxis",
    "ShowRotate",
    "RefreshInfo",
    "GotoSubCategories",
    "GotoItems",
];

/// What the movie shows, from the editor each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectDropperFeed {
    /// The open-world dropper (FE state 0x37), else the park editor (0x35).
    pub world: bool,
    /// Super category keys (`catalog.json` / `fe_park_categories`), in order.
    pub supers: Vec<String>,
    /// The current super category's sub categories: key and unlocked item count.
    pub subs: Vec<(String, usize)>,
    /// The current sub category's items' model names (`Item::name`).
    pub items: Vec<String>,
    pub super_index: usize,
    pub sub_index: usize,
    pub item_index: usize,
    /// The sub category list shows (`S+0x2CF`).
    pub selecting_sub: bool,
    /// The item previewed or held: its model name and size (x, y, z metres).
    pub item: Option<(String, [f32; 3])>,
    /// The item at the cursor in the catalogue is terrain (else a prop).
    pub terrain: bool,
    pub fine_tune: bool,
    pub duplicate: bool,
    /// The meter: (used, to add) for terrain, then props (`D+0x173C..0x1748`), and the warning (`D+0x174C`).
    pub terrain_bar: [f32; 2],
    pub props_bar: [f32; 2],
    pub warning: bool,
    /// The selection's colour (rgb 0..1; white when uncoloured), for `GetColour`.
    pub colour: [f32; 3],
    /// The Quick Menu while it is open over the editor (FE state 0x36).
    pub quick: Option<QuickFeed>,
}

/// What the `ObjectQuickMenu` movie shows (the Quick Menu natives, `825B57D0` and its neighbours).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuickFeed {
    /// The main list's rows that can be chosen (Color, Branding, Style, Options, Copy, Paste, Undo, Redo).
    pub rows: [bool; 8],
    /// The options (`D+0x17AC` bits 7 / 6 / 5 / 4) and the cursor speed (`D+0x17B0`, 0.5 to 1.5).
    pub snap: bool,
    pub collision: bool,
    pub invert_x: bool,
    pub invert_y: bool,
    pub cursor_speed: f32,
    /// The page: 0 main, 1 options, 2 colour, 3 branding, 4 style (`Q+0x70`).
    pub page: u8,
    /// The selection's style row and whether Select can keep a style (`825D0BA8`).
    pub style_index: usize,
    pub style_allowed: bool,
    /// The Colour page: the swatches (rgb 0..1), the highlighted one, whether Apply can keep it (a colourisable
    /// selection, `825D0D18`).
    pub colours: Vec<[f32; 3]>,
    pub colour_index: usize,
    pub colour_allowed: bool,
}

/// The Quick Menu's pages whose list the movie drives (`mbAllowMenuPickerInput`).
pub const PAGE_BRANDING: u8 = 3;
pub const PAGE_STYLE: u8 = 4;

/// The Quick Menu's screen modes' names, in value order (the C++ table `0x8302C4B8`), registered as
/// `FEObjectQuickMenuScreenMode_<name>`.
pub const QUICK_SCREEN_MODES: [&str; 26] = [
    "EnableColor",
    "EnableBranding",
    "EnableStyle",
    "EnableOptions",
    "EnableCopy",
    "EnablePaste",
    "EnableUndo",
    "EnableRedo",
    "ActivateOptionsMenu",
    "ActivateQuickMenu",
    "ActivateColorMenu",
    "ActivateStyleMenu",
    "ActivateBrandingMenu",
    "RefreshSnap",
    "RefreshCollision",
    "RefreshInvertYAxis",
    "RefreshInvertXAxis",
    "RefreshCursorSpeed",
    "EnableSnap",
    "EnableCollision",
    "EnableInvertYAxis",
    "EnableInvertXAxis",
    "EnableCursor",
    "RefreshCursor",
    "RefreshColourMenuButtonCallouts",
    "RefreshPropertiesMenuButtonCallouts",
];

/// The park brands (`825B5BE8`, seven).
const BRANDS: [&str; 7] = [
    "ID_SKATEPARK_BRAND_OFF",
    "ID_SKATEPARK_BRAND_LOCAL",
    "ID_SKATEPARK_BRAND_ONLINE",
    "ID_SKATEPARK_BRAND_USERCREATED1",
    "ID_SKATEPARK_BRAND_USERCREATED2",
    "ID_SKATEPARK_BRAND_USERCREATED3",
    "ID_SKATEPARK_BRAND_USERCREATED4",
];
/// The style variants: the `fe_park_itemtypes` records with an item type, by their name field (`82877998`).
const VISUAL_TYPES: [&str; 3] = ["university", "industrial", "plaza"];
/// The colour grid's swatches (the movie's SetupColorGrid asks 64).
const SWATCHES: usize = 64;
/// Swatch renderer slots (the grid's 64 and the screens' own swatches).
pub const SWATCH_SLOTS: usize = 96;
/// Texture key prefix of a `SwatchRenderComponent` placeholder: `swatch/<slot>`.
pub const SWATCH_TEXTURE_PREFIX: &str = "swatch/";

#[derive(Clone, Debug, Default)]
pub struct ObjectDropperHost {
    /// The screen is open (its natives are ours).
    pub open: bool,
    /// The movie finished its intro and shows.
    pub shown: bool,
    pub feed: ObjectDropperFeed,
    /// Screen modes to post, in order.
    pub states: Vec<i32>,
    /// The Quick Menu screen is open, shown, and its modes to post.
    pub quick_open: bool,
    pub quick_shown: bool,
    pub quick_states: Vec<i32>,
    /// The Style / Branding list's callbacks for the editor: (row, selected).
    pub list_calls: Vec<(usize, bool)>,
    /// The swatch renderer's slots (`Game.SwatchRenderer_*`): each reserved slot's colour (rgb 0..1), drawn over the
    /// movie's `SwatchRenderComponent` clips named by their slot.
    pub swatches: Vec<Option<[f32; 3]>>,
    /// The natives the movie asked (tests).
    pub seen: std::collections::BTreeSet<String>,
}

/// A model name's size token (`8287EBA8`, `GetSize`): `sm` / `sml` 1, `md` / `med` 2, `lg` / `lrg` 3, else 0.
fn size(name: &str) -> u8 {
    if name.find('_') != Some(2) {
        return 0;
    }
    match name.split('_').nth(2).map(str::to_ascii_lowercase).as_deref() {
        Some("sm" | "sml") => 1,
        Some("md" | "med") => 2,
        Some("lg" | "lrg") => 3,
        _ => 0,
    }
}

/// A model name's style frame (`8287E980`, `GetStyle`: the last token against `fe_park_itemtypes`).
fn style(name: &str) -> &'static str {
    match name.rsplit('_').next().map(str::to_ascii_lowercase).as_deref() {
        Some("downtown") => "downtown",
        Some("industrial") => "industrial",
        Some("university" | "uni") => "uni",
        _ => "invalid",
    }
}

/// An item row's string id (`82881750` terrain: the second token; `82881860` props: the last), `#name` for a name
/// without the `xx_` prefix.
pub fn item_text(name: &str, terrain: bool) -> String {
    if name.find('_') != Some(2) {
        return format!("#{name}");
    }
    let token = if terrain { name.split('_').nth(1) } else { name.rsplit('_').next() };
    format!("ID_SKATEPARK_OBJECT_{}", token.unwrap_or_default().to_ascii_uppercase())
}

impl ObjectDropperHost {
    /// One native call; None = not ours.
    pub fn call(
        &mut self,
        vm: &mut Vm,
        language: &std::collections::BTreeMap<String, String>,
        object: &str,
        method: &str,
        args: &[Value],
    ) -> Result<Option<Value>, String> {
        if !self.open {
            return Ok(None);
        }
        // The item list is a menu picker: it asks the focus screen whether each row is enabled (vt+72: `8281DD70`,
        // true for both dropper states) and greys the rows it is not told about (no highlight).
        if object == "Game" {
            let slot_arg = |i: usize| args.get(i).map_or(0, to_integer).max(0) as usize;
            return Ok(match method {
                "OnMenuGetItemEnabled" => Some(Value::Bool(true)),
                // The Style and Branding lists (825D0288 highlight, 825D0120 select): for the editor.
                "OnMenuHilight" | "OnMenuSelect" => {
                    if self.feed.quick.as_ref().is_some_and(|q| q.page == PAGE_STYLE || q.page == PAGE_BRANDING) {
                        let row = args.get(1).map_or(0, to_integer).max(0) as usize;
                        self.list_calls.push((row, method == "OnMenuSelect"));
                    }
                    Some(Value::Undefined)
                }
                // The colour grid: the tint table as r, g, b triples (x 255, truncated as 825B62B8); white past it.
                "ColourPicker_GetColours" => {
                    let colours = self.feed.quick.as_ref().map(|q| q.colours.clone()).unwrap_or_default();
                    let rgb = (0..SWATCHES).flat_map(|i| colours.get(i).copied().unwrap_or([1.0; 3])).map(|c| Value::Number((c * 255.0).trunc() as f64)).collect();
                    Some(Value::Object(vm.array(rgb)?))
                }
                "ColourPicker_GetColourIndex" => Some(Value::Number(self.feed.quick.as_ref().map_or(0, |q| q.colour_index) as f64)),
                "SwatchRenderer_ReserveIndex" => {
                    let slot = self.swatches.iter().position(Option::is_none).unwrap_or(self.swatches.len());
                    if slot >= SWATCH_SLOTS {
                        return Ok(Some(Value::Number(-1.0)));
                    }
                    if slot == self.swatches.len() {
                        self.swatches.push(None);
                    }
                    self.swatches[slot] = Some([1.0; 3]);
                    Some(Value::Number(slot as f64))
                }
                // (slot, red, green, blue), 0 to 255.
                "SwatchRenderer_SetColour" => {
                    let c = |i: usize| args.get(i).map_or(0.0, |v| v.number() as f32 / 255.0).clamp(0.0, 1.0);
                    if let Some(slot) = self.swatches.get_mut(slot_arg(0)).filter(|s| s.is_some()) {
                        *slot = Some([c(1), c(2), c(3)]);
                    }
                    Some(Value::Undefined)
                }
                "SwatchRenderer_FreeIndex" => {
                    if let Some(slot) = self.swatches.get_mut(slot_arg(0)) {
                        *slot = None;
                    }
                    Some(Value::Undefined)
                }
                _ => None,
            });
        }
        if object != "SkatePark" {
            return Ok(None);
        }
        self.seen.insert(method.to_owned());
        let f = &self.feed;
        let index = |i: usize| args.get(i).map_or(0, to_integer).max(0) as usize;
        let category = |key: &str| format!("ID_SKATEPARK_CATEGORY_{}", key.to_ascii_uppercase());
        let item_name = f.item.as_ref().map(|(n, _)| n.as_str()).unwrap_or_default();
        Ok(Some(match method {
            // 825B45B0: a flag nothing reads.
            "ItemSelectionReady" => Value::Undefined,
            // 825B4780: `ID_SKATEPARK_CATEGORY_<KEY>` per super category.
            "GetSuperCategories" => {
                let tabs = f.supers.iter().map(|k| Value::Text(category(k))).collect();
                Value::Object(vm.array(tabs)?)
            }
            // 825B4988: literal "<localised name> (<unlocked count>)" (the movie adds `#`).
            "GetSubCategories" => {
                let rows = f
                    .subs
                    .iter()
                    .map(|(k, n)| {
                        let id = category(k);
                        Value::Text(format!("{} ({n})", language.get(&id).cloned().unwrap_or(id)))
                    })
                    .collect();
                Value::Object(vm.array(rows)?)
            }
            "GetNumSubCategories" => Value::Number(f.subs.len() as f64),
            // 825B4C30 / 825B4D10: the sub category's name and description ids.
            "GetSubCategoryName" => Value::Text(f.subs.get(index(0)).map_or("ID_STRING_MISSING".into(), |(k, _)| category(k))),
            "GetSubCategoryDescription" => {
                Value::Text(f.subs.get(index(0)).map_or("ID_STRING_MISSING".into(), |(k, _)| format!("{}_DESCRIPTION", category(k))))
            }
            "GetTotalItemsUnlockedForSubCategory" => Value::Number(f.subs.get(index(0)).map_or(0, |s| s.1) as f64),
            // 825B4E70: the rows' string ids, split by whether the current item is terrain.
            "GetItems" => {
                let rows = f.items.iter().map(|n| Value::Text(item_text(n, f.terrain))).collect();
                Value::Object(vm.array(rows)?)
            }
            // 825B5180 / 825B5320: everything unlocked, nothing new (until the career).
            "GetItemsUnlocked" | "GetItemsNewlyUnlocked" => {
                let on = method == "GetItemsUnlocked";
                Value::Object(vm.array(f.items.iter().map(|_| Value::Bool(on)).collect())?)
            }
            "GetCurrentSuperCategoryIndex" => Value::Number(f.super_index as f64),
            "GetCurrentSubCategoryIndex" => Value::Number(f.sub_index as f64),
            "GetCurrentItemIndex" => Value::Number(f.item_index as f64),
            "IsSelectingSubCategory" => Value::Bool(f.selecting_sub),
            // 825B5FA8: "ID_" + the Unlock cap; none set.
            "GetUnlockString" => Value::Text("ID_".into()),
            "GetSize" => Value::Number(size(item_name) as f64),
            "GetStyle" => Value::Text(style(item_name).into()),
            // 825B62B8: [r, g, b] 0-255 of the selection's colour (x 255, truncated).
            "GetColour" => Value::Object(vm.array(f.colour.iter().map(|c| Value::Number((c * 255.0).trunc() as f64)).collect())?),
            // 825B64E0: `ID_SKATEPARK_ITEM_SIZE` with the held item's x / y / z.
            "GetCurrentItemSizeInfo" => {
                let s = f.item.as_ref().map_or([0.0; 3], |(_, s)| *s);
                let template = language.get("ID_SKATEPARK_ITEM_SIZE").cloned().unwrap_or_default();
                let args: Vec<_> = s.iter().map(|x| crate::fe::fe_language::Arg::Text(format!("{x:.1}m"))).collect();
                Value::Text(format!("#{}", crate::fe::fe_language::format_string(&Default::default(), &template, &args)))
            }
            "IsCurrentItemTerrain" => Value::Bool(f.terrain),
            // 825B5840 / 825B5870: the Lua show / hide commands' flags (always on here).
            "IsUIEnabled" | "IsCursorVisible" => Value::Bool(true),
            "IsItemSelected" => Value::Bool(f.item.is_some()),
            "IsAnyItemHighlightedSelected" => Value::Bool(f.item.is_some()),
            "IsSpawnerPresent" => Value::Bool(false),
            // 825B54F0: FE state 0x37 open.
            "IsWorldSpawner" => Value::Bool(f.world),
            "IsFineTuneModeEnabled" => Value::Bool(!f.world && f.fine_tune),
            "IsDuplicateModeEnabled" => Value::Bool(f.duplicate),
            "IsDuplicateModeAllowed" => Value::Bool(true),
            "IsFengShuiShowingWarning" => Value::Bool(f.warning),
            "IsObjectDropperEnabled" | "AreWeInSkatePark" => Value::Bool(!f.world),
            // The Quick Menu (825B5650 ...): the options, the rows, the brands and styles.
            "GetCurrentSnapState" => Value::Bool(f.quick.as_ref().is_none_or(|q| q.snap)),
            "GetCurrentCollisionState" => Value::Bool(f.quick.as_ref().is_none_or(|q| q.collision)),
            "GetCurrentInvertXAxisState" => Value::Bool(f.quick.as_ref().is_some_and(|q| q.invert_x)),
            "GetCurrentInvertYAxisState" => Value::Bool(f.quick.as_ref().is_some_and(|q| q.invert_y)),
            // 825B5730: int((speed - 0.4999) x 10), 0 to 10.
            "GetCurrentCursorSpeed" => {
                let speed = f.quick.as_ref().map_or(1.0, |q| q.cursor_speed);
                Value::Number(((speed - 0.4999) * 10.0).floor().clamp(0.0, 10.0) as f64)
            }
            "IsCollisionUnlocked" | "QuickMenu_IsOptionItemEnabled" => Value::Bool(true),
            "QuickMenu_IsQuickMenuItemEnabled" => Value::Bool(f.quick.as_ref().is_none_or(|q| q.rows.get(index(0)).copied().unwrap_or(true))),
            "GetNumBrands" => Value::Number(BRANDS.len() as f64),
            "GetCurrentBrand" => Value::Number(0.0),
            "GetCurrentVisualTypeIndex" => Value::Number(f.quick.as_ref().map_or(0, |q| q.style_index) as f64),
            "GetBrandName" => Value::Text(BRANDS.get(index(0)).map_or("#N/A".into(), |b| (*b).to_owned())),
            "GetNumVisualTypes" => Value::Number(VISUAL_TYPES.len() as f64),
            "GetVisualTypeName" => {
                Value::Text(VISUAL_TYPES.get(index(0)).map_or("ID_STRING_MISSING".into(), |t| format!("ID_SKATEPARK_VISUALTYPE_{}", t.to_ascii_uppercase())))
            }
            "IsStyleChangeAllowed" => Value::Bool(f.quick.as_ref().is_some_and(|q| q.style_allowed)),
            "IsColorChangeApplyAllowed" => Value::Bool(f.quick.as_ref().is_some_and(|q| q.colour_allowed)),
            "IsColorChangeApplyAllAllowed" | "IsColorChangeBackAllowed" | "IsPropertiesMenuAcceptButtonAllowed" | "IsPropertiesMenuBackButtonAllowed" => {
                Value::Bool(true)
            }
            _ => return Ok(None),
        }))
    }

    /// The bars over a `FungShui` (terrain, `props` false) or `FungShuiDynamic` placeholder: the used part from the
    /// clip's left edge, then the part to add after it (`825119C8`), solid fills.
    pub fn bar_draws(&self, placeholder: &Draw, props: bool) -> Vec<Draw> {
        if !self.open || placeholder.vertices.len() < 3 {
            return Vec::new();
        }
        let [used, add] = if props { self.feed.props_bar } else { self.feed.terrain_bar };
        let [o, px, py] = [0, 1, 2].map(|i| placeholder.vertices[i].position);
        let at = |x: f32, y: f32| Vertex {
            position: [o[0] + x * (px[0] - o[0]) + y * (py[0] - o[0]), o[1] + x * (px[1] - o[1]) + y * (py[1] - o[1])],
            uv: [0.0; 2],
        };
        let quad = |x0: f32, x1: f32, rgba: [f32; 4]| Draw {
            texture: String::new(),
            fill: Fill::Solid,
            vertices: vec![at(x0, 0.0), at(x1, 0.0), at(x1, 1.0), at(x0, 0.0), at(x1, 1.0), at(x0, 1.0)],
            multiply: [rgba[0], rgba[1], rgba[2], rgba[3] * placeholder.multiply[3]],
            add: [0.0; 4],
        };
        // Base colours 0x8232FA60 (terrain) / 0x8232F980 (props), replaced at 0.75 and 0.9.
        let colour = if used >= 0.9 {
            [1.0, 0.0, 0.0, 1.0]
        } else if used >= 0.75 {
            [1.0, 1.0, 0.0, 1.0]
        } else if props {
            [0.0, 1.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 1.0, 1.0]
        };
        let used = used.clamp(0.0, 1.0);
        let mut out = vec![quad(0.0, used, colour)];
        if add > 0.0 {
            out.push(quad(used, (used + add).min(1.0), [1.0, 1.0, 1.0, 0.5]));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sizes_and_styles_follow_the_model_names() {
        assert_eq!(item_text("aa_padStreet_md_University", true), "ID_SKATEPARK_OBJECT_PADSTREET");
        assert_eq!(item_text("am_DMO_Glbl_safetyBarrierLong", false), "ID_SKATEPARK_OBJECT_SAFETYBARRIERLONG");
        assert_eq!(item_text("SkatePark_Spawner", true), "#SkatePark_Spawner");
        assert_eq!((size("aa_padStreet_md_University"), size("am_DMO_Glbl_x")), (2, 0));
        assert_eq!((style("aa_padStreet_md_University"), style("aa_x_lrg_Downtown"), style("am_DMO_Glbl_x")), ("uni", "downtown", "invalid"));
    }

    #[test]
    fn the_bars_fill_the_used_part_then_the_part_to_add() {
        let v = |x: f32, y: f32| Vertex { position: [x, y], uv: [0.0; 2] };
        let placeholder = Draw { texture: String::new(), fill: Fill::Solid, vertices: vec![v(100.0, 50.0), v(300.0, 50.0), v(100.0, 60.0)], multiply: [1.0; 4], add: [0.0; 4] };
        let mut host = ObjectDropperHost { open: true, ..Default::default() };
        host.feed.props_bar = [0.5, 0.25];
        let draws = host.bar_draws(&placeholder, true);
        assert_eq!(draws.len(), 2);
        assert_eq!((draws[0].vertices[1].position, draws[0].multiply), ([200.0, 50.0], [0.0, 1.0, 0.0, 1.0]));
        assert_eq!(draws[1].vertices[1].position, [250.0, 50.0]);
        host.feed.terrain_bar = [0.95, 0.0];
        assert_eq!(host.bar_draws(&placeholder, false)[0].multiply, [1.0, 0.0, 0.0, 1.0]);
    }
}
