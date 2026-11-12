//! Board path 82C05EC0 in the ground post-physics (82D387A8), only while the AI record steers (Processed 1776 bit 31).
//! Details: docs/hails-additions/26e-living-world-skater-ai.md#board-path-game-side

use skate_core::math::{Basis3, Vector3};
use skate_core::physics::board::BodyId;
use skate_core::physics::drive_frames::RetailAffineTransform;
use skate_core::player::state::PhysicalStateId;
use skate_core::riding::grounded::state::board_path::{self, BoardPathServices, PhysicsAiTuning, SteerTarget};

/// `physics_ai` `default` from the setup collections (field hashes = layout +0..+20).
pub(crate) fn load_physics_ai(data: &skate_data::collections::Collections) -> Result<PhysicsAiTuning, String> {
    let f = |field: &str| data.float("physics_ai", "default", field).or_else(|_| data.float("Hash_527C93F55CFC663D", "default", field));
    Ok(PhysicsAiTuning {
        velocity_max_change: f("Hash_789DB7452B1D4849")?,
        velocity_gain: f("Hash_B94905DC80B9AF3A")?,
        position_max_step: f("Hash_9C4EE757D0E6595D")?,
        position_gain: f("Hash_0CDADEBEB684D3FC")?,
        facing_max_degrees: f("Hash_FF6F2622D3F9B765")?,
        facing_gain: f("Hash_D3214727731FBA60")?,
    })
}

struct Services<'a> {
    board: &'a mut skate_core::physics::board_runtime::BoardRuntime,
    ground: Basis3,
}

impl BoardPathServices for Services<'_> {
    fn deck_transform(&self) -> RetailAffineTransform {
        self.board.part_transforms()[BodyId::Deck.index()]
    }
    fn set_deck_transform(&mut self, transform: RetailAffineTransform) {
        self.board.set_single_part_transform(BodyId::Deck, transform);
    }
    fn deck_velocity(&self) -> Vector3 {
        self.board.bodies()[BodyId::Deck.index()].rates.linear_velocity
    }
    fn set_deck_velocity(&mut self, velocity: Vector3) {
        self.board.bodies_mut()[BodyId::Deck.index()].rates.linear_velocity = velocity;
    }
    fn ground_frame(&self) -> Basis3 {
        self.ground
    }
    fn set_board_transform(&mut self, transform: RetailAffineTransform) {
        self.board.set_transform(transform);
    }
}

/// After the ground wipeout check: `82C05EC0` for PhysicsGround / SlideGround while the record
/// steers. Returns whether it ran.
pub(super) fn post_physics(physics: &mut super::GamePhysics, skater: &super::SkaterRuntime) -> bool {
    if !matches!(skater.player_state.current(), PhysicalStateId::PhysicsGround | PhysicalStateId::SlideGround) {
        return false;
    }
    let record = &skater.player_input.processed.external_physics_1616;
    if record.flags & board_path::flags::STEER == 0 {
        return false;
    }
    let g = physics.riding.reckoning_frames.ground;
    let ground = Basis3 { columns: [[g[0][0], g[0][1], g[0][2]], [g[1][0], g[1][1], g[1][2]], [g[2][0], g[2][1], g[2][2]]] };
    let tuning = physics.settings.physics_ai;
    let mut services = Services { board: &mut physics.board, ground };
    let target = SteerTarget::from_words(&record.vectors, record.flags);
    let before = services.deck_velocity();
    board_path::update_board_path(&target, &tuning, &mut services);
    // NPC_BOARD_PULL (research b817): the velocity pull against its target while the deck is well off it; the next
    // line's `before` shows how much of the pull the board solve kept.
    if target.flags & board_path::flags::VELOCITY != 0 && super::log_actor() == "npc" {
        let len = |v: Vector3| (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        let (before, after, goal) = (len(before), len(services.deck_velocity()), len(target.velocity));
        if (before - goal).abs() > 1.5 {
            tracing::info!("NPC_BOARD_PULL flags {:08X} target {goal:.2} before {before:.2} after {after:.2} m/s", record.flags);
        }
    }
    true
}
