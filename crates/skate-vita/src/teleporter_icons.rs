//! The teleporters' floating icons (skate-game `visual_indicators.rs`, PR #52 `74c124a`; research b532 to b539): retail's
//! challenge visual indicator over each armed teleporter, a ribbon glyph built from the `teleport` spline, drawn in two
//! textured passes (`vita_render::scene::IconInstance`, from [`Icon`]).
//!
//! Retail, as upstream ported it: the icon stands at the anchor locator + the `ribbon_indicator` row's Offset in the
//! anchor's frame; the style row (`smallarrow`) gives the ribbon width (half width 0.25 x (A + B)), the samples per
//! segment (at most 10), three crossed strips, the fade window on view depth (in from x0 to x1, out from x2 to x3,
//! hidden past x3 on camera distance; x3 x10 on park maps) and the passes' intensity; the nearest 22 within 400 m of
//! the player are kept; a spline of kind bit 0 billboards about its own up along the camera's flattened view axis.
//!
//! Ours: one ribbon (every disc teleporter names the same `teleport` row; another row's icons are skipped), the bake's
//! slim `visual-indicators.json` and its two textures in ui.pak (`vita-bake ui`).
use glam::{Mat3, Mat4, Vec3};
use serde::Deserialize;
use std::collections::BTreeMap;

/// One icon to draw: its model matrix and the pass factors (front rgb = I.x x fade; background rgb = I.y, alpha =
/// sqrt(I.y x fade)), as `vita_render::scene::IconInstance` takes them.
#[derive(Clone, Copy, Debug)]
pub struct Icon {
    pub model: Mat4,
    pub front: f32,
    pub background: [f32; 2],
}

const PATH: &str = "private/visual-indicators.json";
/// Retail caps the samples per spline segment at 10 (`827B0BA8`, b534).
const MAX_SAMPLES: usize = 10;
/// Drawn only within 400 m of the player (`82D4F8A8`), at most 22 (`82D5D478`), nearest first (b538).
const DRAW_DISTANCE: f32 = 400.0;
const MAX_DRAWN: usize = 22;

#[derive(Clone, Debug, Deserialize)]
struct Style {
    width: [f32; 2],
    samples: usize,
    three_strips: bool,
    fade: [f32; 4],
    #[serde(default = "intensity")]
    intensity: [f32; 2],
}

fn intensity() -> [f32; 2] {
    [1.0, 0.7]
}

impl Style {
    /// Opacity at view depth `distance` (`827B1B70`'s fade window, linear; a zero-length ramp is a step).
    fn alpha(&self, distance: f32) -> f32 {
        let [x0, x1, x2, x3] = self.fade;
        if distance > x3 {
            return 0.0;
        }
        let ramp = |from: f32, to: f32| if to > from { ((distance - from) / (to - from)).clamp(0.0, 1.0) } else if distance >= from { 1.0 } else { 0.0 };
        ramp(x0, x1).min(1.0 - if x3 > x2 { ramp(x2, x3) } else { 0.0 })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Spline {
    /// Power-basis cubics, rows c3, c2, c1, c0.
    segments: Vec<[[f32; 3]; 4]>,
    #[serde(default)]
    kind: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct Indicator {
    offset: [f32; 3],
    #[serde(default)]
    spline: Option<String>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    style: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Colour {
    front: String,
    background: String,
}

#[derive(Deserialize)]
struct File {
    format: String,
    version: u32,
    splines: BTreeMap<String, Spline>,
    indicators: BTreeMap<String, Indicator>,
    #[serde(default)]
    styles: BTreeMap<String, Style>,
    #[serde(default)]
    colours: BTreeMap<String, Colour>,
    #[serde(default)]
    textures: Option<String>,
    #[serde(default)]
    park_maps: Vec<String>,
    #[serde(default = "park_fade_scale")]
    park_fade_scale: f32,
}

fn park_fade_scale() -> f32 {
    10.0
}

/// A loaded icon: the ribbon (x, y, z, u, v; indices), its textures (width, height, RGBA8) and how it is placed.
pub struct Icons {
    /// The `ribbon_indicator` row the ribbon is for.
    pub key: String,
    pub vertices: Vec<[f32; 5]>,
    pub indices: Vec<u16>,
    pub front: (u32, u32, Vec<u8>),
    pub background: (u32, u32, Vec<u8>),
    offset: [f32; 3],
    billboard: bool,
    style: Style,
    park_maps: Vec<String>,
    park_fade_scale: f32,
}

fn png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize].to_vec(),
        png::ColorType::Rgb => buf[..(w * h * 3) as usize].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        other => return Err(format!("unexpected colour type {other:?}")),
    };
    Ok((w, h, rgba))
}

/// The spline as retail's strips (`827B0BA8` / `827B2200`, b534): per strip the sampled points with a fixed edge offset
/// each side in the icon's frame; u = the spline parameter in segments, v across the strip.
fn ribbon(spline: &Spline, style: &Style) -> (Vec<[f32; 5]>, Vec<u16>) {
    let samples = style.samples.clamp(1, MAX_SAMPLES);
    let mut points: Vec<(Vec3, f32)> = Vec::new();
    for (s, [c3, c2, c1, c0]) in spline.segments.iter().enumerate() {
        for i in 0..=samples {
            let t = i as f32 / samples as f32;
            let p = |k: usize| c3[k] * t * t * t + c2[k] * t * t + c1[k] * t + c0[k];
            let point = Vec3::new(p(0), p(1), p(2));
            if points.last().is_none_or(|(last, _)| last.distance(point) > 1e-4) {
                points.push((point, s as f32 + t));
            }
        }
    }
    let h = 0.25 * (style.width[0] + style.width[1]);
    let edges: Vec<(Vec3, Vec3)> = if style.three_strips {
        vec![(Vec3::new(0.0, 0.0, -h), Vec3::new(0.0, 0.0, h)), (Vec3::new(0.0, -h, 0.0), Vec3::new(0.0, h, 0.0)), (Vec3::new(-h, 0.0, 0.0), Vec3::new(h, 0.0, 0.0))]
    } else {
        vec![(Vec3::ZERO, Vec3::new(2.0 * h, 2.0 * h, 0.0))]
    };
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    for (a, b) in edges {
        let first = vertices.len() as u16;
        for (i, &(p, t)) in points.iter().enumerate() {
            let (pa, pb) = (p + a, p + b);
            vertices.push([pa.x, pa.y, pa.z, t, 0.0]);
            vertices.push([pb.x, pb.y, pb.z, t, 1.0]);
            if i > 0 {
                let v = first + (i as u16) * 2;
                indices.extend_from_slice(&[v - 2, v - 1, v, v - 1, v + 1, v]);
            }
        }
    }
    (vertices, indices)
}

impl Icons {
    /// The icon of `key` from ui.pak (None: no file, or the row has no spline or textures).
    pub fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>, key: &str) -> Option<Self> {
        let file: File = match read(PATH).and_then(|b| serde_json::from_slice::<File>(&b).map_err(|e| e.to_string())) {
            Ok(f) if f.format == "skate3rust.visual-indicators" && f.version == 1 => f,
            Ok(f) => {
                println!("teleporter icons: {} v{} not supported", f.format, f.version);
                return None;
            }
            Err(e) => {
                println!("teleporter icons: none ({e})");
                return None;
            }
        };
        let indicator = file.indicators.get(key).filter(|i| i.enabled)?;
        let spline = file.splines.get(indicator.spline.as_deref()?)?;
        let style = file.styles.get(indicator.style.as_deref().unwrap_or("default")).or(file.styles.get("default"))?.clone();
        let colour = file.colours.get("default")?;
        let folder = file.textures.as_deref().unwrap_or("visual-indicators");
        let texture = |name: &str| read(&format!("private/{folder}/{name}.png")).and_then(|b| png(&b));
        let (front, background) = match (texture(&colour.front), texture(&colour.background)) {
            (Ok(f), Ok(b)) => (f, b),
            (f, b) => {
                println!("teleporter icons: textures {:?} {:?}", f.err(), b.err());
                return None;
            }
        };
        let (vertices, indices) = ribbon(spline, &style);
        println!("teleporter icons: {key}, {} vertices, {} triangles", vertices.len(), indices.len() / 3);
        Some(Self {
            key: key.to_owned(),
            vertices,
            indices,
            front,
            background,
            offset: indicator.offset,
            billboard: spline.kind & 1 == 1,
            style,
            park_maps: file.park_maps,
            park_fade_scale: file.park_fade_scale,
        })
    }

    /// This frame's icons for the anchors (locator matrices, rows right, up, forward, translation) on `map`, with the
    /// player at `player` and the camera at `camera` (its world matrix: forward = -z).
    pub fn instances(&self, map: &str, anchors: &[[[f32; 4]; 4]], player: Vec3, camera: &Mat4, out: &mut Vec<Icon>) {
        out.clear();
        let mut style = self.style.clone();
        if self.park_maps.iter().any(|m| m.eq_ignore_ascii_case(map)) {
            style.fade[3] *= self.park_fade_scale;
        }
        let eye = camera.w_axis.truncate();
        let forward = -camera.z_axis.truncate().normalize_or_zero();
        let mut near: Vec<(f32, Vec3, Mat3)> = anchors
            .iter()
            .map(|m| {
                let row = |i: usize| Vec3::new(m[i][0], m[i][1], m[i][2]);
                let at = row(3) + row(0) * self.offset[0] + row(1) * self.offset[1] + row(2) * self.offset[2];
                let basis = Mat3::from_cols(row(0).normalize_or_zero(), row(1).normalize_or_zero(), row(2).normalize_or_zero());
                (at.distance(player), at, basis)
            })
            .filter(|(d, ..)| *d < DRAW_DISTANCE)
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, at, basis) in near.into_iter().take(MAX_DRAWN) {
            let to = at - eye;
            let depth = to.dot(forward);
            if depth <= 0.0 || to.length() > style.fade[3] {
                continue;
            }
            let alpha = style.alpha(depth);
            if alpha <= 0.0 {
                continue;
            }
            // `827B1A00`: up kept, forward = the camera's view axis reversed, flattened against up.
            let mut rotation = basis;
            if self.billboard {
                let up = basis.y_axis;
                let back = -forward;
                if let Some(f) = (back - up * back.dot(up)).try_normalize() {
                    rotation = Mat3::from_cols(up.cross(f), up, f);
                }
            }
            let model = Mat4::from_cols(rotation.x_axis.extend(0.0), rotation.y_axis.extend(0.0), rotation.z_axis.extend(0.0), at.extend(1.0));
            let [front, back] = style.intensity;
            out.push(Icon { model, front: front * alpha, background: [back, (back * alpha).max(0.0).sqrt()] });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style { width: [0.2, 0.15], samples: 10, three_strips: true, fade: [4.23, 13.36, 100.0, 100.0], intensity: [1.0, 0.7] }
    }

    /// The smallarrow window: hidden up close, full from 13.4 m, gone past 100 m.
    #[test]
    fn the_fade_window_hides_icons_up_close_and_far() {
        let s = style();
        assert_eq!(s.alpha(2.0), 0.0);
        assert!((s.alpha(8.8) - 0.5).abs() < 0.01);
        assert_eq!(s.alpha(50.0), 1.0);
        assert_eq!(s.alpha(101.0), 0.0);
    }

    /// Three crossed strips over one straight segment: (10 + 1) points x 2 edges per strip.
    #[test]
    fn a_ribbon_has_three_strips() {
        let spline = Spline { segments: vec![[[0.0; 3], [0.0; 3], [1.0, 0.0, 0.0], [0.0; 3]]], kind: 3 };
        let (v, i) = ribbon(&spline, &style());
        assert_eq!(v.len(), 3 * 11 * 2);
        assert_eq!(i.len(), 3 * 10 * 6);
        assert_eq!(v[1][4], 1.0, "v across the strip");
    }
}
