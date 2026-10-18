//! skate-game's mod hooks the copied physics calls (upstream PR #52). No mods run on the Vita:
//! the stand-ins return what upstream returns with none loaded.

pub(crate) mod player_physics {
    use crate::physics::SkaterRuntime;

    /// Upstream applies the mods' per-part overrides and returns their undo; with no mod, none.
    pub(crate) struct PartRestore;

    impl PartRestore {
        pub(crate) fn restore(self, _skater: &mut SkaterRuntime) {}
    }

    pub(crate) fn apply_parts(_skater: &mut SkaterRuntime) -> PartRestore {
        PartRestore
    }
}
