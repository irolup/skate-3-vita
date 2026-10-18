//! Retained movie traversal. All geometry and glyphs come from the owned APT.
use crate::{apt_movie::Movie, apt_vm::{Value, Vm}};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
pub struct Vertex {
    pub position: [f32; 2],
    // skate_vita (M9): untextured fills carry no uv.
    #[serde(default)]
    pub uv: [f32; 2],
}
#[derive(Clone, Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: String,
}
#[derive(Clone, Deserialize)]
pub struct Shape {
    // skate_vita (M9): front-end shapes can be untextured solid fills (None); they are
    // drawn with the texture key `WHITE`, which the renderer provides (1 x 1, white).
    pub texture: Option<Texture>,
    pub triangles: Vec<[Vertex; 3]>,
    pub color: [f32; 4],
}
pub type Shapes = BTreeMap<i32, Vec<Shape>>;
/// skate_vita (M9): the texture key of untextured shapes.
pub const WHITE: &str = "white";
pub struct Draw {
    pub texture: String,
    pub size: [u32; 2],
    pub vertices: Vec<Vertex>,
    pub multiply: [f32; 4],
    pub add: [f32; 4],
}
fn compose(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
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
pub fn draw(movie: &Movie, vm: &Vm, shapes: &Shapes) -> Result<Vec<Draw>, String> {
    let mut draws = Vec::new();
    visit(
        movie,
        vm,
        shapes,
        movie.root,
        [1., 0., 0., 1., 0., 0.],
        [1.; 4],
        [0.; 4],
        &mut draws,
    )?;
    Ok(draws)
}
fn visit(
    movie: &Movie,
    vm: &Vm,
    shapes: &Shapes,
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
    let mut matrix = instance
        .placement
        .as_ref()
        .map_or([1., 0., 0., 1., 0., 0.], |p| p.matrix);
    // skate_vita (P1): a position the scripts computed from a clip's `_width` (which clips do not
    // have) is NaN; such a clip stays at its origin.
    let finite = |v: f64| if v.is_finite() { v as f32 } else { 0.0 };
    matrix[4] = finite(vm.get(id, "_x").number());
    matrix[5] = finite(vm.get(id, "_y").number());
    // skate_vita: `_xscale` / `_yscale` set by a script (percent, negative mirrors), e.g. the
    // trick analyzer's gesture in switch. Unset, the placement's own scale stays.
    for (axis, key) in [(0, "_xscale"), (2, "_yscale")] {
        if let Value::Number(percent) = vm.get(id, key) {
            let length = matrix[axis].hypot(matrix[axis + 1]);
            let (x, y) = if length > 0. { (matrix[axis] / length, matrix[axis + 1] / length) } else if axis == 0 { (1., 0.) } else { (0., 1.) };
            let s = percent as f32 / 100.;
            (matrix[axis], matrix[axis + 1]) = (x * s, y * s);
        }
    }
    let matrix = compose(parent, matrix);
    if matrix.iter().any(|x| !x.is_finite()) {
        return Err(format!(
            "Nonfinite HUD transform on character {}",
            instance.character
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
    if character.type_name == "shape" {
        for shape in shapes
            .get(&character.id)
            .ok_or("Missing original shape geometry")?
        {
            out.push(Draw {
                texture: shape.texture.as_ref().map_or_else(|| WHITE.to_owned(), |t| t.rgba.clone()),
                size: shape.texture.as_ref().map_or([1, 1], |t| [t.width, t.height]),
                vertices: shape
                    .triangles
                    .iter()
                    .flatten()
                    .map(|v| transform(matrix, v))
                    .collect(),
                multiply: std::array::from_fn(|i| multiply[i] * shape.color[i]),
                add,
            });
        }
    } else if let Some(text) = &character.text {
        // A text field in a font with no atlas draws nothing (M9 edit).
        // skate_vita (P1): the font and size may come from the script (`setTextFormat`).
        let (font_id, field_height) = movie.text_assets.field_font(vm, id, text).ok_or("Invalid text font")?;
        let Some(font) = movie.text_assets.fonts.get(&font_id) else {
            return Ok(());
        };
        // Native shadow text uses a black atlas pass followed by the sharp
        // futuraheavy glyphs translated +1 in text X (825D6B68/82CA1FD8).
        let passes = if let Some(foreground) = font.foreground {
            vec![
                (font, true, 0.),
                (&movie.text_assets.fonts[&foreground], false, 1.),
            ]
        } else {
            vec![(font, false, 0.)]
        };
        for (font, shadow, advance_x) in passes {
            let height = field_height;
            let sx = font.scale[0] * height;
            let sy = font.scale[1] * height;
            let value = vm.get(id, "_displayText").text();
            let bounds = character.bounds.ok_or("Missing text bounds")?;
            let alignment = text["alignment"].as_u64().unwrap_or(0);
            let autosize = vm.get(id, "autoSize").text();
            let alignment = if autosize == "left" { 0 } else { alignment };
            // skate_vita (P1): word-wrapped fields (the replay help pages) break lines at the
            // field's width, and `^` is the localized text's line break.
            let lines: Vec<String> = if text["word_wrap"].as_bool().unwrap_or(false) {
                wrap(&value, bounds[2] - bounds[0], |s| font.width(s, height))
            } else {
                vec![value.clone()]
            };
            let mut vertices = Vec::new();
            for (line_index, line) in lines.iter().enumerate() {
            let width = font.width(line, height);
            let mut x = bounds[0]
                + advance_x
                + font.offset[0] * sx
                + match alignment {
                    1 => bounds[2] - bounds[0] - width,
                    2 => (bounds[2] - bounds[0] - width) * 0.5,
                    _ => 0.,
                };
            let y = bounds[1] + font.offset[1] * sy + line_index as f32 * height * 1.25;
            for c in line.chars() {
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
            // skate_vita (P1): a colour the script gave the field (`textColor`, `setTextFormat`).
            let argb = match vm.get(id, "textColor") {
                Value::Number(rgb) if rgb.is_finite() && rgb >= 0.0 => (argb & 0xff00_0000) | (rgb as u32 & 0x00ff_ffff),
                _ => argb,
            };
            let color = [
                ((argb >> 16) & 255) as f32 / 255.,
                ((argb >> 8) & 255) as f32 / 255.,
                (argb & 255) as f32 / 255.,
                (argb >> 24) as f32 / 255.,
            ];
            if !vertices.is_empty() {
                out.push(Draw {
                    texture: font.texture.clone(),
                    size: font.size,
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

/// skate_vita (P1): the size of a clip's drawn content (what Flash's `_width` / `_height` read),
/// from the shapes of its visible descendants, in the clip's own coordinates.
pub fn clip_size(movie: &Movie, vm: &Vm, shapes: &Shapes, id: usize) -> Option<[f32; 2]> {
    fn grow(b: &mut Option<[f32; 4]>, x: f32, y: f32) {
        let r = b.get_or_insert([x, y, x, y]);
        *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
    }
    fn walk(movie: &Movie, vm: &Vm, shapes: &Shapes, id: usize, m: [f32; 6], b: &mut Option<[f32; 4]>, depth: usize) {
        let Some(instance) = movie.instances.get(&id) else { return };
        // The clip itself may be hidden (a menu's spare icons): its size still counts.
        if depth > 12 || depth > 0 && !vm.get(id, "_visible").truth() {
            return;
        }
        let place = instance.placement.as_ref().map_or([1., 0., 0., 1., 0., 0.], |p| p.matrix);
        let mut local = place;
        if depth > 0 {
            local[4] = vm.get(id, "_x").number() as f32;
            local[5] = vm.get(id, "_y").number() as f32;
        }
        let m = if depth > 0 { compose(m, local) } else { m };
        if let Some(list) = movie.characters.get(&instance.character).filter(|c| c.type_name == "shape").and_then(|c| shapes.get(&c.id)) {
            for shape in list {
                for v in shape.triangles.iter().flatten() {
                    let v = transform(m, v);
                    if v.position.iter().all(|x| x.is_finite()) {
                        grow(b, v.position[0], v.position[1]);
                    }
                }
            }
        }
        for child in instance.children.values() {
            walk(movie, vm, shapes, *child, m, b, depth + 1);
        }
    }
    let mut b = None;
    walk(movie, vm, shapes, id, [1., 0., 0., 1., 0., 0.], &mut b, 0);
    b.map(|b| [b[2] - b[0], b[3] - b[1]])
}

/// skate_vita (P1): the visible clips whose `key` field holds text (the button items' `mButtonImage`:
/// the retail game draws the controller glyph there natively), as (text, x, y, width, height) in
/// stage coordinates.
pub fn text_fields(movie: &Movie, vm: &Vm, shapes: &Shapes, key: &str) -> Vec<(String, [f32; 4])> {
    #[allow(clippy::too_many_arguments)]
    fn walk(movie: &Movie, vm: &Vm, shapes: &Shapes, key: &str, id: usize, parent: [f32; 6], alpha: f32, out: &mut Vec<(String, [f32; 4])>, depth: usize) {
        let Some(instance) = movie.instances.get(&id) else { return };
        if depth > 16 || !vm.get(id, "_visible").truth() {
            return;
        }
        let mut m = instance.placement.as_ref().map_or([1., 0., 0., 1., 0., 0.], |p| p.matrix);
        m[4] = vm.get(id, "_x").number() as f32;
        m[5] = vm.get(id, "_y").number() as f32;
        let m = compose(parent, m);
        let alpha = alpha
            * (vm.get(id, "_alpha").number() as f32 / 100.).clamp(0., 1.)
            * instance.placement.as_ref().map_or(1., |p| p.color[0] as f32 / 255.);
        if alpha <= 0. || m.iter().any(|x| !x.is_finite()) {
            return;
        }
        if let Value::Text(text) = vm.get(id, key) {
            if !text.is_empty() {
                let [w, h] = clip_size(movie, vm, shapes, id).unwrap_or([0., 0.]);
                out.push((text, [m[4], m[5], w * m[0].abs(), h * m[3].abs()]));
            }
        }
        for child in instance.children.values() {
            walk(movie, vm, shapes, key, *child, m, alpha, out, depth + 1);
        }
    }
    let mut out = Vec::new();
    walk(movie, vm, shapes, key, movie.root, [1., 0., 0., 1., 0., 0.], 1., &mut out, 0);
    out
}

/// skate_vita (P1): breaks `text` into lines no wider than `limit` at spaces; `^` and newlines
/// force a break (the localized help texts' line break).
fn wrap(text: &str, limit: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split(['^', '\n']) {
        let mut line = String::new();
        for word in paragraph.split(' ').filter(|w| !w.is_empty()) {
            let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if !line.is_empty() && width(&candidate) > limit {
                lines.push(std::mem::take(&mut line));
                line = word.to_owned();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// skate_vita (P1): the stage position of a clip's origin (its `_x` / `_y` composed with its
/// parents'), or None when it is not on the display list.
pub fn world_position(movie: &Movie, vm: &Vm, id: usize) -> Option<[f32; 2]> {
    fn walk(movie: &Movie, vm: &Vm, target: usize, at: usize, parent: [f32; 6], depth: usize) -> Option<[f32; 2]> {
        let instance = movie.instances.get(&at)?;
        if depth > 24 {
            return None;
        }
        let mut m = instance.placement.as_ref().map_or([1., 0., 0., 1., 0., 0.], |p| p.matrix);
        m[4] = vm.get(at, "_x").number() as f32;
        m[5] = vm.get(at, "_y").number() as f32;
        let m = compose(parent, m);
        if at == target {
            return Some([m[4], m[5]]);
        }
        instance.children.values().find_map(|c| walk(movie, vm, target, *c, m, depth + 1))
    }
    walk(movie, vm, id, movie.root, [1., 0., 0., 1., 0., 0.], 0)
}

/// skate_vita (P1): a black quad over the whole 1280 x 720 stage at `alpha` (what the retail
/// game's dimmer does behind a popup).
pub fn dim(alpha: f32) -> Draw {
    let corner = |x: f32, y: f32| Vertex { position: [x, y], uv: [0.0, 0.0] };
    Draw {
        texture: WHITE.to_owned(),
        size: [1, 1],
        vertices: vec![corner(0.0, 0.0), corner(1280.0, 0.0), corner(1280.0, 720.0), corner(0.0, 0.0), corner(1280.0, 720.0), corner(0.0, 720.0)],
        multiply: [0.0, 0.0, 0.0, alpha],
        add: [0.0; 4],
    }
}

/// skate_vita (P1): the button menus lay their icons out by the icons' `_width`, which a clip does
/// not have until measured: sets `_width` and `_height` of every `mButton0..9` clip from its
/// drawn content.
pub fn size_buttons(movie: &Movie, vm: &mut Vm, shapes: &Shapes) -> Result<(), String> {
    let mut found = Vec::new();
    for object in &vm.objects {
        for (key, value) in &object.fields {
            if let Value::Object(child) = value {
                let digit = key.strip_prefix("mButton").is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit());
                if digit && movie.instances.contains_key(child) {
                    found.push(*child);
                }
            }
        }
    }
    for id in found {
        if let Some([w, h]) = clip_size(movie, vm, shapes, id) {
            vm.set(id, "_width", Value::Number(f64::from(w)))?;
            vm.set(id, "_height", Value::Number(f64::from(h)))?;
        }
    }
    Ok(())
}
