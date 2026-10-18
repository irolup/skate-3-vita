//! The static collision world as the physics step sees it: the geometry shared behind an `Arc`, plus this step's own
//! query scratch. The player's `GamePhysics` and every simulated NPC skater's worker hold a handle to the same world,
//! so their steps can run on separate threads (a query only reads the world). Reads go through `Deref`; the few
//! writes (map load, prop streaming, tests) need the world not to be shared at that moment.
use skate_core::physics::board_runtime::BoardRuntime;
use skate_core::physics::board_step::BoardCollision;
use skate_core::physics::board_world::{BoardWorld, BoardWorldVolume, ContactRetentionSettings, QueryScratch, WheelWorldSettings};
use skate_core::physics::collision::WorldContactSettings;
use std::sync::Arc;

pub(crate) struct WorldHandle {
    shared: Arc<BoardWorld>,
    scratch: Box<QueryScratch>,
}

impl From<BoardWorld> for WorldHandle {
    fn from(world: BoardWorld) -> Self {
        Self { shared: Arc::new(world), scratch: Box::default() }
    }
}

impl WorldHandle {
    /// A handle to the same world with fresh scratch (a worker for another skater's step).
    pub(crate) fn share(&self) -> Self {
        Self { shared: Arc::clone(&self.shared), scratch: Box::default() }
    }

    /// Point at `other`'s world again, keeping this handle's scratch.
    pub(crate) fn reshare(&mut self, other: &WorldHandle) {
        self.shared = Arc::clone(&other.shared);
    }

    /// Drop the share (an empty world), so the owner can change the shared one.
    pub(crate) fn unshare(&mut self) {
        static EMPTY: std::sync::OnceLock<Arc<BoardWorld>> = std::sync::OnceLock::new();
        self.shared = Arc::clone(EMPTY.get_or_init(|| Arc::new(BoardWorld::new(Vec::new()))));
    }

    pub(crate) fn query_primitives(
        &mut self,
        volumes: &[BoardWorldVolume],
        query: WorldContactSettings,
        retention: ContactRetentionSettings,
    ) -> &[BoardCollision] {
        self.shared.query_primitives_in(&mut self.scratch, volumes, query, retention)
    }

    pub(crate) fn query(&mut self, board: &BoardRuntime, settings: WheelWorldSettings) -> &[BoardCollision] {
        let volumes = BoardWorld::wheel_volumes(board, settings);
        self.shared.query_primitives_in(&mut self.scratch, &volumes, settings.query, settings.retention)
    }

    pub(crate) fn contacts(&self) -> &[BoardCollision] {
        self.scratch.scratch_contacts()
    }

    pub(crate) fn dropped_contacts(&self) -> u32 {
        self.scratch.scratch_dropped_contacts()
    }
}

impl std::ops::Deref for WorldHandle {
    type Target = BoardWorld;
    fn deref(&self) -> &BoardWorld {
        &self.shared
    }
}

impl std::ops::DerefMut for WorldHandle {
    fn deref_mut(&mut self) -> &mut BoardWorld {
        Arc::get_mut(&mut self.shared).expect("the collision world is changed only while no NPC worker shares it")
    }
}
