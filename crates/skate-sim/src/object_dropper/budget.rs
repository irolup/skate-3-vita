//! The placement budgets and the Feng Shui meter (retail 8288F1B8 / 8288EE08 / 8288EF58 with the three budgets
//! 8288E430, 8288EA18, 8288EBE8; docs/object-dropper-survey.md "Limits").
//!
//! Retail's rule, kept as it is: a selection may be added only when its memory, props and terrain budgets all
//! pass, each `used + added <= 1` where a fraction is the larger of the budget's parts; the meter shows terrain (the
//! memory fraction, terrain cost, terrain pieces) and props (memory, dropped props, live props), and warns above 0.75
//! of either. The Vita's units: memory in bytes (the items' vertices and textures, and the collision a placed terrain
//! piece adds to the world), terrain cost in collision triangles.
//!
//! Device measurement (31 Oct, MegaPark): a placed terrain piece's collision costs about 360 bytes a triangle of heap
//! (the props layer's triangles, index and instance), and the heap had about 68 MiB free with the editor open; 110
//! stacked Mega ramps (880 collision triangles each, 66 drawn) took 35 MiB and a 56 MiB growth failed. The stock
//! parks' own collision: DownTown 56k triangles, StartPark 94k, Industrial 128k, MegaPark 198k.
use super::catalog::{Catalog, Item, Part};

/// The limits. Retail's counts (47 dropped props, 48 live objects, 748 pieces) are kept; the cost and memory limits
/// are the Vita's own (D6d).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Bytes of the items' render data (vertices and the textures they use) and of the placed terrain pieces'
    /// collision (retail main and GPU memory, one here).
    pub memory: u64,
    pub props_dropped: u32,
    pub props_live: u32,
    /// Collision triangles of the terrain pieces (retail 200,000 mesh commands).
    pub terrain_cost: u64,
    pub terrain_pieces: u32,
}

impl Default for Limits {
    fn default() -> Self {
        // NOT RETAIL: the Vita's measured limits (D6d). Memory 20 MiB: about 50k placed collision triangles beside the
        // render data (57 Mega ramps), inside the heap left with the editor open even while the props layer's
        // triangle list grows (it needs twice its size then). Terrain cost 300k collision triangles: MegaPark's
        // Fullpark (198k) at 66%, under the meter's warning.
        Self { memory: 20 << 20, props_dropped: 47, props_live: 48, terrain_cost: 300_000, terrain_pieces: 748 }
    }
}

impl Limits {
    /// The measuring limits (D6d, the debug overlay on): four times the memory and terrain placeholders, so a device
    /// run can go past them and show where the Vita slows down; the retail prop counts stay.
    pub fn measure() -> Self {
        let d = Self::default();
        Self { memory: d.memory * 4, terrain_cost: d.terrain_cost * 4, terrain_pieces: d.terrain_pieces * 4, ..d }
    }
}

/// What the world holds, or what a selection would add.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub memory: u64,
    pub props_dropped: u32,
    pub props_live: u32,
    pub terrain_cost: u64,
    pub terrain_pieces: u32,
}

/// Heap a placed terrain piece's collision takes per triangle (device measurement, D6d).
pub const PLACED_COLLISION_BYTES: u64 = 360;

impl std::ops::Add for Usage {
    type Output = Usage;
    fn add(self, o: Usage) -> Usage {
        Usage {
            memory: self.memory + o.memory,
            props_dropped: self.props_dropped + o.props_dropped,
            props_live: self.props_live + o.props_live,
            terrain_cost: self.terrain_cost + o.terrain_cost,
            terrain_pieces: self.terrain_pieces + o.terrain_pieces,
        }
    }
}

impl Usage {
    /// Placed items' usage: as [`Usage::of`], and each terrain piece's collision in memory (a park's own pieces have
    /// theirs baked in its world; a placed one adds it).
    pub fn placed<'a>(catalog: &Catalog, items: impl IntoIterator<Item = &'a Item>) -> Self {
        let items: Vec<&Item> = items.into_iter().collect();
        let mut usage = Self::of(catalog, items.iter().copied());
        usage.memory += items.iter().filter(|i| i.part == Part::Terrain).map(|i| u64::from(i.cost.collision) * PLACED_COLLISION_BYTES).sum::<u64>();
        usage
    }

    /// Items' usage: each item's vertices, its textures once among all of them, its count and cost in its part.
    pub fn of<'a>(catalog: &Catalog, items: impl IntoIterator<Item = &'a Item>) -> Self {
        let mut usage = Self::default();
        let mut textures = std::collections::BTreeSet::new();
        for item in items {
            usage.memory += u64::from(item.cost.vertex_bytes);
            for &t in &item.cost.textures {
                if textures.insert((item.part, t)) {
                    usage.memory += u64::from(catalog.texture_bytes.get(&item.part).and_then(|b| b.get(usize::from(t))).copied().unwrap_or(0));
                }
            }
            match item.part {
                Part::Props => {
                    usage.props_dropped += 1;
                    usage.props_live += 1;
                }
                Part::Terrain => {
                    usage.terrain_cost += u64::from(item.cost.collision);
                    usage.terrain_pieces += 1;
                }
            }
        }
        usage
    }
}

fn fraction(value: u64, limit: u64) -> f32 {
    if limit == 0 { f32::INFINITY } else { value as f32 / limit as f32 }
}

/// The props budget's fraction (8288EB40 / 8288F1B8): the larger of dropped / 47 and live / 48.
fn props(limits: &Limits, dropped: u32, live: u32) -> f32 {
    fraction(u64::from(dropped), u64::from(limits.props_dropped)).max(fraction(u64::from(live), u64::from(limits.props_live)))
}

/// The terrain budget's fraction (8288ED40): the larger of cost / limit and pieces / 748.
fn terrain(limits: &Limits, cost: u64, pieces: u32) -> f32 {
    fraction(cost, limits.terrain_cost).max(fraction(u64::from(pieces), u64::from(limits.terrain_pieces)))
}

/// 8288EF58: may `added` join `used`? All three budgets must pass.
pub fn allows(limits: &Limits, used: &Usage, added: &Usage) -> bool {
    let memory = fraction(used.memory, limits.memory) + fraction(added.memory, limits.memory) <= 1.0;
    let props = props(limits, added.props_dropped, added.props_live) + props(limits, used.props_dropped, used.props_live) <= 1.0;
    let terrain = terrain(limits, added.terrain_cost, added.terrain_pieces) + terrain(limits, used.terrain_cost, used.terrain_pieces) <= 1.0;
    memory && props && terrain
}

/// The Feng Shui meter (the dropper's +0x173C..+0x174C, filled by 8288F1B8).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meter {
    /// Terrain bar: used, and what the selection would add.
    pub terrain: [f32; 2],
    /// Props ("Smackable") bar: used, and what the selection would add.
    pub props: [f32; 2],
    /// Either bar past 0.75 (0x821814A0) with the selection: the screen's "Warning!".
    pub warning: bool,
}

/// The bar colour steps (825119C8): from 0.75 the bar turns, from 0.9 (0x820997B8) it is full and the warning plays.
pub const METER_AMBER: f32 = 0.75;
pub const METER_RED: f32 = 0.9;

pub fn meter(limits: &Limits, used: &Usage, added: &Usage) -> Meter {
    let memory_used = fraction(used.memory, limits.memory);
    let memory_added = fraction(added.memory, limits.memory);
    let terrain = [
        memory_used.max(terrain(limits, used.terrain_cost, used.terrain_pieces)),
        memory_added.max(terrain(limits, added.terrain_cost, added.terrain_pieces)),
    ];
    let props = [
        memory_used.max(props(limits, used.props_dropped, used.props_live)),
        memory_added.max(props(limits, added.props_dropped, added.props_live)),
    ];
    let warning = terrain[0] + terrain[1] > METER_AMBER || props[0] + props[1] > METER_AMBER;
    Meter { terrain, props, warning }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_budgets_and_the_meter_follow_retail() {
        let limits = Limits { memory: 1000, props_dropped: 47, props_live: 48, terrain_cost: 1000, terrain_pieces: 748 };
        let none = Usage::default();
        let one_prop = Usage { memory: 10, props_dropped: 1, props_live: 1, ..none };
        // 46 dropped props: one more fits (47 / 47 + ...), 47 do not take another.
        let used = Usage { props_dropped: 46, props_live: 46, ..none };
        assert!(allows(&limits, &used, &one_prop));
        let full = Usage { props_dropped: 47, props_live: 47, ..none };
        assert!(!allows(&limits, &full, &one_prop));
        // Terrain: cost or pieces, whichever is fuller.
        let pieces = Usage { terrain_cost: 100, terrain_pieces: 748, ..none };
        assert!(!allows(&limits, &pieces, &Usage { terrain_cost: 1, terrain_pieces: 1, ..none }));
        // Memory alone refuses too.
        assert!(!allows(&limits, &Usage { memory: 995, ..none }, &one_prop));
        // The meter: memory shows in both bars; the warning past 0.75.
        let m = meter(&limits, &Usage { memory: 500, terrain_cost: 100, terrain_pieces: 10, ..none }, &none);
        assert_eq!(m.terrain[0], 0.5);
        assert_eq!(m.props[0], 0.5);
        assert!(!m.warning);
        let m = meter(&limits, &Usage { terrain_cost: 700, ..none }, &Usage { terrain_cost: 100, ..none });
        assert!(m.warning);
    }
}
