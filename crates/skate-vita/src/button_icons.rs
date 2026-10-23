//! The Vita's button icons for the glyph slots the retail screens leave to the host (the retail
//! game draws the Xbox glyphs natively): anti-aliased RGBA textures made here, drawn through the
//! movie renderer in the slots' place. Original art (circles and lines), nothing from Skate 3.
#![cfg_attr(not(target_os = "vita"), allow(dead_code))]
use skate_ui::fe::apt_scene::{Draw, Vertex};

/// Texture side in pixels.
pub const SIZE: u32 = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Cross,
    Circle,
    Square,
    Triangle,
    /// A stick: the disc alone (the host writes L or R on it).
    Stick,
}

impl Icon {
    pub const ALL: [Icon; 5] = [Icon::Cross, Icon::Circle, Icon::Square, Icon::Triangle, Icon::Stick];

    pub fn key(self) -> &'static str {
        match self {
            Icon::Cross => "vita-icon/cross",
            Icon::Circle => "vita-icon/circle",
            Icon::Square => "vita-icon/square",
            Icon::Triangle => "vita-icon/triangle",
            Icon::Stick => "vita-icon/stick",
        }
    }

    /// The icon of a retail glyph name (A, B, X, Y are the Vita's Cross, Circle, Square, Triangle).
    pub fn of(name: &str) -> Option<Icon> {
        Some(match name {
            "A" => Icon::Cross,
            "B" => Icon::Circle,
            "X" => Icon::Square,
            "Y" => Icon::Triangle,
            "Stick_Left_hud" | "Stick_Right_hud" => Icon::Stick,
            _ => return None,
        })
    }
}

fn distance_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let t = (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
    let (dx, dy) = (p[0] - a[0] - t * abx, p[1] - a[1] - t * aby);
    (dx * dx + dy * dy).sqrt()
}

/// Coverage of a shape whose edge is at signed distance `d` (negative inside), one pixel of blur.
fn coverage(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

fn over(dst: [f32; 4], src: [f32; 3], alpha: f32) -> [f32; 4] {
    let a = alpha + dst[3] * (1.0 - alpha);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mix = |s: f32, d: f32| (s * alpha + d * dst[3] * (1.0 - alpha)) / a;
    [mix(src[0], dst[0]), mix(src[1], dst[1]), mix(src[2], dst[2]), a]
}

/// The icon as straight-alpha RGBA, `SIZE` x `SIZE`: a dark disc with a light rim and the symbol.
pub fn pixels(icon: Icon) -> Vec<u8> {
    let s = SIZE as f32;
    let c = s / 2.0;
    let radius = s / 2.0 - 2.0;
    let arm = radius * 0.46;
    let stroke = s * 0.055;
    let (colour, segments): ([f32; 3], Vec<([f32; 2], [f32; 2])>) = match icon {
        Icon::Cross => ([0.55, 0.75, 1.0], vec![([c - arm, c - arm], [c + arm, c + arm]), ([c - arm, c + arm], [c + arm, c - arm])]),
        Icon::Square => (
            [1.0, 0.62, 0.88],
            vec![
                ([c - arm, c - arm], [c + arm, c - arm]),
                ([c + arm, c - arm], [c + arm, c + arm]),
                ([c + arm, c + arm], [c - arm, c + arm]),
                ([c - arm, c + arm], [c - arm, c - arm]),
            ],
        ),
        Icon::Triangle => {
            let (top, low) = ([c, c - arm * 1.05], c + arm * 0.75);
            ([0.5, 0.95, 0.65], vec![(top, [c + arm * 1.05, low]), ([c + arm * 1.05, low], [c - arm * 1.05, low]), ([c - arm * 1.05, low], top)])
        }
        Icon::Circle | Icon::Stick => ([1.0, 0.45, 0.5], Vec::new()),
    };
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let r = ((p[0] - c).powi(2) + (p[1] - c).powi(2)).sqrt();
            let mut px = [0.0f32; 4];
            px = over(px, [0.07, 0.09, 0.13], coverage(r - radius) * 0.92);
            px = over(px, [0.86, 0.88, 0.92], coverage((r - (radius - stroke * 0.5)).abs() - stroke * 0.5));
            let symbol = match icon {
                Icon::Circle => coverage((r - arm).abs() - stroke * 0.6),
                Icon::Stick => 0.0,
                _ => segments.iter().map(|(a, b)| coverage(distance_to_segment(p, *a, *b) - stroke * 0.6)).fold(0.0, f32::max),
            };
            px = over(px, colour, symbol);
            out.extend(px.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }
    out
}

/// The draw of an icon over a glyph slot: centred on (x, y) in the 1280 x 720 stage, `side` wide.
pub fn draw(icon: Icon, x: f32, y: f32, side: f32) -> Draw {
    let h = side / 2.0;
    let v = |px: f32, py: f32, u: f32, w: f32| Vertex { position: [px, py], uv: [u, w] };
    Draw {
        texture: icon.key().to_owned(),
        fill: skate_ui::fe::apt_scene::Fill::TextureClamped,
        vertices: vec![
            v(x - h, y - h, 0.0, 0.0),
            v(x + h, y - h, 1.0, 0.0),
            v(x + h, y + h, 1.0, 1.0),
            v(x - h, y - h, 0.0, 0.0),
            v(x + h, y + h, 1.0, 1.0),
            v(x - h, y + h, 0.0, 1.0),
        ],
        multiply: [1.0; 4],
        add: [0.0; 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes the icons as raw RGBA next to each other (ICONS_OUT=path) to look at them.
    #[test]
    #[ignore]
    fn dump() {
        let Ok(path) = std::env::var("ICONS_OUT") else { return };
        let mut out = Vec::new();
        for icon in Icon::ALL {
            out.extend(pixels(icon));
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn icons_are_round_and_transparent_outside() {
        for icon in Icon::ALL {
            let p = pixels(icon);
            assert_eq!(p.len(), (SIZE * SIZE * 4) as usize);
            // A corner is clear; the centre of the rim's left edge is opaque.
            assert_eq!(p[3], 0);
            let row = SIZE / 2;
            let at = ((row * SIZE + 2) * 4 + 3) as usize;
            assert!(p[at] > 200, "{icon:?}");
        }
    }
}
