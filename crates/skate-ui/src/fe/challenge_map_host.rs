// skate_vita: skate-game `challenge_map_host.rs` (PR #52 `fad5aae`) in skate-ui `fe` (`crate::fe::apt_vm`).
//! The Challenge Map screen's natives (FE screen 17): FEChallengeMap (registrar sub_8264A8A8, 24 getters, research
//! b255) and the movie's picker callbacks (Game.OnMenuHilight / OnMenuSelect, b256). The navigation is
//! `skate_core::challenge_map`; this host answers the movie from it and queues the screen states the C++ posts
//! (`SetState(n)`, applied by `retail_menu_stage` after the call returns) and the leaf a select picked (dispatched
//! through the menu registry, whose handler decides: a teleport, a mod).
use crate::fe::apt_vm::{to_integer, Value, Vm};
use skate_core::challenge_map::{Action, ChallengeMapState};
use skate_core::menus::{EntryKind, EntryRef, MenuContext, MenuRegistry};

#[derive(Clone, Default)]
pub struct ChallengeMapHost {
    /// The map markers to draw (icon, map pixels from the centre, highlighted), refreshed after each update while open.
    pub markers: Vec<(String, [f32; 2], bool)>,
    /// The map clip's translation on screen (MapRenderComponent's origin, retail `0x830B8600` = row 3 of the matrix
    /// the render callback `825D4390` writes; b282), set while drawing: markers and the cursor sit at origin + point.
    pub origin: [f32; 2],
    pub state: ChallengeMapState,
    /// The screen is on top of the stack (the picker callbacks are ours).
    pub open: bool,
    /// Screen states to post to the movie, in order.
    pub states: Vec<i32>,
    /// A leaf the player selected, for the stage to dispatch.
    pub selected: Option<EntryRef>,
    /// B at the root asked to close the screen.
    pub close: bool,
}

impl ChallengeMapHost {
    /// Screen init (`sub_826457E0`) on the mode root (`progression` offline).
    pub fn open(&mut self, registry: &MenuRegistry, ctx: &MenuContext, root: &str) {
        let (state, states) = ChallengeMapState::open(registry, ctx, root);
        *self = Self { state, open: true, states, ..Default::default() };
    }

    /// An action from a callback or a key: its states, its leaf, its close.
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Refresh(states) => self.states.extend(states),
            Action::Select(entry) => self.selected = Some(entry),
            Action::Close => self.close = true,
        }
    }

    /// One native call; None = not ours.
    pub fn call(
        &mut self,
        vm: &mut Vm,
        registry: &MenuRegistry,
        ctx: &MenuContext,
        language: &std::collections::BTreeMap<String, String>,
        object: &str,
        method: &str,
        args: &[Value],
    ) -> Result<Option<Value>, String> {
        let text = |id: &str| Value::Text(language.get(id).cloned().unwrap_or_else(|| id.to_owned()));
        let index = |i: usize| args.get(i).map_or(0, to_integer).max(0) as usize;
        if object == "Game" && self.open {
            return Ok(match method {
                "OnMenuHilight" => {
                    let states = self.state.hilight(registry, ctx, index(1));
                    self.states.extend(states);
                    Some(Value::Undefined)
                }
                "OnMenuSelect" => {
                    let action = self.state.select(registry, ctx, index(1));
                    self.apply(action);
                    Some(Value::Undefined)
                }
                // The picker asks per row (menu, row): our greyed rule (no handler, greyed; folders by their children).
                "OnMenuGetItemEnabled" => Some(Value::Bool(self.state.rows(registry, ctx).get(index(1)).is_some_and(|r| r.enabled))),
                _ => None,
            });
        }
        if object != "FEChallengeMap" {
            return Ok(None);
        }
        let s = &self.state;
        Ok(Some(match method {
            // 8264AF28: [count, selected, labels, header, icon 1, icon 2]; an empty folder gives one `false` per list.
            // List 3 is `!(child vt+0x28)`, not the greying: the movie adds a `true` row as a picker header
            // (AddSpecificItem(mHeaderItem)) that the picker skips, so only `map_category` labels are true; greyed rows
            // come from OnMenuGetItemEnabled.
            "GetStuffMenuItems" => {
                let rows = s.rows(registry, ctx);
                let lists: [Vec<Value>; 4] = if rows.is_empty() {
                    std::array::from_fn(|_| vec![Value::Bool(false)])
                } else {
                    [
                        // A folder's row shows its location count, "Downtown (14)" (upstream `74c124a`).
                        rows.iter().map(|r| match (text(&r.label), r.count) {
                            (Value::Text(t), Some(n)) => Value::Text(format!("{t} ({n})")),
                            (v, _) => v,
                        }).collect(),
                        rows.iter().map(|r| Value::Bool(r.entry.kind == EntryKind::MapCategory)).collect(),
                        rows.iter().map(|_| Value::Text("none".into())).collect(),
                        rows.iter().map(|_| Value::Text("none".into())).collect(),
                    ]
                };
                let mut values = vec![Value::Number(rows.len() as f64), Value::Number(s.hilight as f64)];
                for list in lists {
                    values.push(Value::Object(vm.array(list)?));
                }
                Value::Object(vm.array(values)?)
            }
            "GetStuffTitleText" | "GetChallengeTitle" => text(&s.title(registry, ctx)),
            // 8264C208: the tabs' names; 8264C3F0 the current tab.
            "GetSuperCategories" => {
                let tabs = s.tabs(registry, ctx).iter().map(|t| text(&registry.data().map_node(t).map_or(t.id.clone(), |n| n.label.clone()))).collect();
                Value::Object(vm.array(tabs)?)
            }
            "GetCurrentSuperCategoryIndex" => Value::Number(s.tab as f64),
            // 8264B658: false only on an empty filter folder.
            "IsSelectAvailable" => Value::Bool(s.rows(registry, ctx).get(s.hilight).is_none_or(|r| {
                !r.entry.kind.is_map_folder() || !registry.data().map_children(&r.entry).is_empty()
            })),
            // The map picture (b255 / b256: 8264B488, 8264B4A8, 8264B4B8): the highlighted location's `world` map, else
            // the default. GetMapTexture = [name or "map_missing", +0xF0 (1 unless the park screenshot), +0xF1 (the
            // record flag sub_824FB718, not read: 0)]. NOT RETAIL YET: the park screenshot.
            "IsUsingDefaultMap" => Value::Bool(s.picture.is_none()),
            "GetDefaultMapTexture" => Value::Text("map1024".into()),
            "GetMapTexture" => Value::Object(vm.array(vec![
                Value::Text(s.picture.clone().unwrap_or_else(|| "map_missing".into())),
                Value::Bool(true),
                Value::Bool(false),
            ])?),
            // 8264B570 / 8264B588 (b256 section 2): the cursor sits on the highlighted marker (+0xF2, +0x100 / +0x104,
            // whole pixels); no highlighted marker hides it. The coordinates include the map clip's origin (b282: the
            // movie sets mCursor._x / _y on the root with no offset).
            "IsCursorVisible" => Value::Bool(s.markers(registry, ctx).iter().any(|(.., h)| *h)),
            "GetCursorCoordinates" => {
                let at = s.markers(registry, ctx).into_iter().find(|(.., h)| *h).map_or([0.0; 2], |(_, p, _)| p);
                let o = self.origin;
                Value::Object(vm.array(vec![Value::Number((at[0] + o[0]).trunc() as f64), Value::Number((at[1] + o[1]).trunc() as f64)])?)
            }
            // Retail stubs / constants (b255).
            "GetStuffMenuChallengeFilterType" => Value::Bool(true),
            "GetMapDistrictText" => Value::Text("#GetMapDistrictText".into()),
            "GetPropertyPriceOrPaid" => Value::Text("#GetPropertyPriceOrPaid".into()),
            "IsPropertyOwned" | "IsChallengeTeamChallenge" | "HasOnlineDifficultySelector" => Value::Bool(false),
            "GetMapHelperInfoString" => Value::Text("#".into()),
            // Challenge details: no challenge system yet (the challenge rows stay greyed).
            "GetChallengeObjectiveTitle" => Value::Text("#OBJECTIVE NOT VALID!".into()),
            "GetChallengeNumObjectives" | "GetChallengeComponentNumOptions" => Value::Number(0.0),
            "GetChallengeComponentOption" | "GetCurrentOnlineDifficultyName" => Value::Text(String::new()),
            // Xbox Live friends (online): empty.
            "GetChallengeFriendsLeaderboardData" => Value::Object(vm.array(vec![
                Value::Number(0.0),
                Value::Bool(false),
                Value::Number(-1.0),
                Value::Bool(false),
                Value::Bool(false),
            ])?),
            _ => return Ok(None),
        }))
    }
}
