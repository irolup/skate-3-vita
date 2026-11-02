//! Snapping (docs/object-dropper-survey.md "Snapping"): a held park piece locks onto the sockets of the park pieces
//! near it, the sockets authored in each piece's model (`Item::sockets`). A socket of group 0 is a point (a frame);
//! the sockets sharing another group are a face (the group's first socket frames it, the others are its corners).
//! A pair qualifies when its sockets face each other and are close; the best puts the held socket exactly on the
//! target socket, facing it, turned to the target's nearest quarter (`82889F58` -> `828710B8` -> `82870208`).
//!
//! Retail's search (`8288A5E0`, jobs `8286F2E0`): the neighbours inside the held piece's box grown by 0.75 m; point
//! to point, point to face, face to point and face to face (`8286E880`, `8286EA48`, `8286F018`); score = weight x
//! distance squared, the weight 0.025 for two sockets of the same `AdvanceSnap` class and 1 otherwise, the lowest
//! under 1.5 wins. Corner mode puts a socket on a face's nearest corner (flush); grid mode slides it on the face in
//! 0.75 m steps, inside the face. When no piece qualifies, the held sockets' normals are probed against the ground
//! (`82870388`).
//!
//! NOT RETAIL YET: the world probe is vertical only (`Ground::below`; retail casts along any socket normal and refines
//! sideways, `828709E8`), the solve runs in the same update as the move (retail one update later), and the placement
//! check on the snapped pose (`82889940`) is not run.
use glam::{Mat3, Quat, Vec2, Vec3};

use super::catalog::Item;

/// The largest socket distance and the starting best score (vault `editor_cursor` `B0C05776CE3E2630`).
pub const MAX_DISTANCE: f32 = 1.5;
/// The facing test: the angle between one socket's z and the other's reversed z (vault `31255295B85ECC39`).
pub const MAX_ANGLE_DEGREES: f32 = 45.0;
/// The score weight of two sockets of the same AdvanceSnap class (0x8208E9D8); 1 otherwise.
pub const SAME_CLASS_WEIGHT: f32 = 0.025;
/// The neighbour box growth, the grid step and the world probe's reach in front (0x821814A0).
pub const STEP: f32 = 0.75;
/// The world probe's start behind the socket (0x820D06C0).
pub const PROBE_BACK: f32 = 0.3;
/// An AdvanceSnap point counts as vertical above this |z.y| (0x820641A8).
const VERTICAL: f32 = 0.1;
/// How far behind a target socket's face a held socket may be and still snap (ours, see `on_face`).
const BEHIND: f32 = 0.05;
/// A face's bounds margin (0x82099920).
const FACE_MARGIN: f32 = 0.005;

/// A socket frame in the world: x, y, z axes (unit) and position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub axes: [Vec3; 3],
    pub position: Vec3,
}

impl Frame {
    fn local(&self, p: Vec3) -> Vec3 {
        let d = p - self.position;
        Vec3::new(d.dot(self.axes[0]), d.dot(self.axes[1]), d.dot(self.axes[2]))
    }

    fn world(&self, l: Vec3) -> Vec3 {
        self.position + self.axes[0] * l.x + self.axes[1] * l.y + self.axes[2] * l.z
    }

    fn moved(&self, position: Vec3) -> Self {
        Self { position, ..*self }
    }

    fn basis(&self) -> Mat3 {
        Mat3::from_cols(self.axes[0], self.axes[1], self.axes[2])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub frame: Frame,
    pub advanced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub frame: Frame,
    /// The corners in the face's plane (its frame's x, y), at most four.
    pub corners: Vec<Vec2>,
    pub min: Vec2,
    pub max: Vec2,
    pub advanced: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sockets {
    pub points: Vec<Point>,
    pub faces: Vec<Face>,
}

/// An item's sockets placed in the world (its template scaled, turned and moved).
pub fn sockets(item: &Item, position: Vec3, rotation: Quat, scale: Vec3) -> Sockets {
    let advanced = item.advance_snap();
    let place = |s: &[[f32; 3]; 4]| Frame {
        axes: std::array::from_fn(|a| (rotation * (Vec3::from_array(s[a]) * scale)).normalize_or_zero()),
        position: position + rotation * (Vec3::from_array(s[3]) * scale),
    };
    let mut out = Sockets::default();
    let mut groups: Vec<(u32, Vec<Frame>)> = Vec::new();
    for (group, s) in &item.sockets {
        let frame = place(s);
        if *group == 0 {
            out.points.push(Point { frame, advanced });
        } else if let Some((_, members)) = groups.iter_mut().find(|(g, _)| g == group) {
            members.push(frame);
        } else {
            groups.push((*group, vec![frame]));
        }
    }
    for (_, members) in groups {
        let frame = members[0];
        let corners: Vec<Vec2> = members.iter().take(4).map(|m| frame.local(m.position).truncate()).collect();
        let min = corners.iter().fold(Vec2::splat(f32::MAX), |a, c| a.min(*c)) - FACE_MARGIN;
        let max = corners.iter().fold(Vec2::splat(f32::MIN), |a, c| a.max(*c)) + FACE_MARGIN;
        out.faces.push(Face { frame, corners, min, max, advanced });
    }
    out
}

/// The sockets' world box (to find the neighbours of a held piece).
pub fn bounds(s: &Sockets) -> Option<(Vec3, Vec3)> {
    let positions = s.points.iter().map(|p| p.frame.position).chain(s.faces.iter().flat_map(|f| f.corners.iter().map(|c| f.frame.world(c.extend(0.0)))));
    positions.fold(None, |b, p| Some(b.map_or((p, p), |(lo, hi): (Vec3, Vec3)| (lo.min(p), hi.max(p)))))
}

/// The best pair: the held socket and where it goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair {
    pub from: Frame,
    pub to: Frame,
    pub score: f32,
}

struct Search {
    best: Option<Pair>,
    score: f32,
    /// Corner mode allowed for this search (retail: the advanced modifier up and the held piece not AdvanceSnap).
    corner: bool,
    cos: f32,
}

impl Search {
    fn new(corner: bool) -> Self {
        Self { best: None, score: MAX_DISTANCE, corner, cos: MAX_ANGLE_DEGREES.to_radians().cos() }
    }

    /// `8286F258`: the from z against the reversed target z, under the angle.
    fn facing(&self, from: &Frame, to: &Frame) -> bool {
        from.axes[2].dot(-to.axes[2]) > self.cos
    }

    fn weight(a: bool, b: bool) -> f32 {
        if a == b {
            SAME_CLASS_WEIGHT
        } else {
            1.0
        }
    }

    fn accept(&mut self, from: Frame, to: Frame, w: f32) {
        let d = from.position.distance(to.position);
        let score = w * d * d;
        if score < self.score && d < MAX_DISTANCE {
            self.score = score;
            self.best = Some(Pair { from, to, score });
        }
    }

    /// `8286EA48`: point `p` onto face `f`; the face position it would take, or None.
    fn on_face(&self, p: Vec3, f: &Face, corner: bool, w: f32) -> Option<Vec3> {
        let l = f.frame.local(p);
        // NOT RETAIL (owner, device): a socket behind the face (under a top, inside the piece) does not snap to it;
        // retail compares |z| (8286EA48), so a piece under another's top was pulled up onto it.
        if l.z < -BEHIND {
            return None;
        }
        if !(w * l.z * l.z < self.score && l.z.abs() < MAX_DISTANCE) {
            return None;
        }
        let at = if corner {
            let c = f.corners.iter().min_by(|a, b| a.distance_squared(l.truncate()).total_cmp(&b.distance_squared(l.truncate())))?;
            (w * c.distance_squared(l.truncate()) < self.score).then_some(*c)?
        } else {
            let g = ((l.truncate() / STEP + 0.5).floor()) * STEP;
            (g.cmpgt(f.min).all() && g.cmplt(f.max).all()).then_some(g)?
        };
        Some(f.frame.world(at.extend(0.0)))
    }

    fn point_point(&mut self, p: &Point, t: &Point) {
        if self.facing(&p.frame, &t.frame) && t.frame.local(p.frame.position).z >= -BEHIND {
            self.accept(p.frame, t.frame, Self::weight(p.advanced, t.advanced));
        }
    }

    fn point_face(&mut self, p: &Point, f: &Face) {
        let vertical = p.advanced && p.frame.axes[2].y.abs() > VERTICAL;
        if (vertical && f.advanced) || !self.facing(&p.frame, &f.frame) {
            return;
        }
        let w = Self::weight(p.advanced, f.advanced);
        if let Some(at) = self.on_face(p.frame.position, f, self.corner && !f.advanced, w) {
            self.accept(p.frame, f.frame.moved(at), w);
        }
    }

    fn face_point(&mut self, g: &Face, t: &Point) {
        let vertical = t.advanced && t.frame.axes[2].y.abs() > VERTICAL;
        if (vertical && g.advanced) || !self.facing(&g.frame, &t.frame) {
            return;
        }
        let w = Self::weight(g.advanced, t.advanced);
        if let Some(at) = self.on_face(t.frame.position, g, self.corner && !t.advanced, w) {
            self.accept(g.frame.moved(at), t.frame, w);
        }
    }

    fn face_face(&mut self, g: &Face, f: &Face) {
        if !self.facing(&g.frame, &f.frame) {
            return;
        }
        let w = Self::weight(g.advanced, f.advanced);
        let corner = self.corner && !f.advanced;
        // Corner mode: G's corners onto F's corners; grid mode: a 0.75 m raster over G onto F's grid (`8286EE98`).
        let points: Vec<Vec2> = if corner {
            g.corners.clone()
        } else {
            let mut out = Vec::new();
            let mut y = g.min.y + FACE_MARGIN;
            while y <= g.max.y {
                let mut x = g.min.x + FACE_MARGIN;
                while x <= g.max.x {
                    out.push(Vec2::new(x, y));
                    x += STEP;
                }
                y += STEP;
            }
            out
        };
        for c in points {
            let from = g.frame.moved(g.frame.world(c.extend(0.0)));
            if let Some(at) = self.on_face(from.position, f, corner, w) {
                self.accept(from, f.frame.moved(at), w);
            }
        }
    }

    fn run(&mut self, from: &Sockets, to: &Sockets) {
        for p in &from.points {
            for t in &to.points {
                self.point_point(p, t);
            }
            for f in &to.faces {
                self.point_face(p, f);
            }
        }
        for g in &from.faces {
            for t in &to.points {
                self.face_point(g, t);
            }
            for f in &to.faces {
                self.face_face(g, f);
            }
        }
    }
}

/// The best pair between the held piece's sockets and its neighbours' (`advanced`: the advanced modifier, LT, held;
/// `held_advanced`: the held piece is AdvanceSnap). None when nothing qualifies, or for an AdvanceSnap piece with
/// the modifier held (free placement).
pub fn best_pair(from: &Sockets, neighbours: &Sockets, advanced: bool, held_advanced: bool) -> Option<Pair> {
    if held_advanced && advanced {
        return None;
    }
    let mut search = Search::new(!advanced && !held_advanced);
    search.run(from, neighbours);
    search.best
}

/// World targets under the held sockets that face down (`82870388`, vertical only here): a probe from 0.3 m above to
/// 0.75 m below each, the ground hit framed by an up normal.
pub fn ground_targets(from: &Sockets, below: impl Fn(Vec3) -> Option<f32>) -> Sockets {
    let mut out = Sockets::default();
    let mut probe = |frame: &Frame| {
        let n = frame.axes[2];
        if n.y > -0.7 {
            return;
        }
        let p = frame.position;
        if let Some(h) = below(p + Vec3::Y * PROBE_BACK).filter(|h| p.y - h <= STEP) {
            let x = frame.axes[0];
            // A right-handed frame (as every model socket is): z up, y = z x x.
            out.points.push(Point { frame: Frame { axes: [x, Vec3::Y.cross(x).normalize_or_zero(), Vec3::Y], position: Vec3::new(p.x, h, p.z) }, advanced: false });
        }
    };
    for p in &from.points {
        probe(&p.frame);
    }
    for f in &from.faces {
        for c in &f.corners {
            probe(&f.frame.moved(f.frame.world(c.extend(0.0))));
        }
    }
    out
}

/// `82870208` / `828710B8`: the held piece's pose that puts `pair.from` on `pair.to`, facing it, turned to the
/// target's nearest quarter. `base`: the held piece's origin pose (position, rotation).
pub fn snapped_pose(pair: &Pair, base: (Vec3, Quat)) -> (Vec3, Quat) {
    let t = pair.to;
    let z = -t.axes[2];
    let from_y = pair.from.axes[1];
    let y = [t.axes[0], -t.axes[0], t.axes[1], -t.axes[1]].into_iter().max_by(|a, b| a.dot(from_y).total_cmp(&b.dot(from_y))).unwrap_or(t.axes[1]);
    let target = Frame { axes: [y.cross(z), y, z], position: t.position };
    let turn = target.basis() * pair.from.basis().transpose();
    let rotation = Quat::from_mat3(&(turn * Mat3::from_quat(base.1))).normalize();
    let position = target.position + turn * (base.0 - pair.from.position);
    (position, rotation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object_dropper::catalog::{Cost, Item, Part};

    /// A 6 x 6 x 1.5 m pad with retail's six faces (corners on the box, normals out), like `aa_padStreet`.
    fn pad(advanced: bool) -> Item {
        let mut sockets = Vec::new();
        let (lo, hi) = (Vec3::new(0.0, 0.0, -6.0), Vec3::new(6.0, 1.5, 0.0));
        let faces: [(Vec3, Vec3, Vec3, [Vec3; 4]); 6] = [
            (Vec3::X, Vec3::NEG_Z, Vec3::NEG_Y, [Vec3::new(6.0, 0.0, 0.0), Vec3::new(6.0, 0.0, -6.0), Vec3::new(0.0, 0.0, -6.0), Vec3::new(0.0, 0.0, 0.0)]),
            (Vec3::Z, Vec3::Y, Vec3::NEG_X, [Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -6.0), Vec3::new(0.0, 1.5, -6.0), Vec3::new(0.0, 1.5, 0.0)]),
            (Vec3::NEG_Z, Vec3::Y, Vec3::X, [Vec3::new(6.0, 0.0, 0.0), Vec3::new(6.0, 0.0, -6.0), Vec3::new(6.0, 1.5, -6.0), Vec3::new(6.0, 1.5, 0.0)]),
            (Vec3::X, Vec3::Y, Vec3::Z, [Vec3::new(0.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0), Vec3::new(6.0, 1.5, 0.0), Vec3::new(0.0, 1.5, 0.0)]),
            (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z, [Vec3::new(0.0, 0.0, -6.0), Vec3::new(6.0, 0.0, -6.0), Vec3::new(6.0, 1.5, -6.0), Vec3::new(0.0, 1.5, -6.0)]),
            (Vec3::X, Vec3::Z, Vec3::Y, [Vec3::new(0.0, 1.5, 0.0), Vec3::new(6.0, 1.5, 0.0), Vec3::new(6.0, 1.5, -6.0), Vec3::new(0.0, 1.5, -6.0)]),
        ];
        for (g, (x, _, z, corners)) in faces.iter().enumerate() {
            // Right-handed, as the model sockets (y = z x x; the bottom face: x, +z, -y).
            let y = z.cross(*x);
            for c in corners {
                sockets.push(((g as u32 + 1) << 24, [x.to_array(), y.to_array(), z.to_array(), c.to_array()]));
            }
        }
        let mut caps = std::collections::BTreeMap::new();
        if advanced {
            caps.insert("AdvanceSnap".to_owned(), "True".to_owned());
        }
        Item {
            id: "PAD".into(),
            name: "aa_padStreet_md_Downtown".into(),
            part: Part::Terrain,
            sub: "street".into(),
            template: Some("PAD".into()),
            teleporter: false,
            marker: None,
            teleport: None,
            caps,
            cost: Cost { bounds: [lo.to_array(), hi.to_array()], ..Cost::default() },
            sockets,
        }
    }

    #[test]
    fn a_pad_near_another_locks_flush_against_it() {
        let item = pad(false);
        let placed = sockets(&item, Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        // The held pad 0.4 m right of the placed one's +x side, 0.2 m further back, a little turned.
        let base = (Vec3::new(6.4, 0.0, -0.2), Quat::from_rotation_y(0.1));
        let held = sockets(&item, base.0, base.1, Vec3::ONE);
        let pair = best_pair(&held, &placed, false, false).expect("a pair");
        let (position, rotation) = snapped_pose(&pair, base);
        // Flush: its -x side on the placed +x side, corner on corner, square to it.
        assert!((position - Vec3::new(6.0, 0.0, 0.0)).length() < 1e-4, "{position}");
        assert!(rotation.angle_between(Quat::IDENTITY) < 1e-4, "{rotation}");
        // Too far: nothing.
        let far = sockets(&item, Vec3::new(8.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE);
        assert!(best_pair(&far, &placed, false, false).is_none());
    }

    #[test]
    fn grid_mode_slides_on_the_face_in_steps() {
        let item = pad(false);
        let placed = sockets(&item, Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        // On top of the placed pad, 0.3 m up and 1.1 m along x: the advanced modifier (grid mode) keeps the 0.75 step.
        let base = (Vec3::new(1.1, 1.8, 0.0), Quat::IDENTITY);
        let held = sockets(&item, base.0, base.1, Vec3::ONE);
        let pair = best_pair(&held, &placed, true, false).expect("a pair");
        let (position, _) = snapped_pose(&pair, base);
        assert!((position.y - 1.5).abs() < 1e-4, "{position}");
        assert!(((position.x / STEP) - (position.x / STEP).round()).abs() < 1e-3, "{position}");
        // An AdvanceSnap piece with the modifier held does not snap.
        assert!(best_pair(&sockets(&pad(true), base.0, base.1, Vec3::ONE), &placed, true, true).is_none());
    }

    #[test]
    fn a_piece_below_another_top_is_not_pulled_up_onto_it() {
        let item = pad(false);
        let placed = sockets(&item, Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        // Held on the ground inside the placed pad's footprint, its base 1.5 m under that pad's top: no snap up.
        let held = sockets(&item, Vec3::new(1.0, 0.0, -1.0), Quat::IDENTITY, Vec3::ONE);
        let pair = best_pair(&held, &placed, false, false);
        assert!(pair.is_none_or(|p| (snapped_pose(&p, (Vec3::new(1.0, 0.0, -1.0), Quat::IDENTITY)).0.y - 1.5).abs() > 0.1), "{pair:?}");
        // Held just above the top: it locks onto it.
        let above = sockets(&item, Vec3::new(1.0, 1.8, -1.0), Quat::IDENTITY, Vec3::ONE);
        let pair = best_pair(&above, &placed, false, false).expect("onto the top");
        assert!((snapped_pose(&pair, (Vec3::new(1.0, 1.8, -1.0), Quat::IDENTITY)).0.y - 1.5).abs() < 1e-4);
    }

    #[test]
    fn a_piece_just_above_the_ground_settles_on_it() {
        let item = pad(false);
        let base = (Vec3::new(3.0, 0.4, 2.0), Quat::IDENTITY);
        let held = sockets(&item, base.0, base.1, Vec3::ONE);
        let ground = ground_targets(&held, |_| Some(0.0));
        let pair = best_pair(&held, &ground, false, false).expect("the ground");
        let (position, _) = snapped_pose(&pair, base);
        assert!((position - Vec3::new(3.0, 0.0, 2.0)).length() < 1e-4, "{position}");
        // Out of reach (more than 0.75 m up): nothing.
        let high = sockets(&item, Vec3::new(3.0, 1.0, 2.0), Quat::IDENTITY, Vec3::ONE);
        assert!(ground_targets(&high, |_| Some(0.0)).points.is_empty());
    }
}
