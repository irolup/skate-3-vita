//! Retained movie traversal. All geometry and glyphs come from the owned APT.
//!
//! Shapes are the retail GEO units (pre-tessellated, one render type each; doc 31 "Milestone 4"):
//! `solid` units draw their unit colour, `texture_clamped` / `texture_wrapped` units sample the
//! bitmap character the unit names (GEO unit +0x14 -> bitmap character -> texture resource) with
//! UVs from the unit's UV matrix (GEO unit +0x18, divided by the texture size at export). The
//! texture a unit draws with comes from a `ShapeSource`, so menus can resolve imported shapes and
//! mod-supplied bitmaps by stable id while the HUD keeps its own table.
use crate::fe::{apt_movie::Movie, apt_vm::Vm};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
pub struct Vertex {
    pub position: [f32; 2],
    /// Absent on solid units (no texture).
    #[serde(default)]
    pub uv: [f32; 2],
}
#[derive(Clone, Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: String,
}
/// GEO unit render type (unit +0x00): 0 line, 1 solid, 2 texture clamped, 3 texture wrapped.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    Line,
    Solid,
    #[default]
    TextureClamped,
    TextureWrapped,
}
impl Fill {
    pub fn textured(self) -> bool {
        matches!(self, Self::TextureClamped | Self::TextureWrapped)
    }
}
#[derive(Clone, Deserialize)]
pub struct Shape {
    /// None on solid / line units.
    #[serde(default)]
    pub texture: Option<Texture>,
    pub triangles: Vec<[Vertex; 3]>,
    pub color: [f32; 4],
    #[serde(default, rename = "render_type")]
    pub fill: Fill,
    /// Bitmap character the unit samples (GEO unit +0x14), in the movie that defines the shape.
    #[serde(default)]
    pub bitmap: Option<i32>,
}
pub type Shapes = BTreeMap<i32, Vec<Shape>>;
/// Where the scene finds a shape character's units and the texture each unit draws with.
pub trait ShapeSource {
    fn shape(&self, character: i32) -> Option<&[Shape]>;
    /// Texture key for a unit ("" = untextured); defaults to the exported retail RGBA payload.
    fn texture(&self, _character: i32, shape: &Shape) -> String {
        shape.texture.as_ref().map_or_else(String::new, |t| t.rgba.clone())
    }
    /// Retail unit colour rule (825D5580: unit colour * multiply + add as one colour, skip at
    /// alpha <= 0). The HUD keeps its earlier `texel * multiply + add` until it is checked in game.
    fn retail_unit_colour(&self) -> bool {
        false
    }
}
impl ShapeSource for Shapes {
    fn shape(&self, character: i32) -> Option<&[Shape]> {
        self.get(&character).map(Vec::as_slice)
    }
}
pub struct Draw {
    /// Texture key ("" for solid units: draw the colour only).
    pub texture: String,
    pub vertices: Vec<Vertex>,
    pub multiply: [f32; 4],
    pub add: [f32; 4],
    /// Shapes: the unit's render type (clamp or wrap addressing); text draws are `TextureClamped`.
    pub fill: Fill,
}
pub fn compose(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn transform(m: [f32; 6], v: &Vertex) -> Vertex {
    Vertex {
        position: [
            m[0] * v.position[0] + m[2] * v.position[1] + m[4],
            m[1] * v.position[0] + m[3] * v.position[1] + m[5],
        ],
        uv: v.uv,
    }
}
pub fn draw(movie: &Movie, vm: &Vm, shapes: &dyn ShapeSource) -> Result<Vec<Draw>, String> {
    draw_from(movie, vm, shapes, IDENTITY, [1.; 4], [0.; 4])
}
pub const IDENTITY: [f32; 6] = [1., 0., 0., 1., 0., 0.];
/// Draws a movie whose root sits under `parent` (a movie loaded into a clip of another movie on the
/// front-end stage), with the parent's colour transform.
pub fn draw_from(
    movie: &Movie,
    vm: &Vm,
    shapes: &dyn ShapeSource,
    parent: [f32; 6],
    multiply: [f32; 4],
    add: [f32; 4],
) -> Result<Vec<Draw>, String> {
    let mut draws = Vec::new();
    visit(movie, vm, shapes, movie.root, parent, multiply, add, &mut draws)?;
    Ok(draws)
}
fn visit(
    movie: &Movie,
    vm: &Vm,
    shapes: &dyn ShapeSource,
    id: usize,
    parent: [f32; 6],
    pm: [f32; 4],
    pa: [f32; 4],
    out: &mut Vec<Draw>,
) -> Result<(), String> {
    if !vm.get(id, "_visible").truth() {
        return Ok(());
    }
    let instance = &movie.instances[&id];
    let matrix = compose(parent, crate::fe::apt_movie::clip_matrix(movie, vm, id));
    if matrix.iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "Nonfinite HUD transform on character {} ({} _x {} _y {})",
            instance.character,
            instance.placement.as_ref().map_or("", |p| p.name.as_str()),
            vm.get(id, "_x").text(),
            vm.get(id, "_y").text()
        ));
    }
    let color = instance
        .placement
        .as_ref()
        .map_or([255, 255, 255, 255, 0, 0, 0, 0], |p| p.color);
    let rgba = [1, 2, 3, 0];
    let mut multiply = std::array::from_fn(|i| pm[i] * color[rgba[i]] as f32 / 255.);
    let mut add = std::array::from_fn(|i| pa[i] + pm[i] * color[4 + rgba[i]] as f32 / 255.);
    let alpha = (vm.get(id, "_alpha").number() as f32 / 100.).clamp(0., 1.);
    multiply[3] *= alpha;
    add[3] *= alpha;
    if multiply[3] <= 0. && add[3] <= 0. {
        return Ok(());
    }
    let character = &movie.characters[&instance.character];
    // A clip whose `_type` names a native component is drawn by the player in place of its placeholder shapes
    // (research `.local/research/menu-white-squares.md`). CustomButton (button_item2's mButtonRender, `_test` = the
    // retail button "A" / "B" / ...): one quad over the placeholder's bounds with texture `prompt/<button>`, which
    // the host resolves to the connected pad's prompt (SDL button labels). Other components draw nothing yet.
    if let crate::fe::apt_vm::Value::Text(kind) = vm.get(id, "_type") {
        // Every retail button name resolves to the connected pad's glyph (prompt_glyphs); "Unknown" = an unused item.
        let button = vm.get(id, "_test").text();
        // ImageRenderComponent (research b258 section 3, retail 825E2028): the slot is the last run of decimal digits in
        // the clip's name (its Init renames it to the reserved index), below 40; the host resolves `slot/<n>` to the
        // slot's image, stretched over the placeholder (the Challenge Map never locks the aspect).
        let texture = match kind.as_str() {
            "CustomButton" if crate::fe::prompt_glyphs::known(&button) => Some(format!("prompt/{button}")),
            "ImageRenderComponent" => last_digits(&vm.get(id, "_name").text())
                .filter(|slot| *slot < crate::fe::image_renderer_host::SLOTS)
                .map(|slot| format!("{}{slot}", crate::fe::image_renderer_host::SLOT_TEXTURE_PREFIX)),
            _ => None,
        };
        // MapRenderComponent (b262 / b263, retail 825E2618 -> 8268D898): the host draws the map markers in the clip's
        // own space, centred on its origin; this placeholder carries the clip transform (the images of (0, 0), (1, 0)
        // and (0, 1)) for the stage to expand (`MAP_MARKERS`).
        // CustomTimeline (the replay editor's strip, mCustomRender; retail dispatcher 825D4828 -> 82509B80, b403): drawn
        // by the host in the clip's own space (x = normalised time x 817); the placeholder carries the full clip
        // transform for the stage to expand (`REPLAY_TIMELINE`).
        if kind == "CustomTimeline" {
            let point = |x: f32, y: f32| transform(matrix, &Vertex { position: [x, y], uv: [0., 0.] });
            out.push(Draw { texture: REPLAY_TIMELINE.to_owned(), fill: Fill::TextureClamped, vertices: vec![point(0., 0.), point(1., 0.), point(0., 1.)], multiply, add });
        }
        if kind == "MapRenderComponent" {
            let point = |x: f32, y: f32| transform(matrix, &Vertex { position: [x, y], uv: [0., 0.] });
            out.push(Draw { texture: MAP_MARKERS.to_owned(), fill: Fill::TextureClamped, vertices: vec![point(0., 0.), point(1., 0.), point(0., 1.)], multiply, add });
        }
        if let Some(texture) = texture {
            if let Some(b) = movie.character_bounds(instance.character, 0) {
                let corner = |x: f32, y: f32, u: f32, v: f32| transform(matrix, &Vertex { position: [x, y], uv: [u, v] });
                let (a, bb, c, d) =
                    (corner(b[0], b[1], 0., 0.), corner(b[2], b[1], 1., 0.), corner(b[2], b[3], 1., 1.), corner(b[0], b[3], 0., 1.));
                out.push(Draw {
                    texture,
                    fill: Fill::TextureClamped,
                    vertices: vec![a.clone(), bb, c.clone(), a, c, d],
                    multiply,
                    add,
                });
            }
        }
        if !kind.is_empty() {
            return Ok(());
        }
    }
    if character.type_name == "shape" {
        for shape in shapes
            .shape(character.id)
            .ok_or("Missing original shape geometry")?
        {
            // Retail sub_825D5DC8 builds draw objects for unit types 1, 2 and 3 only.
            if shape.fill == Fill::Line {
                continue;
            }
            let (unit_multiply, unit_add) = if shapes.retail_unit_colour() {
                // Retail draw 825D5580: colour = unit colour * cxform multiply + add, one colour
                // for the whole unit (TheSimpleDraw sub_82805368); nothing drawn when its alpha <= 0.
                let colour: [f32; 4] = std::array::from_fn(|i| shape.color[i].mul_add(multiply[i], add[i]));
                if colour[3] <= 0. {
                    continue;
                }
                (colour, [0.; 4])
            } else {
                (std::array::from_fn(|i| multiply[i] * shape.color[i]), add)
            };
            out.push(Draw {
                texture: shapes.texture(character.id, shape),
                fill: shape.fill,
                vertices: shape
                    .triangles
                    .iter()
                    .flatten()
                    .map(|v| transform(matrix, v))
                    .collect(),
                multiply: unit_multiply,
                add: unit_add,
            });
        }
    } else if let Some(text) = &character.text {
        let font = &movie.text_assets.fonts
            [&(text["font_id"].as_i64().ok_or("Invalid text font")? as i32)];
        // Native shadow text uses a black atlas pass followed by the sharp
        // futuraheavy glyphs translated +1 in text X (825D6B68/82CA1FD8).
        let passes = if let Some(foreground) = &font.foreground {
            vec![
                (font, true, 0.),
                (&**foreground, false, 1.),
            ]
        } else {
            vec![(font, false, 0.)]
        };
        for (font, shadow, advance_x) in passes {
            let height = text["font_height"].as_f64().ok_or("Invalid text height")? as f32;
            let sx = font.scale[0] * height;
            let sy = font.scale[1] * height;
            let value = vm.get(id, "_displayText").text();
            let bounds = character.bounds.ok_or("Missing text bounds")?;
            let alignment = text["alignment"].as_u64().unwrap_or(0);
            let autosize = vm.get(id, "autoSize").text();
            let alignment = if autosize == "left" { 0 } else { alignment };
            // Word-wrapped fields break into lines (82CA14D0), each aligned on its own and drawn one line advance
            // below the last, unclipped (82CA2118). NOT RETAIL YET: the wrap flag (descriptor +28) is taken as the
            // field's word_wrap (the descriptor fill is not decoded; the trick guide sets word_wrap and multiline
            // together).
            let lines = if text["word_wrap"].as_bool() == Some(true) || vm.get(id, "wordWrap") == crate::fe::apt_vm::Value::Bool(true) {
                font.wrap(&value, height, bounds[2] - bounds[0])
            } else {
                vec![value]
            };
            let mut vertices = Vec::new();
            for (row, value) in lines.iter().enumerate() {
            let width = font.width(value, height);
            // 82CA23B4: 1 right, 2 and 3 centred (factor 0.5 at 0x8209975C), else left.
            let mut x = bounds[0]
                + advance_x
                + font.offset[0] * sx
                + match alignment {
                    1 => bounds[2] - bounds[0] - width,
                    2 | 3 => (bounds[2] - bounds[0] - width) * 0.5,
                    _ => 0.,
                };
            let y = bounds[1] + font.offset[1] * sy + row as f32 * font.line_advance(height);
            for c in value.chars() {
                if let Some(g) = font.glyph(c) {
                    let x0 = x + g.x_offset * sx;
                    let y0 = y + (font.ascent - g.y_offset) * sy;
                    let x1 = x0 + g.width * sx;
                    let y1 = y0 + g.height * sy;
                    let [u0, v0, u1, v1] = g.atlas_bounds;
                    let points = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                    let uvs = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
                    for i in [0, 1, 2, 0, 2, 3] {
                        vertices.push(transform(
                            matrix,
                            &Vertex {
                                position: points[i],
                                uv: [
                                    uvs[i][0] / font.size[0] as f32,
                                    uvs[i][1] / font.size[1] as f32,
                                ],
                            },
                        ));
                    }
                    x = g.x_advance.mul_add(sx, x);
                }
            }
            }
            let argb = u32::from_str_radix(
                text["color_argb"]
                    .as_str()
                    .ok_or("Missing text color")?
                    .trim_start_matches('#'),
                16,
            )
            .map_err(|e| e.to_string())?;
            // `TextField.textColor` (0xRRGGBB) set by the movie's script replaces the field's authored colour; the
            // alpha stays the field's. The settings rows set it per state in SetState (highlight_selector /
            // highlight_slider / menu_part_text_hilite: selected 0x37A5FF, rows on / off / disabled); without it the
            // selected value ("< Normal >", "100%") drew in its dark authored colour (play test, 2026-10-12).
            let rgb = match vm.get(id, "textColor") {
                crate::fe::apt_vm::Value::Undefined => argb & 0xFF_FFFF,
                v => crate::fe::apt_vm::to_integer(&v) as u32 & 0xFF_FFFF,
            };
            let color = [
                ((rgb >> 16) & 255) as f32 / 255.,
                ((rgb >> 8) & 255) as f32 / 255.,
                (rgb & 255) as f32 / 255.,
                (argb >> 24) as f32 / 255.,
            ];
            if !vertices.is_empty() {
                out.push(Draw {
                    texture: font.texture.clone(),
                    fill: Fill::TextureClamped,
                    vertices,
                    multiply: std::array::from_fn(|i| {
                        if shadow && i < 3 {
                            0.
                        } else {
                            multiply[i] * color[i]
                        }
                    }),
                    add,
                });
            }
        }
    }
    for child in instance.children.values() {
        visit(movie, vm, shapes, *child, matrix, multiply, add, out)?;
    }
    Ok(())
}

/// The last run of decimal digits in a clip name (`mRenderComponent7` -> 7), as retail 825E2028 reads the slot.
fn last_digits(name: &str) -> Option<usize> {
    let end = name.rfind(|c: char| c.is_ascii_digit())? + 1;
    let start = name[..end].rfind(|c: char| !c.is_ascii_digit()).map_or(0, |i| i + 1);
    name[start..end].parse().ok()
}

/// Texture key of the MapRenderComponent placeholder draw (three transformed points, expanded by the host).
pub(crate) const MAP_MARKERS: &str = "map-markers/";
/// The replay editor timeline placeholder (`CustomTimeline`), expanded by the stage from the replay screen's feed.
pub(crate) const REPLAY_TIMELINE: &str = "replay-timeline/";
