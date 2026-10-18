//! TU3 actor checkpoint manager, bound to the single-player static scene.
use super::{GamePhysics, SkaterRuntime, offboard::contact_queries, teleport_state::Checkpoint};
use skate_core::{
    math::Vector3,
    physics::{
        board_world::{BoardWorld, query_metadata::Bounds},
        skeleton_animation_record::AnimationPartTransform as Matrix,
    },
    player::{
        input_phase::BoundaryContact,
        respawn::{Candidate, Ground, History, Observation, Validation, surface_allowed},
    },
};
use skate_data::collections::Collections;
use super::offboard::contact_queries::Probe;

pub(super) struct Runtime {
    history: History,
    settings: Settings,
    measurements: i32,
    /// Why the next checkpoint reply happens (first request since the last reply wins).
    pending: Option<Pending>,
    /// Completed respawns, drained into [`PlayerRespawn`] messages after the physics tick.
    pub(super) outbox: Vec<PlayerRespawn>,
    /// Completed placements (teleports of any kind) of this skater, retail place-skater counter
    /// owner +1864 (sub_825926F8). Read by `skater_ghost` to restart the fade-in.
    pub(crate) placements: u32,
    /// An AI skater's respawn destination (its path's chosen node, retail `sub_8246ED68`), used instead of the
    /// checkpoint history by the next wipeout-end teleport. Set by the NPC skater simulation when a bail starts.
    pub(crate) destination: Option<Checkpoint>,
}

/// Stable id of the local player in [`PlayerRespawn`] (single-player scene).
pub(crate) const LOCAL_PLAYER_ID: u32 = 0;

/// Why the skater was sent to the checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RespawnReason {
    /// `CalcSuggestedState` air count passed the limit (retail 300 ticks): fell with no ground.
    AirTimeout,
    /// Wipeout auto reset (`physics_wipeout` Teleport* times), output byte 69.
    WipeoutAutoReset,
    /// Touched a type-6 `physics_unrideable` surface (ProcessOutput 82DB8120: board/feet/plant
    /// contact via 82DB80C8, or the ragdoll SkeletonCollision+214 branch), e.g. Industrial's
    /// invisible sea floor at y -4.0 (surface 768).
    Boundary,
    /// Any other teleport request reaching the checkpoint manager (flags 2468 bit 1 / 2472 bit 18).
    Requested,
    /// Reserved for type-12 water. The Industrial sea reset is the type-6 floor under the water
    /// ([`Self::Boundary`]); no type-12 reset path is decoded, so nothing emits this.
    #[allow(dead_code)]
    Water,
}

impl RespawnReason {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::AirTimeout => "air_timeout",
            Self::WipeoutAutoReset => "wipeout_auto_reset",
            Self::Boundary => "boundary",
            Self::Requested => "requested",
            Self::Water => "water",
        }
    }
}

/// One checkpoint respawn of a player, emitted by the owning simulation on its fixed 1/60 s
/// tick (serialisable, stable ids; no networking).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PlayerRespawn {
    pub player_id: u32,
    /// Physics tick of the checkpoint reply.
    pub tick: u64,
    pub reason: RespawnReason,
    /// Checkpoint position (m, world).
    pub checkpoint: [f32; 3],
    /// Checkpoint heading (forward axis).
    pub heading: [f32; 3],
    pub on_board: bool,
    /// Air frames counted when an `AirTimeout` request was made (0 otherwise).
    pub air_frames: i32,
}

/// Respawn tuning a host setting or a mod (`sdk.world.set_tuning('respawn', ...)`) changes.
/// `default()` = retail (mod disable). Pushed into the live selector before each physics tick,
/// so a map load (new skater) keeps it. The wipeout auto-reset times stay vault data
/// (`physics_wipeout` Teleport*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RespawnSettings {
    /// Air ticks (1/60 s) before the checkpoint teleport request; retail 300 (5 s).
    pub air_timeout_ticks: i32,
}

impl Default for RespawnSettings {
    fn default() -> Self {
        Self { air_timeout_ticks: skate_core::player::selector::RETAIL_AIR_TIMEOUT_FRAMES }
    }
}



#[derive(Clone, Copy, Debug)]
struct Pending {
    reason: RespawnReason,
    air_frames: i32,
    position: [f32; 3],
    /// Branch that raised a [`RespawnReason::Boundary`] request.
    contact: Option<BoundaryContact>,
}
struct Settings {
    height: f32,
    radius: f32,
    drop: f32,
    normal_y: f32,
    minimum_frames: u32,
}
impl Runtime {
    pub fn load(data: &Collections, transform: Matrix, stance: u32) -> Result<Self, String> {
        let class = "Hash_12B64C0E804B0853";
        let value = |hash| data.float(class, "default", hash);
        Ok(Self {
            history: History::new(Candidate {
                transform,
                stance,
                offboard: false,
                score: 0.,
            }),
            measurements: 0,
            pending: None,
            outbox: Vec::new(),
            placements: 0,
            destination: None,
            settings: Settings {
                height: value("Hash_C0526C883AF0ECCA")?,
                radius: value("Hash_CEB092E418A5B001")?,
                drop: value("Hash_8ABE098D3806D273")?,
                normal_y: value("Hash_ADD032CACF6A1C15")?,
                minimum_frames: data.integer(class, "default", "Hash_10B7C3A9CC8D3721")? as u32,
            },
        })
    }
    pub fn reset_measurements(&mut self) {
        self.measurements = 0;
    }
    /// Record why a teleport was requested; the first request before the reply is kept.
    pub fn note_request(&mut self, reason: RespawnReason, air_frames: i32, position: [f32; 3]) {
        if self.pending.is_none() {
            self.pending = Some(Pending { reason, air_frames, position, contact: None });
        }
    }
    /// Record a type-6 `physics_unrideable` teleport request (same first-request rule).
    pub fn note_boundary(&mut self, contact: BoundaryContact, position: [f32; 3]) {
        if self.pending.is_none() {
            self.pending = Some(Pending {
                reason: RespawnReason::Boundary,
                air_frames: 0,
                position,
                contact: Some(contact),
            });
        }
    }
    /// Tag the checkpoint reply with the pending reason (default `requested`), log it and queue
    /// the [`PlayerRespawn`] message. `position` = skater root, used when nothing was noted.
    fn complete(&mut self, tick: u64, candidate: &Candidate, position: [f32; 3]) {
        let pending = self.pending.take().unwrap_or(Pending {
            reason: RespawnReason::Requested,
            air_frames: 0,
            contact: None,
            position,
        });
        let checkpoint: [f32; 3] = candidate.transform[3][..3].try_into().unwrap();
        if pending.reason == RespawnReason::AirTimeout {
            tracing::info!(
                "AIR_TIMEOUT_RESPAWN air_frames={} position={:?} checkpoint={:?} on_board={}",
                pending.air_frames,
                pending.position,
                checkpoint,
                !candidate.offboard
            );
        }
        let boundary = pending.contact.map_or(String::new(), |c| {
            let surface = c.packed_surface.map_or("none".to_owned(), |s| s.to_string());
            format!(" state={} surface={surface} from={:?}", c.state, pending.position)
        });
        tracing::info!(
            "BAIL_CHECKPOINT reason={}{boundary} position={:?} heading={:?} stance={} offboard={} score={}",
            pending.reason.name(),
            candidate.transform[3],
            candidate.transform[2],
            candidate.stance,
            candidate.offboard,
            candidate.score
        );
        self.outbox.push(PlayerRespawn {
            player_id: LOCAL_PLAYER_ID,
            tick,
            reason: pending.reason,
            checkpoint,
            heading: candidate.transform[2][..3].try_into().unwrap(),
            on_board: !candidate.offboard,
            air_frames: if pending.reason == RespawnReason::AirTimeout { pending.air_frames } else { 0 },
        });
    }
}

///Actor82592518 saves stance then queues the ordinary deferred reply825926F8.
pub(super) fn request(physics: &GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    // An AI skater respawns at its path node, through the same actor reply (one reset).
    if let Some(destination) = skater.respawn.destination.take() {
        skater.teleport_state.reply(destination);
        return Ok(());
    }
    let stance = skater.animation.checkpoint_stance();
    let runtime = &mut skater.respawn;
    let candidate = runtime.history.automatic(
        stance,
        &mut Scene {
            world: &physics.world,
            window: physics.window.as_ref(),
            aux: None,
            settings: &runtime.settings,
        },
    )?;
    skater.animation.request_checkpoint_stance(candidate.stance);
    skater.teleport_state.reply(Checkpoint {
        transform: candidate.transform,
        on_board: !candidate.offboard,
    });
    tracing::info!(
        "BAIL_CHECKPOINT position={:?} heading={:?} stance={} offboard={} score={}",
        candidate.transform[3],
        candidate.transform[2],
        candidate.stance,
        candidate.offboard,
        candidate.score
    );
    let position = skater.animated_skeleton.roots.animation_to_world[3][..3].try_into().unwrap();
    skater.respawn.complete(physics.ticks, &candidate, position);
    Ok(())
}

///One completed physical output, including the empirical conditioner sample.
pub(super) fn observe(physics: &GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    let physical = &skater.player_input.physical;
    let processed = &skater.player_input.processed;
    //82592A00 -> SkateboardReckoning64 ->82C01BF8: solved deck, with bit20 flip.
    let mut deck = super::solve::deck_frame(&physics.board);
    if processed.flags_2468 & 0x0010_0000 != 0 {
        flip(&mut deck);
    }
    deck[3][1] += 0.2;
    let mut offboard = skater.animated_skeleton.roots.animation_to_world;
    if processed.flags_2476 & 4 != 0 {
        flip(&mut offboard);
    }
    let velocity = physical.skateboard.vector_80.map(f32::from_bits);
    let stance = skater.animation.checkpoint_stance();
    let runtime = &mut skater.respawn;
    runtime.measurements = runtime.measurements.wrapping_add(1);
    let observation = Observation {
        measurements: runtime.measurements,
        root_position: skater.animated_skeleton.roots.animation_to_world[3],
        com_position: physical.reckoning.vector_64.map(f32::from_bits),
        teleport_requested: physical.state.flag_69 != 0,
        physical_state: physical.state.state_16,
        state_frames: skater.player_state.state_count,
        ground_suppressed: processed.grind.flags_1516 & 0x0800_0000 != 0,
        offboard_correction: skater.biped_ground.controller.state.contact.active,
        ground_category: super::ground_runtime::active_surface(&physics.riding, &physics.board)
            & 0xffff,
        //82DB6EC0 publishes the same processed foot record to both fields.
        foot_categories: [(processed.left_surface_2596 >> 7) & 31; 2],
        riding_transform: heading(deck, velocity),
        //82591E30 returns immediately for offboard, before the velocity branch.
        offboard_transform: offboard,
        stance,
        //No alternate-world/challenge controller exists in this local scene.
        alternate_world: false,
    };
    runtime.history.observe(
        &observation,
        runtime.settings.minimum_frames,
        &mut Scene {
            world: &physics.world,
            window: physics.window.as_ref(),
            aux: None,
            settings: &runtime.settings,
        },
    )
}

fn flip(matrix: &mut Matrix) {
    for i in [0, 2] {
        matrix[i] = matrix[i].map(|v| -v);
    }
}
///82591E30 retains the source axes at low speed or near vertical travel.
fn heading(mut matrix: Matrix, velocity: [f32; 4]) -> Matrix {
    let square = velocity[0] * velocity[0] + velocity[1] * velocity[1] + velocity[2] * velocity[2];
    if square > 0.25 {
        let inv = square.sqrt().recip();
        let forward = [velocity[0] * inv, 0., velocity[2] * inv, 0.];
        if forward[0] * forward[0] + forward[2] * forward[2] > 0.9 {
            matrix[0] = [forward[2], 0., -forward[0], 0.];
            matrix[1] = [0., 1., 0., 0.];
            matrix[2] = forward;
        }
    }
    matrix
}

struct Scene<'a> {
    world: &'a BoardWorld,
    /// Streamed collision: a candidate far from the skater is checked against a
    /// world built around it, which holds what the whole world holds there.
    window: Option<&'a crate::collision_window::Window>,
    aux: Option<([f32; 3], BoardWorld)>,
    settings: &'a Settings,
}
/// skate_vita (streamed collision): a far candidate's world gives its buffers back.
impl Drop for Scene<'_> {
    fn drop(&mut self) {
        if let Some((_, world)) = self.aux.take() {
            crate::collision_window::recycle(world);
        }
    }
}

/// The world a candidate at `at` is checked against: the whole world, or (streamed
/// collision) the current window when it holds everything the checks can reach (30 m: the
/// ground probe and the capsule), else one built around the candidate, cached while
/// candidates stay within 8 m. Building one costs 1.2 s on the Vita: only far candidates
/// (a respawn checkpoint far from the skater) pay it.
fn world_at<'b>(
    base: &'b BoardWorld,
    window: Option<&crate::collision_window::Window>,
    aux: &'b mut Option<([f32; 3], BoardWorld)>,
    at: [f32; 4],
) -> Result<&'b BoardWorld, String> {
    let Some(window) = window else { return Ok(base) };
    let at = [at[0], at[1], at[2]];
    if window.covers(at, 30.0) {
        return Ok(base);
    }
    if aux.as_ref().is_none_or(|(c, _)| (c[0] - at[0]).abs().max((c[2] - at[2]).abs()) > 8.0) {
        let fresh = window.around(at)?;
        if let Some((_, old)) = aux.replace((at, fresh)) {
            crate::collision_window::recycle(old);
        }
    }
    Ok(&aux.as_ref().unwrap().1)
}
impl Validation for Scene<'_> {
    type Error = String;
    fn ground(&mut self, transform: &Matrix) -> Result<Option<Ground>, String> {
        let mut start = transform[3];
        let world = world_at(self.world, self.window, &mut self.aux, start)?;
        start[1] += 0.1;
        let mut end = start;
        end[1] -= 10.;
        let probe = |start, end, radius| Probe {
            start,
            end,
            radius,
        };
        //The only actor has matching identity0; canonical map groups remain active.
        let Some(hit) = contact_queries::query(world, probe(start, end, 0.), 0)? else {
            return Ok(None);
        };
        let category = (u32::from(hit.packed_surface) >> 7) & 31;
        if hit.geometry.normal.y < self.settings.normal_y
            || start[1] - hit.geometry.position.y > self.settings.drop
            || !surface_allowed(category)
        {
            return Ok(None);
        }
        let mut bottom = start;
        bottom[1] += self.settings.radius;
        let mut top = bottom;
        top[1] += self.settings.height;
        if contact_queries::query(world, probe(bottom, top, self.settings.radius), 0)?
            .is_some()
        {
            return Ok(None);
        }
        let p = hit.geometry.position;
        Ok(Some(Ground {
            position: [p.x, p.y, p.z, 0.],
            offboard: category == 8,
        }))
    }
    fn location(&mut self, _: &Matrix) -> Result<bool, String> {
        //82BFBB48 returns true in normal-world mode, before provider invocation.
        Ok(true)
    }
    fn occupants(&mut self, _: &Matrix) -> Result<bool, String> {
        //82BFB928 queries LivingWorldManager pedestrians/vehicles, not terrain.
        //This BoardWorld has no living-world actors. Static obstacles are checked
        //by ground's capsule; the player and board must not reject themselves.
        self.world.query_metadata().map_err(str::to_owned)?;
        Ok(true)
    }
    fn edges(&mut self, transform: &Matrix) -> Result<bool, String> {
        let p = transform[3];
        let bounds = Bounds {
            min: Vector3::new(p[0] - 0.3, p[1] - 0.6, p[2] - 0.3),
            max: Vector3::new(p[0] + 0.3, p[1] + 0.6, p[2] + 0.3),
        };
        let metadata = self.world.query_metadata().map_err(str::to_owned)?;
        //82C1EAD8: authored static order, shared capacity40; no triangle diagonals.
        for edge in metadata
            .static_edges
            .iter()
            .filter(|e| e.local_bounds.overlaps(bounds))
            .take(40)
        {
            let start = [edge.start.x, edge.start.y, edge.start.z];
            let end = [edge.end.x, edge.end.y, edge.end.z];
            let delta: [f32; 3] = std::array::from_fn(|i| end[i] - start[i]);
            let square: f32 = delta.iter().map(|v| v * v).sum();
            let along: f32 = (0..3).map(|i| (p[i] - start[i]) * delta[i]).sum();
            let fraction = if square > 0. {
                (along / square).clamp(0., 1.)
            } else {
                0.
            };
            let distance: f32 = (0..3)
                .map(|i| (p[i] - start[i] - delta[i] * fraction).powi(2))
                .sum();
            if distance < 0.09 {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::physics::{
        contact::RetailContactMaterial, skeleton_animation_record::IDENTITY,
    };
    fn runtime() -> Runtime {
        Runtime {
            history: History::new(Candidate { transform: IDENTITY, stance: 0, offboard: false, score: 0. }),
            settings: Settings { height: 0.4, radius: 0.5, drop: 1., normal_y: 0.75, minimum_frames: 15 },
            measurements: 0,
            pending: None,
            outbox: Vec::new(),
            placements: 0,
            destination: None,
        }
    }
    /// Runs the core 82DB8120 port the way player_state/publication.rs does and completes the
    /// checkpoint reply; returns the queued message.
    fn reset_after(
        state: u32,
        set: impl Fn(&mut skate_core::player::input_phase::PhysicalPlayerInput),
        earlier: Option<RespawnReason>,
    ) -> PlayerRespawn {
        use skate_core::player::input_phase::{PhysicalPlayerInput, ProcessedPhysicsInput};
        let mut runtime = runtime();
        if let Some(reason) = earlier {
            runtime.note_request(reason, 7, [0.; 3]);
        }
        let mut out = PhysicalPlayerInput::default();
        set(&mut out);
        let processed = ProcessedPhysicsInput { state_2508: state, ..Default::default() };
        let contact = skate_core::player::input_phase::publish_special_surface(
            &mut out,
            &processed,
            &mut [false; 36],
            0,
            0.,
        )
        .expect("type-6 contact raises the teleport request");
        assert_eq!(out.state.flag_69, 1);
        runtime.note_boundary(contact, [1., -4., 2.]);
        let mut candidate = Candidate { transform: IDENTITY, stance: 0, offboard: false, score: 0. };
        candidate.transform[3] = [5., 1., 6., 1.];
        runtime.complete(42, &candidate, [9.; 3]);
        assert!(runtime.pending.is_none());
        let mut out = std::mem::take(&mut runtime.outbox);
        assert_eq!(out.len(), 1);
        out.remove(0)
    }
    #[test]
    fn type_six_board_contact_resets_with_reason_boundary() {
        let event = reset_after(100, |out| out.collision.surface_type_16 = 6, None);
        assert_eq!(event.reason, RespawnReason::Boundary);
        assert_eq!((event.tick, event.player_id, event.air_frames), (42, LOCAL_PLAYER_ID, 0));
        assert_eq!(event.checkpoint, [5., 1., 6.]);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"reason\":\"boundary\""), "{json}");
    }
    #[test]
    fn type_six_ragdoll_contact_resets_with_reason_boundary() {
        let event = reset_after(300, |out| out.collision.flag_214 = 1, None);
        assert_eq!(event.reason, RespawnReason::Boundary);
    }
    #[test]
    fn earlier_request_before_the_reply_keeps_its_reason() {
        let event = reset_after(
            100,
            |out| out.collision.surface_type_16 = 6,
            Some(RespawnReason::AirTimeout),
        );
        assert_eq!((event.reason, event.air_frames), (RespawnReason::AirTimeout, 7));
        // With nothing noted the reply stays `requested`.
        let mut runtime = runtime();
        let candidate = Candidate { transform: IDENTITY, stance: 0, offboard: false, score: 0. };
        runtime.complete(1, &candidate, [0.; 3]);
        assert_eq!(runtime.outbox[0].reason, RespawnReason::Requested);
    }
    #[test]
    fn real_scene_records_a_checkpoint_and_recovers_it_from_an_unsupported_position() {
        let world = super::super::ground::world(RetailContactMaterial {
            static_friction: 0.,
            dynamic_friction: 0.,
            restitution: 0.,
        });
        let settings = Settings {
            height: 0.4,
            radius: 0.5,
            drop: 1.,
            normal_y: 0.75,
            minimum_frames: 15,
        };
        let mut scene = Scene {
            world: &world,
            window: None,
            aux: None,
            settings: &settings,
        };
        let mut history = History::new(Candidate {
            transform: IDENTITY,
            stance: 0,
            offboard: false,
            score: 0.,
        });
        //The first conditioning samples clear the construction cooldown.
        assert!(!history.recording_due(119, [2., 0., 0., 0.]));
        let mut transform = IDENTITY;
        transform[3] = [2., 0.2, 0., 0.];
        history
            .observe(
                &Observation {
                    measurements: 120,
                    root_position: transform[3],
                    com_position: [1000., 0., 0., 0.],
                    teleport_requested: false,
                    physical_state: 100,
                    state_frames: 16,
                    ground_suppressed: false,
                    offboard_correction: false,
                    ground_category: 1,
                    foot_categories: [1; 2],
                    riding_transform: transform,
                    offboard_transform: IDENTITY,
                    stance: 1,
                    alternate_world: false,
                },
                15,
                &mut scene,
            )
            .unwrap();
        let selected = history.automatic(0, &mut scene).unwrap();
        assert_eq!(selected.transform, transform);
        assert_eq!(selected.stance, 1);
        //Successful historical entries are consumed: a second failure uses spawn.
        assert_eq!(
            history.automatic(0, &mut scene).unwrap().transform,
            IDENTITY
        );
    }
    #[test]
    fn native_heading_keeps_low_speed_and_vertical_axes() {
        let mut source = IDENTITY;
        source[0] = [0., 0., -1., 0.];
        source[2] = [1., 0., 0., 0.];
        assert_eq!(heading(source, [0., 0., 0.5, 0.]), source);
        assert_eq!(heading(source, [0., 5., 0.1, 0.]), source);
        let result = heading(source, [0., 0., 2., 0.]);
        assert_eq!(result[2], [0., 0., 1., 0.]);
        assert_eq!(result[3], source[3]);
    }
}
