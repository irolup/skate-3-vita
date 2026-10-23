//! Retained APT depth list. Placement flags update individual properties;
//! omitted properties on a move retain their previous values.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
pub struct Control {
    pub label: Option<String>,
    pub type_name: String,
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub depth: i32,
    pub character_id: Option<i32>,
    pub matrix: Option<[f32; 6]>,
    pub color_transform: Option<[u8; 8]>,
    pub ratio: Option<f32>,
    pub name: Option<String>,
    pub clip_depth: Option<i32>,
    pub blend_mode: Option<i32>,
    #[serde(default)]
    pub filter_pointer: u32,
    #[serde(default)]
    pub actions_offset: u32,
    /// Decoded clip-event table of a placement with flag 0x80 (doc 31, Milestone 3d).
    #[serde(default)]
    pub clip_actions: Vec<ClipAction>,
}

/// One onClipEvent handler of a placement (retail [TU3]: placement+60 points at
/// `{u32 count, u32 events}`, each event 12 bytes `{u32 flags, u32 key_code, u32 actions}`;
/// sub_82E7B340 passes it on, sub_82E7B090 stores it at display object +24).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ClipAction {
    pub flags: u32,
    // skate_vita: the M9 linked screens (`tools/prepare_ui_screen.py`: the HUD meters) name them `key` / `offset`.
    #[serde(default, alias = "key")]
    pub key_code: u32,
    #[serde(alias = "offset")]
    pub actions_offset: u32,
}

/// Clip-event flag bits, the SWF ClipEventFlags values (retail [TU3]: placement sub_82E7AF18
/// dispatches 0x200 and 0x40000, the tick sub_82E5B640 dispatches 2 and 1, removal
/// sub_82E55DA8 dispatches 4; the dynamic-handler table 0x82FC9A3C maps 1 / 2 / 4 / 0x40 /
/// 0x80 / 0x100 to onLoad / onEnterFrame / onUnload / onKeyDown / onKeyUp / onData).
// Input-driven events (mouse, key, press...) are dispatched by the host's input later.
#[allow(dead_code)]
pub mod clip_event {
    pub const LOAD: u32 = 0x1;
    pub const ENTER_FRAME: u32 = 0x2;
    pub const UNLOAD: u32 = 0x4;
    pub const MOUSE_MOVE: u32 = 0x8;
    pub const MOUSE_DOWN: u32 = 0x10;
    pub const MOUSE_UP: u32 = 0x20;
    pub const KEY_DOWN: u32 = 0x40;
    pub const KEY_UP: u32 = 0x80;
    pub const DATA: u32 = 0x100;
    pub const INITIALIZE: u32 = 0x200;
    pub const PRESS: u32 = 0x400;
    pub const RELEASE: u32 = 0x800;
    pub const RELEASE_OUTSIDE: u32 = 0x1000;
    pub const ROLL_OVER: u32 = 0x2000;
    pub const ROLL_OUT: u32 = 0x4000;
    pub const DRAG_OVER: u32 = 0x8000;
    pub const DRAG_OUT: u32 = 0x1_0000;
    pub const KEY_PRESS: u32 = 0x2_0000;
    pub const CONSTRUCT: u32 = 0x4_0000;
}

/// Most clip events per placement we accept (NOT RETAIL, safety limit for mod movies).
pub const MAX_CLIP_ACTIONS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub character: i32,
    pub matrix: [f32; 6],
    pub color: [u8; 8],
    pub ratio: f32,
    pub name: String,
    /// Clip depth (-1 = none). Retail [TU3] stores it but its renderer never reads it: the
    /// only reader is the bounds walk sub_82E59C28, which leaves out children with a clip
    /// depth >= 0. So a clip-depth shape is drawn like any other shape and masks nothing
    /// (doc 31, Milestone 3f); apt_scene.rs draws it the same way.
    pub clip_depth: i32,
    pub blend_mode: i32,
    pub clip_actions: Vec<ClipAction>,
}

impl Placement {
    /// skate_vita (performance): `*self = other.clone()` without reallocating the name and the event table when they
    /// are the same.
    pub fn assign(&mut self, other: &Placement) {
        self.character = other.character;
        self.matrix = other.matrix;
        self.color = other.color;
        self.ratio = other.ratio;
        self.clip_depth = other.clip_depth;
        self.blend_mode = other.blend_mode;
        if self.name != other.name {
            self.name.clone_from(&other.name);
        }
        if self.clip_actions != other.clip_actions {
            self.clip_actions.clone_from(&other.clip_actions);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    pub depths: BTreeMap<i32, Placement>,
}
impl DisplayList {
    pub fn apply(&mut self, control: &Control) -> Result<(), String> {
        match control.type_name.as_str() {
            "remove_object2" | "remove_object3" => {
                self.depths.remove(&control.depth);
            }
            "place_object2" | "place_object3" => {
                if control.filter_pointer != 0 {
                    return Err("APT placement filter is not implemented".into());
                }
                if control.flags & 0x80 != 0
                    && control.actions_offset != 0
                    && control.clip_actions.is_empty()
                {
                    // Movie exported before Milestone 3d: the table was not decoded.
                    return Err("APT placement clip actions were not exported".into());
                }
                if control.clip_actions.len() > MAX_CLIP_ACTIONS {
                    return Err("APT placement has too many clip actions".into());
                }
                let mut placement = if control.flags & 1 != 0 {
                    self.depths.get(&control.depth).cloned().ok_or_else(|| {
                        format!("APT move references empty depth {}", control.depth)
                    })?
                } else {
                    Placement {
                        character: control
                            .character_id
                            .filter(|_| control.flags & 2 != 0)
                            .ok_or("APT new placement lacks character")?,
                        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                        color: [255, 255, 255, 255, 0, 0, 0, 0],
                        ratio: 0.0,
                        name: String::new(),
                        clip_depth: -1,
                        blend_mode: -1,
                        clip_actions: Vec::new(),
                    }
                };
                if control.flags & 2 != 0 {
                    placement.character = control
                        .character_id
                        .ok_or("APT character flag lacks value")?;
                }
                if control.flags & 4 != 0 {
                    placement.matrix = control.matrix.ok_or("APT matrix flag lacks value")?;
                }
                if control.flags & 8 != 0 {
                    placement.color = control
                        .color_transform
                        .ok_or("APT color flag lacks value")?;
                }
                if control.flags & 0x10 != 0 {
                    placement.ratio = control.ratio.ok_or("APT ratio flag lacks value")?;
                }
                if control.flags & 0x20 != 0 {
                    placement.name = control.name.clone().ok_or("APT name flag lacks value")?;
                }
                // Retail [TU3] (doc 31, Milestone 3f): sub_82E7B340 hands the record's clip
                // depth (+56) to every placement that creates a display object (flag 2) without
                // testing flag 0x40, and sub_82E7A950 stores it as a 16-bit value (sth props+22).
                // A move of an existing object passes -1 and sub_82E7B090 keeps the stored value,
                // so flag 0x40 on a move changes nothing.
                if control.flags & 2 != 0 {
                    placement.clip_depth = control.clip_depth.map_or(-1, |d| d as i16 as i32);
                }
                // Retail sub_82E7B090 stores the table only when the placement carries one
                // (flag 0x80), on a new placement and on a move alike.
                if control.flags & 0x80 != 0 {
                    placement.clip_actions = control.clip_actions.clone();
                }
                if control.type_name == "place_object3" {
                    placement.blend_mode = control.blend_mode.unwrap_or(-1);
                }
                self.depths.insert(control.depth, placement);
            }
            "do_action" | "do_init_action" | "frame_label" | "background_color" => {}
            kind => return Err(format!("Unsupported APT control {kind}")),
        }
        Ok(())
    }
}
