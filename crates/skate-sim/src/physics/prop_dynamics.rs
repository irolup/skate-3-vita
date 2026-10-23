//! Dynamic rigid bodies for DMO prop instances (Phase 2a).
//!
//! Every spawned prop gets a box body built on the TU3 rigid-body integrator
//! (`integrate_body_rates`): gravity and cool-down/sleep come from the retail
//! simulation step; the mass properties are the retail rounded-box finalize
//! path (`primitive_mass_properties`) with the instance's scaled template AABB.
//! Density is the authored MOBJ per-object value (`ObjectPhysics`, schema 3+);
//! friction, restitution and linear / angular drag come from the prop type's
//! retail DMO data ([`PropDynamics::set_type_data`]), with the authored MOBJ
//! values (project defaults friction 0.55, restitution 0.05, damping
//! 0.05/0.15) only for props whose type data is missing.
//!
//! Narrowphase uses the recovered GP pair query (`primitive_pair_contacts`):
//! box vs static-world triangles, box vs box for other props, and box vs the
//! skater's board/skeleton volumes for pushes. Contact response is the retail
//! row solver the board uses (ContactBatchBuild 82AE10C8 rows, 25 iterations
//! of 82AE27D0, `PropSolverSettings`); the engine's older impulse pass stays
//! selectable as a mod option. Sleep is the retail counter (integrator
//! 82AE6590, sleep pass 82DC3130) with the DMO island values.
//!
//! Props start asleep and cost one AABB test per skater volume per tick. A
//! skater contact or a moving prop wakes them. Moved instances re-bake their
//! triangle range in the prop collision layer so skater queries stay exact.
//! The held (carried) prop is exempt from both skater pushes and the rebake:
//! while carried it is velocity-driven and its layer triangles are parked far
//! below the world so they cannot push the carrier.
use glam::{Mat3, Quat, Vec3};
use skate_core::{
    math::{Basis3, Vector3},
    physics::{
        board_world::{BoardWorld, BoardWorldVolume},
        contact::{
            RetailContactBodyState, RetailContactInput, RetailContactMaterial,
            combine_contact_materials, generate_contact,
        },
        contact_solver::{ACTIVE_BODY, RetailContactJacobian, build_contact_jacobian},
        mass::{DmoBodyData, RETAIL_UNBOUNDED_VELOCITY, dmo_body_inertia, primitive_mass_properties},
        rigid_body::{
            RetailInertiaDynamics, RetailQuaternion, RetailReactionCorrections, RetailBodyRates,
            RetailSimulationStep, integrate_body_rates, pack_world_inverse_inertia,
            world_inverse_inertia,
        },
        solver::solve_constraints,
        collision::WorldContactSettings,
        world_contact::{
            ContactPrimitive, PrimitiveContactManifold, PrimitivePairSettings,
            primitive_pair_contacts, primitive_triangle_world_contacts,
        },
    },
};

/// Template-space contact box that replaces the render-AABB box of one prop
/// type (centre and half extents, per-axis scale applied on top).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropBox {
    pub center: Vector3,
    pub half_extents: Vector3,
}

/// Every tunable of the prop impulse pass and the skater push. One value set
/// is the default for all props; `PropTuningTable` can replace it per prop
/// type (the MOBJ template name, the stable id a mod uses). The defaults are
/// #15's constants plus the 2026-10-05 caps (doc 26, "Board stuck inside a
/// prop"); no retail values for DMO contact response are recovered yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropTuning {
    /// Contact band of the prop pair queries (m).
    pub contact_padding: f32,
    /// Penetration ignored by the positional correction (m; older impulse
    /// pass only, `PropSolverSettings::row_solver` off).
    pub penetration_slop: f32,
    /// Fraction of the penetration removed per tick (Baumgarte; older
    /// impulse pass only).
    pub penetration_correction: f32,
    /// Upper bound on the positional correction of one body in one tick (m;
    /// older impulse pass only, retail rows have no cap).
    /// A body deep inside geometry comes out over several ticks instead of
    /// being thrown out in one.
    pub max_depenetration_per_tick: f32,
    /// Older impulse pass only: restitution applies only above this closing
    /// speed (m/s); below it
    /// contacts are inelastic so resting stacks settle.
    pub restitution_threshold: f32,
    /// Effective skater mass (kg) for prop pushes.
    pub skater_push_mass: f32,
    /// Fraction of the closing speed transferred to a prop by a skater hit.
    pub push_transfer: f32,
    /// Top speed a body bump can impart along the push direction (m/s).
    pub body_push_speed: f32,
    /// Top speed a board hit can impart along the push direction (m/s).
    pub board_push_speed: f32,
    /// Push speed used when a skater volume is inside the prop but not
    /// closing (m/s): nudges an overlapping prop apart.
    pub penetration_push_speed: f32,
    /// After this many consecutive ticks in which a skater volume sits inside
    /// a prop without closing on it, the overlap nudge stops so the prop can
    /// cool down and sleep (the skater's own contacts still separate it).
    /// 0 disables the nudge entirely.
    pub stuck_release_ticks: u32,
    /// Replaces the render-AABB contact box of this prop type.
    pub collision_box: Option<PropBox>,
}

impl Default for PropTuning {
    fn default() -> Self {
        Self {
            contact_padding: 0.02,
            penetration_slop: 0.005,
            penetration_correction: 0.4,
            max_depenetration_per_tick: 0.05,
            restitution_threshold: 1.0,
            skater_push_mass: 75.0,
            push_transfer: 0.5,
            body_push_speed: 1.2,
            board_push_speed: 6.0,
            penetration_push_speed: 0.5,
            stuck_release_ticks: 30,
            collision_box: None,
        }
    }
}

/// Prop tuning for the whole map: a default plus per prop type overrides,
/// keyed by MOBJ template name. `reset` restores the shipped defaults (mod
/// disable).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PropTuningTable {
    pub default: PropTuning,
    pub by_template: std::collections::BTreeMap<String, PropTuning>,
    /// Island settings shared by every prop (one DMO simulation in retail).
    pub solver: PropSolverSettings,
    /// Self-righting window of the phone's per-object Upright (cMsgUprightDMO).
    pub upright: PropUprightSettings,
}

/// Retail DMO self-righting ("Upright", doc 27 "Upright"). The phone's
/// per-object Upright posts cMsgUprightDMO; the DMO manager slot +40 82C4B8C0
/// sets DMO+4464 bit 0x40 and zeroes the timer DMO+4376. While the bit is set
/// the DMO update 82C56780 adds `tick_seconds` to the timer and clears the bit
/// once it exceeds `window_seconds`, then (same update) 82C573D0 measures the
/// angle between the body's up row and world up: below `stop_angle_deg` the
/// bit and timer are cleared; otherwise it builds an angular command that the
/// DMO's own angular slot 37 (82C52EE0 -> 82D9CCF0) writes into the body's
/// angular accumulator (+160), waking the body and marking it commanded.
/// External yaw commands (82C52E68) are refused while the bit is set; the
/// linear Move Object command (82C52DC0) is not gated. The defaults are the
/// retail constants; every field is a mod knob
/// (`sdk.world.set_tuning('props', {upright = {...}})`), reset on mod disable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropUprightSettings {
    /// Window length (retail 2.0 s, 0x82060C50).
    pub window_seconds: f32,
    /// Timer increment per update (retail 1/60, 0x820849C8: a fixed per-frame
    /// step, matching our 60 Hz prop step).
    pub tick_seconds: f32,
    /// Tilt below which the window ends (retail 10 deg, 0x821963E4).
    pub stop_angle_deg: f32,
    /// Tilt cap of the righting speed (retail 70 deg: 70 (0x820BB1E0) x
    /// 0.0174533 (0x8206D110)).
    pub max_angle_deg: f32,
    /// Dead band subtracted from the capped tilt (retail 5 deg, 0x820BB1D8).
    pub dead_band_deg: f32,
    /// Righting gain (1/s) at the low end of the blend (retail 3, 0x82063B08).
    pub gain_min: f32,
    /// Righting gain at the high end of the blend (retail 5, 0x821F1790).
    pub gain_max: f32,
    /// Blend weight t = clamp(|A| - gain_blend_start, 0, 1) on the body
    /// vector A (state block +72); retail 0.1 (0x820641A8) + 1.0 (0x8231A844).
    pub gain_blend_start: f32,
    /// Fraction of the off-axis spin the command removes per update (retail
    /// 0.1, 0x820641A8: the command targets w_axis + 0.1 w_perp).
    pub off_axis_spin: f32,
    /// Command scale (retail 60, 0x821FF080: one 60 Hz step to the target).
    pub command_rate: f32,
    /// Above this tilt (or with a degenerate axis) the body's own X or Z axis
    /// is used instead of up x world-up (retail 120 deg, 0x82256FE0).
    pub fallback_angle_deg: f32,
    /// External yaw commands refused while righting (retail 82C52E68).
    pub block_yaw: bool,
}

impl Default for PropUprightSettings {
    fn default() -> Self {
        Self {
            window_seconds: 2.0,
            tick_seconds: 1.0 / 60.0,
            stop_angle_deg: 10.0,
            max_angle_deg: 70.0,
            dead_band_deg: 5.0,
            gain_min: 3.0,
            gain_max: 5.0,
            gain_blend_start: 1.1,
            off_axis_spin: 0.1,
            command_rate: 60.0,
            fallback_angle_deg: 120.0,
            block_yaw: true,
        }
    }
}

/// The righting command of 82C573D0 for a body with orientation `basis`,
/// angular velocity `w` and body vector `a` (retail: the vector at the
/// physics state block +72; which body quantity that is has not been
/// identified, see [`PropDynamics::upright_vector`]). `None` = tilt below
/// `stop_angle_deg` (window ends). Pure and deterministic.
pub(crate) fn upright_command(basis: Basis3, w: Vector3, a: Vector3, s: &PropUprightSettings) -> Option<Vector3> {
    let up = mul_basis(basis, Vector3::new(0.0, 1.0, 0.0));
    let up_len = length(up);
    let cos = if up_len > 0.0 { (up.y / up_len).clamp(-1.0, 1.0) } else { 1.0 };
    let angle = cos.acos();
    let degrees = angle.to_degrees();
    if degrees < s.stop_angle_deg {
        return None;
    }
    let normalize = |v: Vector3| {
        let l = length(v);
        if l > 0.0 { scale(v, 1.0 / l) } else { Vector3::ZERO }
    };
    let mut axis = normalize(cross(up, Vector3::new(0.0, 1.0, 0.0)));
    let degenerate = axis.x.abs() <= f32::EPSILON && axis.y.abs() <= f32::EPSILON && axis.z.abs() <= f32::EPSILON;
    if degenerate || degrees > s.fallback_angle_deg {
        let local = if a.x > a.z { Vector3::new(1.0, 0.0, 0.0) } else { Vector3::new(0.0, 0.0, 1.0) };
        axis = normalize(mul_basis(basis, local));
    }
    let capped = angle.min(s.max_angle_deg.to_radians());
    let t = (length(a) - s.gain_blend_start).clamp(0.0, 1.0);
    let gain = (1.0 - t) * s.gain_min + t * s.gain_max;
    let speed = gain * (capped - s.dead_band_deg.to_radians()).max(0.0);
    let along = scale(axis, dot(axis, w));
    let across = sub(w, along);
    let keep = add(along, scale(across, s.off_axis_spin));
    Some(scale(sub(scale(axis, speed), keep), s.command_rate))
}

/// Island settings of the DMO simulation. Retail 8275DCC8 passes a 52-byte
/// block to the simulation ctor 82DC2840, which copies +16 -> island +176
/// (solver iterations), +32 -> island +172 (sleep energy) and +36 -> island
/// +168 (sleep counter cap; also the sleep pass threshold sim +204). The
/// defaults are those retail values; every field is a mod knob
/// (`sdk.world.set_tuning('props', {solver = {...}})`), reset on mod disable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropSolverSettings {
    /// Contact solve: true = the retail row solver (rows built by
    /// ContactBatchBuild 82AE10C8, iterated by 82AE27D0, the same skate-core
    /// path the board uses); false = the engine's older impulse pass with
    /// penetration slop / fraction / cap (NOT RETAIL, kept as a mod option).
    pub row_solver: bool,
    /// Row solver iterations per step (retail 25).
    pub iterations: u32,
    /// Sleep energy threshold on E = |v|^2 + s * m^-1 * |w|^2 after damping
    /// and the speed caps (integrator 82AE6590; retail 1e-5, 0x8219B100).
    pub sleep_energy: f32,
    /// Steps with E below `sleep_energy` and not rising before the sleep
    /// pass (82DC3130) puts a body to sleep; also the counter cap (retail 2).
    pub sleep_frames: u32,
    /// Bodies the sleep pass puts to sleep per step at most (retail 100).
    pub max_sleeps_per_step: u32,
    /// The engine's rest snap (NOT RETAIL): zero the velocities of a touching
    /// body whose energy is below `sleep_energy`. Retail has no snap; off by
    /// default, kept as a mod option (with the old 0.5 / 30 sleep values it
    /// reproduces the pre-2026-10-08 settling).
    pub rest_snap: bool,
    /// The bail clamp (retail `82C530F0`, doc 27 "What wakes a resting prop"): when a bailing actor's volume
    /// touches a prop, the prop's velocity is capped and it is woken, so bailing into a prop does not throw it.
    pub bail_clamp: bool,
    /// Linear cap = `bail_linear_factor` x inverse mass, m/s (retail 5.0, `0x821F1790`).
    pub bail_linear_factor: f32,
    /// Spin cap = `bail_angular_factor` x inverse mass, rad/s (retail 15.0).
    pub bail_angular_factor: f32,
    /// From this mass on (kg) the linear allowance is 0 and the spin is zeroed (retail 200).
    pub bail_heavy_mass: f32,
    /// Ball radius blend: radius = blend x (max(cap, |d|) + cap) (retail 0.5).
    pub bail_blend: f32,
    /// A prop type of at least this mass (kg) is collision type 12, lighter ones 14 (retail 5.5, `0x822F8C68`, slot 33
    /// `82C56BA0`; spawns `82C4E940` -> `82E0E338` give the same pair, trace .local/research/prop-types). The on-foot
    /// walker's queries reject types 13 and 14, so only the heavier props are ground for it.
    pub walkable_mass: f32,
    /// Retail's ground contact reduction (`82779EB0`): coplanar contacts of a prop against the world keep at most four
    /// points (upstream `b110bab`..`74c124a`).
    pub ground_reduction: skate_core::physics::contact_reduction::ReductionRules,
}

impl Default for PropSolverSettings {
    fn default() -> Self {
        Self {
            row_solver: true,
            iterations: 25,
            sleep_energy: 1e-5,
            sleep_frames: 2,
            max_sleeps_per_step: 100,
            rest_snap: false,
            bail_clamp: true,
            bail_linear_factor: 5.0,
            bail_angular_factor: 15.0,
            bail_heavy_mass: 200.0,
            bail_blend: 0.5,
            walkable_mass: 5.5,
            ground_reduction: Default::default(),
        }
    }
}

/// Retail's bail clamp `82C530F0` on one prop: `w` = inverse mass (motion record +0x7C, b209), `d` = the prop's
/// own velocity as its transform tracker measures it (`[dmo+0xA4]+0x90`, last tick's displacement / dt, written by
/// `82E0C578` from the DMO update, b210); ours: the prop's velocity before this tick's skater pushes. The linear velocity is pulled into the ball of radius `mag` around `n * mag`
/// (n = d / |d|, mag = blend x (max(cap, |d|) + cap), cap = factor x w); the spin is capped at the angular factor x w,
/// or zeroed for a heavy prop (and the linear cap is 0). Returns (linear, angular); `None` for a static body (w = 0).
pub(crate) fn bail_clamp(s: &PropSolverSettings, w: f32, d: Vector3, linear: Vector3, angular: Vector3) -> Option<(Vector3, Vector3)> {
    if !(w > 0.0) {
        return None;
    }
    let light = 1.0 / w < s.bail_heavy_mass;
    let cap = if light { s.bail_linear_factor * w } else { 0.0 };
    let angular = if light {
        let limit = s.bail_angular_factor * w;
        let l = length(angular);
        if l > limit { scale(angular, limit / l) } else { angular }
    } else {
        Vector3::new(0.0, 0.0, 0.0)
    };
    let dl = length(d);
    let n = if dl > 1.5e-5 { scale(d, 1.0 / dl) } else { Vector3::new(0.0, 0.0, 0.0) };
    let mag = s.bail_blend * (cap.max(dl) + cap);
    let centre = scale(n, mag);
    let off = sub(linear, centre);
    let ol = length(off);
    let linear = if ol > mag { add(centre, scale(off, if ol > 0.0 { mag / ol } else { 0.0 })) } else { linear };
    Some((linear, angular))
}

impl PropTuningTable {
    pub(crate) fn for_template(&self, template: &str) -> &PropTuning {
        self.by_template.get(template).unwrap_or(&self.default)
    }
}

/// Per-step work counters, for tests and the frame-cost check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PropStepStats {
    pub awake: u32,
    pub rebakes: u32,
    pub skater_pushes: u32,
}
/// Actor id of the local player's skater volumes in the prop step (NPC skaters
/// use their proxy solid id, `living_world::npc_skaters::PROXY_ID_TAG | id`).
pub(crate) const LOCAL_PUSHER: u64 = 0;

/// Where the held prop's collision triangles are parked so skater queries
/// cannot see them while it is carried.
pub(crate) const HELD_PARK: Vector3 = Vector3::new(0.0, -10000.0, 0.0);

/// Props get their own simulation step (retail: the DMO simulation, its own
/// island settings, 8275DCC8 -> 82DC2840): sleep counter cap 2 and sleep
/// energy 1e-5 ([`PropSolverSettings`] defaults; the step applies the live
/// settings). The board's simulation carries cool_down = 0 (the host never
/// sleeps it).
pub(crate) fn prop_simulation(
    base: skate_core::physics::rigid_body::RetailSimulationStep,
) -> skate_core::physics::rigid_body::RetailSimulationStep {
    let solver = PropSolverSettings::default();
    skate_core::physics::rigid_body::RetailSimulationStep {
        cool_down: solver.sleep_frames,
        minimum_energy: solver.sleep_energy,
        ..base
    }
}

/// Props in the grab scene and Move Object on their authored grab records: always on in the game (set by
/// `LivingWorldPlugin`, user 2026-10-15; doc 26i "Move Object step 3"); off (unit tests' default), the scene holds no props and
/// carrying uses the box stand-in edge.
static PROP_GRAB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) fn prop_grab_enabled() -> bool {
    PROP_GRAB.load(std::sync::atomic::Ordering::Relaxed)
}
/// Set by `living_world::opt_ins` (the player's choice); in effect from the next tick.
pub(crate) fn set_prop_grab_enabled(on: bool) {
    PROP_GRAB.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Height of a generated grab spline over the box bottom, m (the authored splines sit at about hand height, ~0.98 m
/// on the DownTown tables); a lower box is held by its top.
const GENERATED_GRAB_HEIGHT: f32 = 1.0;

/// Better Object Mover's grab splines for a prop without authored ones: one per vertical box face along its
/// horizontal edge at [`GENERATED_GRAB_HEIGHT`], approach = the face's outward normal and flags 4, as the authored
/// splines are laid out (template space, scale divided out: `grab_object_of` multiplies it back in).
fn generated_grab_splines(b: &PropBody) -> Vec<crate::living_world::vehicles::CarGrabSpline> {
    let (c, h, s) = ([b.local_center.x, b.local_center.y, b.local_center.z], [b.half_extents.x, b.half_extents.y, b.half_extents.z], [b.axis_scale.x, b.axis_scale.y, b.axis_scale.z]);
    if h.iter().chain(&s).any(|v| !(v.is_finite() && v.abs() > 1e-4)) {
        return Vec::new();
    }
    let y = (c[1] - h[1] + GENERATED_GRAB_HEIGHT).min(c[1] + h[1]);
    let unscale = |p: [f32; 3]| [p[0] / s[0], p[1] / s[1], p[2] / s[2]];
    let mut out = Vec::new();
    for (axis, along) in [(0, 2), (2, 0)] {
        for side in [-1.0f32, 1.0] {
            let mut mid = c;
            mid[1] = y;
            mid[axis] += side * h[axis];
            let (mut a, mut b) = (mid, mid);
            a[along] -= h[along] * 0.9;
            b[along] += h[along] * 0.9;
            let mut direction = [0.0; 3];
            direction[axis] = side;
            out.push(crate::living_world::vehicles::CarGrabSpline { points: vec![unscale(a), unscale(a), unscale(b), unscale(b)], direction, flags: 4 });
        }
    }
    out
}

/// Grab-scene id tag of props (cars use `living_world::vehicles::CAR_GRAB_TAG`, the high bit).
pub(crate) const PROP_GRAB_TAG: u32 = 0x4000_0000;

pub(crate) struct PropBody {
    /// Index into the collision layer's instance list (and its rebake target).
    instance: usize,
    id: u32,
    /// Dense grab-scene number (map props in load order, then mid-game bodies in creation order): the grab ids
    /// pack it as `PROP_GRAB_TAG | slot << 6 | spline`, which the 32-bit map instance ids do not fit (only ids
    /// below 2^24 did, so the grab scene held 2 of DownTown's 786 props, laptop 2026-10-18).
    grab_slot: u32,
    /// Box centre and half extents in template space, scale folded in.
    local_center: Vector3,
    half_extents: Vector3,
    rates: RetailBodyRates,
    /// Effective Inertia ([`PropDynamics::body_inertia`], refreshed when the
    /// type data or mod rules change).
    inertia: RetailInertiaDynamics,
    /// Inertia from the authored MOBJ block (density x box volume, damping),
    /// the fallback for fields the type data and mods leave unset.
    authored_inertia: RetailInertiaDynamics,
    /// Authored MOBJ contact material (friction/restitution).
    material: RetailContactMaterial,
    enable_sleep: bool,
    asleep: bool,
    /// Streamed out by the DMO census (`skate_core::living_world::dmo`): asleep, collision parked, never woken,
    /// pushed, grabbed or listed until it streams back in.
    dormant: bool,
    /// MOBJ template name: the prop type key of `PropTuningTable`.
    template: String,
    /// Retail type data of this prop (vault record name and values,
    /// [`PropDynamics::set_type_data`]); `None` = not resolved.
    type_data: Option<DmoType>,
    /// Resolved tuning of this prop type.
    tuning: PropTuning,
    /// The template's authored grab splines (template space; `set_grab_splines`), or with Better Object Mover on and
    /// none authored, generated ones ([`generated_grab_splines`]).
    grab_splines: Vec<crate::living_world::vehicles::CarGrabSpline>,
    /// skate_vita (performance): the grab-scene splines [`PropDynamics::grab_object_of`] gives this body, built once
    /// by `set_grab_splines` (their keys, scaled points and approach depend only on the grab slot, the axis scale and
    /// the splines, which do not change after that); the grab scene ran them through 3 allocations a spline every tick.
    grab_scene_splines: Vec<skate_core::player::offboard::grab_scene::Spline>,
    /// The template has authored grab splines (retail can move it).
    authored_grab: bool,
    /// Box derived from the render AABB (scale folded in), kept so a
    /// `collision_box` override can be removed again.
    authored_center: Vector3,
    authored_half_extents: Vector3,
    axis_scale: Vector3,
    /// Consecutive ticks a skater volume sat inside this prop without
    /// closing on it.
    stuck_ticks: u32,
    /// Stable actor id of the skater whose contact last pushed this prop
    /// ([`LOCAL_PUSHER`] or an NPC skater proxy id): the authority owner a
    /// future host uses for the moved prop. `None` until something pushes it.
    pushed_by: Option<u64>,
    /// Pose last written to the collision layer; an unchanged pose skips the
    /// (identical) rebake.
    baked: Option<(Vector3, Basis3)>,
    /// Contact manifolds (static triangles and other props) that touched this
    /// body in its last step, for the HELD_PROP diagnostics.
    contacts: u32,
    /// Box centre height where this body last rested (spawn, sleep or grab):
    /// the ground probe for HELD_PROP / PROP_BELOW_GROUND starts above it, so
    /// a body that sank under the floor still finds the floor it left.
    rest_y: f32,
    /// Authored (spawn) template-origin pose: what a DMO reset returns the body to
    /// (retail cMsgResetDMO, doc 27 "Object Dropper and reset").
    spawn_origin: Vector3,
    spawn_basis: Basis3,
    /// A Move Object command arrived for this body since its last step
    /// (retail DMO+4465 bit 0x02, set by the slot 9 sinks 82C52DC0 /
    /// 82C52E68).
    commanded: bool,
    /// The body runs on the commanded parameter block (retail DMO+4465 bit
    /// 0x01: the previous tick's commanded bit; 82C53EF8 swaps the block when
    /// the two differ).
    commanded_block: bool,
    /// Self-righting window timer (retail DMO+4376; `Some` = DMO+4464 bit
    /// 0x40 set, [`PropUprightSettings`]).
    upright_timer: Option<f32>,
    /// skate_vita: the body was set down on the floor below it ([`PropDynamics::seat`]).
    seated: bool,
    /// Created mid-game ([`PropDynamics::spawn_body`]), not placed by the map:
    /// no authored pose to reset to or save in the layout.
    runtime: bool,
    /// The authored physics block the body was built from (copied by
    /// [`PropDynamics::copy_spec`]).
    physics: skate_data::skate_map::ObjectPhysics,
}

/// Friction pair `[static, dynamic]` of a retail body contact material block.
/// The block is three floats at physics component +48 / +52 / +56 = {static
/// friction, dynamic friction, restitution (DMO data +272)}, written by
/// 82C550A8 and pointed at by every body's +80; the collision-object builders
/// 82DC3A68 / 82DC4158 / 82DC4588 copy it to CO +116..+124 and aaCollision
/// 8277A508 combines two objects' blocks with 82763078 (static max, dynamic
/// max, restitution min = [`combine_contact_materials`]).
pub(crate) type MaterialBlock = [f32; 2];

/// Retail commanded friction pair {0.03 (0x8208EA80), 0.02 (0x821E9580)}
/// (82C53EF8 while DMO+4465 bit 0x02 is set).
pub(crate) const RETAIL_COMMANDED_MATERIAL: MaterialBlock = [0.03, 0.02];

/// Retail upright test of 82C54B00: the body's up axis (transform row 1) has
/// y > 0.65 (0x820BB0EC); sets DMO+4465 bit 0x08.
pub(crate) const RETAIL_UPRIGHT_COS: f32 = 0.65;

/// Per prop type material data. Retail reads these from the DMO type data
/// (DMO+4380 -> +4, ctor 82C51E28): the layout of the type's vault record of
/// class `livingworld_dynamicobject_characteristics` (see [`dmo_type_blocks`]).
/// Each body carries the retail values of its type (resolved at map load from
/// the installation's stock vault); a mod's `carry.by_template` entry
/// overrides them field by field. A `None` left after both falls back to the
/// authored MOBJ material (only props whose type record is missing, e.g. an
/// older setup without the type map: NOT RETAIL YET there).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PropMaterialBlocks {
    /// Friction pair while commanded; `None` = `MoveCommandRules::commanded_material`.
    pub held: Option<MaterialBlock>,
    /// Free friction pair (DMO data +320 / +328; also the only free pair when
    /// `upright_pair` is off); `None` = authored MOBJ friction for both.
    pub free: Option<MaterialBlock>,
    /// Free friction pair while upright (DMO data +316 / +324), used only when
    /// `upright_pair` is set; `None` = `free`.
    pub free_upright: Option<MaterialBlock>,
    /// Type flag DMO data +312 bit 0 (-> DMO+4465 bit 0x10, ctor 82C51E28):
    /// the free pair depends on the upright test. `None` = false.
    pub upright_pair: Option<bool>,
    /// Restitution of every block of this type (DMO data +272); `None` = the
    /// authored MOBJ restitution.
    pub restitution: Option<f32>,
    /// Record+272 of this prop type (Move Object speeds x
    /// `record_272_speed_scale`): retail 82C4B960 sets it (1 or 2) only when
    /// DMO data +312 is set. `None` = false.
    pub record_272: Option<bool>,
    /// Linear drag of this prop type's rigid body (DMO data +308 `LinearDrag`,
    /// copied to Inertia +32 by 82C4E568). Per second: the integrator 82AE6590
    /// rebuilds velocity as displacement x max(frequency - drag, 0), i.e.
    /// `v *= 1 - drag dt` per fixed step. `None` = the authored MOBJ damping.
    pub linear_drag: Option<f32>,
    /// Angular drag (DMO data +336 `AngularDrag` -> Inertia +36, same rule).
    /// `None` = the authored MOBJ damping.
    pub angular_drag: Option<f32>,
    /// Body mass in kg (DMO data +304; 82C4E568 stores 1 / mass at Inertia
    /// +16 and builds the box inertia from it). `None` = the authored MOBJ
    /// density x box volume.
    pub mass: Option<f32>,
    /// Linear speed cap in m/s (DMO data +292 -> Inertia +24; the integrator
    /// 82AE6590 scales v down to this length after drag). `None` = unbounded.
    pub maximum_linear_velocity: Option<f32>,
    /// Angular speed cap in rad/s (DMO data +296 -> Inertia +28, same rule).
    /// `None` = unbounded.
    pub maximum_angular_velocity: Option<f32>,
    /// Box inertia shape (DMO data +16 scale, +32 offset): 82C4E568 takes the
    /// body's AABB half extents x scale + offset as the box 82C47FC8 fills
    /// the inverse tensor from. `None` = the class default record's
    /// [`DMO_DEFAULT_INERTIA_SCALE`] / zero when another body field is set,
    /// else the authored box inertia.
    pub inertia_scale: Option<[f32; 3]>,
    pub inertia_offset: Option<[f32; 3]>,
    /// The ped-plugin check (`82C56DB0`, research b136 / b137): per-axis rotation tolerance from the authored pose in
    /// degrees (DMO data +0 `Hash_E43445036AAEE7C5`, stock (2.5, 15, 2.5)) and the farthest the prop may move from
    /// its authored spot (+280 `Hash_2114F9329E98F00C`, stock 5 m) before its seats stop being offered.
    pub plugin_rotation_tolerance: Option<[f32; 3]>,
    pub plugin_max_distance: Option<f32>,
}

/// Box inertia scale of the class's `default` record (DMO data +16), used
/// for a mod's mass on a prop without type data.
pub(crate) const DMO_DEFAULT_INERTIA_SCALE: [f32; 3] = [1.2, 1.2, 1.2];

impl PropMaterialBlocks {
    /// Field by field: this block's value where set, else `base`'s.
    pub(crate) fn over(&self, base: &PropMaterialBlocks) -> PropMaterialBlocks {
        PropMaterialBlocks {
            held: self.held.or(base.held),
            free: self.free.or(base.free),
            free_upright: self.free_upright.or(base.free_upright),
            upright_pair: self.upright_pair.or(base.upright_pair),
            restitution: self.restitution.or(base.restitution),
            record_272: self.record_272.or(base.record_272),
            linear_drag: self.linear_drag.or(base.linear_drag),
            angular_drag: self.angular_drag.or(base.angular_drag),
            mass: self.mass.or(base.mass),
            maximum_linear_velocity: self.maximum_linear_velocity.or(base.maximum_linear_velocity),
            maximum_angular_velocity: self.maximum_angular_velocity.or(base.maximum_angular_velocity),
            inertia_scale: self.inertia_scale.or(base.inertia_scale),
            inertia_offset: self.inertia_offset.or(base.inertia_offset),
            plugin_rotation_tolerance: self.plugin_rotation_tolerance.or(base.plugin_rotation_tolerance),
            plugin_max_distance: self.plugin_max_distance.or(base.plugin_max_distance),
        }
    }
}

/// Vault class of the per-type DMO data.
pub(crate) const DMO_TYPE_CLASS: &str = "livingworld_dynamicobject_characteristics";

/// Retail values of one DMO type, read from its record of
/// [`DMO_TYPE_CLASS`] (parents included). The DMO constructor 82C51E28 keeps
/// the record's layout at DMO+4380 -> +4; schema layout offsets (skaterschema):
/// +272 restitution (`Hash_5CCD5998E03C299B`), +312 upright flag
/// (`Hash_C4D8A03586A31915`, -> DMO+4465 bit 0x10 and 82C4B960's record+272),
/// +316 / +324 upright pair (`Hash_E0101A9DFD63DEE9` / `Hash_CDA7A31C5EDBEB6E`),
/// +320 / +328 default pair (`Hash_6E0BB4F5881A4841` / `Hash_086956BCA2187458`),
/// read by 82C53EF8 / 82C550A8; +308 `LinearDrag` and +336 `AngularDrag`,
/// read by 82C4E568 into the body's Inertia (+32 / +36), with +304 mass
/// (`Hash_E5778CDD4576D890`), +292 / +296 velocity caps
/// (`Hash_4890392C91829954` / `Hash_BAA01E2BA1237455`) and the +16 / +32
/// inertia box scale / offset vectors (`Hash_F4D1C84C36A854AC` /
/// `Hash_D3CDE380DBB3ADC0`). The held pair is the retail constant
/// (`commanded_material`), not type data.
pub(crate) fn dmo_type_blocks(
    collections: &skate_data::collections::Collections,
    record: &str,
) -> Result<PropMaterialBlocks, String> {
    let f = |name: &str| collections.float(DMO_TYPE_CLASS, record, name);
    let upright = collections.boolean(DMO_TYPE_CLASS, record, "Hash_C4D8A03586A31915")?;
    let v = |name: &str| -> Result<[f32; 3], String> {
        let w = collections.words::<4>(DMO_TYPE_CLASS, record, name)?;
        let out = [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2])];
        if out.iter().all(|x| x.is_finite()) { Ok(out) } else { Err(format!("Non-finite {record}/{name}")) }
    };
    Ok(PropMaterialBlocks {
        held: None,
        free: Some([f("Hash_6E0BB4F5881A4841")?, f("Hash_086956BCA2187458")?]),
        free_upright: Some([f("Hash_E0101A9DFD63DEE9")?, f("Hash_CDA7A31C5EDBEB6E")?]),
        upright_pair: Some(upright),
        restitution: Some(f("Hash_5CCD5998E03C299B")?),
        record_272: Some(upright),
        linear_drag: Some(f("LinearDrag")?),
        angular_drag: Some(f("AngularDrag")?),
        mass: Some(f("Hash_E5778CDD4576D890")?),
        maximum_linear_velocity: Some(f("Hash_4890392C91829954")?),
        maximum_angular_velocity: Some(f("Hash_BAA01E2BA1237455")?),
        inertia_scale: Some(v("Hash_F4D1C84C36A854AC")?),
        inertia_offset: Some(v("Hash_D3CDE380DBB3ADC0")?),
        // Optional for older exports (every stock record stores them).
        plugin_rotation_tolerance: v("Hash_E43445036AAEE7C5").ok(),
        plugin_max_distance: f("Hash_2114F9329E98F00C").ok(),
    })
}

/// One prop type's retail data for [`PropDynamics::set_type_data`]: the vault
/// record's name (the mod-facing type key, e.g. `dt_garbagebin`) and values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DmoType {
    pub key: String,
    pub blocks: PropMaterialBlocks,
    /// The type's `livingworld_dynamicobject_priority` value (the census score; `None` = `keepalways`).
    pub priority: Option<u32>,
}

/// The priority class the characteristics' `Priority` RefSpec points into, and its value field.
pub(crate) const DMO_PRIORITY_CLASS: &str = "livingworld_dynamicobject_priority";
const DMO_PRIORITY_VALUE: &str = "Hash_52BD74D46C494CB7";

/// A DMO type's priority [data]: the characteristics record's `Priority` RefSpec (key hash in its first 8 bytes)
/// -> the `livingworld_dynamicobject_priority` record's value (`neverkeep` 0, `lowpriority` 1, `default` 100,
/// `mediumpriority` 200, `highpriority` 450, `keepalways` 0xFFFFFFFF = `None`).
pub(crate) fn dmo_type_priority(collections: &skate_data::collections::Collections, record: &str) -> Result<Option<u32>, String> {
    let w = collections.words::<4>(DMO_TYPE_CLASS, record, "Priority")?;
    let key = format!("Hash_{:016X}", (u64::from(w[0]) << 32) | u64::from(w[1]));
    let value = collections.integer(DMO_PRIORITY_CLASS, &key, DMO_PRIORITY_VALUE)?;
    Ok((value != u32::MAX).then_some(value))
}

/// How a Move Object command reaches the held body (retail interface slot 9
/// -> 82C4C370 / 82C4C3E0 -> 82D9CC78 / 82D9CCF0, spec section 7). The
/// defaults are the retail behaviour; every field is a mod knob
/// (`sdk.world.set_tuning('carry', ...)`), cleared on mod disable.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MoveCommandRules {
    /// Friction pair every commanded body switches to (retail {0.03, 0.02});
    /// the block's restitution stays the type's (DMO data +272).
    pub commanded_material: MaterialBlock,
    /// Upright test threshold on the body's up axis y (retail 0.65).
    pub upright_cos: f32,
    /// Per prop type overrides of the held / free blocks.
    pub by_template: std::collections::BTreeMap<String, PropMaterialBlocks>,
    /// Linear command at the centre of mass (retail: accumulator +144, no
    /// torque). `false` = applied at the grip point (lever torque; mod only).
    pub apply_at_com: bool,
    /// Yaw command replaces the angular accumulator (retail 82D9CCF0
    /// overwrites +160). `false` = added to the body's torque accumulator.
    pub yaw_replaces_torque: bool,
    /// Vertical command dropped (retail 82C4C370 passes only &linear).
    pub ignore_vertical: bool,
    /// Every command wakes the body and clears its sleep counter (retail
    /// 82ADF7B8 in both sinks). `false` = only a non-zero command wakes it.
    pub wake_on_command: bool,
}

impl Default for MoveCommandRules {
    fn default() -> Self {
        Self {
            commanded_material: RETAIL_COMMANDED_MATERIAL,
            upright_cos: RETAIL_UPRIGHT_COS,
            by_template: Default::default(),
            apply_at_com: true,
            yaw_replaces_torque: true,
            ignore_vertical: true,
            wake_on_command: true,
        }
    }
}

impl MoveCommandRules {
    /// The body's own contact material block (82C53EF8 / 82C54BF0 ->
    /// 82C550A8): while commanded {held pair, restitution}; otherwise the free
    /// pair, which for a type with the upright flag (data +312 bit 0) is the
    /// upright pair while `up_y > upright_cos` (82C54B00) and the default pair
    /// when tipped. Pure function of the commanded bit and the up axis.
    pub(crate) fn body_material(&self, template: &str, authored: RetailContactMaterial, commanded: bool, up_y: f32) -> RetailContactMaterial {
        self.typed_body_material(&self.blocks(template, None, &PropMaterialBlocks::default()), authored, commanded, up_y)
    }

    /// A body's effective type blocks: the mod entry for its MOBJ template
    /// name, else for its type key (vault record name), over the retail type
    /// data.
    pub(crate) fn blocks(&self, template: &str, type_key: Option<&str>, retail: &PropMaterialBlocks) -> PropMaterialBlocks {
        let modded = self.by_template.get(template).or_else(|| type_key.and_then(|k| self.by_template.get(k)));
        modded.map_or(*retail, |m| m.over(retail))
    }

    /// [`Self::body_material`] on resolved blocks.
    pub(crate) fn typed_body_material(&self, blocks: &PropMaterialBlocks, authored: RetailContactMaterial, commanded: bool, up_y: f32) -> RetailContactMaterial {
        let restitution = blocks.restitution.unwrap_or(authored.restitution);
        let [static_friction, dynamic_friction] = if commanded {
            blocks.held.unwrap_or(self.commanded_material)
        } else {
            let free = blocks.free.unwrap_or([authored.static_friction, authored.dynamic_friction]);
            let upright = blocks.upright_pair.unwrap_or(false) && up_y > self.upright_cos;
            if upright { blocks.free_upright.unwrap_or(free) } else { free }
        };
        RetailContactMaterial { static_friction, dynamic_friction, restitution }
    }
}

/// The Move Object part of a HELD_PROP line (`stick=[x, z, rot]`: left stick
/// X / Z and right stick X as OB_ObjectMv intents).
pub(crate) fn move_fields(mv: Option<MoveDiagnostics>) -> String {
    match mv {
        Some(m) => format!(
            "stick=[{:.2}, {:.2}, {:.2}] command=[{:.2}, {:.2}] yaw_cmd={:.2} lever={:.2} rot={:.2} blocked={} drift={:.3} yaw_rate={:.3}",
            m.stick[0], m.stick[1], m.stick[2], m.linear[0], m.linear[2], m.yaw, m.lever, m.rotation, m.blocked, m.drift, m.yaw_rate
        ),
        None => "stick=none".into(),
    }
}

/// Seconds a released prop keeps logging HELD_PROP lines.
const RELEASE_LOG_SECONDS: f32 = 3.0;
/// HELD_PROP lines per second with trace-all (1 otherwise): a short push, pull, side step or
/// turn of a held prop lasts well under a second.
const HELD_PROP_HZ_TRACE_ALL: u64 = 5;
/// Minimum seconds between two PROP_BELOW_GROUND lines for one prop.
const BELOW_GROUND_LOG_SECONDS: f32 = 10.0;

/// One body's ground relation, for the diagnostics and tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropGroundProbe {
    /// Box centre.
    pub center: Vector3,
    /// World-space half height of the (possibly tilted) box.
    pub half_height: f32,
    /// Height of the first static surface under the body, probed from above
    /// its last rest height; `None` if nothing is under it.
    pub ground: Option<f32>,
}

impl PropGroundProbe {
    /// Box bottom minus the ground under it (negative = inside or below the floor).
    pub(crate) fn gap(&self) -> Option<f32> {
        self.ground.map(|g| self.center.y - self.half_height - g)
    }

    /// The centre is more than its half height below the floor: the box has
    /// passed the floor's face and the one-sided triangle fixup can no longer
    /// push it back up (see `step_with_actors`).
    pub(crate) fn below_ground(&self) -> bool {
        self.ground.is_some_and(|g| g - self.center.y > self.half_height)
    }
}

pub(crate) struct PropDynamics {
    bodies: Vec<PropBody>,
    by_id: std::collections::HashMap<u32, usize>,
    /// skate_vita (performance): the indices of the bodies streamed in (not `dormant`), ascending. The per-step
    /// loops that skip dormant bodies walk these instead of every body: DownTown streams ~50 of 786 in, and touching
    /// the others' bodies (large records) cost the Vita about 1 ms a tick.
    live: Vec<usize>,
    /// skate_vita (performance): the map props (every body `new` builds; mid-game bodies are `runtime` and map
    /// bodies are never removed), for [`Self::dmo_placement_count`], which the census reads every tick.
    map_bodies: usize,
    simulation: RetailSimulationStep,
    pair: PrimitivePairSettings,
    /// Prop currently carried: exempt from skater pushes and rebake.
    held: Option<u32>,
    tuning: PropTuningTable,
    stats: PropStepStats,
    /// Steps taken (diagnostics clock; deterministic, plain data).
    tick: u64,
    /// Prop released from a carry and the tick until which it keeps logging.
    released: Option<(u32, u64)>,
    /// Last tick a PROP_BELOW_GROUND line was written per prop id.
    below_logged: std::collections::BTreeMap<u32, u64>,
    /// Move Object values of the held prop for HELD_PROP.
    move_diagnostics: Option<MoveDiagnostics>,
    /// How Move Object commands reach a body (retail defaults, mod knobs).
    move_rules: MoveCommandRules,
    /// Next id [`PropDynamics::next_runtime_id`] tries.
    next_runtime_id: u32,
    /// Next grab slot ([`PropBody::grab_slot`]) for a body created mid-game.
    next_grab_slot: u32,
    /// skate_vita (performance): changes whenever a body's collision type can change (bodies added or removed, the
    /// tuning table replaced: [`collision_type`] reads the mass and `walkable_mass`).
    collision_type_epoch: u64,
    /// skate_vita (performance): changes whenever a body's pose or spawn pose can change (integration, teleport,
    /// placement, seating, tuning, bodies added or removed): the ped seats on props follow the poses only then.
    pose_epoch: u64,
    /// skate_vita (performance): changes with every `set_grab_splines` (the grab-scene splines are rebuilt).
    grab_generation: u32,
    /// Better Object Mover (PC overlay, not retail; user 2026-10-20): props without authored grab splines get
    /// generated ones, so every prop can be moved. Off = retail (only props with authored splines).
    better_object_mover: bool,
    /// Grind rails per template id, template space (`native-props/<map>.json` `grind_splines`); looked up by
    /// template at query time, so runtime copies of a template carry them too.
    grind_rails: std::collections::BTreeMap<String, std::sync::Arc<[crate::grind_world::props::PropRail]>>,
    /// The prop the local skater grinds now (`set_grinded`): the skater's volumes do not push it.
    grinded: Option<u32>,
    /// skate_vita (performance): the bodies with grind rails, in id order (the order of
    /// [`Self::rail_poses`]), for the rail queries; kept by [`Self::reorder`] when bodies or rails change.
    rail_bodies: Vec<usize>,
    /// skate_vita (performance): [`Self::rail_bodies`] that are streamed in (not `dormant`), the same order; the
    /// rail queries skip the dormant ones without touching them.
    live_rail_bodies: Vec<usize>,
    /// Pushers (actor ids, [`LOCAL_PUSHER`] for the local skater) bailing this tick (`set_bailing`): their contacts
    /// apply the bail clamp.
    bailing: Vec<u64>,
    /// Pushers that are not a 75 kg skater (`set_actor_push_mass`): actor id, kg. Peds' body A (10 kg, b342).
    actor_push_mass: Vec<(u64, f32)>,
}

/// First id of bodies created mid-game: ids from here up to [`PROP_GRAB_TAG`]
/// stay clear of the grab-scene tags (props `0x4000_0000`, cars the high bit).
/// Ours: retail DMOs are addressed by manager slot, not by such an id.
pub(crate) const RUNTIME_PROP_ID_BASE: u32 = 0x2000_0000;

/// A prop body created mid-game ([`PropDynamics::spawn_body`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RuntimeProp {
    /// Prop type key (template name), as a map prop's MOBJ name: picks the
    /// prop tuning and is logged.
    pub template: String,
    /// Template-space collision triangles, scale folded in; the body box is
    /// their bounds.
    pub local: Vec<[Vector3; 3]>,
    /// Authored physics block (density, damping, friction, restitution, gravity scale, sleep).
    pub physics: skate_data::skate_map::ObjectPhysics,
    /// Retail per-type data of the template, when known.
    pub type_data: Option<DmoType>,
    /// Template origin and rotation at creation.
    pub origin: Vector3,
    pub basis: Basis3,
    pub linear_velocity: Vector3,
    pub angular_velocity: Vector3,
}

/// Move Object values logged on HELD_PROP lines.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MoveDiagnostics {
    /// OB_ObjectMvX / Z / Rot.
    pub stick: [f32; 3],
    pub linear: [f32; 3],
    pub yaw: f32,
    pub lever: f32,
    pub rotation: f32,
    pub blocked: bool,
    pub drift: f32,
    /// Measured yaw rate the yaw controller tracks (82D45318 +1192 sign kept).
    pub yaw_rate: f32,
}

/// 82C4C4E0, our prop pass of the shared rail query: props in id order (retail walks its DMO hash table; NOT
/// RETAIL ORDER), each placed by its last stepped pose (`grind_world::props::PosedRails`).
impl crate::grind_world::RailSource for PropDynamics {
    fn append_rails(&self, min: [f32; 3], max: [f32; 3], skip: Option<u32>, rules: &crate::grind_world::RailRules, out: &mut Vec<crate::grind_world::RailHit>) {
        if self.grind_rails.is_empty() {
            return;
        }
        // skate_vita (performance): the bodies with rails in id order (`rail_poses`' order) tested in place,
        // the distance reject of `PosedRails::append` (same expression) first; upstream built and sorted every
        // prop's posed rails for each query.
        for &index in &self.live_rail_bodies {
            let b = &self.bodies[index];
            if Some(b.id) == skip {
                continue;
            }
            let o = b.origin();
            let origin = [o.x, o.y, o.z];
            let d: [f32; 3] = std::array::from_fn(|i| origin[i] - (min[i] + max[i]) * 0.5);
            if !(d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < rules.prop_radius * rules.prop_radius) {
                continue;
            }
            let Some(rails) = self.rails_of(b) else { continue };
            let s = b.axis_scale;
            let posed = crate::grind_world::props::PosedRails { id: b.id, tag: crate::grind_world::props::PROP_OWNER_TAG, origin, basis: b.rates.basis.columns, scale: [s.x, s.y, s.z], rails: rails.clone() };
            if !posed.append(min, max, rules, out) {
                return;
            }
        }
    }
    fn describe(&self, id: u32) -> Option<String> {
        self.template_of(id).map(|_| self.describe_template(id))
    }
}

fn mul_basis(basis: Basis3, v: Vector3) -> Vector3 {
    Vector3::new(
        basis.columns[0][0] * v.x + basis.columns[1][0] * v.y + basis.columns[2][0] * v.z,
        basis.columns[0][1] * v.x + basis.columns[1][1] * v.y + basis.columns[2][1] * v.z,
        basis.columns[0][2] * v.x + basis.columns[1][2] * v.y + basis.columns[2][2] * v.z,
    )
}

fn cross(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn dot(a: Vector3, b: Vector3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn add(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}

fn sub(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn scale(v: Vector3, s: f32) -> Vector3 {
    Vector3::new(v.x * s, v.y * s, v.z * s)
}

fn length(v: Vector3) -> f32 {
    dot(v, v).sqrt()
}

fn mul_components(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}

/// Shepperd's method; the basis is a pure rotation by construction.
fn quaternion_from_basis(basis: Basis3) -> RetailQuaternion {
    let m = |c: usize, r: usize| basis.columns[c][r];
    let trace = m(0, 0) + m(1, 1) + m(2, 2);
    let (x, y, z, w) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        (
            (m(1, 2) - m(2, 1)) / s,
            (m(2, 0) - m(0, 2)) / s,
            (m(0, 1) - m(1, 0)) / s,
            0.25 * s,
        )
    } else if m(0, 0) > m(1, 1) && m(0, 0) > m(2, 2) {
        let s = (1.0 + m(0, 0) - m(1, 1) - m(2, 2)).sqrt() * 2.0;
        (
            0.25 * s,
            (m(1, 0) + m(0, 1)) / s,
            (m(2, 0) + m(0, 2)) / s,
            (m(1, 2) - m(2, 1)) / s,
        )
    } else if m(1, 1) > m(2, 2) {
        let s = (1.0 + m(1, 1) - m(0, 0) - m(2, 2)).sqrt() * 2.0;
        (
            (m(1, 0) + m(0, 1)) / s,
            0.25 * s,
            (m(2, 1) + m(1, 2)) / s,
            (m(2, 0) - m(0, 2)) / s,
        )
    } else {
        let s = (1.0 + m(2, 2) - m(0, 0) - m(1, 1)).sqrt() * 2.0;
        (
            (m(2, 0) + m(0, 2)) / s,
            (m(2, 1) + m(1, 2)) / s,
            0.25 * s,
            (m(0, 1) - m(1, 0)) / s,
        )
    };
    let inverse = 1.0 / (x * x + y * y + z * z + w * w).sqrt().max(1e-20);
    RetailQuaternion {
        x: x * inverse,
        y: y * inverse,
        z: z * inverse,
        w: w * inverse,
    }
}

impl PropBody {
    fn box_primitive(&self) -> ContactPrimitive {
        ContactPrimitive::RoundedBox {
            center: self.rates.position,
            basis: self.rates.basis,
            half_extents: self.half_extents,
            radius: 0.0,
        }
    }

    fn bounds(&self) -> skate_core::physics::board_world::query_metadata::Bounds {
        let half = [self.half_extents.x, self.half_extents.y, self.half_extents.z];
        let extent = |axis: usize| {
            self.rates
                .basis
                .columns
                .iter()
                .zip(half)
                .map(|(column, h)| h * column[axis].abs())
                .sum::<f32>()
        };
        let (ex, ey, ez) = (extent(0), extent(1), extent(2));
        let c = self.rates.position;
        skate_core::physics::board_world::query_metadata::Bounds {
            min: Vector3::new(c.x - ex, c.y - ey, c.z - ez),
            max: Vector3::new(c.x + ex, c.y + ey, c.z + ez),
        }
    }

    fn wake(&mut self) {
        self.asleep = false;
        self.rates.cool_down = 0;
    }

    /// Impulse applied at a world point, directly to velocities. Returns the
    /// impulse vector for the other body / wake accounting.
    fn apply_impulse(&mut self, impulse: Vector3, point: Vector3) -> Vector3 {
        self.wake();
        self.rates.linear_velocity = add(
            self.rates.linear_velocity,
            scale(impulse, self.inertia.inverse_mass),
        );
        let r = sub(point, self.rates.position);
        let angular = cross(r, impulse);
        let delta = mul_basis(self.rates.world_inverse_inertia, angular);
        self.rates.angular_velocity = add(self.rates.angular_velocity, delta);
        impulse
    }

    fn velocity_at(&self, point: Vector3) -> Vector3 {
        add(
            self.rates.linear_velocity,
            cross(self.rates.angular_velocity, sub(point, self.rates.position)),
        )
    }

    /// Template-origin pose for rendering and collision rebake.
    fn origin(&self) -> Vector3 {
        sub(
            self.rates.position,
            mul_basis(self.rates.basis, self.local_center),
        )
    }
}

/// Pose tolerance for "moved" (metres / basis component). Ours: float noise
/// of a body that slept at its authored pose; retail's per-DMO moved flag is a
/// record field (sub_826666A8 reads it), not a distance. NOT RETAIL YET.
const SPAWN_POSE_EPSILON: f32 = 1e-4;

/// skate_vita: a contact normal at least this upright is the skater riding or standing on a
/// prop (`step_with_actors`), not pushing it (cos 45°).
const SUPPORT_NORMAL_Y: f32 = 0.7071;

/// skate_vita: the largest gap between a prop and the floor [`PropDynamics::seat`] closes.
const SEAT_REACH: f32 = 0.06;

impl PropDynamics {
    /// One box body per collision-layer instance, asleep at its authored pose.
    /// Placement rows are the world images of the local axes; their lengths
    /// are the constant per-axis scale folded into the box extents. Density,
    /// damping, friction, restitution and sleep flags come from the authored
    /// MOBJ physics block (`ObjectPhysics`).
    pub(crate) fn new(
        objects: &[skate_data::skate_map::StaticObject],
        instances: &[crate::skate_world::PropCollisionInstance],
        simulation: RetailSimulationStep,
    ) -> Self {
        let mut bodies = Vec::new();
        let mut by_id = std::collections::HashMap::new();
        for (index, entry) in instances.iter().enumerate() {
            let object = &objects[entry.object];
            let Some(mut body) = Self::build_body(index, entry.id, &object.name, entry.local_points(), object.physics, &object.transform, simulation) else {
                continue;
            };
            body.grab_slot = bodies.len() as u32;
            bodies.push(body);
            by_id.insert(entry.id, bodies.len() - 1);
        }
        let next_grab_slot = bodies.len() as u32;
        let live = (0..bodies.len()).collect();
        let map_bodies = bodies.len();
        let mut dynamics = Self {
            bodies,
            by_id,
            live,
            map_bodies,
            next_grab_slot,
            collision_type_epoch: 0,
            pose_epoch: 0,
            grab_generation: 0,
            better_object_mover: false,
            simulation,
            pair: PrimitivePairSettings {
                padding_a: PropTuning::default().contact_padding,
                padding_b: PropTuning::default().contact_padding,
                additional_padding: 0.0,
                edge_cos_bend_normal_threshold: 0.999,
                convexity_epsilon: 0.01,
            },
            held: None,
            tuning: PropTuningTable::default(),
            stats: PropStepStats::default(),
            tick: 0,
            released: None,
            below_logged: std::collections::BTreeMap::new(),
            move_diagnostics: None,
            move_rules: MoveCommandRules::default(),
            next_runtime_id: RUNTIME_PROP_ID_BASE,
            grind_rails: std::collections::BTreeMap::new(),
            grinded: None,
            rail_bodies: Vec::new(),
            live_rail_bodies: Vec::new(),
            bailing: Vec::new(),
            actor_push_mass: Vec::new(),
        };
        dynamics.reorder();
        dynamics
    }

    /// skate_vita: recomputes [`Self::rail_bodies`] (bodies added or removed, rails set).
    fn reorder(&mut self) {
        let mut order: Vec<usize> = (0..self.bodies.len()).filter(|&i| self.rails_of(&self.bodies[i]).is_some()).collect();
        order.sort_by_key(|&i| self.bodies[i].id);
        self.live_rail_bodies = order.iter().copied().filter(|&i| !self.bodies[i].dormant).collect();
        self.rail_bodies = order;
    }

    /// No bodies yet (a map without placed props, which can still get props
    /// created mid-game).
    pub(crate) fn empty(simulation: RetailSimulationStep) -> Self {
        Self::new(&[], &[], simulation)
    }

    /// A free id for a body created mid-game: the first unused id from
    /// [`RUNTIME_PROP_ID_BASE`] up, in creation order (deterministic, so a host
    /// and its peers agree when the host hands them out).
    pub(crate) fn next_runtime_id(&mut self) -> u32 {
        let after = |id: u32| if id + 1 >= PROP_GRAB_TAG { RUNTIME_PROP_ID_BASE } else { id + 1 };
        let mut id = self.next_runtime_id.max(RUNTIME_PROP_ID_BASE);
        while self.by_id.contains_key(&id) {
            id = after(id);
        }
        self.next_runtime_id = after(id);
        id
    }

    /// Create a body mid-game (retail: the DMO manager creates a physics DMO for
    /// a released hand prop) on collision instance `instance` (added with
    /// `PropCollisionLayer::add_instance` at the same pose). Awake, moving at the
    /// spec's velocities, with the prop type's tuning and type data like a map
    /// prop. Returns false when the triangles give no body (no volume) or `id`
    /// is taken.
    pub(crate) fn spawn_body(&mut self, instance: usize, id: u32, spec: &RuntimeProp) -> bool {
        if self.by_id.contains_key(&id) {
            return false;
        }
        let c = spec.basis.columns;
        let t = [
            c[0][0], c[0][1], c[0][2], c[1][0], c[1][1], c[1][2], c[2][0], c[2][1], c[2][2],
            spec.origin.x, spec.origin.y, spec.origin.z,
        ];
        let Some(mut body) = Self::build_body(instance, id, &spec.template, &spec.local, spec.physics, &t, self.simulation)
        else {
            return false;
        };
        body.runtime = true;
        if self.better_object_mover {
            body.grab_splines = generated_grab_splines(&body);
        }
        body.grab_slot = self.next_grab_slot;
        self.next_grab_slot = self.next_grab_slot.wrapping_add(1);
        if self.better_object_mover {
            body.grab_scene_splines = Self::grab_scene_splines_of(&body);
            self.grab_generation = self.grab_generation.wrapping_add(1);
        }
        body.type_data = spec.type_data.clone();
        body.rates.linear_velocity = spec.linear_velocity;
        body.rates.angular_velocity = spec.angular_velocity;
        let tuning = *self.tuning.for_template(&spec.template);
        Self::apply_tuning(&mut body, tuning);
        body.asleep = false;
        self.bodies.push(body);
        self.collision_type_epoch += 1;
        self.pose_epoch += 1;
        let index = self.bodies.len() - 1;
        self.live.push(index);
        self.by_id.insert(id, index);
        self.reorder();
        let inertia = self.body_inertia(index);
        let body = &mut self.bodies[index];
        body.inertia = inertia;
        body.rates.world_inverse_inertia = world_inverse_inertia(body.rates.basis, inertia.inverse_tensor);
        true
    }

    /// Remove a body created mid-game (retail removes the released hand prop's
    /// DMO). Map props are never removed. Returns its collision instance for
    /// `PropCollisionLayer::retire_instance`.
    pub(crate) fn remove_body(&mut self, id: u32) -> Option<usize> {
        let index = *self.by_id.get(&id)?;
        if !self.bodies[index].runtime {
            return None;
        }
        if self.held == Some(id) {
            self.set_held(None);
        }
        let body = self.bodies.remove(index);
        self.collision_type_epoch += 1;
        self.pose_epoch += 1;
        self.by_id = self.bodies.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        self.live = (0..self.bodies.len()).filter(|&i| !self.bodies[i].dormant).collect();
        self.reorder();
        if self.released.is_some_and(|(r, _)| r == id) {
            self.released = None;
        }
        self.below_logged.remove(&id);
        Some(body.instance)
    }

    /// A [`RuntimeProp`] copying body `id`'s template, physics block and type
    /// data, at its current pose and at rest; `local` is left empty for the
    /// caller to fill from the collision layer.
    pub(crate) fn copy_spec(&self, id: u32) -> Option<RuntimeProp> {
        let body = self.bodies.get(*self.by_id.get(&id)?)?;
        Some(RuntimeProp {
            template: body.template.clone(),
            local: Vec::new(),
            physics: body.physics,
            type_data: body.type_data.clone(),
            origin: body.origin(),
            basis: body.rates.basis,
            linear_velocity: Vector3::ZERO,
            angular_velocity: Vector3::ZERO,
        })
    }

    /// Whether `id` is a body created mid-game.
    pub(crate) fn is_runtime(&self, id: u32) -> bool {
        self.by_id.get(&id).is_some_and(|&i| self.bodies[i].runtime)
    }

    /// One box body for a prop instance asleep at its placement `t` (row-vector
    /// affine: rows are the world images of the local axes, their lengths the
    /// per-axis scale folded into the box extents) with the authored physics block.
    /// `None` for an instance without triangles or mass. Shared by the map load and
    /// [`Self::spawn_body`].
    #[allow(clippy::too_many_arguments)]
    fn build_body(
        instance: usize,
        id: u32,
        template: &str,
        local_points: &[[Vector3; 3]],
        authored: skate_data::skate_map::ObjectPhysics,
        t: &[f32; 12],
        simulation: RetailSimulationStep,
    ) -> Option<PropBody> {
        let axis_scale = [
            (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt(),
            (t[3] * t[3] + t[4] * t[4] + t[5] * t[5]).sqrt(),
            (t[6] * t[6] + t[7] * t[7] + t[8] * t[8]).sqrt(),
        ];
        let basis = Basis3 {
            columns: [
                [t[0] / axis_scale[0], t[1] / axis_scale[0], t[2] / axis_scale[0]],
                [t[3] / axis_scale[1], t[4] / axis_scale[1], t[5] / axis_scale[1]],
                [t[6] / axis_scale[2], t[7] / axis_scale[2], t[8] / axis_scale[2]],
            ],
        };
        let first = local_points.first()?[0];
        let mut min = first;
        let mut max = first;
        for point in local_points.iter().flatten() {
            min = Vector3::new(min.x.min(point.x), min.y.min(point.y), min.z.min(point.z));
            max = Vector3::new(max.x.max(point.x), max.y.max(point.y), max.z.max(point.z));
        }
        let local_center = scale(add(min, max), 0.5);
        let half_extents = scale(sub(max, min), 0.5);
        let volume = 8.0 * half_extents.x * half_extents.y * half_extents.z;
        let properties = primitive_mass_properties(
            skate_core::physics::mass::PartMassInput {
                shape: skate_core::physics::mass::MassShape::RoundedBox {
                    half_extents,
                    radius: 0.0,
                },
                requested_mass: volume * authored.density.max(0.001),
            },
            RETAIL_UNBOUNDED_VELOCITY,
            authored.angular_damping,
        )?;
        let mut inertia = properties.dynamics;
        inertia.linear_drag = authored.linear_damping;
        inertia.maximum_linear_velocity = RETAIL_UNBOUNDED_VELOCITY;
        let origin = Vector3::new(t[9], t[10], t[11]);
        let center = add(origin, mul_basis(basis, local_center));
        Some(PropBody {
            instance,
            id,
            grab_slot: 0,
            local_center,
            half_extents,
            template: template.to_string(),
            type_data: None,
            grab_splines: Vec::new(),
            grab_scene_splines: Vec::new(),
            authored_grab: false,
            tuning: PropTuning::default(),
            authored_center: local_center,
            authored_half_extents: half_extents,
            axis_scale: Vector3::new(axis_scale[0], axis_scale[1], axis_scale[2]),
            stuck_ticks: 0,
            pushed_by: None,
            baked: None,
            contacts: 0,
            rest_y: center.y,
            spawn_origin: origin,
            spawn_basis: basis,
            commanded: false,
            commanded_block: false,
            upright_timer: None,
            rates: RetailBodyRates {
                orientation: quaternion_from_basis(basis),
                basis,
                world_inverse_inertia: world_inverse_inertia(basis, inertia.inverse_tensor),
                position: center,
                linear_velocity: Vector3::ZERO,
                angular_velocity: Vector3::ZERO,
                force_acceleration: scale(
                    simulation.gravity_acceleration,
                    authored.gravity_scale,
                ),
                torque_acceleration: Vector3::ZERO,
                kinetic_energy: 0.0,
                cool_down: simulation.cool_down,
            },
            inertia,
            authored_inertia: inertia,
            material: RetailContactMaterial {
                static_friction: authored.friction,
                dynamic_friction: authored.friction,
                restitution: authored.restitution,
            },
            enable_sleep: authored.enable_sleep,
            asleep: !authored.initially_awake,
            dormant: false,
            runtime: false,
            seated: false,
            physics: authored,
        })
    }

    /// Move Object command rules in effect.
    pub(crate) fn move_rules(&self) -> &MoveCommandRules {
        &self.move_rules
    }

    /// Replace the Move Object command rules (mod tuning; `Default` restores
    /// retail on mod disable).
    pub(crate) fn set_move_rules(&mut self, rules: MoveCommandRules) {
        self.move_rules = rules;
        self.refresh_inertia();
    }

    /// Body `index`'s own material block: what retail copies into its
    /// collision objects (CO +116..+124) before the pair combine.
    fn body_material(&self, index: usize) -> RetailContactMaterial {
        let body = &self.bodies[index];
        self.move_rules.typed_body_material(&self.body_blocks(body), body.material, body.commanded_block, body.rates.basis.columns[1][1])
    }

    /// Effective type blocks of a body: mod entry over its retail type data.
    fn body_blocks(&self, body: &PropBody) -> PropMaterialBlocks {
        let (key, retail) = match &body.type_data {
            Some(t) => (Some(t.key.as_str()), t.blocks),
            None => (None, PropMaterialBlocks::default()),
        };
        self.move_rules.blocks(&body.template, key, &retail)
    }

    /// Body `index`'s Inertia (82C4E568, filled once when retail builds the
    /// body and never switched by the commanded block): with type data, mass
    /// (data +304), velocity caps (+292 / +296), drag (+308 / +336) and the box
    /// inertia of the AABB half extents x +16 + +32 ([`dmo_body_inertia`]); a
    /// mod's `carry.by_template` fields over it. Fields neither sets keep the
    /// authored MOBJ values (density mass, box inertia, damping, no caps: NOT
    /// RETAIL YET, only without type data). Pure function of the type data,
    /// the mod rules and the authored box, so every peer gets the same.
    fn body_inertia(&self, index: usize) -> RetailInertiaDynamics {
        let body = &self.bodies[index];
        let blocks = self.body_blocks(body);
        let authored = body.authored_inertia;
        let mut inertia = if blocks.mass.is_some() || blocks.inertia_scale.is_some() || blocks.inertia_offset.is_some() {
            let v = |a: [f32; 3]| Vector3::new(a[0], a[1], a[2]);
            dmo_body_inertia(
                body.authored_half_extents,
                DmoBodyData {
                    mass: blocks.mass.unwrap_or(1.0 / authored.inverse_mass),
                    maximum_linear_velocity: authored.maximum_linear_velocity,
                    maximum_angular_velocity: authored.maximum_angular_velocity,
                    linear_drag: authored.linear_drag,
                    angular_drag: authored.angular_drag,
                    inertia_scale: v(blocks.inertia_scale.unwrap_or(DMO_DEFAULT_INERTIA_SCALE)),
                    inertia_offset: v(blocks.inertia_offset.unwrap_or([0.0; 3])),
                },
            )
        } else {
            authored
        };
        if let Some(cap) = blocks.maximum_linear_velocity {
            inertia.maximum_linear_velocity = cap;
        }
        if let Some(cap) = blocks.maximum_angular_velocity {
            inertia.maximum_angular_velocity = cap;
        }
        if let Some(drag) = blocks.linear_drag {
            inertia.linear_drag = drag;
        }
        if let Some(drag) = blocks.angular_drag {
            inertia.angular_drag = drag;
        }
        inertia
    }

    /// Store every body's effective Inertia ([`Self::body_inertia`]) and its
    /// world inverse inertia: after type data or mod rules change (mod
    /// disable restores retail through `MoveCommandRules::default()`).
    fn refresh_inertia(&mut self) {
        for index in 0..self.bodies.len() {
            let inertia = self.body_inertia(index);
            let body = &mut self.bodies[index];
            body.inertia = inertia;
            body.rates.world_inverse_inertia = world_inverse_inertia(body.rates.basis, inertia.inverse_tensor);
        }
    }

    /// Attach the retail per-type data, keyed by template id (the MOBJ name's
    /// part before '/'). Bodies whose template is not in `types` keep their
    /// authored material. Returns how many bodies got type data.
    pub(crate) fn set_type_data(&mut self, types: &std::collections::BTreeMap<String, DmoType>) -> usize {
        let mut count = 0;
        for body in &mut self.bodies {
            let id = body.template.split('/').next().unwrap_or_default();
            body.type_data = types.get(id).cloned();
            count += usize::from(body.type_data.is_some());
        }
        self.refresh_inertia();
        count
    }

    /// The authored grab splines per template id (`native-props/<map>.json` `grab_splines`). Returns the props that
    /// got splines.
    pub(crate) fn set_grab_splines(&mut self, splines: &std::collections::BTreeMap<String, Vec<crate::living_world::vehicles::CarGrabSpline>>) -> usize {
        let mut count = 0;
        let better = self.better_object_mover;
        for body in &mut self.bodies {
            let id = body.template.split('/').next().unwrap_or_default();
            body.grab_splines = splines.get(id).cloned().unwrap_or_default();
            body.authored_grab = !body.grab_splines.is_empty();
            count += usize::from(body.authored_grab);
            if better && !body.authored_grab {
                body.grab_splines = generated_grab_splines(body);
            }
            body.grab_scene_splines = Self::grab_scene_splines_of(body);
            self.grab_generation = self.grab_generation.wrapping_add(1);
        }
        count
    }

    pub(crate) fn better_object_mover(&self) -> bool {
        self.better_object_mover
    }

    /// Better Object Mover on / off: props without authored grab splines get generated ones / lose them. Returns how
    /// many props have generated splines now.
    pub(crate) fn set_better_object_mover(&mut self, on: bool) -> usize {
        self.better_object_mover = on;
        let mut generated = 0;
        for body in self.bodies.iter_mut().filter(|b| !b.authored_grab) {
            body.grab_splines = if on { generated_grab_splines(body) } else { Vec::new() };
            body.grab_scene_splines = Self::grab_scene_splines_of(body);
            generated += usize::from(!body.grab_splines.is_empty());
        }
        self.grab_generation = self.grab_generation.wrapping_add(1);
        generated
    }

    /// The grind rails per template id (`native-props/<map>.json` `grind_splines`). Returns the props that got
    /// rails.
    pub(crate) fn set_grind_rails(&mut self, rails: std::collections::BTreeMap<String, Vec<crate::grind_world::props::PropRail>>) -> usize {
        self.grind_rails = rails.into_iter().filter(|(_, r)| !r.is_empty()).map(|(k, r)| (k, r.into())).collect();
        self.reorder();
        self.bodies.iter().filter(|b| self.rails_of(b).is_some()).count()
    }

    /// The prop the local skater grinds this tick (none when not grinding a prop).
    pub(crate) fn set_grinded(&mut self, id: Option<u32>) {
        self.grinded = id;
    }

    /// The pushers bailing this tick (retail's bail clamp gate; ours: the wipeout state, `board_wiping_out`).
    pub(crate) fn set_bailing(&mut self, actors: &[u64]) {
        self.bailing.clear();
        self.bailing.extend_from_slice(actors);
    }

    /// The push mass of actors that are not a skater (actor id, kg), for this tick's step; any other actor pushes with
    /// the prop's `skater_push_mass`.
    pub(crate) fn set_actor_push_mass(&mut self, actors: &[(u64, f32)]) {
        self.actor_push_mass.clear();
        self.actor_push_mass.extend_from_slice(actors);
    }

    fn rails_of(&self, body: &PropBody) -> Option<&std::sync::Arc<[crate::grind_world::props::PropRail]>> {
        self.grind_rails.get(body.template.split('/').next().unwrap_or_default())
    }

    /// Every awake or resting (not streamed-out) prop with rails at its last stepped pose, id order, without
    /// `skip` (the carried prop).
    pub(crate) fn rail_poses(&self, skip: Option<u32>) -> crate::grind_world::props::PropRailPoses {
        // skate_vita (performance): the bodies with rails, already in id order (upstream scanned and sorted
        // every body each call).
        let out: Vec<_> = self
            .live_rail_bodies
            .iter()
            .map(|&i| &self.bodies[i])
            .filter(|b| Some(b.id) != skip)
            .filter_map(|b| {
                let rails = self.rails_of(b)?.clone();
                let o = b.origin();
                let s = b.axis_scale;
                Some(crate::grind_world::props::PosedRails { id: b.id, tag: crate::grind_world::props::PROP_OWNER_TAG, origin: [o.x, o.y, o.z], basis: b.rates.basis.columns, scale: [s.x, s.y, s.z], rails })
            })
            .collect();
        crate::grind_world::props::PropRailPoses(out)
    }

    /// Every prop with grab splines as a grab-scene object of the world-object provider (scene `+4084`, query mode
    /// bit 0x04; single-player worlds use it, not the DMO provider, b54): template origin and basis as the frame,
    /// points scaled like the body, linear velocity as the record vector, type-2 records. Ids: object
    /// `PROP_GRAB_TAG | grab slot`, spline / geometry `PROP_GRAB_TAG | grab slot << 6 | index`
    /// ([`Self::prop_of_grab_key`] maps a key back to the prop id). NOT RETAIL YET: the assembly is a stand-in
    /// with the object id (the `+172` object is not identified).
    pub(crate) fn grab_objects(&self) -> Vec<skate_core::player::offboard::grab_scene::Object> {
        self.live.iter().map(|&i| &self.bodies[i]).filter_map(Self::grab_object_of).collect()
    }

    /// One prop's grab-scene object (see [`Self::grab_objects`]) from its current pose; `None` without
    /// authored grab splines. Move Object rebuilds the held record from it every tick (82D44A10 path A).
    pub(crate) fn grab_object(&self, id: u32) -> Option<skate_core::player::offboard::grab_scene::Object> {
        Self::grab_object_of(self.bodies.get(*self.by_id.get(&id)?)?)
    }

    /// The prop id of a grab-scene object or spline key (`PROP_GRAB_TAG | slot << 6 | index`, or the object id
    /// `PROP_GRAB_TAG | slot` with `spline_key` false); `None` for a key that is not a present prop's.
    pub(crate) fn prop_of_grab_key(&self, key: u32, spline_key: bool) -> Option<u32> {
        if key & PROP_GRAB_TAG == 0 {
            return None;
        }
        let slot = if spline_key { (key & !PROP_GRAB_TAG) >> 6 } else { key & !PROP_GRAB_TAG };
        self.bodies.iter().find(|b| b.grab_slot == slot).map(|b| b.id)
    }

    fn grab_object_of(b: &PropBody) -> Option<skate_core::player::offboard::grab_scene::Object> {
        use skate_core::player::offboard::grab_scene::{AssemblyData, Object, PartData, Provider, RatesData};
        (!b.grab_splines.is_empty() && b.grab_slot < (1 << 24))
            .then(|| {
                let id = PROP_GRAB_TAG | b.grab_slot;
                let o = b.origin();
                let c = b.rates.basis.columns;
                Object {
                    id,
                    provider: Provider::LivingWorld,
                    disabled: false,
                    assembly_ready: true,
                    // 82585F58 (b227): the part (assembly+8) is the prop's one body, its rates (part+0x4C) the body's
                    // rigid body: words 56..65 = its Inertia (inverse tensor, inverse mass, spherical, max linear /
                    // angular velocity, drags; the tensor's w lane is not read), +144 = angular velocity, +160 =
                    // centre of mass. Move Object reads +216 only as a null gate (82D442D0 / 82D44A10), so the
                    // stable prop id serves as part and rates identity.
                    assembly: Some(AssemblyData {
                        identity: id,
                        first_part: Some(PartData {
                            identity: id,
                            coefficients_0_to_36: {
                                let (t, i) = (b.inertia.inverse_tensor, &b.inertia);
                                [t.x, t.y, t.z, 0.0, i.inverse_mass, i.spherical, i.maximum_linear_velocity, i.maximum_angular_velocity, i.linear_drag, i.angular_drag]
                            },
                            rates: Some(RatesData {
                                identity: id,
                                vector_48: [b.rates.angular_velocity.x, b.rates.angular_velocity.y, b.rates.angular_velocity.z, 0.0],
                                transform_position_48: [b.rates.position.x, b.rates.position.y, b.rates.position.z, 0.0],
                            }),
                        }),
                    }),
                    frame: [[c[0][0], c[0][1], c[0][2], 0.0], [c[1][0], c[1][1], c[1][2], 0.0], [c[2][0], c[2][1], c[2][2], 0.0], [o.x, o.y, o.z, 1.0]],
                    object_vector_128: [b.rates.linear_velocity.x, b.rates.linear_velocity.y, b.rates.linear_velocity.z, 0.0],
                    // skate_vita (performance): built once (`grab_scene_splines_of`).
                    splines: b.grab_scene_splines.clone(),
                }
            })
    }

    /// skate_vita (performance): every input of [`Self::grab_objects`] that can change, as bits, in its order (each
    /// object's grab slot, frame, velocities, rigid body position and inertia, and the splines' generation): the same
    /// signature gives the same objects, so the grab scene needs no refresh.
    pub(crate) fn grab_signature(&self, out: &mut Vec<u32>) {
        out.clear();
        out.push(self.grab_generation);
        for &i in &self.live {
            let b = &self.bodies[i];
            if b.grab_splines.is_empty() || b.grab_slot >= (1 << 24) {
                continue;
            }
            let (o, r, t, n) = (b.origin(), &b.rates, b.inertia.inverse_tensor, &b.inertia);
            out.push(b.grab_slot);
            out.extend(
                [o.x, o.y, o.z, r.linear_velocity.x, r.linear_velocity.y, r.linear_velocity.z, r.angular_velocity.x, r.angular_velocity.y, r.angular_velocity.z, r.position.x, r.position.y, r.position.z]
                    .into_iter()
                    .chain(r.basis.columns.into_iter().flatten())
                    .chain([t.x, t.y, t.z, n.inverse_mass, n.spherical, n.maximum_linear_velocity, n.maximum_angular_velocity, n.linear_drag, n.angular_drag])
                    .map(f32::to_bits),
            );
        }
    }

    /// The grab-scene splines of a body (upstream's `grab_object_of` spline list, verbatim): keys
    /// `PROP_GRAB_TAG | grab slot << 6 | index`, the points scaled like the body.
    fn grab_scene_splines_of(b: &PropBody) -> Vec<skate_core::player::offboard::grab_scene::Spline> {
        use skate_core::player::offboard::grab_scene::{Descriptor, Geometry, Spline};
        let s = b.axis_scale;
        b.grab_splines
            .iter()
            .take(64)
            .enumerate()
            .map(|(i, g)| {
                let key = PROP_GRAB_TAG | (b.grab_slot << 6) | i as u32;
                Spline {
                    descriptor: Descriptor { kind: 2, id: key },
                    geometry: std::sync::Arc::new(Geometry {
                        id: key,
                        points: g.points.iter().map(|p| [p[0] * s.x, p[1] * s.y, p[2] * s.z, 1.0]).collect(),
                        approach_vectors: vec![[g.direction[0], g.direction[1], g.direction[2], 0.0]],
                        word_60: g.flags,
                    }),
                    word_272: 0,
                }
            })
            .collect()
    }

    /// Vault record name of a body's prop type (diagnostics, mod keys).
    pub(crate) fn type_key(&self, id: u32) -> Option<&str> {
        self.bodies.get(*self.by_id.get(&id)?)?.type_data.as_ref().map(|t| t.key.as_str())
    }

    /// Contact material of body `index` against a surface material: the
    /// retail pair combine 82763078 of the body's block and the other side.
    fn contact_material(&self, index: usize, other: RetailContactMaterial) -> RetailContactMaterial {
        combine_contact_materials(self.body_material(index), other)
    }

    /// Current tuning table (defaults plus per prop type overrides).
    pub(crate) fn tuning(&self) -> &PropTuningTable {
        &self.tuning
    }

    /// Replace the tuning table and re-resolve every body. A changed
    /// `collision_box` moves the box centre so the template origin (the
    /// rendered pose) stays where it is; mass properties are unchanged.
    pub(crate) fn set_tuning(&mut self, table: PropTuningTable) {
        for body in &mut self.bodies {
            let tuning = *table.for_template(&body.template);
            // Untouched prop types stay asleep: a mod changing one type must
            // not wake (and rebake) every prop on the map.
            if tuning == body.tuning {
                continue;
            }
            Self::apply_tuning(body, tuning);
            body.wake();
        }
        self.tuning = table;
        self.collision_type_epoch += 1;
        self.pose_epoch += 1;
    }

    /// Give one body its prop type's tuning: `collision_box` moves the box
    /// centre so the template origin stays where it is.
    fn apply_tuning(body: &mut PropBody, tuning: PropTuning) {
        let origin = body.origin();
        let (center, half) = match tuning.collision_box {
            Some(override_box) => (
                mul_components(override_box.center, body.axis_scale),
                mul_components(override_box.half_extents, body.axis_scale),
            ),
            None => (body.authored_center, body.authored_half_extents),
        };
        body.local_center = center;
        body.half_extents = Vector3::new(half.x.abs(), half.y.abs(), half.z.abs());
        body.rates.position = add(origin, mul_basis(body.rates.basis, center));
        body.tuning = tuning;
        body.stuck_ticks = 0;
    }

    /// Restore the shipped defaults (mod disable).
    /// (Mod API entry point; the Lua binding is a follow-up.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn reset_tuning(&mut self) {
        self.set_tuning(PropTuningTable::default());
    }

    /// Stable actor id of the skater that last pushed prop `id` ([`LOCAL_PUSHER`] or an NPC
    /// skater proxy id); `None` if nothing has pushed it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn pushed_by(&self, id: u32) -> Option<u64> {
        self.bodies.get(*self.by_id.get(&id)?)?.pushed_by
    }

    /// Work done by the last `step`.
    /// (Mod API entry point; the Lua binding is a follow-up.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn last_step_stats(&self) -> PropStepStats {
        self.stats
    }

    /// Prop volume against static world triangles: the physics/world query
    /// (TU3 8277B720 via triangle dispatch 8277BC58), NOT the GP volume-pair
    /// query 82AD43A8. The world query forwards the query context's object
    /// byte (+61) into triangle fixup (82AD3130), which for objects accepts a
    /// contact across a welded flat edge (cosine 1, no convex bit) while the
    /// normal stays within convexity_epsilon of the face (acos(1 - 0.01) =
    /// 8.1 deg) and does not reject disabled vertices. The pair query passes
    /// false and drops every such contact: a box lying a few degrees tilted
    /// on a tiled floor lost all floor manifolds and fell through (docs 26,
    /// "contact gap"). Limit = the body's own padding, the same gap the
    /// static resolver accepts (`gap > contact_padding` is skipped there);
    /// no velocity prediction, as the prop solver has no speculative rows.
    fn world_query_for(&self, index: usize) -> WorldContactSettings {
        WorldContactSettings {
            volume_padding: self.bodies[index].tuning.contact_padding,
            maximum_separating_distance: 0.0,
            edge_cos_bend_normal_threshold: self.pair.edge_cos_bend_normal_threshold,
            convexity_epsilon: self.pair.convexity_epsilon,
            is_object: true,
        }
    }

    /// Pair query settings with this body's contact band.
    fn pair_for(&self, index: usize) -> PrimitivePairSettings {
        let padding = self.bodies[index].tuning.contact_padding;
        PrimitivePairSettings {
            padding_a: padding,
            padding_b: padding,
            ..self.pair
        }
    }

    /// Template-origin pose of one prop for render sync.
    pub(crate) fn pose(&self, id: u32) -> Option<(Vector3, Basis3)> {
        let body = self.bodies.get(*self.by_id.get(&id)?)?;
        Some((body.origin(), body.rates.basis))
    }

    /// Every body as a navigation obstacle for peds (doc 26, fix 11): id, box centre, basis,
    /// half extents (the contact box, so a `collision_box` override counts), linear velocity and
    /// whether it is carried (retail switches a carried object's obstacle off). Id order.
    pub(crate) fn obstacle_boxes(&self) -> Vec<(u32, Vector3, Basis3, Vector3, Vector3, bool)> {
        let mut out: Vec<_> = self
            .live
            .iter()
            .map(|&i| &self.bodies[i])
            .map(|b| (b.id, b.rates.position, b.rates.basis, b.half_extents, b.rates.linear_velocity, self.held == Some(b.id)))
            .collect();
        out.sort_by_key(|b| b.0);
        out
    }

    /// A body's model name for logs ("DMO_DT_BenchFlat_1015"), or "?".
    pub(crate) fn describe_template(&self, id: u32) -> String {
        self.template_of(id).map_or("?".to_owned(), |t| t.split_once('/').map_or(t, |x| x.1).split("_0x").next().unwrap_or(t).to_owned())
    }

    /// A body's MOBJ template name (its model / prop type), for logs.
    pub(crate) fn template_of(&self, id: u32) -> Option<&str> {
        self.bodies.get(*self.by_id.get(&id)?).map(|b| b.template.as_str())
    }

    /// One body's box centre, basis, half extents, linear and angular velocity and effective inertia (a solid proxy
    /// in the skater's contact solve).
    pub(crate) fn body_state(&self, id: u32) -> Option<(Vector3, Basis3, Vector3, Vector3, Vector3, RetailInertiaDynamics)> {
        let b = self.bodies.get(*self.by_id.get(&id)?)?;
        Some((b.rates.position, b.rates.basis, b.half_extents, b.rates.linear_velocity, b.rates.angular_velocity, b.inertia))
    }

    /// Stream one body out (`dormant` true: asleep, still, its collision parked at [`HELD_PARK`]) or back in (asleep
    /// as at map load; at its authored spawn pose when `authored`, as retail respawns a culled DMO from its placement
    /// record, research b98). Refused (None) for the held prop and unknown ids; returns the layer instance to rebake
    /// and the pose (origin, basis) for streaming in, or the park for streaming out.
    pub(crate) fn set_dormant(&mut self, id: u32, dormant: bool, authored: bool) -> Option<(usize, Vector3, Basis3)> {
        if self.held == Some(id) || self.is_dormant(id) == dormant {
            return None;
        }
        if !dormant && authored {
            let (origin, basis) = self.spawn_pose(id)?;
            self.teleport(id, origin, basis);
        }
        let index = *self.by_id.get(&id)?;
        let body = self.bodies.get_mut(index)?;
        if body.dormant == dormant {
            return None;
        }
        body.dormant = dormant;
        match (dormant, self.live.binary_search(&index)) {
            (true, Ok(at)) => {
                self.live.remove(at);
            }
            (false, Err(at)) => self.live.insert(at, index),
            _ => {}
        }
        // The rail bodies are in id order.
        if self.rail_bodies.binary_search_by_key(&id, |&i| self.bodies[i].id).is_ok() {
            match (dormant, self.live_rail_bodies.binary_search_by_key(&id, |&i| self.bodies[i].id)) {
                (true, Ok(at)) => {
                    self.live_rail_bodies.remove(at);
                }
                (false, Err(at)) => self.live_rail_bodies.insert(at, index),
                _ => {}
            }
        }
        let body = &mut self.bodies[index];
        body.asleep = true;
        body.rates.linear_velocity = Vector3::ZERO;
        body.rates.angular_velocity = Vector3::ZERO;
        body.baked = (!dormant).then(|| (body.origin(), body.rates.basis));
        let identity = Basis3 { columns: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };
        Some(if dormant { (body.instance, HELD_PARK, identity) } else { (body.instance, body.origin(), body.rates.basis) })
    }

    /// The map props as DMO census placements (runtime props, a ped's released hand prop or a mod's copy, are not
    /// streamed): id, authored position, bounding radius (box half diagonal) and
    /// the type priority (no type data: the `default` record, 100).
    /// skate_vita: how many placements [`Self::dmo_placements`] returns, without building them.
    pub(crate) fn dmo_placement_count(&self) -> usize {
        debug_assert_eq!(self.map_bodies, self.bodies.iter().filter(|b| !b.runtime).count());
        self.map_bodies
    }

    /// skate_vita: the map props streamed in, for drawing: id, template origin, basis and per-axis scale.
    pub(crate) fn streamed_poses(&self) -> impl Iterator<Item = (u32, Vector3, Basis3, Vector3)> + '_ {
        self.live.iter().map(|&i| &self.bodies[i]).filter(|b| !b.runtime).map(|b| (b.id, b.origin(), b.rates.basis, b.axis_scale))
    }

    pub(crate) fn dmo_placements(&self) -> Vec<skate_core::living_world::dmo::DmoPlacement> {
        let mut out: Vec<_> = self
            .bodies
            .iter()
            .filter(|b| !b.runtime)
            .map(|b| {
                // The census places it where it spawns: the authored pose (retail placement record +0).
                let (p, h) = (b.spawn_origin, b.half_extents);
                skate_core::living_world::dmo::DmoPlacement {
                    id: b.id,
                    position: [p.x, p.y, p.z],
                    radius: (h.x * h.x + h.y * h.y + h.z * h.z).sqrt(),
                    priority: b.type_data.as_ref().map_or(Some(100), |t| t.priority),
                    keep: false,
                    streamable: true,
                }
            })
            .collect();
        out.sort_by_key(|p| p.id);
        out
    }

    /// The streamed-out bodies.
    pub(crate) fn dormant_ids(&self) -> std::collections::BTreeSet<u32> {
        self.bodies.iter().filter(|b| b.dormant).map(|b| b.id).collect()
    }

    /// Whether a body is streamed out.
    pub(crate) fn is_dormant(&self, id: u32) -> bool {
        self.by_id.get(&id).is_some_and(|&i| self.bodies[i].dormant)
    }

    /// World position of one body's box centre.
    pub(crate) fn position_of(&self, id: u32) -> Option<Vector3> {
        Some(self.bodies.get(*self.by_id.get(&id)?)?.rates.position)
    }

    /// Nearest body within `radius` of `point`, measured to the box SURFACE
    /// (not the centre, so big ramps are grabbable by their edge), as
    /// `(id, centre)`.
    pub(crate) fn nearest_body(&self, point: Vector3, radius: f32) -> Option<(u32, Vector3)> {
        let mut best: Option<(u32, Vector3, f32)> = None;
        for body in self.live.iter().map(|&i| &self.bodies[i]) {
            // Local-space point clamped into the box: the gap vector to it is
            // the surface distance (zero when the point is inside).
            let d = sub(point, body.rates.position);
            let b = body.rates.basis.columns;
            let local = [
                d.x * b[0][0] + d.y * b[0][1] + d.z * b[0][2],
                d.x * b[1][0] + d.y * b[1][1] + d.z * b[1][2],
                d.x * b[2][0] + d.y * b[2][1] + d.z * b[2][2],
            ];
            let he = body.half_extents;
            let gap = Vector3::new(
                (local[0].abs() - he.x).max(0.0),
                (local[1].abs() - he.y).max(0.0),
                (local[2].abs() - he.z).max(0.0),
            );
            let distance_squared = dot(gap, gap);
            if distance_squared > radius * radius {
                continue;
            }
            if best.map_or(true, |(_, _, b)| distance_squared < b) {
                best = Some((body.id, body.rates.position, distance_squared));
            }
        }
        best.map(|(id, position, _)| (id, position))
    }

    /// Collision-layer instance index for one body (rebake target).
    pub(crate) fn instance_of(&self, id: u32) -> Option<usize> {
        Some(self.bodies.get(*self.by_id.get(&id)?)?.instance)
    }

    /// The prop currently carried, if any.
    pub(crate) fn held(&self) -> Option<u32> {
        self.held
    }

    /// Mark the carried prop: it stops receiving skater pushes and its layer
    /// triangles stay parked until the drop rebakes them.
    pub(crate) fn set_held(&mut self, held: Option<u32>) {
        if self.held == held {
            return;
        }
        self.move_diagnostics = None;
        // PROP_HELD: the grab / release edge itself (the HELD_PROP lines are periodic).
        let describe = |id: Option<u32>| {
            id.and_then(|id| self.by_id.get(&id).map(|&i| (id, &self.bodies[i]))).map_or("none".to_string(), |(id, b)| {
                let (p, v) = (b.rates.position, b.rates.linear_velocity);
                format!("#{id} {} at=[{:.2}, {:.2}, {:.2}] velocity=[{:.2}, {:.2}, {:.2}]", b.template, p.x, p.y, p.z, v.x, v.y, v.z)
            })
        };
        info!("PROP_HELD from={} to={} tick={}", describe(self.held), describe(held), self.tick);
        if let Some(previous) = self.held {
            let ticks = (RELEASE_LOG_SECONDS / self.simulation.time_step.max(1e-4)).ceil() as u64;
            self.released = Some((previous, self.tick + ticks));
        }
        if let Some(index) = held.and_then(|id| self.by_id.get(&id).copied()) {
            let body = &mut self.bodies[index];
            body.rest_y = body.rest_y.max(body.rates.position.y);
        }
        self.held = held;
    }

    /// Ground relation of one prop (centre, world half height, floor under it).
    /// (Diagnostics and tests; a mod-facing readout is a follow-up.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn ground_probe(&self, id: u32, world: &BoardWorld) -> Option<PropGroundProbe> {
        Some(self.probe(*self.by_id.get(&id)?, world))
    }

    fn probe(&self, index: usize, world: &BoardWorld) -> PropGroundProbe {
        let body = &self.bodies[index];
        let bounds = body.bounds();
        let center = body.rates.position;
        let half_height = 0.5 * (bounds.max.y - bounds.min.y);
        // From above the last rest height (a sunk body still finds the floor
        // it fell through), down well past the body.
        let top = center.y.max(body.rest_y) + half_height + 0.5;
        let ground = world
            .query_thin_line(
                Vector3::new(center.x, top, center.z),
                Vector3::new(center.x, center.y - half_height - 30.0, center.z),
            )
            .ok()
            .flatten()
            .map(|hit| hit.geometry.position.y);
        PropGroundProbe { center, half_height, ground }
    }

    /// HELD_PROP (held prop ~1/s, released prop ~1/s for `RELEASE_LOG_SECONDS`)
    /// and PROP_BELOW_GROUND (an awake prop centre more than its half height
    /// under the floor, once per prop per `BELOW_GROUND_LOG_SECONDS`). Probes
    /// run only for the held/released prop and, twice a second, awake bodies.
    fn log_diagnostics(&mut self, world: &BoardWorld) {
        let per_second = (1.0 / self.simulation.time_step.max(1e-4)).round().max(1.0) as u64;
        if self.released.is_some_and(|(_, until)| self.tick > until) {
            self.released = None;
        }
        let held_every = if crate::trace_all::on() { (per_second / HELD_PROP_HZ_TRACE_ALL).max(1) } else { per_second };
        if self.tick % held_every == 0 {
            let watched = [self.held.map(|id| (id, "held")), self.released.map(|(id, _)| (id, "released"))];
            for (id, phase) in watched.into_iter().flatten() {
                let Some(&index) = self.by_id.get(&id) else { continue };
                let probe = self.probe(index, world);
                let body = &self.bodies[index];
                let v = body.rates.linear_velocity;
                let up = body.rates.basis.columns[1];
                let mv = if phase == "held" { self.move_diagnostics } else { None };
                info!(
                    "HELD_PROP id={id} phase={phase} template={} type={} center=[{:.2}, {:.2}, {:.2}] up_y={:.3} velocity=[{:.2}, {:.2}, {:.2}] ground={} gap={} contacts={} asleep={} {} tick={}",
                    body.template, self.type_key(id).unwrap_or("none"), probe.center.x, probe.center.y, probe.center.z, up[1], v.x, v.y, v.z,
                    probe.ground.map_or("none".into(), |g| format!("{g:.2}")),
                    probe.gap().map_or("none".into(), |g| format!("{g:.2}")),
                    body.contacts, body.asleep, move_fields(mv), self.tick
                );
            }
        }
        if self.tick % (per_second / 2).max(1) != 0 {
            return;
        }
        let quiet = (BELOW_GROUND_LOG_SECONDS * per_second as f32) as u64;
        for index in 0..self.bodies.len() {
            if self.bodies[index].asleep {
                continue;
            }
            let id = self.bodies[index].id;
            if self.below_logged.get(&id).is_some_and(|&at| self.tick < at + quiet) {
                continue;
            }
            let probe = self.probe(index, world);
            if !probe.below_ground() {
                continue;
            }
            self.below_logged.insert(id, self.tick);
            let body = &self.bodies[index];
            let v = body.rates.linear_velocity;
            info!(
                "PROP_BELOW_GROUND id={id} template={} center=[{:.2}, {:.2}, {:.2}] half_height={:.2} ground={:.2} rest_y={:.2} velocity=[{:.2}, {:.2}, {:.2}] up_y={:.3} held={} contacts={} tick={}",
                body.template, probe.center.x, probe.center.y, probe.center.z, probe.half_height,
                probe.ground.unwrap_or(f32::NAN), body.rest_y, v.x, v.y, v.z, body.rates.basis.columns[1][1],
                self.held == Some(id), body.contacts, self.tick
            );
        }
    }

    fn is_held(&self, index: usize) -> bool {
        self.held == Some(self.bodies[index].id)
    }

    /// The held prop as Move Object reads it (centre, box axes, half
    /// extents, velocity, mass, yaw inertia): the interim grab record's source.
    pub(crate) fn held_body(&self, id: u32) -> Option<crate::physics::prop_carry::HeldBody> {
        let body = self.bodies.get(*self.by_id.get(&id)?)?;
        let inverse_yaw = body.inertia.inverse_tensor.y;
        Some(crate::physics::prop_carry::HeldBody {
            center: body.rates.position,
            basis: body.rates.basis,
            half_extents: body.half_extents,
            velocity: body.rates.linear_velocity,
            mass: if body.inertia.inverse_mass > 0.0 { 1.0 / body.inertia.inverse_mass } else { f32::INFINITY },
            yaw_inertia: if inverse_yaw > 0.0 { 1.0 / inverse_yaw } else { f32::INFINITY },
            record_272: self.body_blocks(body).record_272.unwrap_or(false),
        })
    }

    /// Move Object command for the held prop, as retail interface slot 9
    /// applies it (spec section 7: 8275FF00 queue -> flush 827601F0 -> DMO
    /// handler 82C4C370 / 82C4C3E0 -> sinks 82D9CC78 / 82D9CCF0), once per
    /// 60 Hz physics step, before this step's contact solve:
    /// - skipped for an unknown, locked or non-dynamic body (retail gates
    ///   DMO+4464 bit 0x08 and component+36; props have no lock state yet);
    /// - wakes the body and clears its sleep counter on EVERY command
    ///   (82ADF7B8), zero or not;
    /// - linear: an acceleration with no mass factor at the centre of mass
    ///   (`v += L dt`, no torque; the accumulator +144 already holds gravity,
    ///   which our integrator adds for every prop); the vertical argument is
    ///   dropped (82C4C370 forwards only &linear);
    /// - angular: the yaw command replaces the angular accumulator (+160 =
    ///   (0, Y, 0), no inertia factor): any torque queued on the body is
    ///   discarded and `w += (0, Y, 0) dt`; contacts still change pitch and
    ///   roll in the solve;
    /// - marks the body commanded, so its parameter block switches to the
    ///   commanded block on its next step (82C53EF8).
    ///
    /// The command lasts one step: the carry re-sends it every tick and our
    /// prop step runs once per tick (no substeps). `grip` is used only when a
    /// mod turns `apply_at_com` off. Returns false if the id is unknown.
    pub(crate) fn apply_move_command(&mut self, id: u32, linear: Vector3, yaw: f32, grip: Vector3, time_step: f32) -> bool {
        let Some(&index) = self.by_id.get(&id) else {
            return false;
        };
        let rules = &self.move_rules;
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        let body = &mut self.bodies[index];
        if body.inertia.inverse_mass <= 0.0 {
            return true;
        }
        let vertical = if rules.ignore_vertical { 0.0 } else { finite(linear.y) };
        let l = Vector3::new(finite(linear.x), vertical, finite(linear.z));
        // 82C52E68 refuses the angular command while the righting window
        // (DMO+4464 0x40) is open; the linear sink 82C52DC0 is not gated.
        let yaw_blocked = body.upright_timer.is_some() && self.tuning.upright.block_yaw;
        let yaw = if yaw_blocked { 0.0 } else { finite(yaw) };
        if rules.wake_on_command || l != Vector3::ZERO || yaw != 0.0 {
            body.wake();
        }
        body.commanded = true;
        body.rates.linear_velocity = add(body.rates.linear_velocity, scale(l, time_step));
        if !rules.apply_at_com {
            // Mod option: the same force (m L) at the grip point adds the
            // lever torque I^-1 (r x m L).
            let mass = 1.0 / body.inertia.inverse_mass;
            let r = sub(grip, body.rates.position);
            let spin = mul_basis(body.rates.world_inverse_inertia, cross(r, scale(l, mass)));
            body.rates.angular_velocity = add(body.rates.angular_velocity, scale(spin, time_step));
        }
        if yaw_blocked {
            return true;
        }
        if rules.yaw_replaces_torque {
            body.rates.torque_acceleration = Vector3::ZERO;
        }
        let w = body.rates.angular_velocity;
        body.rates.angular_velocity = Vector3::new(w.x, w.y + yaw * time_step, w.z);
        true
    }

    /// Start the self-righting window of one body (retail cMsgUprightDMO ->
    /// DMO manager slot +40 82C4B8C0: DMO+4464 |= 0x40, timer DMO+4376 = 0).
    /// Retail sets it only when the DMO's slot 28 test (82C564D8, physics body
    /// field +28) returns 0, the same test that offers Upright on the phone
    /// (82666748 -> 82C4B578); that field is not decoded, so ours refuses only
    /// an unknown id or a body without dynamics (NOT RETAIL YET). Restarting an
    /// open window zeroes the timer, as retail does. Returns false if refused.
    pub(crate) fn upright(&mut self, id: u32) -> bool {
        let Some(&index) = self.by_id.get(&id) else {
            return false;
        };
        let body = &mut self.bodies[index];
        if body.inertia.inverse_mass <= 0.0 {
            return false;
        }
        body.upright_timer = Some(0.0);
        true
    }

    /// True while the body's righting window is open (DMO+4464 bit 0x40).
    pub(crate) fn is_uprighting(&self, id: u32) -> bool {
        self.by_id.get(&id).is_some_and(|&i| self.bodies[i].upright_timer.is_some())
    }

    /// Body vector A of 82C573D0 (retail: the vector at the physics state
    /// block +72; it scales the gain blend and picks the fallback axis by
    /// A.x > A.z). Which body quantity that block holds is not identified;
    /// ours uses the body-space inverse inertia diagonal (NOT RETAIL YET).
    fn upright_vector(body: &PropBody) -> Vector3 {
        body.inertia.inverse_tensor
    }

    /// The DMO update's righting pass (82C56780), once per step before the
    /// contact solve, for every body with an open window (asleep or not): the
    /// timer advances and closes the window past `window_seconds` (that
    /// update still sends its command); a tilt under `stop_angle_deg` closes
    /// it and zeroes the timer; otherwise the command goes through the DMO's
    /// angular slot 37 (82C52EE0): skipped for a body without dynamics, else
    /// wake, mark commanded (DMO+4465 0x02) and replace the angular
    /// accumulator (82D9CCF0 writes +160), integrated like the Move Object
    /// yaw command (`w += C dt`).
    fn apply_upright(&mut self, time_step: f32) {
        let settings = self.tuning.upright;
        let replaces = self.move_rules.yaw_replaces_torque;
        for &index in &self.live {
            let body = &mut self.bodies[index];
            let Some(timer) = body.upright_timer else { continue };
            let timer = timer + settings.tick_seconds;
            body.upright_timer = (timer <= settings.window_seconds).then_some(timer);
            let a = Self::upright_vector(body);
            let Some(command) = upright_command(body.rates.basis, body.rates.angular_velocity, a, &settings) else {
                body.upright_timer = None;
                continue;
            };
            if body.inertia.inverse_mass <= 0.0 {
                continue;
            }
            body.wake();
            body.commanded = true;
            if replaces {
                body.rates.torque_acceleration = Vector3::ZERO;
            }
            body.rates.angular_velocity = add(body.rates.angular_velocity, scale(command, time_step));
        }
    }

    /// Move Object values for the HELD_PROP line (stick, command, lever,
    /// rotation demand, blocked flag, latch drift).
    pub(crate) fn set_move_diagnostics(&mut self, diagnostics: Option<MoveDiagnostics>) {
        self.move_diagnostics = diagnostics;
    }

    /// Kinematic follow while carried: wake and steer the body toward
    /// `target` by velocity (never teleport), capped at `max_speed`, with
    /// rotation frozen. Returns false if the id is unknown.
    pub(crate) fn carry_to(
        &mut self,
        id: u32,
        target: Vector3,
        max_speed: f32,
        time_step: f32,
    ) -> bool {
        let Some(&index) = self.by_id.get(&id) else {
            return false;
        };
        let body = &mut self.bodies[index];
        body.wake();
        let delta = sub(target, body.rates.position);
        let distance = dot(delta, delta).sqrt();
        let speed = (distance / time_step).min(max_speed);
        body.rates.linear_velocity = if distance > 1e-6 {
            scale(delta, speed / distance)
        } else {
            Vector3::ZERO
        };
        body.rates.angular_velocity = Vector3::ZERO;
        true
    }

    /// Placement follow: like `carry_to`, but also snaps the orientation to
    /// `basis` (ghost yaw edit). Position still moves by velocity only.
    pub(crate) fn carry_to_pose(
        &mut self,
        id: u32,
        target: Vector3,
        basis: Basis3,
        max_speed: f32,
        time_step: f32,
    ) -> bool {
        if !self.carry_to(id, target, max_speed, time_step) {
            return false;
        }
        self.pose_epoch += 1;
        let body = &mut self.bodies[self.by_id[&id]];
        body.rates.basis = basis;
        body.rates.orientation = quaternion_from_basis(basis);
        body.rates.world_inverse_inertia =
            world_inverse_inertia(basis, body.inertia.inverse_tensor);
        true
    }

    /// Confirming a placement sets the prop down gently: velocity zeroed so
    /// the leftover follow velocity does not throw it.
    pub(crate) fn release_still(&mut self, id: u32) {
        if let Some(&index) = self.by_id.get(&id) {
            self.bodies[index].rates.linear_velocity = Vector3::ZERO;
            self.bodies[index].rates.angular_velocity = Vector3::ZERO;
        }
    }

    /// Teleport a body to a saved layout pose, asleep. Returns the collision
    /// instance index so the caller can rebake its triangles.
    pub(crate) fn teleport(&mut self, id: u32, origin: Vector3, basis: Basis3) -> Option<usize> {
        let cool_down = self.step_simulation().cool_down;
        self.pose_epoch += 1;
        let body = self.bodies.get_mut(*self.by_id.get(&id)?)?;
        body.rates.basis = basis;
        body.rates.orientation = quaternion_from_basis(basis);
        body.rates.world_inverse_inertia =
            world_inverse_inertia(basis, body.inertia.inverse_tensor);
        body.rates.position = add(origin, mul_basis(basis, body.local_center));
        body.rates.linear_velocity = Vector3::ZERO;
        body.rates.angular_velocity = Vector3::ZERO;
        body.rates.kinetic_energy = 0.0;
        body.rates.cool_down = cool_down;
        body.asleep = true;
        Some(body.instance)
    }

    /// skate_vita: see [`Self::pose_epoch`] (the field).
    pub(crate) fn pose_epoch(&self) -> u64 {
        self.pose_epoch
    }

    /// skate_vita: see [`Self::collision_type_epoch`] (the field).
    pub(crate) fn collision_type_epoch(&self) -> u64 {
        self.collision_type_epoch
    }

    /// skate_vita (performance): whether body `id` is in [`Self::moved_ids`] (the same test, for one body).
    pub(crate) fn is_moved(&self, id: u32) -> bool {
        self.by_id.get(&id).is_some_and(|&i| Self::moved(&self.bodies[i]))
    }

    fn moved(b: &PropBody) -> bool {
        if b.runtime {
            return false;
        }
        let o = b.origin();
        let d = sub(o, b.spawn_origin);
        d.x * d.x + d.y * d.y + d.z * d.z > SPAWN_POSE_EPSILON * SPAWN_POSE_EPSILON
            || b.rates.basis.columns.iter().zip(b.spawn_basis.columns.iter()).any(|(a, s)| {
                a.iter().zip(s.iter()).any(|(x, y)| (x - y).abs() > SPAWN_POSE_EPSILON)
            })
    }

    /// Ids of bodies whose pose differs from the authored spawn pose, in id order.
    /// Retail offers the per-object reset (cMsgResetDMO) only for a moved object
    /// (sub_826666A8 gate); this is the engine-side list behind "reset moved objects".
    pub(crate) fn moved_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .bodies
            .iter()
            .filter(|b| !b.runtime)
            .filter(|b| {
                let o = b.origin();
                let d = sub(o, b.spawn_origin);
                d.x * d.x + d.y * d.y + d.z * d.z > SPAWN_POSE_EPSILON * SPAWN_POSE_EPSILON
                    || b.rates.basis.columns.iter().zip(b.spawn_basis.columns.iter()).any(|(a, s)| {
                        a.iter().zip(s.iter()).any(|(x, y)| (x - y).abs() > SPAWN_POSE_EPSILON)
                    })
            })
            .map(|b| b.id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Authored spawn pose of one body (template origin, basis).
    pub(crate) fn spawn_pose(&self, id: u32) -> Option<(Vector3, Basis3)> {
        let body = self.bodies.get(*self.by_id.get(&id)?)?;
        Some((body.spawn_origin, body.spawn_basis))
    }

    /// The ped-plugin check limits of a body's type (mod entry over retail data): rotation tolerance (degrees, per
    /// axis) and max distance from the authored spot. `None` = no type data (the seats stay offered).
    pub(crate) fn plugin_check_limits(&self, id: u32) -> Option<([f32; 3], f32)> {
        let body = self.bodies.get(*self.by_id.get(&id)?)?;
        let b = self.body_blocks(body);
        Some((b.plugin_rotation_tolerance?, b.plugin_max_distance?))
    }

    /// Return one body to its authored spawn pose, at rest and asleep (the
    /// receiving end of retail's cMsgResetDMO; how retail moves it, teleport or
    /// fade, is not decoded yet: NOT RETAIL YET, an instant teleport). Returns the
    /// collision instance so the caller can rebake its triangles.
    pub(crate) fn reset_to_spawn(&mut self, id: u32) -> Option<usize> {
        if self.is_runtime(id) {
            return None;
        }
        let (origin, basis) = self.spawn_pose(id)?;
        if self.held == Some(id) {
            self.set_held(None);
        }
        self.teleport(id, origin, basis)
    }

    /// Advance awake bodies one tick; wake bodies the skater touches. Moved
    /// instances re-bake their triangles in the collision layer afterwards.
    pub(crate) fn step(
        &mut self,
        world: &BoardWorld,
        layer: &mut crate::skate_world::PropCollisionLayer,
        skater_volumes: &[BoardWorldVolume],
    ) {
        self.step_with_actors(world, layer, skater_volumes, &[]);
    }

    /// [`Self::step`] with other skaters' volumes too (NPC skaters, doc 26 fix 19): retail NPC
    /// skaters are full skaters, so their board and body push a DMO by the same contact rule as
    /// the player's. `others` = (stable actor id, volume), sorted by actor id; the local
    /// player's volumes go first ([`LOCAL_PUSHER`]), so the push order is deterministic.
    pub(crate) fn step_with_actors(
        &mut self,
        world: &BoardWorld,
        layer: &mut crate::skate_world::PropCollisionLayer,
        skater_volumes: &[BoardWorldVolume],
        others: &[(u64, BoardWorldVolume)],
    ) {
        self.stats = PropStepStats::default();
        self.prepare_streamed(world, layer);
        let time_step = self.step_simulation().time_step;
        self.apply_upright(time_step);
        let push = skate_core::profile::scope("z_sp_push");
        // skate_vita (performance): the volumes' bounds once per step (upstream: per body and volume) and their
        // union: a body outside it can touch none of them (it skips them; the counters below run as before).
        let skater_bounds: Vec<_> = skater_volumes.iter().map(|v| volume_bounds(v.primitive)).collect();
        let other_bounds: Vec<_> = others.iter().map(|(_, v)| volume_bounds(v.primitive)).collect();
        let reach = skate_core::physics::board_world::query_metadata::Bounds::from_points(
            skater_bounds.iter().chain(&other_bounds).flatten().flat_map(|b| [b.min, b.max]),
        );
        for k in 0..self.live.len() {
            let index = self.live[k];
            let held = self.is_held(index);
            // Skater push: cheap bounds reject, then the retail pair query.
            // The carried prop is velocity-driven by the carrier; letting the
            // skater push it (or be pushed by it) fights the drag.
            if !held {
                let body_bounds = self.bodies[index].bounds();
                let mut overlapping = false;
                let mut closing = false;
                let before = self.bodies[index].rates.linear_velocity;
                let mut bail_touch = false;
                // The prop under the local skater's grind is not pushed by them (others still can).
                let spared = self.grinded == Some(self.bodies[index].id);
                let reachable = reach.is_some_and(|r| body_bounds.overlaps(r));
                let actors = skater_volumes
                    .iter()
                    .zip(&skater_bounds)
                    .filter(|_| !spared)
                    .map(|(v, b)| (LOCAL_PUSHER, v, *b))
                    .chain(others.iter().zip(&other_bounds).map(|((actor, v), b)| (*actor, v, *b)))
                    .filter(|_| reachable);
                for (actor, volume, bounds) in actors {
                    let Some(volume_bounds) = bounds else {
                        continue;
                    };
                    if !body_bounds.overlaps(volume_bounds) {
                        continue;
                    }
                    // skate_vita: the push tests the prop's own triangles (what the skater
                    // rides), not its contact box: a quarter pipe's box is a 2 m wall at its lip and a
                    // rail's a slab under the bar, so riding up the ramp or beside the rail shoved the
                    // prop away. The deepest triangle contact stands for the pair.
                    let Some(manifold) = Self::triangle_contacts(layer, self.bodies[index].instance, volume.primitive, volume_bounds, self.pair_for(index)) else {
                        continue;
                    };
                    // skate_vita: a face turned up carries the skater (riding or standing on
                    // the prop); its push points down into the floor, so it is no push: riding a
                    // quarter pipe kept it awake (solver rows and a rebake every tick). Side hits
                    // and the steep part of a transition still push.
                    if manifold.normal.y > SUPPORT_NORMAL_Y {
                        continue;
                    }
                    let pushes = self.stats.skater_pushes;
                    let push_mass = self.actor_push_mass.iter().find(|a| a.0 == actor).map(|a| a.1);
                    let contact = self.push_from_skater(index, volume, &manifold, push_mass);
                    if self.stats.skater_pushes > pushes {
                        self.bodies[index].pushed_by = Some(actor);
                    }
                    match contact {
                        SkaterContact::Closing => closing = true,
                        SkaterContact::OverlapOnly => overlapping = true,
                        SkaterContact::Separate => {}
                    }
                    bail_touch |= contact != SkaterContact::Separate && self.bailing.contains(&actor);
                }
                // A bailing actor touched it: retail caps the prop's velocity and wakes it (once per prop per tick).
                let solver = self.tuning.solver;
                if bail_touch && solver.bail_clamp {
                    let body = &mut self.bodies[index];
                    if let Some((linear, angular)) = bail_clamp(&solver, body.inertia.inverse_mass, before, body.rates.linear_velocity, body.rates.angular_velocity) {
                        body.rates.linear_velocity = linear;
                        body.rates.angular_velocity = angular;
                        body.wake();
                    }
                }
                // A skater volume parked inside the box (the board under a
                // bench seat, inside the render AABB) would otherwise nudge
                // the prop every tick forever: it never sleeps and rebakes
                // its triangles every tick. Count those ticks; past
                // `stuck_release_ticks` only real hits push it.
                let release = self.bodies[index].tuning.stuck_release_ticks;
                let body = &mut self.bodies[index];
                body.stuck_ticks = if closing {
                    0
                } else if overlapping {
                    body.stuck_ticks.saturating_add(1)
                } else {
                    0
                };
                // Diagnostics: a skater volume that sat inside the prop long enough to stop nudging it (the board
                // under a seat, or the skater pulled into a fallen bench), once per stay.
                if body.stuck_ticks == release {
                    let (id, c, up) = (body.id, body.rates.position, body.rates.basis.columns[1][1]);
                    info!("PROP_OVERLAP prop={id} ({}) skater inside for {release} ticks at [{:.2}, {:.2}, {:.2}] up_y {up:.2}", self.describe_template(id), c.x, c.y, c.z);
                }
            }
        }
        drop(push);
        let solver = self.tuning.solver;
        let simulation = self.step_simulation();
        // Awake bodies in body order (deterministic; plain indices).
        let awake: Vec<usize> = self.live.iter().copied().filter(|&i| !self.bodies[i].asleep).collect();
        self.stats.awake = awake.len() as u32;
        // Parameter block swap (82C53EF8): a body commanded since its last
        // step runs on the commanded block, otherwise on its free block;
        // the switch happens on the step the commanded bit changes. The
        // command flag lasts one step (retail shifts 0x02 -> 0x01).
        let mut commanded_now = vec![false; self.bodies.len()];
        for &index in &awake {
            let body = &mut self.bodies[index];
            commanded_now[index] = body.commanded;
            body.commanded_block = body.commanded;
            body.commanded = false;
        }
        // Retail: every awake body's contact rows go through one shared row
        // solve (contact stage 82DC30A8 -> 82AE27D0, island +176 iterations)
        // and only then does any body integrate (BatchIntegrator).
        skate_core::profile::count("z_sp_awake_n", awake.len() as u64);
        let rows_scope = skate_core::profile::scope("z_sp_rows");
        // skate_vita (performance): no awake body reads the rows, so none are solved (upstream packed and
        // iterated a reaction slot per body anyway).
        let rows = (solver.row_solver && !awake.is_empty()).then(|| self.row_corrections(world, &awake, solver.iterations));
        drop(rows_scope);
        let _integrate = skate_core::profile::scope("z_sp_integrate_rebake");
        if !awake.is_empty() {
            self.pose_epoch += 1;
        }
        let mut sleeps = 0u32;
        for &index in &awake {
            let held = self.is_held(index);
            let commanded = commanded_now[index];
            // The held prop is a normal dynamic body: pitch, roll, gravity
            // and contacts stay with the simulation (retail Move Object sends
            // only a horizontal + yaw command, 82D45318), so it can tip and
            // the floor holds it.
            let corrections = match &rows {
                Some(rows) => rows[index],
                None => self.contact_corrections(index, world),
            };
            // A commanded (or held) body never snaps to rest or sleeps.
            // Sleep: retail clears the sleep counter on every command
            // (82ADF7B8 in both slot 9 sinks), so a commanded body cannot
            // freeze. Held covers placement (`carry_to`, NOT RETAIL).
            let body_sleep_capable = self.bodies[index].enable_sleep && !held && !commanded;
            // Rest snap (NOT RETAIL, mod option `rest_snap`): only while
            // something is touching the body, so it never zeroes a fall.
            let resting = solver.rest_snap
                && body_sleep_capable
                && (dot(corrections.linear_displacement, corrections.linear_displacement)
                    > 0.0
                    || dot(corrections.position_displacement, corrections.position_displacement)
                        > 0.0
                    || dot(corrections.angular_displacement, corrections.angular_displacement)
                        > 0.0);
            let inertia = self.body_inertia(index);
            let body = &mut self.bodies[index];
            // Integrator 82AE6590: E = |v|^2 + s m^-1 |w|^2 after damping and
            // the caps; counter = 0 when E >= island +172, else +1 if E did
            // not rise, capped at island +168.
            let step = integrate_body_rates(body.rates, inertia, simulation, corrections);
            body.rates = step.state;
            if resting && body.rates.kinetic_energy < simulation.minimum_energy {
                body.rates.linear_velocity = Vector3::ZERO;
                body.rates.angular_velocity = Vector3::ZERO;
                body.rates.kinetic_energy = 0.0;
                // The snap zeroes the energy the integrator compares against,
                // so its counter stalls; count snapped resting ticks here.
                body.rates.cool_down = (body.rates.cool_down + 1).min(simulation.cool_down);
            }
            // Sleep pass 82DC3130: counter >= sim +204 (= island +168) moves
            // the body to the sleeping list, at most 100 bodies per call.
            if body_sleep_capable
                && body.rates.cool_down >= solver.sleep_frames
                && sleeps < solver.max_sleeps_per_step
            {
                sleeps += 1;
                body.asleep = true;
                body.rates.cool_down = simulation.cool_down;
                body.rest_y = body.rates.position.y;
            }
            // The held prop's triangles stay parked (set_held/HELD_PARK) so
            // skater queries never see them while carrying.
            if held {
                continue;
            }
            // An unchanged pose would rebake identical triangles (and rebuild
            // the layer's query index for nothing); skip it.
            let body = &self.bodies[index];
            let pose = (body.origin(), body.rates.basis);
            if body.baked.is_some_and(|baked| same_pose(baked, pose)) {
                continue;
            }
            if let Err(error) = layer.rebake(body.instance, pose.1.columns, pose.0) {
                warn!("SKATE_PROP_DYNAMICS: rebake instance {}: {error}", body.instance);
            }
            self.bodies[index].baked = Some(pose);
            self.stats.rebakes += 1;
        }
        self.log_diagnostics(world);
        self.tick += 1;
    }

    /// skate_vita: every streamed-in body (the held one aside) gets its collision slot
    /// (`PropCollisionLayer`: a map prop has triangles only while streamed in) and, once, its
    /// seat on the floor ([`Self::seat`]), in one batched rebake. Runs before each step and when a
    /// session loads (after the census's first pass).
    pub(crate) fn prepare_streamed(&mut self, world: &BoardWorld, layer: &mut crate::skate_world::PropCollisionLayer) {
        let mut poses = Vec::new();
        for k in 0..self.live.len() {
            let index = self.live[k];
            if self.is_held(index) {
                continue;
            }
            let seated = !self.bodies[index].seated && self.bodies[index].asleep && self.seat(world, index);
            let body = &mut self.bodies[index];
            if seated || !layer.has_slot(body.instance) {
                let pose = (body.origin(), body.rates.basis);
                poses.push((body.instance, Some((pose.1.columns, pose.0))));
                body.baked = Some(pose);
            }
        }
        if poses.is_empty() {
            return;
        }
        for (instance, error) in layer.rebake_many(&poses) {
            warn!("SKATE_PROP_DYNAMICS: prepare instance {instance}: {error}");
        }
    }

    /// skate_vita: set a sleeping upright body down on the floor below it, once. The map
    /// places props a little above the floor (2.5 cm in the parks) and they sleep where placed,
    /// so a ramp's lip stood a wheel's radius above the ground: a bump onto every ramp. The
    /// solver's own rest (a woken prop settles) is on the floor. The floor is probed under the
    /// box bottom's centre and corners; a gap up to [`SEAT_REACH`] is closed (the nearest hit)
    /// and the seated pose becomes the spawn pose (streaming in and resets return to it).
    /// Returns whether the body moved (its collision is rebaked by the caller).
    fn seat(&mut self, world: &BoardWorld, index: usize) -> bool {
        let body = &mut self.bodies[index];
        body.seated = true;
        let basis = body.rates.basis;
        if basis.columns[1][1] < 0.99 {
            return false;
        }
        let h = body.half_extents;
        let mut drop = f32::MAX;
        for (x, z) in [(0.0, 0.0), (-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let w = add(body.rates.position, mul_basis(basis, Vector3::new(x * h.x, -h.y, z * h.z)));
            let start = Vector3::new(w.x, w.y + 0.005, w.z);
            let end = Vector3::new(w.x, w.y - SEAT_REACH, w.z);
            if let Ok(Some(hit)) = world.query_thin_line(start, end) {
                drop = drop.min(w.y - hit.geometry.position.y);
            }
        }
        if !(drop > 0.001 && drop <= SEAT_REACH) {
            return false;
        }
        body.rates.position.y -= drop;
        body.rest_y = body.rates.position.y;
        body.spawn_origin.y -= drop;
        self.pose_epoch += 1;
        true
    }

    /// skate_vita: a skater volume against one prop instance's baked triangles: the contact
    /// of the triangle the volume sinks deepest into (`None` when it touches none). The triangles
    /// near the volume come from the layer's per-mesh hierarchy.
    fn triangle_contacts(
        layer: &crate::skate_world::PropCollisionLayer,
        instance: usize,
        primitive: ContactPrimitive,
        volume_bounds: skate_core::physics::board_world::query_metadata::Bounds,
        pair: PrimitivePairSettings,
    ) -> Option<PrimitiveContactManifold> {
        let range = layer.instances().get(instance)?.range.clone();
        if range.is_empty() {
            return None;
        }
        let reach = pair.padding_a + pair.padding_b + pair.additional_padding;
        let world = layer.world();
        let mut near = Vec::new();
        world.triangles_overlapping(range, volume_bounds.expanded(reach), &mut near);
        let mut best: Option<(f32, PrimitiveContactManifold)> = None;
        for index in near {
            let triangle = world.triangles()[index].triangle;
            let Some(manifold) = primitive_pair_contacts(primitive, ContactPrimitive::Triangle(triangle), pair) else { continue };
            let depth = manifold.points[..manifold.count]
                .iter()
                .map(|p| dot(sub(p.a, p.b), manifold.normal))
                .fold(f32::MAX, f32::min);
            if best.as_ref().is_none_or(|(deepest, _)| depth < *deepest) {
                best = Some((depth, manifold));
            }
        }
        best.map(|(_, manifold)| manifold)
    }

    /// Skater volumes treat the prop as a pushable weight: the prop receives a
    /// fraction of the closing speed through the reduced mass of the pair, as
    /// an impulse at the contact point. The skater's own response still comes
    /// from the exact triangle layer.
    fn push_from_skater(
        &mut self,
        index: usize,
        volume: &BoardWorldVolume,
        manifold: &PrimitiveContactManifold,
        push_mass: Option<f32>,
    ) -> SkaterContact {
        let tuning = self.bodies[index].tuning;
        // The overlap nudge stops once the volume has sat inside for
        // `stuck_release_ticks`; from then on slow drift (solver jitter of a
        // parked board) does not push either, only a real hit does.
        let stuck = self.bodies[index].stuck_ticks >= tuning.stuck_release_ticks;
        // Upstream (`74c124a`): a contact on the prop's mesh gets no overlap nudge (riding up a ramp prop does not
        // shove it); the skater volumes here always test the mesh.
        let nudge = 0.0;
        let hit_floor = if stuck { tuning.penetration_push_speed } else { 0.0 };
        // The manifold normal points from the prop (B) toward the skater (A).
        let push = scale(manifold.normal, -1.0);
        let mut strongest = 0.0_f32;
        let mut point = self.bodies[index].rates.position;
        let mut any_closing = false;
        let mut any_overlap = false;
        for pair in &manifold.points[..manifold.count] {
            let closing = dot(sub(volume.motion.linear_velocity, self.bodies[index].velocity_at(pair.b)), push);
            let closing = if closing > hit_floor { closing } else { 0.0 };
            let penetration = dot(sub(pair.a, pair.b), manifold.normal);
            any_closing |= closing > 0.0;
            any_overlap |= penetration < 0.0;
            let drive = closing.max(if penetration < 0.0 { nudge } else { 0.0 });
            if drive > strongest {
                strongest = drive;
                point = pair.b;
            }
        }
        let contact = if any_closing {
            SkaterContact::Closing
        } else if any_overlap {
            SkaterContact::OverlapOnly
        } else {
            SkaterContact::Separate
        };
        if strongest <= 0.0 {
            return contact;
        }
        // Momentum-style transfer: the skater shares its closing speed through
        // the reduced mass of the pair, so a 20 kg box skips away while a
        // 500 kg ramp barely budges. Δv = strongest × transfer × M/(M+m).
        // Board hits carry the full transfer; body bumps are capped to a
        // nudge speed so walking into a prop cannot keep accelerating it.
        // Board and body pushes are both capped along the push direction
        // (#15 capped only body bumps), so a board held against a prop cannot
        // keep accelerating it either.
        let mass = 1.0 / self.bodies[index].inertia.inverse_mass;
        let pusher = push_mass.unwrap_or(tuning.skater_push_mass).max(0.0);
        let reduced = mass * pusher / (pusher + mass).max(1e-6);
        let mut amount = strongest * reduced * tuning.push_transfer;
        let cap = if matches!(
            volume.body,
            skate_core::physics::board_step::CollisionBody::Board(_)
        ) {
            tuning.board_push_speed
        } else {
            tuning.body_push_speed
        };
        let along = dot(self.bodies[index].rates.linear_velocity, push);
        let allowed = (cap - along).max(0.0) * mass;
        amount = amount.min(allowed);
        if amount <= 0.0 {
            return contact;
        }
        let impulse = scale(push, amount);
        self.bodies[index].apply_impulse(impulse, point);
        self.stats.skater_pushes += 1;
        // Diagnostics: a big shove (2 m/s or more in one tick), with what the push saw.
        let dv = amount / mass;
        if dv >= 2.0 {
            let id = self.bodies[index].id;
            let board = matches!(volume.body, skate_core::physics::board_step::CollisionBody::Board(_));
            info!("PROP_SHOVE prop={id} ({}) dv {dv:.2} m/s mass {mass:.0} kg closing {strongest:.2} m/s by {} at [{:.2}, {:.2}, {:.2}]", self.describe_template(id), if board { "board" } else { "body" }, point.x, point.y, point.z);
        }
        contact
    }

    /// The simulation step with the live island settings (sleep energy and
    /// counter cap from [`PropSolverSettings`]).
    fn step_simulation(&self) -> RetailSimulationStep {
        let solver = self.tuning.solver;
        RetailSimulationStep {
            cool_down: solver.sleep_frames,
            minimum_energy: solver.sleep_energy,
            ..self.simulation
        }
    }

    /// Solver-side state of an awake prop (reaction slot = body index).
    fn row_body(&self, index: usize) -> RetailContactBodyState {
        let body = &self.bodies[index];
        let inertia = pack_world_inverse_inertia(body.rates.world_inverse_inertia);
        RetailContactBodyState {
            contact_body_id: index as u32,
            reaction_id: index as u32,
            center_of_mass: body.rates.position,
            inverse_inertia_full: inertia.full,
            inverse_inertia_split: inertia.split,
            inverse_mass: body.inertia.inverse_mass,
            state: ACTIVE_BODY,
            force_acceleration: body.rates.force_acceleration,
            torque_acceleration: body.rates.torque_acceleration,
            linear_velocity: body.rates.linear_velocity,
            angular_velocity: body.rates.angular_velocity,
            kinetic_energy: body.rates.kinetic_energy,
            cool_down: body.rates.cool_down,
        }
    }

    /// Solver-side state of an immovable support: the static world, or an
    /// asleep prop (inactive, so the row solver gives it no response).
    fn row_support(&self, world_reaction: usize, center: Vector3) -> RetailContactBodyState {
        RetailContactBodyState {
            contact_body_id: u32::MAX,
            reaction_id: world_reaction as u32,
            center_of_mass: center,
            inverse_inertia_full: Vector3::ZERO,
            inverse_inertia_split: Vector3::ZERO,
            inverse_mass: 0.0,
            state: 0,
            force_acceleration: Vector3::ZERO,
            torque_acceleration: Vector3::ZERO,
            linear_velocity: Vector3::ZERO,
            angular_velocity: Vector3::ZERO,
            kinetic_energy: 0.0,
            cool_down: 0,
        }
    }

    /// Retail contact solve for every awake prop at once: one row per
    /// manifold point (A = the prop, B = the triangle or the other prop,
    /// normal from B toward A as the pair queries return it), built by the
    /// retail ContactBatchBuild (82AE10C8: targets in displacement units,
    /// predicted separation v dt + separation + a dt^2, restitution -v dt e)
    /// and iterated `iterations` times by 82AE27D0 (contacts only). Returns
    /// the per-body correction buffers, indexed by body (the integrator turns
    /// the +0 / +32 pair into velocity, +16 / +48 into position only).
    /// No slop, no correction fraction, no per-tick cap (retail has none).
    fn row_corrections(
        &mut self,
        world: &BoardWorld,
        awake: &[usize],
        iterations: u32,
    ) -> Vec<RetailReactionCorrections> {
        let count = self.bodies.len();
        let world_reaction = count;
        let dt = self.simulation.time_step;
        let mut rows = Vec::new();
        let mut wake = Vec::new();
        for &index in awake {
            self.bodies[index].contacts = 0;
        }
        let push_point = |rows: &mut Vec<RetailContactJacobian>,
                              on_a: Vector3,
                              on_b: Vector3,
                              normal: Vector3,
                              material: RetailContactMaterial,
                              a: RetailContactBodyState,
                              b: RetailContactBodyState| {
            let contact = generate_contact(
                RetailContactInput {
                    position_on_a: on_a,
                    position_on_b: on_b,
                    normal,
                    restitution: material.restitution,
                    static_friction: material.static_friction,
                    dynamic_friction: material.dynamic_friction,
                    tag: 0,
                },
                a,
                b,
            );
            rows.push(build_contact_jacobian(contact, dt));
        };
        let push_rows = |rows: &mut Vec<RetailContactJacobian>,
                             manifold: &PrimitiveContactManifold,
                             material: RetailContactMaterial,
                             a: RetailContactBodyState,
                             b: RetailContactBodyState| {
            for pair in &manifold.points[..manifold.count] {
                push_point(rows, pair.a, pair.b, manifold.normal, material, a, b);
            }
        };
        let reduction = self.tuning.solver.ground_reduction;
        let mut ground: Vec<(skate_core::physics::contact_reduction::ContactRecord, RetailContactMaterial)> = Vec::new();
        for &index in awake {
            let tuning = self.bodies[index].tuning;
            let box_primitive = self.bodies[index].box_primitive();
            let world_query = self.world_query_for(index);
            let pair_settings = self.pair_for(index);
            let bounds = self.bodies[index].bounds().expanded(tuning.contact_padding + 0.05);
            let a = self.row_body(index);
            let support = self.row_support(world_reaction, Vector3::ZERO);
            // The world contacts go through retail's reduction (at most four coplanar points per group).
            ground.clear();
            for index_in_world in near_triangles(world, bounds) {
                let triangle = &world.triangles()[index_in_world];
                let Some(manifold) = primitive_triangle_world_contacts(
                    box_primitive,
                    triangle.triangle,
                    Vector3::ZERO,
                    world_query,
                ) else {
                    continue;
                };
                self.bodies[index].contacts += 1;
                let material = self.contact_material(index, triangle.material);
                ground.extend(manifold.points[..manifold.count].iter().map(|pair| {
                    (skate_core::physics::contact_reduction::ContactRecord { p0: pair.a, p1: pair.b, normal: manifold.normal }, material)
                }));
            }
            let records: Vec<_> = ground.iter().map(|g| g.0).collect();
            for k in skate_core::physics::contact_reduction::reduce(&records, &reduction) {
                let (r, material) = ground[k];
                push_point(&mut rows, r.p0, r.p1, r.normal, material, a, support);
            }
            for other in 0..count {
                let other_awake = !self.bodies[other].asleep;
                // Each awake pair once, from its lower index.
                if other == index || (other_awake && other < index) {
                    continue;
                }
                if !self.bodies[index].bounds().overlaps(self.bodies[other].bounds().expanded(tuning.contact_padding)) {
                    continue;
                }
                let Some(manifold) = primitive_pair_contacts(
                    box_primitive,
                    self.bodies[other].box_primitive(),
                    pair_settings,
                ) else {
                    continue;
                };
                self.bodies[index].contacts += 1;
                let material = self.contact_material(index, self.body_material(other));
                let b = if other_awake {
                    self.bodies[other].contacts += 1;
                    self.row_body(other)
                } else {
                    // An asleep prop is an immovable support; a hard hit
                    // (closing faster than 1 m/s) wakes it for the next step
                    // (NOT RETAIL YET: retail merges touching bodies into the
                    // island; the wake rule is ours).
                    let closing = manifold.points[..manifold.count]
                        .iter()
                        .map(|pair| dot(self.bodies[index].velocity_at(pair.a), manifold.normal))
                        .fold(0.0_f32, f32::min);
                    if closing < -1.0 {
                        wake.push(other);
                    }
                    self.row_support(world_reaction, self.bodies[other].rates.position)
                };
                push_rows(&mut rows, &manifold, material, a, b);
            }
        }
        let mut reactions = vec![RetailReactionCorrections::default(); count + 1];
        solve_constraints(&mut rows, &mut [], &mut [], &mut reactions, iterations);
        for other in wake {
            self.bodies[other].wake();
        }
        reactions.truncate(count);
        reactions
    }

    /// Impulse and positional corrections for one awake body against the
    /// static world and every other prop box (asleep props are immovable).
    /// The engine's older pass (NOT RETAIL), used when `row_solver` is off.
    fn contact_corrections(&mut self, index: usize, world: &BoardWorld) -> RetailReactionCorrections {
        let mut corrections = RetailReactionCorrections::default();
        let box_primitive = self.bodies[index].box_primitive();
        let tuning = self.bodies[index].tuning;
        let pair_settings = self.pair_for(index);
        let world_query = self.world_query_for(index);
        let bounds = self.bodies[index].bounds().expanded(tuning.contact_padding + 0.05);
        let mut contacts = 0u32;
        // All static manifolds of this tick first: every one resolves from the
        // same pre-contact velocity, so the closing impulse is shared over
        // ALL simultaneous points, not per triangle. Per-triangle sharing
        // applied the full impulse once per touching triangle (a box edge on
        // a tiled floor touches ~10), which launched a tipping bench at
        // 15 m/s (2026-10-08 street drag trace).
        let mut manifolds = Vec::new();
        for index_in_world in near_triangles(world, bounds) {
            let triangle = &world.triangles()[index_in_world];
            let Some(manifold) = primitive_triangle_world_contacts(
                box_primitive,
                triangle.triangle,
                Vector3::ZERO,
                world_query,
            ) else {
                continue;
            };
            contacts += 1;
            manifolds.push((manifold, triangle.material));
        }
        let points: usize = manifolds.iter().map(|(m, _)| m.count.max(1)).sum();
        for (manifold, triangle_material) in &manifolds {
            let material = self.contact_material(index, *triangle_material);
            self.resolve_static(index, manifold, material, points as f32, &mut corrections);
        }
        let box_primitive = self.bodies[index].box_primitive();
        for k in 0..self.live.len() {
            let other = self.live[k];
            if other == index {
                continue;
            }
            if !self.bodies[index].bounds().overlaps(self.bodies[other].bounds().expanded(tuning.contact_padding)) {
                continue;
            }
            let Some(manifold) = primitive_pair_contacts(
                box_primitive,
                self.bodies[other].box_primitive(),
                pair_settings,
            ) else {
                continue;
            };
            contacts += 1;
            let material = self.contact_material(index, self.body_material(other));
            if self.bodies[other].asleep {
                // An asleep prop is an immovable support; a hard hit wakes it.
                let closing = manifold.points[..manifold.count]
                    .iter()
                    .map(|pair| {
                        dot(
                            sub(
                                self.bodies[index].velocity_at(pair.a),
                                self.bodies[other].velocity_at(pair.b),
                            ),
                            manifold.normal,
                        )
                    })
                    .fold(0.0_f32, |a, b| a.min(b));
                self.resolve_static(index, &manifold, material, manifold.count.max(1) as f32, &mut corrections);
                if closing < -1.0 {
                    self.bodies[other].wake();
                }
            } else {
                self.resolve_dynamic(index, other, &manifold, material, &mut corrections);
            }
        }
        self.bodies[index].contacts = contacts;
        // Bounded depenetration: the per-point corrections add up (every
        // triangle and prop touching a deep body contributes), so a body
        // pushed deep into geometry would otherwise jump out in one tick.
        let cap = tuning.max_depenetration_per_tick;
        let depth = length(corrections.position_displacement);
        if cap.is_finite() && cap >= 0.0 && depth > cap {
            corrections.position_displacement =
                scale(corrections.position_displacement, cap / depth);
        }
        corrections
    }

    /// Resolve contacts against an immovable surface (static world or asleep
    /// prop). The manifold normal points from the surface toward this prop.
    fn resolve_static(
        &mut self,
        index: usize,
        manifold: &PrimitiveContactManifold,
        material: RetailContactMaterial,
        shared_points: f32,
        corrections: &mut RetailReactionCorrections,
    ) {
        let dt = self.simulation.time_step;
        let tuning = self.bodies[index].tuning;
        let count = manifold.count.max(1) as f32;
        let share = shared_points.max(count);
        for pair in &manifold.points[..manifold.count] {
            let normal = manifold.normal;
            let gap = dot(sub(pair.a, pair.b), normal);
            if gap > tuning.contact_padding {
                continue;
            }
            let body = &self.bodies[index];
            let r = sub(pair.a, body.rates.position);
            let velocity = body.velocity_at(pair.a);
            let vn = dot(velocity, normal);
            let inverse_mass = body.inertia.inverse_mass;
            let angular = mul_basis(body.rates.world_inverse_inertia, cross(r, normal));
            let denominator = inverse_mass + dot(normal, cross(angular, r));
            if denominator <= 1e-9 {
                continue;
            }
            if vn < 0.0 {
                let restitution = if vn < -tuning.restitution_threshold {
                    material.restitution.max(0.0)
                } else {
                    0.0
                };
                // Each manifold point applies its share of the impulse: with
                // N simultaneous points at the same closing speed (a face
                // landing flat), the unshared impulses would sum to N× the
                // needed correction and bounce the body off the surface.
                let impulse = -(1.0 + restitution) * vn / (denominator * share);
                let mut delta = scale(normal, impulse * inverse_mass);
                let mut spin = mul_basis(
                    body.rates.world_inverse_inertia,
                    cross(r, scale(normal, impulse)),
                );
                // Retail combine friction caps the tangential impulse.
                let tangent = sub(velocity, scale(normal, vn));
                let speed = length(tangent);
                if speed > 1e-6 {
                    let limit = material.dynamic_friction.max(0.0) * impulse;
                    let friction = (-speed / denominator).clamp(-limit, limit);
                    let direction = scale(tangent, 1.0 / speed);
                    delta = add(delta, scale(direction, friction * inverse_mass));
                    spin = add(
                        spin,
                        mul_basis(
                            body.rates.world_inverse_inertia,
                            cross(r, scale(direction, friction)),
                        ),
                    );
                }
                corrections.linear_displacement =
                    add(corrections.linear_displacement, scale(delta, dt));
                corrections.angular_displacement =
                    add(corrections.angular_displacement, scale(spin, dt));
            }
            let penetration = (-gap - tuning.penetration_slop).max(0.0) * tuning.penetration_correction;
            corrections.position_displacement = add(
                corrections.position_displacement,
                scale(normal, penetration / count),
            );
        }
    }

    /// Two awake props split the impulse by their inverse masses. Positional
    /// correction is applied to this body only; the other accumulates its own
    /// when its turn comes (the pair is visited twice per tick).
    fn resolve_dynamic(
        &mut self,
        index: usize,
        other: usize,
        manifold: &PrimitiveContactManifold,
        material: RetailContactMaterial,
        corrections: &mut RetailReactionCorrections,
    ) {
        let dt = self.simulation.time_step;
        let tuning = self.bodies[index].tuning;
        let normal = manifold.normal;
        let count = manifold.count.max(1) as f32;
        for pair in &manifold.points[..manifold.count] {
            let gap = dot(sub(pair.a, pair.b), normal);
            if gap > tuning.contact_padding {
                continue;
            }
            let (body, other_body) = if index < other {
                let (a, b) = self.bodies.split_at_mut(other);
                (&a[index], &b[0])
            } else {
                let (a, b) = self.bodies.split_at_mut(index);
                (&b[0], &a[other])
            };
            let inverse_mass = body.inertia.inverse_mass + other_body.inertia.inverse_mass;
            let velocity = sub(body.velocity_at(pair.a), other_body.velocity_at(pair.b));
            let vn = dot(velocity, normal);
            if vn >= 0.0 {
                continue;
            }
            let restitution = if vn < -tuning.restitution_threshold {
                material.restitution.max(0.0)
            } else {
                0.0
            };
            // Same per-point sharing as the static contact above.
            let impulse = -(1.0 + restitution) * vn / (inverse_mass * count);
            let share = impulse * body.inertia.inverse_mass;
            corrections.linear_displacement = add(
                corrections.linear_displacement,
                scale(normal, share * dt),
            );
            let penetration = (-gap - tuning.penetration_slop).max(0.0) * tuning.penetration_correction;
            corrections.position_displacement = add(
                corrections.position_displacement,
                scale(normal, penetration * 0.5),
            );
        }
    }
}

/// How one skater volume touched a prop this tick.
#[derive(Clone, Copy, PartialEq)]
enum SkaterContact {
    Separate,
    /// Inside the box but not closing on it.
    OverlapOnly,
    Closing,
}

fn same_pose(a: (Vector3, Basis3), b: (Vector3, Basis3)) -> bool {
    a.0.x.to_bits() == b.0.x.to_bits()
        && a.0.y.to_bits() == b.0.y.to_bits()
        && a.0.z.to_bits() == b.0.z.to_bits()
        && a.1
            .columns
            .iter()
            .flatten()
            .zip(b.1.columns.iter().flatten())
            .all(|(x, y)| x.to_bits() == y.to_bits())
}

/// skate_vita (performance): the static triangles a prop box within `bounds` can touch, in the
/// order upstream visited them (candidate meshes, ascending indices): those whose bounds reach
/// `bounds` padded by the world's largest triangle fatness and thin margin (the padding of the
/// mesh query). Upstream ran the box-triangle test on every triangle of the candidate meshes: a
/// district's meshes hold hundreds to thousands, ~12 ms per awake prop per tick on the Vita.
fn near_triangles(world: &BoardWorld, bounds: skate_core::physics::board_world::query_metadata::Bounds) -> Vec<usize> {
    let (fatness, margin) = world.broadphase_limits();
    let mut out = Vec::new();
    world.triangles_overlapping(0..world.triangles().len(), bounds.expanded(fatness + margin), &mut out);
    out
}

fn volume_bounds(
    primitive: ContactPrimitive,
) -> Option<skate_core::physics::board_world::query_metadata::Bounds> {
    let expanded = |center: Vector3, radius: f32| {
        let r = radius.abs();
        skate_core::physics::board_world::query_metadata::Bounds::from_points([
            Vector3::new(center.x - r, center.y - r, center.z - r),
            Vector3::new(center.x + r, center.y + r, center.z + r),
        ])
    };
    match primitive {
        ContactPrimitive::Sphere(sphere) => expanded(sphere.center, sphere.radius),
        ContactPrimitive::Capsule {
            center,
            axis,
            half_length,
            radius,
        } => {
            let offset = scale(axis, half_length);
            let a = sub(center, offset);
            let b = add(center, offset);
            let lo = Vector3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
            let hi = Vector3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
            let r = radius.abs();
            skate_core::physics::board_world::query_metadata::Bounds::from_points([
                Vector3::new(lo.x - r, lo.y - r, lo.z - r),
                Vector3::new(hi.x + r, hi.y + r, hi.z + r),
            ])
        }
        _ => None,
    }
}

/// Prop tuning a mod (or the engine) sets: the default plus per prop type
/// overrides by MOBJ template name. Setting it back to `default()` restores
/// the shipped values (mod disable). Applied to the live props before each
/// physics tick and again after a map load builds new props.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PropTuningSettings(pub PropTuningTable);




#[cfg(test)]
mod tests {
    use super::*;
    use crate::skate_world::build_prop_layer;
    use skate_core::physics::{
        board::BodyId,
        board_step::CollisionBody,
        collision::Sphere,
    };

    fn material() -> RetailContactMaterial {
        RetailContactMaterial {
            static_friction: 0.8,
            dynamic_friction: 0.6,
            restitution: 0.0,
        }
    }

    /// The static world material the game gives prop contacts
    /// (`PhysicsSettings::floor_material`, agCollision 8277C5D8 context
    /// 83034F34 / 38 / 3C): {0, 0, 1}, so the max / max / min combine keeps
    /// the prop's own block.
    fn floor_material() -> RetailContactMaterial {
        RetailContactMaterial { static_friction: 0.0, dynamic_friction: 0.0, restitution: 1.0 }
    }

    fn simulation() -> RetailSimulationStep {
        super::prop_simulation(RetailSimulationStep::fixed_60_hz(
            0,
            0.01,
            Vector3::new(0., -9.81, 0.),
        ))
    }

    /// Unit-cube template (±0.5) with a single instance placed at `origin`.
    fn fixture(
        origin: [f32; 3],
    ) -> (
        BoardWorld,
        crate::skate_world::PropCollisionLayer,
        PropDynamics,
    ) {
        box_fixture(origin, [0.5, 0.5, 0.5])
    }

    /// Box template with half extents `half` and one instance at `origin`.
    fn box_fixture(
        origin: [f32; 3],
        half: [f32; 3],
    ) -> (
        BoardWorld,
        crate::skate_world::PropCollisionLayer,
        PropDynamics,
    ) {
        let (corners, faces) = box_mesh(half);
        mesh_fixture(origin, &corners, &faces)
    }

    /// A box template's corners and outward faces (skate_vita: split from `box_fixture`).
    fn box_mesh(half: [f32; 3]) -> ([[f32; 3]; 8], [[u32; 3]; 12]) {
        let corners = [
            [-1., -1., -1.], [1., -1., -1.], [1., -1., 1.], [-1., -1., 1.],
            [-1., 1., -1.], [1., 1., -1.], [1., 1., 1.], [-1., 1., 1.],
        ]
        .map(|c: [f32; 3]| [c[0] * half[0], c[1] * half[1], c[2] * half[2]]);
        let faces = [
            [4, 7, 6], [4, 6, 5], // +Y top
            [0, 1, 2], [0, 2, 3], // -Y bottom
            [1, 5, 6], [1, 6, 2], // +X
            [0, 7, 4], [0, 3, 7], // -X
            [3, 2, 6], [3, 6, 7], // +Z
            [0, 5, 1], [0, 4, 5], // -Z
        ];
        (corners, faces)
    }

    /// One prop of the given template mesh (template-space corners and faces) placed at `origin`
    /// on the flat test floor (skate_vita: `box_fixture` split for other shapes).
    fn mesh_fixture(
        origin: [f32; 3],
        corners: &[[f32; 3]],
        faces: &[[u32; 3]],
    ) -> (
        BoardWorld,
        crate::skate_world::PropCollisionLayer,
        PropDynamics,
    ) {
        let (map, objects) = mesh_map(corners, faces, &[(7, origin)]);
        let layer = build_prop_layer(&map, &objects, floor_material()).unwrap().unwrap();
        let dynamics = PropDynamics::new(&objects, layer.instances(), simulation());
        let world = super::super::ground::Terrain::Flat.world(floor_material());
        (world, layer, dynamics)
    }

    /// A map of one template mesh and its placed props `(id, origin)` (skate_vita: split from
    /// `mesh_fixture`).
    fn mesh_map(
        corners: &[[f32; 3]],
        faces: &[[u32; 3]],
        placed: &[(u32, [f32; 3])],
    ) -> (skate_data::skate_map::SkateMap, Vec<skate_data::skate_map::StaticObject>) {
        let vertex = |position| skate_data::skate_map::Vertex {
            position,
            normal: [0., 1., 0.],
            uv: [0.; 2],
            lightmap_uv: [0.; 2],
            material: 1,
            decal_uv: None,
            tangent_frame: None,
        };
        let map = skate_data::skate_map::SkateMap {
            version: 14,
            name: "props".into(),
            spawn: [0.; 3],
            heading: 0.,
            environment: vec![0.; 45],
            materials: vec![skate_data::skate_map::Material {
                name: "prop".into(),
                flags: 0,
                friction: 0.5,
                restitution: 0.1,
                color: [1.; 3],
                roughness: 0.5,
                emissive: 0.,
                textures: [0; 5],
                indirect_strength: 0.,
                alpha_mode: 0,
                alpha_cutoff: 0.5,
                audio: 3,
                physics: 1,
                pattern: 0,
                depth_layer: None,
                retail_definition: None,
            }],
            textures: vec![],
            geometry: skate_data::skate_map::Geometry {
                vertices: corners.iter().copied().map(vertex).collect(),
                indices: faces.iter().flatten().copied().collect(),
                collision: vec![],
            },
            rails: vec![],
            doors: vec![],
            lights: vec![],
            routes: vec![],
            extensions: vec![],
        };
        let objects = placed
            .iter()
            .map(|&(id, origin)| skate_data::skate_map::StaticObject {
                id,
                name: "template/crate".into(),
                transform: [
                    1., 0., 0., 0., 1., 0., 0., 0., 1., origin[0], origin[1], origin[2],
                ],
                first_index: 0,
                index_count: faces.len() as u32 * 3,
                first_collision: 0,
                collision_count: 0,
                rails: vec![],
                physics: Default::default(),
            })
            .collect();
        (map, objects)
    }

    const REST_Y: f32 = super::super::ground::HEIGHT + 0.5;

    /// A prop dropped from the air falls, settles on the floor and sleeps; its
    /// collision triangles are re-baked at the new pose.
    #[test]
    fn dropped_prop_falls_settles_and_sleeps() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y + 5., 0.]);
        dynamics.bodies[0].wake();
        for _ in 0..240 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let body = &dynamics.bodies[0];
        assert!(body.asleep, "prop should cool down to sleep: {:#?}", body.rates);
        assert!(
            (body.rates.position.y - REST_Y).abs() < 0.1,
            "resting height {}",
            body.rates.position.y
        );
        // Re-baked triangles: a probe from the drop height hits the top face.
        let hit = layer
            .world()
            .query_thin_line(Vector3::new(0., REST_Y + 5., 0.), Vector3::new(0., REST_Y - 1., 0.))
            .unwrap()
            .unwrap();
        assert!((hit.geometry.position.y - (REST_Y + 0.5)).abs() < 0.1);
    }

    /// Template-space triangles of a box with half extents `half` (outward winding).
    fn box_triangles(half: f32) -> Vec<[Vector3; 3]> {
        let c = [
            [-1., -1., -1.], [1., -1., -1.], [1., -1., 1.], [-1., -1., 1.],
            [-1., 1., -1.], [1., 1., -1.], [1., 1., 1.], [-1., 1., 1.],
        ]
        .map(|c: [f32; 3]| Vector3::new(c[0] * half, c[1] * half, c[2] * half));
        [[4, 7, 6], [4, 6, 5], [0, 1, 2], [0, 2, 3], [1, 5, 6], [1, 6, 2], [0, 7, 4], [0, 3, 7], [3, 2, 6], [3, 6, 7], [0, 5, 1], [0, 4, 5]]
            .map(|f: [usize; 3]| f.map(|i| c[i]))
            .to_vec()
    }

    fn thrown(origin: Vector3, velocity: Vector3) -> RuntimeProp {
        RuntimeProp {
            template: "template/can".into(),
            local: box_triangles(0.5),
            physics: Default::default(),
            type_data: None,
            origin,
            basis: Basis3 { columns: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] },
            linear_velocity: velocity,
            angular_velocity: Vector3::ZERO,
        }
    }

    fn spawn(layer: &mut crate::skate_world::PropCollisionLayer, dynamics: &mut PropDynamics, spec: &RuntimeProp) -> u32 {
        let id = dynamics.next_runtime_id();
        let instance = layer.add_instance(id, spec.local.clone(), 0, spec.basis.columns, spec.origin).unwrap();
        assert!(dynamics.spawn_body(instance, id, spec));
        id
    }

    /// A prop created mid-game on a map without placed props flies with its
    /// throw velocity, lands, sleeps and collides like a map prop; it has no
    /// authored pose (not "moved", no reset); removing it parks its triangles
    /// and the next prop of the same size reuses the slot.
    #[test]
    fn runtime_prop_is_thrown_lands_and_is_removed() {
        let world = super::super::ground::Terrain::Flat.world(floor_material());
        let mut layer = crate::skate_world::PropCollisionLayer::empty(floor_material()).unwrap();
        let mut dynamics = PropDynamics::empty(simulation());
        let id = spawn(&mut layer, &mut dynamics, &thrown(Vector3::new(0., REST_Y + 2., 0.), Vector3::new(3., 2., 0.)));
        assert_eq!(id, RUNTIME_PROP_ID_BASE);
        assert!(dynamics.is_runtime(id));
        dynamics.step(&world, &mut layer, &[]);
        assert!(dynamics.position_of(id).unwrap().x > 0.0, "throw velocity carries it");
        for _ in 0..400 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let (origin, _) = dynamics.pose(id).unwrap();
        let body = &dynamics.bodies[0];
        assert!(body.asleep, "landed prop sleeps: {:#?}", body.rates);
        assert!(origin.x > 1.0, "landed down range: {origin:?}");
        assert!((body.rates.position.y - REST_Y).abs() < 0.1, "resting height {}", body.rates.position.y);
        let probe = |layer: &crate::skate_world::PropCollisionLayer| {
            layer.world().query_thin_line(Vector3::new(origin.x, REST_Y + 5., origin.z), Vector3::new(origin.x, REST_Y - 1., origin.z)).unwrap()
        };
        let hit = probe(&layer).expect("its triangles are rebaked where it landed");
        assert!((hit.geometry.position.y - (REST_Y + 0.5)).abs() < 0.1);
        assert!(dynamics.moved_ids().is_empty(), "a runtime prop has no authored pose");
        assert_eq!(dynamics.reset_to_spawn(id), None);

        let triangles = layer.world().triangles().len();
        let instance = dynamics.remove_body(id).unwrap();
        layer.retire_instance(instance, HELD_PARK).unwrap();
        assert!(dynamics.pose(id).is_none() && !dynamics.is_runtime(id));
        assert!(probe(&layer).is_none(), "removed prop no longer collides");
        assert!(layer.retire_instance(instance, HELD_PARK).is_err(), "retired once");

        let again = spawn(&mut layer, &mut dynamics, &thrown(Vector3::new(5., REST_Y + 1., 0.), Vector3::ZERO));
        assert_ne!(again, id);
        assert_eq!(layer.world().triangles().len(), triangles, "same-size slot reused");
        assert_eq!(dynamics.instance_of(again), Some(instance));
    }

    /// Map props are never removed by the runtime path, and adding a runtime
    /// prop leaves the map prop's body and triangles as they were.
    #[test]
    fn runtime_spawn_keeps_map_props() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        let before: Vec<_> = layer.world().triangles().iter().map(|t| (t.triangle.vertices, t.tag)).collect();
        let pose = dynamics.pose(7);
        let id = spawn(&mut layer, &mut dynamics, &thrown(Vector3::new(10., REST_Y + 1., 0.), Vector3::ZERO));
        let after: Vec<_> = layer.world().triangles()[..before.len()].iter().map(|t| (t.triangle.vertices, t.tag)).collect();
        assert_eq!(after, before);
        assert_eq!(dynamics.pose(7), pose);
        assert_eq!(dynamics.remove_body(7), None);
        assert!(layer.retire_instance(0, HELD_PARK).is_err());
        for _ in 0..240 {
            dynamics.step(&world, &mut layer, &[]);
        }
        assert!(dynamics.pose(id).is_some());
        assert_eq!(dynamics.moved_ids(), Vec::<u32>::new());
    }

    /// A moving skater sphere wakes a resting prop and pushes it sideways.
    #[test]
    fn skater_volume_pushes_resting_prop() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        let start_x = dynamics.bodies[0].rates.position.x;
        assert!(dynamics.bodies[0].asleep);
        let volumes = [BoardWorldVolume {
            collision_group: 4,
            body: CollisionBody::Board(BodyId::Deck),
            primitive: ContactPrimitive::Sphere(Sphere {
                center: Vector3::new(-0.55, REST_Y, 0.),
                radius: 0.2,
            }),
            motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: Vector3::new(2., 0., 0.), ..Default::default() },
            material: material(),
        }];
        for _ in 0..30 {
            dynamics.step(&world, &mut layer, &volumes);
        }
        let body = &dynamics.bodies[0];
        assert!(
            body.rates.position.x > start_x + 0.02,
            "pushed from {start_x} to {}",
            body.rates.position.x
        );
    }

    /// The prop the skater grinds is not pushed by the skater's volumes (the user, from retail play); once the grind
    /// ends the same volume pushes it again.
    #[test]
    fn the_grinded_prop_is_not_pushed_by_the_skater() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        let start_x = dynamics.bodies[0].rates.position.x;
        let volumes = [BoardWorldVolume {
            collision_group: 4,
            body: CollisionBody::Board(BodyId::Deck),
            primitive: ContactPrimitive::Sphere(Sphere { center: Vector3::new(-0.55, REST_Y, 0.), radius: 0.2 }),
            motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: Vector3::new(2., 0., 0.), ..Default::default() },
            material: material(),
        }];
        dynamics.set_grinded(Some(7));
        for _ in 0..30 {
            dynamics.step(&world, &mut layer, &volumes);
        }
        assert!((dynamics.bodies[0].rates.position.x - start_x).abs() < 1e-3, "grinded prop moved to {}", dynamics.bodies[0].rates.position.x);
        dynamics.set_grinded(None);
        for _ in 0..30 {
            dynamics.step(&world, &mut layer, &volumes);
        }
        assert!(dynamics.bodies[0].rates.position.x > start_x + 0.02, "not pushed after the grind");
    }

    /// A settled prop stays put and finite over long idle ticks.
    #[test]
    fn settled_prop_does_not_sink_or_diverge() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y + 5., 0.]);
        dynamics.bodies[0].wake();
        for _ in 0..240 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let settled = dynamics.bodies[0].rates.position;
        for _ in 0..120 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let body = &dynamics.bodies[0];
        let p = body.rates.position;
        assert!(p.x.is_finite() && p.y.is_finite() && p.z.is_finite());
        assert!(p.y > REST_Y - 0.1, "sank to {}", p.y);
        assert!((p.y - settled.y).abs() < 0.02, "drifted {} -> {}", settled.y, p.y);
    }

    // Board stuck inside a prop (2026-10-05, bench near the Aletown spawn).

    /// Bench-sized box: 2.0 x 0.9 x 0.7 m, about 126 kg at the default
    /// density, resting on the flat ground.
    const BENCH_HALF: [f32; 3] = [1.0, 0.45, 0.35];
    const BENCH_REST_Y: f32 = super::super::ground::HEIGHT + 0.45;

    /// A parked deck capsule inside the bench box (under the seat, where the
    /// render mesh is open but the render-AABB contact box is solid).
    fn parked_deck() -> [BoardWorldVolume; 1] {
        [BoardWorldVolume {
            collision_group: 4,
            body: CollisionBody::Board(BodyId::Deck),
            primitive: ContactPrimitive::Capsule {
                center: Vector3::new(0.1, super::super::ground::HEIGHT + 0.1, 0.05),
                axis: Vector3::new(1., 0., 0.),
                half_length: 0.3,
                radius: 0.06,
            },
            motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: Vector3::ZERO, ..Default::default() },
            material: material(),
        }]
    }

    /// #15's push before the fix: overlap nudge forever, no board cap, no
    /// depenetration bound.
    fn legacy_tuning() -> PropTuningTable {
        PropTuningTable {
            default: PropTuning {
                stuck_release_ticks: u32::MAX,
                board_push_speed: f32::INFINITY,
                max_depenetration_per_tick: f32::INFINITY,
                ..PropTuning::default()
            },
            ..Default::default()
        }
    }

    /// Runs `ticks` steps with the deck parked inside the bench and returns
    /// (awake ticks, rebakes, skater pushes) over the last `tail` ticks plus
    /// the bench's horizontal travel.
    fn park_deck_in_bench(table: Option<PropTuningTable>, ticks: u32, tail: u32) -> (u32, u32, u32, f32) {
        let (world, mut layer, mut dynamics) = box_fixture([0., BENCH_REST_Y, 0.], BENCH_HALF);
        if let Some(table) = table {
            dynamics.set_tuning(table);
        }
        let start = dynamics.bodies[0].rates.position;
        let volumes = parked_deck();
        let (mut awake, mut rebakes, mut pushes) = (0, 0, 0);
        for tick in 0..ticks {
            dynamics.step(&world, &mut layer, &volumes);
            let stats = dynamics.last_step_stats();
            if tick >= ticks - tail {
                awake += stats.awake;
                rebakes += stats.rebakes;
                pushes += stats.skater_pushes;
            }
        }
        let end = dynamics.bodies[0].rates.position;
        let travel = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
        (awake, rebakes, pushes, travel)
    }

    /// Reproduction: with #15's push a parked deck inside a heavy bench box
    /// nudges it every tick (0.5 m/s floor, close to what ground friction
    /// takes off again), so the bench never sleeps and rebakes its triangles
    /// (and the whole prop layer's query index) every tick.
    #[test]
    fn legacy_push_keeps_bench_awake_with_parked_deck() {
        let (awake, rebakes, pushes, travel) = park_deck_in_bench(Some(legacy_tuning()), 600, 300);
        println!("legacy: awake {awake}/300 rebakes {rebakes} pushes {pushes} travel {travel:.3} m");
        // skate_vita: the push tests the bench's triangles, not its box: the deck parked
        // inside, clear of the faces, touches nothing even under #15's tuning (upstream: awake
        // 300, a push every tick, rebakes > 250).
        assert_eq!(awake, 0, "a deck clear of the faces does not wake the bench");
        assert_eq!(pushes, 0);
        assert_eq!(rebakes, 0);
    }

    /// Fix: the overlap nudge stops after `stuck_release_ticks`, so the bench
    /// cools down and sleeps with the deck still inside; no per-tick work.
    #[test]
    fn parked_deck_inside_bench_lets_it_sleep() {
        let (awake, rebakes, pushes, travel) = park_deck_in_bench(None, 600, 300);
        println!("fixed: awake {awake}/300 rebakes {rebakes} pushes {pushes} travel {travel:.3} m");
        assert_eq!(awake, 0, "bench should be asleep");
        assert_eq!(rebakes, 0);
        assert_eq!(pushes, 0);
        assert!(travel < 0.5, "bench travelled {travel} m");
    }

    /// skate_vita: a prop placed 2.5 cm above the floor is set down on it on its first
    /// step (a ramp's lip meets the ground), and the seated pose is its spawn pose.
    #[test]
    fn a_prop_placed_above_the_floor_is_seated() {
        let (world, mut layer, mut dynamics) = box_fixture([0., BENCH_REST_Y + 0.025, 0.], BENCH_HALF);
        dynamics.step(&world, &mut layer, &[]);
        let y = dynamics.bodies[0].rates.position.y;
        assert!((y - BENCH_REST_Y).abs() < 1e-4, "seated at {y}, floor rest {BENCH_REST_Y}");
        assert!(dynamics.moved_ids().is_empty(), "the seated pose is the spawn pose");
        let (origin, _) = dynamics.spawn_pose(7).unwrap();
        assert!((origin.y - BENCH_REST_Y).abs() < 1e-4);
    }

    /// skate_vita: a wheel on the floor at a ramp's lip gets no contact that holds it back.
    /// A wedge (lip at z = 0 on its base, rising 5 cm over 1 m like the retail quarter pipe's
    /// lip, with a bottom face) seated on the floor; a wheel sphere resting on the floor just in
    /// front of the lip, rolling toward it (-z), may meet the slope's face but no contact tilted
    /// back further (the knife-edge between the slope and the bottom face gave up to ~55°).
    #[test]
    fn a_ramp_lip_on_the_floor_does_not_hold_a_wheel_back() {
        let corners = [
            [-1., 0., 0.], [1., 0., 0.], // lip
            [-1., 0., -1.], [1., 0., -1.], // back, floor
            [-1., 0.05, -1.], [1., 0.05, -1.], // back, top
        ];
        let faces = [
            [0, 1, 5], [0, 5, 4], // slope
            [2, 3, 1], [2, 1, 0], // bottom
            [3, 2, 4], [3, 4, 5], // back
            [0, 4, 2], [1, 3, 5], // sides
        ];
        let floor = super::super::ground::HEIGHT;
        let (world, mut layer, mut dynamics) = mesh_fixture([0., floor + 0.025, 0.], &corners, &faces);
        dynamics.step(&world, &mut layer, &[]);
        let (query, retention) = super::super::ground::query_settings();
        let radius = 0.03;
        let face = 0.05 / (1.0_f32 + 0.05 * 0.05).sqrt();
        for z in [0.005_f32, 0.015, 0.025, 0.035] {
            let wheel = [BoardWorldVolume {
                collision_group: 4,
                body: CollisionBody::Board(BodyId::Deck),
                primitive: ContactPrimitive::Sphere(Sphere { center: Vector3::new(0., floor + radius, z), radius }),
                motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: Vector3::new(0., 0., -5.), ..Default::default() },
                material: material(),
            }];
            for c in layer.world_mut().query_primitives(&wheel, query, retention) {
                assert!(c.contact.normal.z < face + 0.01, "wheel {z} m before the lip held back: normal {:?}", c.contact.normal);
            }
        }
    }

    /// skate_vita: a map prop has collision only while streamed in. Streamed out, its slot
    /// is parked and freed; the next prop of its template streamed in takes it (the world does
    /// not grow) and collides where it stands, the first one nowhere.
    #[test]
    fn streamed_props_share_their_templates_collision_slots() {
        let (corners, faces) = box_mesh([0.5; 3]);
        let placed = [(1, [0., REST_Y, 0.]), (2, [10., REST_Y, 0.]), (3, [20., REST_Y, 0.])];
        let (map, objects) = mesh_map(&corners, &faces, &placed);
        let mut layer = crate::skate_world::prop_layer_unbaked(&map, &objects, floor_material(), &Default::default()).unwrap().unwrap();
        assert!(layer.world().triangles().is_empty(), "no collision before streaming in");
        let mut dynamics = PropDynamics::new(&objects, layer.instances(), simulation());
        let world = super::super::ground::Terrain::Flat.world(floor_material());
        let hits = |layer: &crate::skate_world::PropCollisionLayer, x: f32| {
            layer.world().query_thin_line(Vector3::new(x, REST_Y + 5., 0.), Vector3::new(x, REST_Y - 1., 0.)).unwrap().is_some()
        };
        // Prop 3 streams out before the first step: props 1 and 2 take slots.
        let (instance, _, _) = dynamics.set_dormant(3, true, false).unwrap();
        assert!(layer.rebake_many(&[(instance, None)]).is_empty());
        dynamics.step(&world, &mut layer, &[]);
        assert_eq!(layer.world().triangles().len(), 24);
        assert!(hits(&layer, 0.) && hits(&layer, 10.) && !hits(&layer, 20.));
        // Prop 1 out, prop 3 in: 3 takes 1's slot.
        let (out, _, _) = dynamics.set_dormant(1, true, false).unwrap();
        let (back, origin, basis) = dynamics.set_dormant(3, false, true).unwrap();
        assert!(layer.rebake_many(&[(out, None), (back, Some((basis.columns, origin)))]).is_empty());
        assert_eq!(layer.world().triangles().len(), 24, "the freed slot is reused");
        assert!(!hits(&layer, 0.) && hits(&layer, 10.) && hits(&layer, 20.));
        assert!(!layer.has_slot(out) && layer.has_slot(back));
    }

    /// A real hit after the release still pushes the prop.
    #[test]
    fn released_bench_still_takes_a_real_hit() {
        let (world, mut layer, mut dynamics) = box_fixture([0., BENCH_REST_Y, 0.], BENCH_HALF);
        let parked = parked_deck();
        for _ in 0..300 {
            dynamics.step(&world, &mut layer, &parked);
        }
        assert!(dynamics.bodies[0].asleep);
        let start = dynamics.bodies[0].rates.position.z;
        let mut hit = parked;
        hit[0].primitive = ContactPrimitive::Sphere(Sphere {
            center: Vector3::new(0., BENCH_REST_Y, -0.35 - 0.15),
            radius: 0.2,
        });
        hit[0].motion.linear_velocity = Vector3::new(0., 0., 4.);
        for _ in 0..20 {
            dynamics.step(&world, &mut layer, &hit);
        }
        assert!(dynamics.bodies[0].rates.position.z > start + 0.01);
    }

    /// The board push is capped like the body bump: a board held against a
    /// light prop cannot drive it past `board_push_speed`.
    #[test]
    fn board_push_speed_is_capped() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        let mut table = PropTuningTable::default();
        table.default.board_push_speed = 0.8;
        dynamics.set_tuning(table);
        let mut fastest = 0.0_f32;
        for tick in 0..60 {
            let x = dynamics.bodies[0].rates.position.x - 0.55 - 0.05 + tick as f32 * 0.0;
            let volumes = [BoardWorldVolume {
                collision_group: 4,
                body: CollisionBody::Board(BodyId::Deck),
                primitive: ContactPrimitive::Sphere(Sphere {
                    center: Vector3::new(x, REST_Y, 0.),
                    radius: 0.2,
                }),
                motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: Vector3::new(10., 0., 0.), ..Default::default() },
                material: material(),
            }];
            dynamics.step(&world, &mut layer, &volumes);
            fastest = fastest.max(dynamics.bodies[0].rates.linear_velocity.x);
        }
        assert!(fastest <= 0.8 + 1e-3, "pushed to {fastest} m/s");
        assert!(fastest > 0.1, "push still applies: {fastest}");
    }

    /// Depenetration in the engine's older impulse pass (mod option
    /// `row_solver = false`) is bounded per tick: a box dropped deep into the
    /// floor rises at most `max_depenetration_per_tick` per tick from the
    /// positional correction.
    #[test]
    fn depenetration_is_bounded_per_tick() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y - 0.4, 0.]);
        dynamics.set_tuning(PropTuningTable {
            solver: PropSolverSettings { row_solver: false, ..Default::default() },
            ..Default::default()
        });
        dynamics.bodies[0].wake();
        let cap = dynamics.tuning().default.max_depenetration_per_tick;
        let mut previous = dynamics.bodies[0].rates.position.y;
        for _ in 0..30 {
            dynamics.step(&world, &mut layer, &[]);
            let y = dynamics.bodies[0].rates.position.y;
            // Velocity terms add at most a few mm on top of the correction.
            assert!(y - previous <= cap + 0.02, "rose {} m in one tick", y - previous);
            previous = y;
        }
        assert!(previous > REST_Y - 0.4 + 0.05, "the box still comes out: {previous}");
    }

    /// The retail row solver has no per-tick cap: a box 0.4 m deep in the
    /// floor comes out on the full predicted-separation target and settles.
    #[test]
    fn retail_rows_push_a_deep_box_out_and_it_settles() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y - 0.4, 0.]);
        dynamics.bodies[0].wake();
        let mut previous = dynamics.bodies[0].rates.position.y;
        let mut largest_rise = 0.0f32;
        let mut fastest = 0.0f32;
        let mut highest = previous;
        let mut slept_at = None;
        for tick in 0..240 {
            dynamics.step(&world, &mut layer, &[]);
            let body = &dynamics.bodies[0];
            let y = body.rates.position.y;
            largest_rise = largest_rise.max(y - previous);
            fastest = fastest.max(body.rates.linear_velocity.y);
            highest = highest.max(y);
            if slept_at.is_none() && body.asleep {
                slept_at = Some(tick);
            }
            previous = y;
        }
        println!("deep box: largest rise {largest_rise:.4} m/tick, fastest up {fastest:.3} m/s, highest {:.4} over rest, end {:.4} over rest, slept at {slept_at:?}", highest - REST_Y, previous - REST_Y);
        assert!((previous - REST_Y).abs() < 0.02, "the box rests on the floor: {}", previous - REST_Y);
        assert!(slept_at.is_some(), "the box sleeps");
    }

    /// Per prop type tuning: an override keyed by template name applies to
    /// that type only, a collision box override keeps the rendered pose, and
    /// reset restores the defaults.
    #[test]
    fn tuning_overrides_per_template_and_resets() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 0.]);
        let origin = dynamics.bodies[0].origin();
        let mut table = PropTuningTable::default();
        table.by_template.insert(
            "template/crate".into(),
            PropTuning {
                stuck_release_ticks: 5,
                collision_box: Some(PropBox {
                    center: Vector3::new(0., -0.25, 0.),
                    half_extents: Vector3::new(0.5, 0.25, 0.5),
                }),
                ..PropTuning::default()
            },
        );
        dynamics.set_tuning(table);
        let body = &dynamics.bodies[0];
        assert_eq!(body.tuning.stuck_release_ticks, 5);
        assert!((body.half_extents.y - 0.25).abs() < 1e-6);
        let moved = body.origin();
        assert!((moved.y - origin.y).abs() < 1e-5, "rendered pose must not move");
        dynamics.reset_tuning();
        let body = &dynamics.bodies[0];
        assert_eq!(body.tuning, PropTuning::default());
        assert!((body.half_extents.y - 0.5).abs() < 1e-6);
    }

    /// Frame-cost probe on the converted DownTown props (private assets,
    /// `SKATE3_ASSET_ROOT`): time of one prop rebake, which also rebuilds the
    /// prop layer's query index. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn downtown_prop_rebake_cost() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT"),
        );
        let (mut layer, dynamics) = crate::skate_world::load_prop_layer(
            &root,
            "DownTown",
            floor_material(),
            simulation(),
        )
        .expect("DownTown props");
        let bench = dynamics
            .bodies
            .iter()
            .position(|b| b.template.to_ascii_lowercase().contains("bench"))
            .unwrap_or(0);
        let body = &dynamics.bodies[bench];
        let (origin, basis) = (body.origin(), body.rates.basis);
        let started = std::time::Instant::now();
        for _ in 0..100 {
            layer.rebake(body.instance, basis.columns, origin).unwrap();
        }
        let each = started.elapsed() / 100;
        println!(
            "DownTown: {} props, {} triangles; {} ({} tris, box {:?}) rebake {:?} each",
            dynamics.bodies.len(),
            layer.world().triangles().len(),
            body.template,
            layer.instances()[body.instance].range.len(),
            body.half_extents,
            each
        );
    }

    // Phase 3: offboard carry glue (`crate::physics::prop_carry`).

    fn carrier(state: skate_core::player::state::PhysicalStateId, z: f32) -> crate::physics::prop_carry::Carrier {
        crate::physics::prop_carry::Carrier {
            state,
            position: Vector3::new(0., super::super::ground::HEIGHT + 0.9, z),
            forward: Vector3::new(0., 0., 1.),
            time_step: simulation().time_step,
            skeleton: None,
            ground_grab: None,
        }
    }

    /// One carry tick with the grab button held (the carry keeps its prop).
    fn tick() -> crate::physics::prop_carry::Tick {
        crate::physics::prop_carry::Tick {
            grab: true,
            ..Default::default()
        }
    }

    /// Grab button released: drops a carry or confirms a placement.
    fn release() -> crate::physics::prop_carry::Tick {
        crate::physics::prop_carry::Tick::default()
    }

    /// Derived controller words with the given raw flag bits held this tick
    /// (word 13) and on the previous tick (word 6).
    fn controller_words(now: &[u32], before: &[u32]) -> [u32; 26] {
        let mut words = [0u32; 26];
        for bit in now {
            words[13] |= 1 << bit;
        }
        for bit in before {
            words[6] |= 1 << bit;
        }
        words
    }

    /// Video bug (2026-10-05, Aletown spawn): sprinting (A, raw bit 21) past
    /// props grabbed them and dragged them along. A is the retail sprint
    /// button, so pressing or holding it must never grab, and a prop passed
    /// at a run stays where it was.
    #[test]
    fn sprinting_past_a_prop_does_not_grab_it() {
        let (world, mut layer, mut dynamics) = fixture([0.8, REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let state = skate_core::player::state::PhysicalStateId::BipedGround;
        let buttons = carry.buttons();
        let start = dynamics.position_of(7).unwrap();
        // Run along +Z at 5 m/s past the prop, which sits 0.8 m to the side.
        let speed_per_tick = 5.0 * simulation().time_step;
        for i in 0..90 {
            let words = if i == 0 {
                controller_words(&[21], &[])
            } else {
                controller_words(&[21], &[21])
            };
            let tick = crate::physics::prop_carry::Tick::from_controller(&words, buttons, 0., 0., 0.);
            assert!(!tick.grab, "sprint (A) produced a grab on tick {i}");
            carry.update(&mut dynamics, tick, carrier(state, -1.0 + speed_per_tick * i as f32));
            assert_eq!(carry.held(), None, "sprinting grabbed the prop on tick {i}");
            dynamics.step(&world, &mut layer, &[]);
        }
        let end = dynamics.position_of(7).unwrap();
        let moved = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
        assert!(moved < 0.05, "prop followed the runner: moved {moved} m");
    }

    /// The retail GrabWorld button (RB, raw bit 28) grabs while held and
    /// releasing it drops the prop.
    #[test]
    fn grab_world_button_holds_and_release_drops() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let state = skate_core::player::state::PhysicalStateId::BipedGround;
        let buttons = carry.buttons();
        let from = |now: &[u32], before: &[u32]| {
            crate::physics::prop_carry::Tick::from_controller(
                &controller_words(now, before),
                buttons,
                0.,
                0.,
                0.,
            )
        };
        carry.update(&mut dynamics, from(&[28], &[]), carrier(state, 0.));
        assert_eq!(carry.held(), Some(7));
        for i in 0..30 {
            carry.update(&mut dynamics, from(&[28], &[28]), carrier(state, 0.));
            dynamics.step(&world, &mut layer, &[]);
            assert_eq!(carry.held(), Some(7), "held grab dropped on tick {i}");
        }
        carry.update(&mut dynamics, from(&[], &[28]), carrier(state, 1.5));
        assert_eq!(carry.held(), None, "releasing the grab button must drop");
        // B is a rising edge for placement, not a level.
        assert!(from(&[20], &[]).placement);
        assert!(!from(&[20], &[20]).placement);
    }

    /// One held tick with the OB_ObjectMv intents (left stick X / Z, right stick X).
    fn stick(x: f32, z: f32, rot: f32) -> crate::physics::prop_carry::Tick {
        crate::physics::prop_carry::Tick { grab: true, object_move: [x, z, rot], ..Default::default() }
    }

    /// The skater follows the prop's grab frame (Move Object: the prop leads),
    /// as `biped_ground` does in the game; height stays the carrier's.
    fn follow(
        carry: &crate::physics::prop_carry::PropCarry,
        carrier: crate::physics::prop_carry::Carrier,
    ) -> crate::physics::prop_carry::Carrier {
        match carry.skater_target() {
            Some((p, f)) => crate::physics::prop_carry::Carrier {
                position: Vector3::new(p.x, carrier.position.y, p.z),
                forward: f,
                ..carrier
            },
            None => carrier,
        }
    }

    /// Heading in the yaw-rate sense (positive angular velocity about +Y
    /// increases it), like `HeldBody::heading`.
    fn heading_of(dynamics: &PropDynamics, id: u32) -> f32 {
        let z = dynamics.pose(id).unwrap().1.columns[2];
        z[0].atan2(z[2])
    }

    /// The axis convention the Move Object heading relies on: a positive yaw
    /// rate on a free body turns its local +Z toward world +X (right-handed).
    #[test]
    fn positive_yaw_rate_turns_local_z_toward_plus_x() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y + 20., 0.]);
        dynamics.bodies[0].wake();
        dynamics.bodies[0].rates.angular_velocity = Vector3::new(0., 1., 0.);
        for _ in 0..10 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let z = dynamics.pose(7).unwrap().1.columns[2];
        assert!(z[0] > 0.1, "local +Z after a positive yaw rate: {z:?}");
        assert!(heading_of(&dynamics, 7) > 0.1);
    }

    /// What the retail controller math alone predicts for a centre push on a
    /// free point mass (no contacts, no friction): 82D45318 steps 1 to 10
    /// (`move_object::command`) read the body velocity before the step and
    /// the slot 9 sink adds the sent command as an acceleration in the same
    /// step (`v += L dt`, spec 7.5). Returns the speed after every tick.
    fn predicted_centre_push(mass: f32, move_z: f32, ticks: usize) -> Vec<f32> {
        use skate_core::player::offboard::move_object::{command, MoveObjectController, MoveObjectInput};
        let t = crate::physics::prop_carry::CarryLocomotion::default().move_object;
        let dt = simulation().time_step;
        let mut state = MoveObjectController::default();
        let mut v = 0.0f32;
        let mut z = 1.2f32;
        (0..ticks)
            .map(|_| {
                let input = MoveObjectInput {
                    move_z,
                    move_x: 0.0,
                    move_rotation: 0.0,
                    forward: [0.0, 0.0, 1.0],
                    grip: [0.0, 0.0, z - 0.5],
                    center: [0.0, 0.0, z],
                    velocity: [0.0, 0.0, v],
                    heading: 0.0,
                    mass,
                    yaw_inertia: 1.0,
                    contact_normal: [0.0; 3],
                    record_272: false,
                };
                v += command(&t, &mut state, &input).linear[2] * dt;
                z += v * dt;
                v
            })
            .collect()
    }

    /// Object-relative stick (82D45318): with the prop grabbed by the centre
    /// of its near face, the left stick in any direction never turns it
    /// (zero lever, the left stick has no rotation share), Z pushes / pulls
    /// along the grab-edge normal and X slides along the edge, and the
    /// skater stays on the grab frame.
    #[test]
    fn move_object_left_stick_moves_the_prop_in_the_edge_frame() {
        for step in 0..8 {
            let a = step as f32 * std::f32::consts::TAU / 8.0;
            let (x, z) = (a.sin(), a.cos());
            let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
            let mut carry = crate::physics::prop_carry::PropCarry::default();
            let mut at = carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.);
            carry.update(&mut dynamics, tick(), at);
            dynamics.set_held(carry.held());
            let start = dynamics.position_of(7).unwrap();
            at.state = skate_core::player::state::PhysicalStateId::OffBoardPushing;
            for _ in 0..90 {
                at = follow(&carry, at);
                carry.update(&mut dynamics, stick(x, z, 0.0), at);
                dynamics.set_held(carry.held());
                dynamics.step(&world, &mut layer, &[]);
            }
            assert_eq!(carry.held(), Some(7), "stick ({x:.2}, {z:.2}) lost the grab");
            let end = dynamics.position_of(7).unwrap();
            let (dx, dz) = (end.x - start.x, end.z - start.z);
            let travelled = (dx * dx + dz * dz).sqrt();
            let t = crate::physics::prop_carry::CarryLocomotion::default().move_object;
            // Facing +Z, stick right is the skater's right, -X (the +400 side row, b241; laptop 2026-10-18).
            let (ex, ez) = (-x * t.side_speed, z * if z > 0.0 { t.push_speed } else { t.pull_speed });
            let expected = (ex * ex + ez * ez).sqrt();
            assert!(travelled > 0.5, "stick ({x:.2}, {z:.2}) did not move the prop: {travelled}");
            let along = (dx * ex + dz * ez) / (travelled * expected);
            assert!(along > 0.97, "stick ({x:.2}, {z:.2}) moved the prop off its direction: {along}");
            assert!(heading_of(&dynamics, 7).abs() < 0.05, "left stick turned the prop: {}", heading_of(&dynamics, 7));
            let target = carry.skater_target().unwrap().0;
            let lag = ((target.x - at.position.x).powi(2) + (target.z - at.position.z).powi(2)).sqrt();
            assert!(lag < 0.2, "skater fell {lag} m behind the grab frame");
        }
    }

    /// Better Object Mover: a prop without authored grab splines gets one generated spline per vertical box face (the
    /// authored layout: outward approach, flags 4) and loses them again when it goes off; a later map load (new
    /// splines) keeps them while on. Off = retail: no grab object for it.
    #[test]
    fn better_object_mover_gives_props_without_grab_splines_generated_ones() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 1.2]);
        assert_eq!(dynamics.set_grab_splines(&std::collections::BTreeMap::new()), 0);
        assert!(dynamics.grab_object(7).is_none(), "retail: no grab handles, not movable");
        assert_eq!(dynamics.set_better_object_mover(true), 1);
        let object = dynamics.grab_object(7).expect("generated grab object");
        assert_eq!(object.splines.len(), 4);
        for spline in &object.splines {
            let a = spline.geometry.approach_vectors[0];
            assert!((a[0].abs() + a[2].abs() - 1.0).abs() < 1e-6 && a[1] == 0.0, "outward horizontal approach {a:?}");
            assert_eq!(spline.geometry.word_60, 4);
        }
        assert_eq!(dynamics.set_grab_splines(&std::collections::BTreeMap::new()), 0);
        assert!(dynamics.grab_object(7).is_some(), "kept across a spline reload while on");
        assert_eq!(dynamics.set_better_object_mover(false), 0);
        assert!(dynamics.grab_object(7).is_none());
    }

    /// The carry binds exactly the record the Move Object enter passed (82D442D0, b233): the decision on tick N
    /// starts nothing, the published record binds on N+1, even when another record of the prop is nearer (the large
    /// dumpster's bottom edges, b224 / b226).
    #[test]
    fn the_carry_binds_the_published_record_at_the_move_object_enter() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut splines = std::collections::BTreeMap::new();
        // 0: a bottom edge on the near face (approach straight down); 1: the rim higher up the same face.
        splines.insert("template".to_owned(), vec![
            crate::living_world::vehicles::CarGrabSpline { points: vec![[-0.4, -0.5, -0.5], [0.4, -0.5, -0.5]], direction: [0.0, -1.0, 0.0], flags: 0 },
            crate::living_world::vehicles::CarGrabSpline { points: vec![[-0.4, 0.4, -0.5], [0.4, 0.4, -0.5]], direction: [0.0, 0.0, -1.0], flags: 0 },
        ]);
        assert_eq!(dynamics.set_grab_splines(&splines), 1);
        // 82585F58: the record's part / rates block is filled, so the enter's +216 gate (82D442D0) passes.
        let object = dynamics.grab_object(7).unwrap();
        let part = object.assembly.as_ref().and_then(|a| a.first_part.as_ref()).expect("a part");
        // Grab ids pack the body's grab slot (0: the fixture's first body), not its id (map ids are 32-bit).
        assert_eq!((part.identity, part.rates.as_ref().map(|r| r.identity)), (PROP_GRAB_TAG, Some(PROP_GRAB_TAG)));
        assert_eq!(part.coefficients_0_to_36[4], dynamics.bodies[0].inertia.inverse_mass);
        let key = |i: u32| PROP_GRAB_TAG | i;
        assert_eq!(dynamics.prop_of_grab_key(key(1), true), Some(7));
        assert_eq!(dynamics.prop_of_grab_key(PROP_GRAB_TAG, false), Some(7));
        assert_eq!(dynamics.prop_of_grab_key(1 << 6, true), None, "not a prop key");
        let records = crate::physics::prop_carry::authored_records(&dynamics, 7, true).unwrap();
        let rim = records.iter().find(|r| r.0[48] == key(1)).unwrap().clone();
        assert!(rim.0[49] != 0 && rim.0[54] != 0, "the enter's geometry and rates gate");
        let at = crate::physics::prop_carry::Carrier {
            ground_grab: Some(Some((2, key(1)))),
            ..carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.)
        };
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        carry.set_authored_records(true);
        carry.update(&mut dynamics, tick(), at);
        assert_eq!(carry.held(), None, "tick N: the decision alone starts nothing");
        carry.bind_published(rim);
        let at = crate::physics::prop_carry::Carrier { ground_grab: Some(None), state: skate_core::player::state::PhysicalStateId::OffBoardPushing, ..at };
        carry.update(&mut dynamics, tick(), at);
        assert_eq!((carry.held(), carry.held_grip().map(|g| g.descriptor)), (Some(7), Some((2, key(1)))), "binds the published rim");
        // Nothing published: no carry, also with grab held in 502.
        let mut idle = crate::physics::prop_carry::PropCarry::default();
        idle.set_authored_records(true);
        idle.update(&mut dynamics, tick(), at);
        assert_eq!(idle.held(), None);
    }
    /// Move Object step 3: a prop with an authored grab spline is carried by
    /// that record (82D444A0 / 82D45D30), not the box stand-in: the grip sits
    /// on the spline, the facing is minus its approach vector, and the grip
    /// arc stays bound while the prop is pushed.
    #[test]
    fn a_prop_with_grab_splines_is_carried_by_its_authored_record() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut splines = std::collections::BTreeMap::new();
        // Spline on the -Z face (toward the skater), off centre along X.
        splines.insert("template".to_owned(), vec![crate::living_world::vehicles::CarGrabSpline { points: vec![[-0.4, 0.3, -0.5], [0.0, 0.3, -0.5], [0.4, 0.3, -0.5]], direction: [0.0, 0.0, -1.0], flags: 0 }]);
        assert_eq!(dynamics.set_grab_splines(&splines), 1);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        carry.set_authored_records(true);
        let mut at = carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.);
        at.position.x = 0.35;
        carry.update(&mut dynamics, tick(), at);
        assert_eq!(carry.held(), Some(7));
        let grip = carry.held_grip().expect("bound to the authored record");
        let scale = dynamics.bodies[0].axis_scale;
        let length = 0.8 * scale.x;
        // Grip clamped by the end exclusion (0.25) from the skater's x.
        let expected = (0.35f32 + 0.4 * scale.x).min(length - 0.25);
        let arc = if grip.reversed { length - grip.grip } else { grip.grip };
        assert!((arc - expected).abs() < 1e-3, "{grip:?} expected arc {expected}");
        let (_, forward) = carry.skater_target().unwrap();
        assert!((forward.z - 1.0).abs() < 1e-4, "{forward:?}");
        dynamics.set_held(Some(7));
        at.state = skate_core::player::state::PhysicalStateId::OffBoardPushing;
        for _ in 0..60 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 1., 0.), at);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
        }
        assert_eq!(carry.held(), Some(7), "the push dropped the prop");
        assert_eq!(carry.held_grip().unwrap().descriptor, grip.descriptor);
        assert!(dynamics.position_of(7).unwrap().z > 1.25, "the prop did not move");
    }

    /// The grind surface probes (82C205D8 line queries) find a prop's top through the prop collision layer, which
    /// the grind host probes next to the map (retail queries one world with the DMOs in it).
    #[test]
    fn grind_surface_probes_hit_a_prop_through_the_prop_layer() {
        let (world, layer, dynamics) = fixture([0., REST_Y, 1.2]);
        let (origin, _) = dynamics.pose(7).unwrap();
        let c = dynamics.position_of(7).unwrap();
        let probe = skate_core::air::trajectory::grind_surface::Probe { start: [c.x, c.y + 2.0, c.z, 1.0], end: [c.x, c.y - 2.0, c.z, 1.0], radius: 0.0 };
        let on_prop = crate::physics::player_input::grind::world::surface_probe(layer.world(), [0, u32::MAX], 0, probe).unwrap();
        let hit = on_prop.expect("the probe hits the prop's top");
        assert!(hit.position[1] > origin.y && hit.normal[1] > 0.9, "{hit:?}");
        // The map alone (ground under the prop) answers lower or not at all: the host must ask the prop layer.
        if let Some(ground) = crate::physics::player_input::grind::world::surface_probe(&world, [0, u32::MAX], 0, probe).unwrap() {
            assert!(ground.fraction > hit.fraction, "{ground:?} {hit:?}");
        }
    }

    /// Prop rails (82C4C4E0): template-space segments placed by the current pose like the grab splines, owner
    /// (prop id, rail), flags 0; the radius rule, the shared cap and the held prop's exclusion apply.
    #[test]
    fn prop_rails_follow_the_pose_and_the_query_rules() {
        use crate::grind_world::{props::{prop_owner, PropRail, PropSegment}, RailRules, RailSource};
        let (_world, _layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let rail = PropRail { guids: [0x11, 0x22], segments: vec![
            PropSegment { start: [-0.4, 0.3, -0.5, 1.0], end: [0.0, 0.3, -0.5, 1.0], min: [-0.4, 0.3, -0.5], max: [0.0, 0.3, -0.5] },
            PropSegment { start: [0.0, 0.3, -0.5, 1.0], end: [0.4, 0.3, -0.5, 1.0], min: [0.0, 0.3, -0.5], max: [0.4, 0.3, -0.5] },
        ] };
        assert_eq!(dynamics.set_grind_rails([("template".to_owned(), vec![rail])].into()), 1);
        let (origin, basis) = dynamics.pose(7).unwrap();
        let s = dynamics.bodies[0].axis_scale;
        let place = |p: [f32; 3]| add(origin, mul_basis(basis, Vector3::new(p[0] * s.x, p[1] * s.y, p[2] * s.z)));
        let mid = place([0.0, 0.3, -0.5]);
        let (min, max) = ([mid.x - 1.2, mid.y - 1.2, mid.z - 1.2], [mid.x + 1.2, mid.y + 1.2, mid.z + 1.2]);
        let rules = RailRules::default();
        let mut hits = Vec::new();
        dynamics.append_rails(min, max, None, &rules, &mut hits);
        assert_eq!(hits.len(), 2);
        for (i, (hit, [a, b])) in hits.iter().zip([[[-0.4, 0.3, -0.5], [0.0, 0.3, -0.5]], [[0.0, 0.3, -0.5], [0.4, 0.3, -0.5]]]).enumerate() {
            let (a, b) = (place(a), place(b));
            for (got, want) in [(hit.primitive.start, a), (hit.primitive.end, b)] {
                assert!((got[0] - want.x).abs() < 1e-5 && (got[1] - want.y).abs() < 1e-5 && (got[2] - want.z).abs() < 1e-5, "{got:?} {want:?}");
            }
            assert_eq!(hit.primitive.owner, prop_owner(7, 0));
            assert_eq!((hit.metadata.spline_guids, hit.metadata.segment_index, hit.metadata.flags), ([0x11, 0x22], i as u32, 0));
        }
        // A box away from the segments, the radius rule, the shared cap and the held prop.
        let far = [mid.x + 5.0, mid.y, mid.z];
        let mut none = Vec::new();
        dynamics.append_rails([far[0] - 1.0, far[1] - 1.0, far[2] - 1.0], [far[0] + 1.0, far[1] + 1.0, far[2] + 1.0], None, &rules, &mut none);
        dynamics.append_rails(min, max, None, &RailRules { prop_radius: 0.01, ..rules }, &mut none);
        dynamics.append_rails(min, max, Some(7), &rules, &mut none);
        assert!(none.is_empty(), "{none:?}");
        let mut capped = Vec::new();
        dynamics.append_rails(min, max, None, &RailRules { cap: 1, ..rules }, &mut capped);
        assert_eq!(capped.len(), 1);
        assert!(prop_owner(7, 0) > u64::from(u32::MAX), "prop owners never meet static 1-based rail owners");
    }

    /// skate_vita: the in-place prop rail query gives what upstream's posed list (`rail_poses`) gives, in id
    /// order across props placed out of id order, with the radius rule, the skipped prop and the cap.
    #[test]
    fn prop_rail_queries_match_the_posed_list() {
        use crate::grind_world::{props::{PropRail, PropSegment}, RailRules, RailSource};
        let (corners, faces) = box_mesh([0.5; 3]);
        let (map, objects) = mesh_map(&corners, &faces, &[(9, [0., REST_Y, 0.]), (3, [0.6, REST_Y, 0.]), (5, [30., REST_Y, 0.])]);
        let layer = build_prop_layer(&map, &objects, floor_material()).unwrap().unwrap();
        let mut dynamics = PropDynamics::new(&objects, layer.instances(), simulation());
        let rail = PropRail { guids: [0x11, 0x22], segments: vec![
            PropSegment { start: [-0.4, 0.3, -0.5, 1.0], end: [0.0, 0.3, -0.5, 1.0], min: [-0.4, 0.3, -0.5], max: [0.0, 0.3, -0.5] },
            PropSegment { start: [0.0, 0.3, -0.5, 1.0], end: [0.4, 0.3, -0.5, 1.0], min: [0.0, 0.3, -0.5], max: [0.4, 0.3, -0.5] },
        ] };
        assert_eq!(dynamics.set_grind_rails([("template".to_owned(), vec![rail])].into()), 3);
        let near = ([-2., REST_Y - 2., -2.], [2., REST_Y + 2., 2.]);
        let far = ([28., REST_Y - 2., -2.], [32., REST_Y + 2., 2.]);
        for (rules, (min, max), skip) in [
            (RailRules::default(), near, None),
            (RailRules::default(), near, Some(3)),
            (RailRules::default(), far, None),
            (RailRules { cap: 3, ..RailRules::default() }, near, None),
            (RailRules { prop_radius: 0.01, ..RailRules::default() }, near, None),
        ] {
            let (mut direct, mut listed) = (Vec::new(), Vec::new());
            dynamics.append_rails(min, max, skip, &rules, &mut direct);
            dynamics.rail_poses(skip).append_rails(min, max, skip, &rules, &mut listed);
            assert_eq!(format!("{direct:?}"), format!("{listed:?}"));
        }
        let mut hits = Vec::new();
        dynamics.append_rails(near.0, near.1, None, &RailRules::default(), &mut hits);
        assert_eq!(hits.len(), 4, "props 3 and 9, two segments each");
        assert_eq!(hits[0].primitive.owner >> 16 & 0xFFFF_FFFF, 3, "id order: prop 3 first");
        // Upstream's list: every body with rails and not streamed out, sorted by id.
        let ids: Vec<u32> = dynamics.rail_poses(None).0.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![3, 5, 9]);
        assert_eq!(dynamics.rail_poses(Some(5)).0.iter().map(|p| p.id).collect::<Vec<_>>(), vec![3, 9]);
    }

    /// A straight push moves the prop along the grab-edge normal at about the
    /// retail push speed, and the skater stays on the grab frame.
    #[test]
    fn dragged_prop_follows_a_straight_push() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let mut at = carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.);
        carry.update(&mut dynamics, tick(), at);
        assert_eq!(carry.held(), Some(7));
        dynamics.set_held(Some(7));
        at.state = skate_core::player::state::PhysicalStateId::OffBoardPushing;
        let mut max_side = 0.0f32;
        let mut speed = 0.0f32;
        let mut peak = 0.0f32;
        let mut up_min = 1.0f32;
        for _ in 0..180 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 1., 0.), at);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
            let p = dynamics.position_of(7).unwrap();
            max_side = max_side.max(p.x.abs());
            speed = dynamics.bodies[0].rates.linear_velocity.z;
            peak = peak.max(speed);
            up_min = up_min.min(dynamics.bodies[0].rates.basis.columns[1][1]);
        }
        assert_eq!(carry.held(), Some(7), "the push dropped the prop");
        assert!(max_side < 0.05, "prop wandered {max_side} m off the push line");
        let mass = 1.0 / dynamics.bodies[0].inertia.inverse_mass;
        let t = crate::physics::prop_carry::CarryLocomotion::default().move_object;
        let target = t.push_speed * t.speed_scale(mass);
        let predicted = predicted_centre_push(mass, 1.0, 180);
        let predicted_peak = predicted.iter().fold(0.0f32, |a, b| a.max(*b));
        let predicted_end = predicted[179];
        println!("straight push: speed {speed:.3} (retail math {predicted_end:.3}, target {target}), peak {peak:.3} (retail math {predicted_peak:.3}), up_y min {up_min:.4}");
        // The retail controller settles on the target; the sim must follow the
        // controller's own prediction (floor contacts only add the commanded
        // block's small friction, which the integrating controller removes).
        assert!((predicted_end - target).abs() < 0.05 * target, "retail math does not settle on the target: {predicted_end}");
        assert!((speed - predicted_end).abs() < 0.1 * target, "push speed {speed} m/s, retail math {predicted_end}");
        assert!((peak - predicted_peak).abs() < 0.15 * target, "peak {peak} m/s, retail math {predicted_peak}");
        assert!(up_min > 0.99, "a straight push tipped the cube: up_y {up_min}");
    }

    /// The right stick turns the held prop (yaw command only, retail sign)
    /// and the skater is carried round with its grab edge.
    #[test]
    fn right_stick_turns_the_held_prop_and_the_skater_follows() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let mut at = carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.);
        carry.update(&mut dynamics, tick(), at);
        dynamics.set_held(Some(7));
        at.state = skate_core::player::state::PhysicalStateId::OffBoardPushing;
        let mut turned = 0.0f32;
        let mut previous = heading_of(&dynamics, 7);
        let mut framed = previous;
        for _ in 0..90 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 0., 1.), at);
            // The grab frame is built from the pose before this step.
            framed = heading_of(&dynamics, 7);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
            let h = heading_of(&dynamics, 7);
            turned += (h - previous + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            previous = h;
        }
        assert_eq!(carry.held(), Some(7));
        // 82D45318: w = -(curve x MvRot x gain), sent as (0, w, 0) about +Y
        // (right-handed, test `positive_yaw_rate_turns_local_z_toward_plus_x`),
        // so a positive MvRot turns the prop clockwise seen from above (the
        // unwrapped heading atan2(z.x, z.z) decreases).
        assert!(turned < -0.3, "positive MvRot did not turn the prop clockwise: {turned} rad");
        let up = dynamics.bodies[0].rates.basis.columns[1][1];
        assert!(up > 0.99, "a yaw command tipped the prop: up_y {up}");
        // The skater faces the turned edge (frame of the last carry update).
        let (_, facing) = carry.skater_target().unwrap();
        let off = (facing.x.atan2(facing.z) - framed + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        assert!(off.abs() < 0.05, "skater not facing the grab edge: {off} rad off");
    }

    /// Pushing at the grab point off the centre turns the prop through the
    /// lever-arm curves (a bench grabbed near its end).
    #[test]
    fn off_centre_push_turns_a_long_prop() {
        let (world, mut layer, mut dynamics) = box_fixture([0., super::super::ground::HEIGHT + 0.41, 0.], [2.3, 0.41, 0.29]);
        let id = dynamics.bodies[0].id;
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        // Behind the bench, 1.8 m right of its centre.
        let mut at = crate::physics::prop_carry::Carrier {
            position: Vector3::new(1.8, super::super::ground::HEIGHT + 0.9, -0.9),
            ..carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.)
        };
        carry.update(&mut dynamics, tick(), at);
        assert_eq!(carry.held(), Some(id));
        dynamics.set_held(Some(id));
        at.state = skate_core::player::state::PhysicalStateId::OffBoardPushing;
        let mut turned = 0.0f32;
        let mut previous = heading_of(&dynamics, id);
        for _ in 0..120 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 1., 0.), at);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
            let h = heading_of(&dynamics, id);
            turned += (h - previous + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            previous = h;
        }
        assert_eq!(carry.held(), Some(id), "the push dropped the bench");
        // Push along +Z at r = (1.8, 0, -0.3) from the centre: torque about +Y
        // is -1.8 F, and retail's yaw term (82D45318) turns the same way.
        println!("off-centre push: turned {turned:.3} rad");
        assert!(turned < -0.05, "the off-centre push did not turn the bench with its torque: {turned}");
    }

    /// A held right stick turns the prop at the retail yaw-rate target and no
    /// faster. 82D45318 [code, 0x82D45BC8..0x82D45CD0]: yaw error = w - 60 x
    /// (facing change since the previous tick, +384), PID 20 / 0 / 40 with the
    /// output accumulating (integral action) and clamped to 6 rad/s^2. The
    /// retail math on a free yaw body (`yaw_rate_feedback_settles_at_the_target_rate`
    /// in skate-core) spins up at 0.1 rad/s per tick, peaks 0.6 % over |w| and
    /// settles at |w| with zero steady error; the expectation here is |w| from
    /// the command itself (curve BFB3BEF0BB2661C0(|lever|) x rot x gain +1160).
    #[test]
    fn held_right_stick_turn_rate_stays_bounded() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let mut at = carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.);
        carry.update(&mut dynamics, tick(), at);
        dynamics.set_held(Some(7));
        at.state = skate_core::player::state::PhysicalStateId::OffBoardPushing;
        let mut peak = 0.0f32;
        for _ in 0..180 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 0., 1.), at);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
            peak = peak.max(dynamics.bodies[0].rates.angular_velocity.y.abs());
        }
        assert_eq!(carry.held(), Some(7), "the turn dropped the prop");
        let (command, _) = carry.last_command().expect("held");
        let target = command.yaw_target.abs();
        let rate = dynamics.bodies[0].rates.angular_velocity.y.abs();
        println!("right stick turn: rate {rate:.3} rad/s, peak {peak:.3}, retail target {target:.3}");
        assert!(target > 0.5, "no yaw target from the right stick: {command:?}");
        assert!((rate - target).abs() < 0.05 * target, "yaw rate {rate} rad/s, retail target {target}");
        assert!(peak < 1.05 * target, "yaw rate overshoot {peak} rad/s for a target of {target}");
    }

    /// Grabbing the prop ahead picks it up; the stick moves it.
    #[test]
    fn grabbed_prop_moves_with_the_stick() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let mut at = carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.);
        carry.update(&mut dynamics, crate::physics::prop_carry::Tick { grab: true, ..tick() }, at);
        assert_eq!(carry.held(), Some(7));
        dynamics.set_held(carry.held());
        let start = dynamics.position_of(7).unwrap().z;
        for _ in 0..60 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 1., 0.), at);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
        }
        let p = dynamics.position_of(7).unwrap();
        // Distance the retail controller math predicts in one second.
        let dt = simulation().time_step;
        let mass = 1.0 / dynamics.bodies[0].inertia.inverse_mass;
        let predicted: f32 = predicted_centre_push(mass, 1.0, 60).iter().map(|v| v * dt).sum();
        let travelled = p.z - start;
        println!("push distance: {travelled:.3} m in 1 s (retail math {predicted:.3} m)");
        assert!((travelled - predicted).abs() < 0.1 * predicted, "pushed {travelled} m, retail math {predicted} m");
        assert!(p.y > super::super::ground::HEIGHT, "carried prop underground: {p:?}");
    }

    /// Dropping releases the prop; it falls, keeps no NaN, and sleeps again.
    #[test]
    fn dropped_carry_falls_and_sleeps() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let state = skate_core::player::state::PhysicalStateId::BipedGround;
        carry.update(&mut dynamics, crate::physics::prop_carry::Tick { grab: true, ..tick() }, carrier(state, 0.));
        for _ in 0..30 {
            carry.update(&mut dynamics, tick(), carrier(state, 0.));
            dynamics.step(&world, &mut layer, &[]);
        }
        carry.update(&mut dynamics, release(), carrier(state, 0.));
        assert_eq!(carry.held(), None);
        for _ in 0..300 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let p = dynamics.position_of(7).unwrap();
        assert!((p.y - REST_Y).abs() < 0.1, "resting height {}", p.y);
        assert!(dynamics.bodies[0].asleep, "dropped prop never slept");
    }

    /// The grab is ignored unless the skater is on foot.
    #[test]
    fn grab_requires_biped_ground() {
        let (_world, _layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        for state in [
            skate_core::player::state::PhysicalStateId::PhysicsGround,
            skate_core::player::state::PhysicalStateId::BipedAir,
            skate_core::player::state::PhysicalStateId::WipeoutGround,
        ] {
            carry.update(&mut dynamics, crate::physics::prop_carry::Tick { grab: true, ..tick() }, carrier(state, 0.));
            assert_eq!(carry.held(), None, "{state:?} must not grab");
        }
        // Grabbing, then mounting the board, drops the prop automatically.
        carry.update(
            &mut dynamics,
            crate::physics::prop_carry::Tick { grab: true, ..tick() },
            carrier(skate_core::player::state::PhysicalStateId::BipedGround, 0.),
        );
        assert_eq!(carry.held(), Some(7));
        carry.update(
            &mut dynamics,
            tick(),
            carrier(skate_core::player::state::PhysicalStateId::PhysicsGround, 0.),
        );
        assert_eq!(carry.held(), None);
    }

    /// Retail grab-object publication moves the selector to OffBoardPushing
    /// while held; the carry must not treat state 502 as leaving the ground.
    #[test]
    fn carry_survives_offboard_pushing_and_drops_afterwards() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let ground = skate_core::player::state::PhysicalStateId::BipedGround;
        let pushing = skate_core::player::state::PhysicalStateId::OffBoardPushing;
        carry.update(&mut dynamics, crate::physics::prop_carry::Tick { grab: true, ..tick() }, carrier(ground, 0.));
        assert_eq!(carry.held(), Some(7));
        let mut at = carrier(pushing, 0.);
        for _ in 0..30 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 0.2, 0.), at);
            dynamics.step(&world, &mut layer, &[]);
            assert_eq!(carry.held(), Some(7), "state 502 dropped the carry");
        }
        assert!(dynamics.position_of(7).unwrap().z > 1.25, "the stick did not move the prop in 502");
        // Leaving the on-foot states still auto-drops.
        carry.update(
            &mut dynamics,
            tick(),
            carrier(skate_core::player::state::PhysicalStateId::BipedAir, 0.),
        );
        assert_eq!(carry.held(), None);
    }

    // Phase 4: placement mode and layout persistence.

    /// Placement adjusts the ghost pose; confirming drops the prop there,
    /// records the layout pose, and the prop falls and sleeps in place.
    #[test]
    fn placement_adjust_confirm_and_sleep() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let state = skate_core::player::state::PhysicalStateId::BipedGround;
        let grab = crate::physics::prop_carry::Tick { grab: true, ..tick() };
        carry.update(&mut dynamics, grab, carrier(state, 0.));
        assert_eq!(carry.held(), Some(7));
        // Enter placement; push the ghost far (distance axis) and yaw it.
        carry.update(
            &mut dynamics,
            crate::physics::prop_carry::Tick { placement: true, ..tick() },
            carrier(state, 0.),
        );
        assert!(carry.placing());
        for _ in 0..60 {
            carry.update(
                &mut dynamics,
                crate::physics::prop_carry::Tick {
                    distance_axis: 1.0,
                    yaw_axis: 0.25,
                    ..tick()
                },
                carrier(state, 0.),
            );
            dynamics.step(&world, &mut layer, &[]);
        }
        let held_pose = dynamics.position_of(7).unwrap();
        let horizontal = (held_pose.x * held_pose.x + held_pose.z * held_pose.z).sqrt();
        assert!(horizontal > 2.0, "ghost pushed out to r={horizontal}");
        let recorded_basis = dynamics.pose(7).unwrap().1;
        assert!(
            recorded_basis.columns[2][0] > 0.3,
            "ghost yaw never applied: {:?}",
            recorded_basis.columns
        );
        // Confirm (release the grab button): drop at the ghost pose; the prop
        // stays there and sleeps.
        carry.update(&mut dynamics, release(), carrier(state, 0.));
        assert_eq!(carry.held(), None);
        assert!(!carry.placing());
        let recorded = carry.layout().get(&7).copied();
        assert!(recorded.is_some(), "confirmed placement was not recorded");
        for _ in 0..300 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let p = dynamics.position_of(7).unwrap();
        assert!((p.y - REST_Y).abs() < 0.1, "placed prop rests at {}", p.y);
        let placed_horizontal = (p.x * p.x + p.z * p.z).sqrt();
        assert!(placed_horizontal > 1.5, "placed prop kept its distance: {placed_horizontal}");
        assert!(dynamics.bodies[0].asleep, "placed prop never slept");
    }

    /// Cancelling placement returns to plain carry with the prop still held.
    #[test]
    fn placement_cancel_returns_to_carry() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        let state = skate_core::player::state::PhysicalStateId::BipedGround;
        let grab = crate::physics::prop_carry::Tick { grab: true, ..tick() };
        let place = crate::physics::prop_carry::Tick { placement: true, grab: true, ..tick() };
        carry.update(&mut dynamics, grab, carrier(state, 0.));
        carry.update(&mut dynamics, place, carrier(state, 0.));
        assert!(carry.placing());
        carry.update(&mut dynamics, place, carrier(state, 0.));
        assert!(!carry.placing());
        assert_eq!(carry.held(), Some(7), "cancel must keep the carry");
        assert!(carry.layout().is_empty(), "cancel must not record a pose");
        // The Move Object push still works after the cancel.
        carry.update(&mut dynamics, stick(0., 1., 0.), carrier(state, 0.));
        dynamics.step(&world, &mut layer, &[]);
        assert!(dynamics.position_of(7).unwrap().z > 1.2);
    }

    /// A saved layout teleports a fresh body to the stored pose, asleep, and
    /// the rebaked triangles follow.
    #[test]
    fn layout_teleports_fresh_body() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let basis = skate_core::math::Basis3 {
            columns: [[0., 0., -1.], [0., 1., 0.], [1., 0., 0.]],
        };
        let origin = Vector3::new(4., REST_Y + 0.5, -3.);
        let instance = dynamics.teleport(7, origin, basis).unwrap();
        layer.rebake(instance, basis.columns, origin).unwrap();
        let (pose_origin, pose_basis) = dynamics.pose(7).unwrap();
        assert_eq!(pose_origin, origin);
        assert_eq!(pose_basis.columns[2], [1., 0., 0.]);
        assert!(dynamics.bodies[0].asleep);
        // Rotated 90° about Y: the cube is symmetric, but the rebaked probe
        // confirms the range moved to the new origin.
        let hit = layer
            .world()
            .query_thin_line(
                Vector3::new(4., REST_Y + 2., -3.),
                Vector3::new(4., REST_Y - 1., -3.),
            )
            .unwrap()
            .unwrap();
        assert!((hit.geometry.position.y - (REST_Y + 1.)).abs() < 0.05);
        let _ = world;
    }

    /// "Reset moved objects": a moved body is listed, reset returns it to the
    /// authored pose asleep and at rest, and the list empties.
    #[test]
    fn reset_to_spawn_returns_moved_body() {
        let (_world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let (spawn_origin, spawn_basis) = dynamics.spawn_pose(7).unwrap();
        assert_eq!(dynamics.pose(7).unwrap().0, spawn_origin);
        assert!(dynamics.moved_ids().is_empty());
        let basis = skate_core::math::Basis3 {
            columns: [[0., 0., -1.], [0., 1., 0.], [1., 0., 0.]],
        };
        let instance = dynamics.teleport(7, Vector3::new(4., REST_Y + 0.5, -3.), basis).unwrap();
        layer.rebake(instance, basis.columns, Vector3::new(4., REST_Y + 0.5, -3.)).unwrap();
        assert_eq!(dynamics.moved_ids(), vec![7]);
        let instance = dynamics.reset_to_spawn(7).unwrap();
        layer.rebake(instance, spawn_basis.columns, spawn_origin).unwrap();
        let (origin, basis) = dynamics.pose(7).unwrap();
        let d = sub(origin, spawn_origin);
        assert!(d.x.abs() + d.y.abs() + d.z.abs() < 1e-5);
        assert_eq!(basis.columns, spawn_basis.columns);
        assert!(dynamics.bodies[0].asleep);
        assert_eq!(dynamics.bodies[0].rates.linear_velocity, Vector3::ZERO);
        assert!(dynamics.moved_ids().is_empty());
        assert!(dynamics.reset_to_spawn(99).is_none());
    }

    // Upright (retail cMsgUprightDMO, doc 27 "Upright").

    /// Basis rotated by `degrees` about world Z (local Y tips toward -X).
    fn tilted_about_z(degrees: f32) -> skate_core::math::Basis3 {
        let (s, c) = degrees.to_radians().sin_cos();
        skate_core::math::Basis3 { columns: [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]] }
    }

    fn tilt_degrees(basis: skate_core::math::Basis3) -> f32 {
        let up = mul_basis(basis, Vector3::new(0., 1., 0.));
        (up.y / length(up)).clamp(-1., 1.).acos().to_degrees()
    }

    /// 82C573D0 values: stop under 10 deg, axis up x world-up, speed
    /// 3 x (min(tilt, 70 deg) - 5 deg) for |A| <= 1.1, command
    /// (target - w_axis - 0.1 w_perp) x 60, fallback to the body's X / Z axis
    /// above 120 deg.
    #[test]
    fn upright_command_matches_retail_constants() {
        let s = PropUprightSettings::default();
        let a = Vector3::ZERO;
        assert!(upright_command(tilted_about_z(0.), Vector3::ZERO, a, &s).is_none());
        assert!(upright_command(tilted_about_z(9.9), Vector3::ZERO, a, &s).is_none());
        // 30 deg: up = (-sin, cos, 0); up x Y = (0, 0, -sin) -> axis -Z.
        let c = upright_command(tilted_about_z(30.), Vector3::ZERO, a, &s).unwrap();
        let speed = 3.0 * (30f32.to_radians() - 5f32.to_radians());
        assert!((c.z - (-speed * 60.)).abs() < 1e-3 && c.x.abs() < 1e-5 && c.y.abs() < 1e-5, "{c:?}");
        // 90 deg: capped at 70 deg.
        let c = upright_command(tilted_about_z(90.), Vector3::ZERO, a, &s).unwrap();
        let speed = 3.0 * (70f32.to_radians() - 5f32.to_radians());
        assert!((c.z + speed * 60.).abs() < 1e-3, "{c:?}");
        // Spin: along the axis replaced, 10% of the off-axis spin removed.
        let w = Vector3::new(2.0, 0.0, -1.0);
        let c = upright_command(tilted_about_z(90.), w, a, &s).unwrap();
        assert!((c.z - (-speed - (-1.0)) * 60.).abs() < 1e-3, "{c:?}");
        assert!((c.x - (-0.1 * 2.0 * 60.)).abs() < 1e-3, "{c:?}");
        // Gain blend: |A| >= 2.1 -> gain 5.
        let c = upright_command(tilted_about_z(30.), Vector3::ZERO, Vector3::new(0., 3., 0.), &s).unwrap();
        let speed = 5.0 * (30f32.to_radians() - 5f32.to_radians());
        assert!((c.z + speed * 60.).abs() < 1e-3, "{c:?}");
        // Above 120 deg: the body's own Z axis (A.x <= A.z) or X axis (A.x > A.z).
        let basis = tilted_about_z(150.);
        let c = upright_command(basis, Vector3::ZERO, a, &s).unwrap();
        assert!(c.x.abs() < 1e-5 && c.y.abs() < 1e-5 && c.z > 0., "{c:?}");
        let c = upright_command(basis, Vector3::ZERO, Vector3::new(1., 0., 0.), &s).unwrap();
        let x = mul_basis(basis, Vector3::new(1., 0., 0.));
        let along = dot(c, x) / length(c);
        assert!((along - 1.0).abs() < 1e-4, "{c:?}");
    }

    /// A box lying on its side, uprighted, turns back within the 2 s window;
    /// the window closes when the tilt drops under 10 deg.
    #[test]
    fn upright_rights_a_tipped_box_within_the_window() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        let instance = dynamics.teleport(7, Vector3::new(0., REST_Y + 0.05, 0.), tilted_about_z(90.)).unwrap();
        layer.rebake(instance, tilted_about_z(90.).columns, Vector3::new(0., REST_Y + 0.05, 0.)).unwrap();
        assert!(!dynamics.upright(99));
        assert!(dynamics.upright(7));
        assert!(dynamics.is_uprighting(7));
        let mut closed_at = None;
        for step in 1..=120 {
            dynamics.step(&world, &mut layer, &[]);
            if !dynamics.is_uprighting(7) {
                closed_at = Some(step);
                break;
            }
        }
        let tilt = tilt_degrees(dynamics.bodies[0].rates.basis);
        let step = closed_at.expect("window should close by tilt before the 2 s timeout");
        assert!(tilt < 10.0, "closed at step {step} with tilt {tilt}");
        for _ in 0..240 {
            dynamics.step(&world, &mut layer, &[]);
        }
        let tilt = tilt_degrees(dynamics.bodies[0].rates.basis);
        assert!(tilt < 10.0, "settled tilt {tilt}");
    }

    /// With no righting gain the window times out after 2.0 s of 1/60 s
    /// updates (82C56780: open while timer <= 2.0).
    #[test]
    fn upright_window_times_out_at_two_seconds() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        dynamics.teleport(7, Vector3::new(0., REST_Y, 0.), tilted_about_z(90.)).unwrap();
        let mut table = PropTuningTable::default();
        table.upright.gain_min = 0.0;
        table.upright.gain_max = 0.0;
        dynamics.set_tuning(table);
        let s = PropUprightSettings::default();
        let mut expected = 0u32;
        let mut t = 0f32;
        while t <= s.window_seconds {
            t += s.tick_seconds;
            expected += 1;
        }
        assert!((120..=121).contains(&expected));
        assert!(dynamics.upright(7));
        let mut closed_at = None;
        for step in 1..=200 {
            dynamics.step(&world, &mut layer, &[]);
            if !dynamics.is_uprighting(7) {
                closed_at = Some(step);
                break;
            }
        }
        assert_eq!(closed_at, Some(expected));
        assert!(tilt_degrees(dynamics.bodies[0].rates.basis) > 10.0);
    }

    /// 82C52E68 refuses Move Object yaw while the window is open; the linear
    /// command still applies. After the window closes yaw applies again.
    #[test]
    fn upright_blocks_move_object_yaw_during_the_window() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        dynamics.teleport(7, Vector3::new(0., REST_Y, 0.), tilted_about_z(90.)).unwrap();
        assert!(dynamics.upright(7));
        let dt = dynamics.step_simulation().time_step;
        let w0 = dynamics.bodies[0].rates.angular_velocity;
        let v0 = dynamics.bodies[0].rates.linear_velocity;
        assert!(dynamics.apply_move_command(7, Vector3::new(3., 0., 0.), 5.0, Vector3::ZERO, dt));
        assert_eq!(dynamics.bodies[0].rates.angular_velocity, w0, "yaw refused");
        assert!((dynamics.bodies[0].rates.linear_velocity.x - (v0.x + 3. * dt)).abs() < 1e-6, "linear applied");
        // Mod knob: block_yaw off lets the yaw through.
        let mut table = PropTuningTable::default();
        table.upright.block_yaw = false;
        dynamics.set_tuning(table);
        assert!(dynamics.apply_move_command(7, Vector3::ZERO, 5.0, Vector3::ZERO, dt));
        assert!((dynamics.bodies[0].rates.angular_velocity.y - (w0.y + 5. * dt)).abs() < 1e-6);
        dynamics.set_tuning(PropTuningTable::default());
        // An upright body closes the window on its first update; yaw applies again.
        let (world2, mut layer2, mut upright) = fixture([0., REST_Y, 0.]);
        assert!(upright.upright(7));
        upright.step(&world2, &mut layer2, &[]);
        assert!(!upright.is_uprighting(7));
        let w = upright.bodies[0].rates.angular_velocity;
        assert!(upright.apply_move_command(7, Vector3::ZERO, 5.0, Vector3::ZERO, dt));
        assert!((upright.bodies[0].rates.angular_velocity.y - (w.y + 5. * dt)).abs() < 1e-6);
        let _ = (world, &mut layer);
    }

    // NPC skaters against props (doc 26, fix 19).

    /// One NPC skater rolling along +X (its +Z forward turned to +X) at `speed`, at `x`.
    fn npc_sample(x: f32, speed: f32) -> skate_core::living_world::replay::ReplaySample {
        let q = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        skate_core::living_world::replay::ReplaySample {
            line: [0; 16],
            node: 0,
            position: [x, super::super::ground::HEIGHT, 0.0],
            velocity: [speed, 0.0, 0.0],
            heading: std::f32::consts::FRAC_PI_2,
            board: [q.x, q.y, q.z, q.w],
            skater: [q.x, q.y, q.z, q.w],
            flags: 0,
            phase: skate_core::living_world::replay::ReplayPhase::Rolling,
            jump: None,
            phase_frames: 0,
            previous_phase: None,
            previous_phase_frames: 0,
            sub_frame: 0.0,
            fakie: false,
        }
    }

    const NPC: skate_core::living_world::LivingWorldId =
        skate_core::living_world::LivingWorldId { kind: skate_core::living_world::Kind::Skater, serial: 7 };

    /// Run an NPC line along +X through a prop centred at x = 0 for `ticks` 60 Hz ticks.
    /// Returns the prop centre x per tick and the NPC x per tick.
    fn npc_run(
        dynamics: &mut PropDynamics,
        world: &BoardWorld,
        layer: &mut crate::skate_world::PropCollisionLayer,
        start_x: f32,
        speed: f32,
        ticks: usize,
        push: bool,
    ) -> Vec<(f32, f32)> {
        let mut out = Vec::new();
        for tick in 0..ticks {
            let x = start_x + speed * tick as f32 / 60.0;
            let volumes = if push {
                crate::living_world::npc_skaters::prop_volumes(NPC, &npc_sample(x, speed)).to_vec()
            } else {
                Vec::new()
            };
            dynamics.step_with_actors(world, layer, &[], &volumes);
            out.push((dynamics.bodies[0].rates.position.x, x));
        }
        out
    }

    /// Retail's bail clamp `82C530F0` with its defaults: the cap is 5 / mass m/s and 15 / mass rad/s under 200 kg,
    /// a heavier prop is stopped (b206, b209).
    #[test]
    fn bail_clamp_caps_light_props_and_stops_heavy_ones() {
        let s = PropSolverSettings::default();
        let v = |x: f32, y: f32, z: f32| Vector3::new(x, y, z);
        let zero = v(0.0, 0.0, 0.0);
        // 1 kg at rest (d = 0): the ball is centred on 0 with radius 5, so 10 m/s becomes 5 m/s; spin 30 -> 15.
        let (l, a) = bail_clamp(&s, 1.0, zero, v(10.0, 0.0, 0.0), v(0.0, 30.0, 0.0)).unwrap();
        assert!((l.x - 5.0).abs() < 1e-5 && l.y.abs() < 1e-6 && (a.y - 15.0).abs() < 1e-4, "{l:?} {a:?}");
        // 20 kg: 0.25 m/s, 0.75 rad/s.
        let (l, a) = bail_clamp(&s, 1.0 / 20.0, zero, v(0.0, 0.0, 3.0), v(2.0, 0.0, 0.0)).unwrap();
        assert!((length(l) - 0.25).abs() < 1e-5 && (length(a) - 0.75).abs() < 1e-5);
        // Already inside the ball: unchanged.
        let (l, _) = bail_clamp(&s, 1.0 / 20.0, zero, v(0.1, 0.0, 0.0), zero).unwrap();
        assert!((l.x - 0.1).abs() < 1e-6);
        // Moving along d: up to |d| + cap ahead (ball of radius mag around n * mag).
        let (l, _) = bail_clamp(&s, 1.0 / 20.0, v(2.0, 0.0, 0.0), v(9.0, 0.0, 0.0), zero).unwrap();
        assert!((l.x - 2.25).abs() < 1e-5, "{l:?}");
        // 300 kg: stopped, spin zeroed.
        let (l, a) = bail_clamp(&s, 1.0 / 300.0, zero, v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)).unwrap();
        assert!(length(l) < 1e-6 && length(a) < 1e-6);
        // A static body is left alone.
        assert!(bail_clamp(&s, 0.0, zero, v(1.0, 0.0, 0.0), zero).is_none());
    }

    /// A bailing pusher's contacts clamp the prop: the bin barely moves, while the same run without the bail
    /// knocks it ahead.
    #[test]
    fn a_bailing_pusher_does_not_throw_a_bin() {
        let half = [0.3, 0.45, 0.3];
        let rest = super::super::ground::HEIGHT + 0.45;
        let pushed = {
            let (world, mut layer, mut dynamics) = box_fixture([0., rest, 0.], half);
            npc_run(&mut dynamics, &world, &mut layer, -3.0, 5.0, 120, true)
        };
        let (world, mut layer, mut dynamics) = box_fixture([0., rest, 0.], half);
        dynamics.set_bailing(&[crate::living_world::npc_skaters::PROXY_ID_TAG | NPC.to_u64()]);
        let bailed = npc_run(&mut dynamics, &world, &mut layer, -3.0, 5.0, 120, true);
        let (free, clamped) = (pushed.last().unwrap().0, bailed.last().unwrap().0);
        assert!(free > 1.0 && clamped < free * 0.5, "free push {free:.2} m, bail-clamped {clamped:.2} m");
        let speed = length(dynamics.bodies[0].rates.linear_velocity);
        let mass = 1.0 / dynamics.bodies[0].inertia.inverse_mass;
        assert!(speed <= 2.0 * 5.0 / mass + 1e-3, "speed {speed} for {mass} kg");
    }

    /// A bin slid toward a standing ped stops short of it: the ped's body A (group 9, 10 kg) is a pusher with its own
    /// mass, so the prop no longer passes through peds (b342 section 7).
    #[test]
    fn a_sliding_bin_stops_against_a_peds_body_a() {
        use crate::living_world::ped_body::{body_a_id, body_a_volume, PedProxy};
        let half = [0.3, 0.45, 0.3];
        let rest = super::super::ground::HEIGHT + 0.45;
        let ped_x = 1.2;
        let run = |ped: bool| {
            let (world, mut layer, mut dynamics) = box_fixture([0., rest, 0.], half);
            dynamics.bodies[0].rates.linear_velocity = Vector3::new(4.0, 0.0, 0.0);
            dynamics.bodies[0].wake();
            let proxy = PedProxy::default();
            let id = body_a_id(3);
            dynamics.set_actor_push_mass(&[(id, proxy.mass)]);
            let feet = glam::Vec3::new(ped_x, super::super::ground::HEIGHT, 0.0);
            let volumes = if ped { vec![(id, body_a_volume(feet, glam::Vec3::ZERO, &proxy))] } else { Vec::new() };
            for _ in 0..90 {
                dynamics.step_with_actors(&world, &mut layer, &[], &volumes);
            }
            dynamics.bodies[0].rates.position.x
        };
        let (free, blocked) = (run(false), run(true));
        assert!(free > ped_x, "without the ped the bin slides past it: x {free:.2}");
        assert!(blocked < ped_x - 0.3 && blocked < free, "the ped stops the bin: x {blocked:.2} (free {free:.2})");
    }

    /// A bin-sized prop resting on an NPC skater's line is knocked ahead of it (the player's push
    /// rule), the NPC owns the moved prop, and the bin never ends behind the NPC's board.
    #[test]
    fn npc_skater_knocks_a_bin_on_its_line_out_of_the_way() {
        let half = [0.3, 0.45, 0.3];
        let rest = super::super::ground::HEIGHT + 0.45;
        let (world, mut layer, mut dynamics) = box_fixture([0., rest, 0.], half);
        let control = {
            let (world, mut layer, mut dynamics) = box_fixture([0., rest, 0.], half);
            npc_run(&mut dynamics, &world, &mut layer, -3.0, 5.0, 120, false)
        };
        assert!(control.iter().all(|(p, _)| p.abs() < 1e-4), "nothing touches the control bin");
        let run = npc_run(&mut dynamics, &world, &mut layer, -3.0, 5.0, 120, true);
        let (end_prop, end_npc) = *run.last().unwrap();
        assert!(end_prop > 1.0, "bin pushed ahead along the line: x {end_prop}");
        assert_eq!(dynamics.pushed_by(dynamics.bodies[0].id), Some(crate::living_world::npc_skaters::PROXY_ID_TAG | NPC.to_u64()));
        // No pass-through: once the board reached the bin, the bin's centre stays ahead of the
        // board's nose (0.4 m) minus the bin's own half depth, or it was knocked clear sideways.
        for &(prop, npc) in &run {
            assert!(prop - npc > -(0.4 + half[0]) - 0.05 || dynamics.bodies[0].rates.position.z.abs() > 0.6, "bin {prop} behind npc {npc}");
        }
        let _ = end_npc;
    }

    /// A bench (126 kg) on the line is pushed by the same rule (weight-scaled, board-capped);
    /// the result is identical bit for bit on a second run (seeded, ordered by actor id).
    #[test]
    fn npc_skater_bench_push_is_deterministic() {
        let runs: Vec<Vec<(f32, f32)>> = (0..2)
            .map(|_| {
                let (world, mut layer, mut dynamics) = box_fixture([0., BENCH_REST_Y, 0.], BENCH_HALF);
                npc_run(&mut dynamics, &world, &mut layer, -3.0, 5.0, 120, true)
            })
            .collect();
        assert_eq!(runs[0], runs[1]);
        let (end_prop, _) = *runs[0].last().unwrap();
        assert!(end_prop > 0.5, "bench pushed along the line: x {end_prop}");
    }

    /// NPC volumes and the local player's both push in one step; the last actually pushing actor
    /// owns the prop, and a far NPC does not touch it.
    #[test]
    fn far_npc_leaves_props_alone_and_local_push_owns_it() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 0.]);
        let far = crate::living_world::npc_skaters::prop_volumes(NPC, &npc_sample(-20.0, 5.0));
        let local = [BoardWorldVolume {
            collision_group: 4,
            body: CollisionBody::Board(BodyId::Deck),
            primitive: ContactPrimitive::Sphere(Sphere { center: Vector3::new(-0.55, REST_Y, 0.), radius: 0.2 }),
            motion: skate_core::physics::board_world::VolumeMotion { linear_velocity: Vector3::new(2., 0., 0.), ..Default::default() },
            material: material(),
        }];
        dynamics.step_with_actors(&world, &mut layer, &[], &far);
        assert!(dynamics.bodies[0].asleep);
        assert_eq!(dynamics.pushed_by(dynamics.bodies[0].id), None);
        dynamics.step_with_actors(&world, &mut layer, &local, &far);
        assert_eq!(dynamics.pushed_by(dynamics.bodies[0].id), Some(LOCAL_PUSHER));
    }

    // Dragged props sink through the floor (2026-10-07, DownTown, doc 26).

    /// DownTown-like sidewalk height (the log's props rest with bottoms at 12.6).
    const SIDEWALK_Y: f32 = 12.6;
    /// Street one curb lower, from z = CURB_Z on.
    const STREET_Y: f32 = SIDEWALK_Y - 0.15;
    const CURB_Z: f32 = 6.0;

    /// A map collision world like a city street: 1 m one-sided floor tiles
    /// (many interior edges, as the district meshes), a sidewalk, a curb face
    /// and the street below it.
    fn street_world() -> BoardWorld {
        let mut collision = Vec::new();
        let mut push = |points: [[f32; 3]; 3]| {
            collision.push(skate_data::skate_map::Collision { points, surface: 0, material: 1, native_edges: None });
        };
        for xi in -8..8 {
            for zi in -6..18 {
                let (x0, x1, z0, z1) = (xi as f32, xi as f32 + 1.0, zi as f32, zi as f32 + 1.0);
                let y = if z0 < CURB_Z { SIDEWALK_Y } else { STREET_Y };
                push([[x0, y, z0], [x0, y, z1], [x1, y, z1]]);
                push([[x0, y, z0], [x1, y, z1], [x1, y, z0]]);
            }
            let (x0, x1) = (xi as f32, xi as f32 + 1.0);
            push([[x0, STREET_Y, CURB_Z], [x1, STREET_Y, CURB_Z], [x1, SIDEWALK_Y, CURB_Z]]);
            push([[x0, STREET_Y, CURB_Z], [x1, SIDEWALK_Y, CURB_Z], [x0, SIDEWALK_Y, CURB_Z]]);
        }
        let map = skate_data::skate_map::SkateMap {
            version: 14,
            name: "street".into(),
            spawn: [0.; 3],
            heading: 0.,
            environment: vec![0.; 45],
            materials: vec![skate_data::skate_map::Material {
                name: "concrete".into(),
                flags: 0,
                friction: 0.5,
                restitution: 0.1,
                color: [1.; 3],
                roughness: 0.5,
                emissive: 0.,
                textures: [0; 5],
                indirect_strength: 0.,
                alpha_mode: 0,
                alpha_cutoff: 0.5,
                audio: 3,
                physics: 1,
                pattern: 0,
                depth_layer: None,
                retail_definition: None,
            }],
            textures: vec![],
            geometry: skate_data::skate_map::Geometry { vertices: vec![], indices: vec![], collision },
            rails: vec![],
            doors: vec![],
            lights: vec![],
            routes: vec![],
            extensions: vec![],
        };
        crate::skate_world::collision_world(&map, floor_material()).unwrap()
    }

    /// Box sizes of the props the user dragged in the 2026-10-07 session
    /// (half extents from the PED_OBSTACLE lines: bench, bin, vending, rail).
    const DRAGGED_TEMPLATES: [(&str, [f32; 3]); 4] = [
        ("bench", [2.30, 0.41, 0.29]),
        ("bin", [0.28, 0.41, 0.27]),
        ("vending", [0.50, 0.95, 0.45]),
        ("rail", [2.00, 0.30, 0.12]),
    ];

    struct DragRun {
        /// Lowest box bottom minus floor seen over the whole run.
        worst_gap: f32,
        /// Smallest up-axis Y while held (1 = upright).
        held_up_min: f32,
        /// Horizontal distance from the spawn at release and at the end.
        away_at_release: f32,
        away_at_end: f32,
        end: PropGroundProbe,
        asleep: bool,
    }

    /// Grab the prop from behind, push it `seconds` along +Z (full stick) across
    /// the curb with the skater following the grab frame, release, then let it
    /// settle 3 s. The held id is fed back through `set_held` like
    /// `GamePhysics::update_prop_carry`.
    fn drag_and_release(half: [f32; 3], seconds: f32) -> DragRun {
        let world = street_world();
        let (_, mut layer, mut dynamics) = box_fixture([0., SIDEWALK_Y + half[1], 0.], half);
        let id = dynamics.bodies[0].id;
        let spawn = dynamics.position_of(id).unwrap();
        let dt = simulation().time_step;
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        use skate_core::player::state::PhysicalStateId::{BipedGround, OffBoardPushing};
        let mut at = crate::physics::prop_carry::Carrier {
            state: BipedGround,
            position: Vector3::new(0., SIDEWALK_Y + 0.9, -(half[2] + 0.6)),
            forward: Vector3::new(0., 0., 1.),
            time_step: dt,
            skeleton: None,
            ground_grab: None,
        };
        carry.update(&mut dynamics, tick(), at);
        assert_eq!(carry.held(), Some(id), "grab failed");
        dynamics.set_held(carry.held());
        at.state = OffBoardPushing;
        let mut worst_gap = f32::INFINITY;
        let mut held_up_min = 1.0f32;
        let track = |dynamics: &PropDynamics, worst: &mut f32| {
            let probe = dynamics.ground_probe(id, &world).unwrap();
            if let Some(gap) = probe.gap() {
                *worst = worst.min(gap);
            }
            probe
        };
        let ticks = (seconds / dt) as u32;
        for _ in 1..=ticks {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 1., 0.), at);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
            track(&dynamics, &mut worst_gap);
            held_up_min = held_up_min.min(dynamics.bodies[0].rates.basis.columns[1][1]);
        }
        assert_eq!(carry.held(), Some(id), "the push dropped the prop");
        carry.update(&mut dynamics, release(), at);
        dynamics.set_held(carry.held());
        let flat = |p: Vector3| ((p.x - spawn.x).powi(2) + (p.z - spawn.z).powi(2)).sqrt();
        let away_at_release = flat(dynamics.position_of(id).unwrap());
        let mut end = track(&dynamics, &mut worst_gap);
        for _ in 0..180 {
            dynamics.step(&world, &mut layer, &[]);
            end = track(&dynamics, &mut worst_gap);
        }
        DragRun {
            worst_gap,
            held_up_min,
            away_at_release,
            away_at_end: flat(end.center),
            end,
            asleep: dynamics.bodies[0].asleep,
        }
    }

    /// One ped walking back and forth along z (x = 0, head-on) while a bin is dragged toward it by
    /// the real carry path (Move Object) for 1 s, held still for 3 s, then released and left 3 s. Obstacles come from
    /// `obstacle_boxes` through the game's input mapping; the ped steps with
    /// `NavObstacles::resolve_step` (the rule `advance_peds` uses). Returns (ped steps that went deeper into
    /// the prop, past the re-cut tolerance, ticks held + cut, ticks held + moving, the obstacles after rest, spawn, rest).
    fn ped_vs_dragged_bin(params: skate_core::living_world::peds::ObstacleParams) -> (u32, u32, u32, skate_core::living_world::peds::NavObstacles, Vector3, Vector3) {
        use skate_core::living_world::peds::NavObstacles;
        let half = [0.35, 0.5, 0.35];
        let world = street_world();
        let (_, mut layer, mut dynamics) = box_fixture([0., SIDEWALK_Y + half[1], 0.], half);
        let id = dynamics.bodies[0].id;
        let spawn = dynamics.position_of(id).unwrap();
        let dt = simulation().time_step;
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        use skate_core::player::state::PhysicalStateId::{BipedGround, OffBoardPushing};
        let mut at = crate::physics::prop_carry::Carrier {
            state: BipedGround,
            position: Vector3::new(0., SIDEWALK_Y + 0.9, -(half[2] + 0.6)),
            forward: Vector3::new(0., 0., 1.),
            time_step: dt,
            skeleton: None,
            ground_grab: None,
        };
        carry.update(&mut dynamics, tick(), at);
        assert_eq!(carry.held(), Some(id), "grab failed");
        dynamics.set_held(carry.held());
        at.state = OffBoardPushing;
        let mut o = NavObstacles::new(params);
        let (radius, speed) = (0.35f32, 1.4f32);
        let mut ped = [0.0f32, SIDEWALK_Y, 3.5];
        let mut dir = -1.0f32;
        let (mut entered, mut held_cut, mut held_moving) = (0u32, 0u32, 0u32);
        let push_ticks = (1.0 / dt) as u32;
        let hold_ticks = push_ticks + (3.0 / dt) as u32;
        let rest_ticks = (3.0 / dt) as u32;
        for k in 0..hold_ticks + rest_ticks {
            if k < hold_ticks {
                at = follow(&carry, at);
                carry.update(&mut dynamics, if k < push_ticks { stick(0., 1., 0.) } else { stick(0., 0., 0.) }, at);
            } else if k == hold_ticks {
                carry.update(&mut dynamics, release(), at);
            }
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
            o.update(&crate::living_world::peds::prop_obstacle_inputs(&dynamics));
            let s = &o.states[&(id as u64)];
            if s.held && s.cut.is_some() {
                held_cut += 1;
            }
            if s.held && s.moving {
                held_moving += 1;
            }
            if ped[2] < -1.0 || ped[2] > 4.0 {
                dir = if ped[2] < -1.0 { 1.0 } else { -1.0 };
            }
            let to = [ped[0], ped[1], ped[2] + dir * speed * dt];
            let next = o.resolve_step(ped, to, radius).unwrap_or(ped);
            let body = s.now;
            // Deeper than the cut may lag the body (retail re-cut tolerance 0.25 x 0.35 m).
            let grow = radius - 0.25 * 0.35;
            let d = |p: [f32; 3]| (p[0] - body.center[0]).hypot(p[2] - body.center[1]);
            if body.applies(ped[1], 0.0) && body.contains(next, grow) && d(next) < d(ped) - 1e-6 {
                entered += 1;
            }
            ped = next;
        }
        assert_eq!(carry.held(), None);
        (entered, held_cut, held_moving, o, spawn, dynamics.position_of(id).unwrap())
    }

    /// Peds vs a prop held by Move Object (user 2026-10-08: peds walk through it while it is held).
    /// Retail [code]: the hold sets only DMO+4464 bit 0x20; the obstacle-off gate is a different
    /// word (+144+4252), so a held prop stays a NavPower obstacle (cut when slower than 0.4 m/s,
    /// moving avoider when faster). Ours: cut, or solid for the ped's step while moving. The
    /// earlier rule (held = ignored) is the control: the ped then walks into the prop.
    #[test]
    fn peds_do_not_walk_through_a_held_prop_and_see_it_where_it_rests() {
        use skate_core::living_world::peds::ObstacleParams;
        let (entered, held_cut, held_moving, o, spawn, rest) = ped_vs_dragged_bin(ObstacleParams::default());
        let (control, ..) = ped_vs_dragged_bin(ObstacleParams { held_is_obstacle: false, ..ObstacleParams::default() });
        let s = o.states.values().next().unwrap();
        let cut = s.cut.expect("the released bin rests and is cut");
        let cut_at = Vector3::new(s.cut_at[0], s.cut_at[1], s.cut_at[2]);
        let moved = ((rest.x - spawn.x).powi(2) + (rest.z - spawn.z).powi(2)).sqrt();
        let dist = |a: Vector3, b: Vector3| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt();
        println!(
            "held: entered {entered} (control {control}), ticks held+cut {held_cut}, held+moving {held_moving}; rest {rest:?} moved {moved:.2} m from {spawn:?}; cut centre {:?} (body {:.3} m away), version {}",
            cut.center,
            dist(cut_at, rest),
            o.version
        );
        assert_eq!(entered, 0, "a ped never steps deeper into the held prop");
        assert!(control > 0, "control: with held props ignored the ped walks into it ({control})");
        assert!(held_moving > 0, "the drag moved it faster than 0.4 m/s at times");
        assert!(held_cut > 0, "held still, it is cut where it is (retail: below 0.4 m/s)");
        assert!(moved > 0.5, "dragged away from the spawn: {moved}");
        // Re-cut where it rests (within the 0.25 x 0.35 m re-cut tolerance), not at the spawn.
        assert!(dist(cut_at, rest) <= 0.25 * 0.35 + 1e-4);
        let at = |p: Vector3| [p.x, SIDEWALK_Y, p.z];
        assert!(o.blocked(at(rest), 0.35), "the new spot blocks");
        assert!(!o.blocked(at(spawn), 0.35), "the old spot is free");
        // A path leg through the new spot hits the cut (a detour), one through the old spot does not.
        let leg = |p: Vector3| o.first_hit([p.x - 3.0, SIDEWALK_Y, p.z], [p.x + 3.0, SIDEWALK_Y, p.z], 0.45);
        assert!(leg(rest).is_some() && leg(spawn).is_none());
    }

    /// Prop sinking (2026-10-07): every dragged template stays on the floor
    /// (box bottom never more than a few cm into it), rests on the street
    /// after release and does not slide back toward its spawn. Tipping is
    /// allowed (retail leaves pitch / roll to the physics).
    #[test]
    fn dragged_props_rest_on_the_floor_after_release() {
        let mut failures = Vec::new();
        for (name, half) in DRAGGED_TEMPLATES {
            let run = drag_and_release(half, 5.0);
            println!(
                "{name}: worst gap {:.3} m, held up_y min {:.4}, away {:.2} -> {:.2} m, end {:?}, asleep {}",
                run.worst_gap, run.held_up_min, run.away_at_release, run.away_at_end, run.end, run.asleep
            );
            let rest = run.end.gap();
            let checks = [
                (run.worst_gap > -0.08, format!("{name} sank {} m into the floor", -run.worst_gap)),
                (!run.end.below_ground(), format!("{name} ended below the floor: {:?}", run.end)),
                (rest.is_some_and(|g| g.abs() < 0.08), format!("{name} not resting on the street: gap {rest:?}")),
                (run.away_at_end > run.away_at_release - 0.1, format!("{name} slid back toward its spawn: {} -> {}", run.away_at_release, run.away_at_end)),
                // Moved at all (a full push may tip a prop over; tipping is allowed).
                (run.away_at_release > 0.5, format!("{name} was not dragged: {}", run.away_at_release)),
            ];
            failures.extend(checks.into_iter().filter(|c| !c.0).map(|c| c.1));
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Tipping stays (user: "props should still be able to tip.. they do in
    /// retail"): a tall prop pushed off-centre from the street into the curb
    /// face tips (the command has no pitch / roll term, the contacts decide),
    /// and never ends under the floor.
    #[test]
    fn pushing_a_tall_prop_into_the_curb_can_tip_it() {
        let world = street_world();
        let half = [0.50, 0.95, 0.45];
        let (_, mut layer, mut dynamics) = box_fixture([0., STREET_Y + half[1], CURB_Z + 1.2], half);
        let id = dynamics.bodies[0].id;
        let dt = simulation().time_step;
        let mut carry = crate::physics::prop_carry::PropCarry::default();
        use skate_core::player::state::PhysicalStateId::{BipedGround, OffBoardPushing};
        let mut at = crate::physics::prop_carry::Carrier {
            state: BipedGround,
            position: Vector3::new(0.3, STREET_Y + 0.9, CURB_Z + 1.2 + half[2] + 0.6),
            forward: Vector3::new(0., 0., -1.),
            time_step: dt,
            skeleton: None,
            ground_grab: None,
        };
        carry.update(&mut dynamics, tick(), at);
        assert_eq!(carry.held(), Some(id), "grab failed");
        dynamics.set_held(carry.held());
        at.state = OffBoardPushing;
        let mut up_min = 1.0f32;
        let mut worst_gap = f32::INFINITY;
        for _ in 0..180 {
            at = follow(&carry, at);
            carry.update(&mut dynamics, stick(0., 1., 0.), at);
            dynamics.set_held(carry.held());
            dynamics.step(&world, &mut layer, &[]);
            up_min = up_min.min(dynamics.bodies[0].rates.basis.columns[1][1]);
            if let Some(gap) = dynamics.ground_probe(id, &world).unwrap().gap() {
                worst_gap = worst_gap.min(gap);
            }
            if carry.held().is_none() {
                break;
            }
        }
        println!("tall prop into the curb: up_y min {up_min:.3}, worst gap {worst_gap:.3}");
        assert!(up_min < 0.97, "the prop never tipped against the curb: up_y min {up_min}");
        assert!(!dynamics.ground_probe(id, &world).unwrap().below_ground(), "the tipped prop went under the floor");
    }

    /// Retail props do not tip from a push (spec 7.4): the command acts at the
    /// centre of mass with no pitch / roll term and the held prop runs on the
    /// commanded block, so a straight full push on flat ground keeps a 1 m
    /// cube and the bin upright (tipping comes from obstacles, test above).
    #[test]
    fn straight_push_on_flat_ground_does_not_tip_a_cube_or_the_bin() {
        for (name, half) in [("cube", [0.5f32, 0.5, 0.5]), ("bin", [0.28, 0.41, 0.27])] {
            let world = street_world();
            let (_, mut layer, mut dynamics) = box_fixture([0., SIDEWALK_Y + half[1], -4.0], half);
            let id = dynamics.bodies[0].id;
            let dt = simulation().time_step;
            let mut carry = crate::physics::prop_carry::PropCarry::default();
            use skate_core::player::state::PhysicalStateId::{BipedGround, OffBoardPushing};
            let mut at = crate::physics::prop_carry::Carrier {
                state: BipedGround,
                position: Vector3::new(0., SIDEWALK_Y + 0.9, -4.0 - (half[2] + 0.6)),
                forward: Vector3::new(0., 0., 1.),
                time_step: dt,
                skeleton: None,
                ground_grab: None,
            };
            carry.update(&mut dynamics, tick(), at);
            assert_eq!(carry.held(), Some(id), "{name}: grab failed");
            dynamics.set_held(carry.held());
            at.state = OffBoardPushing;
            let mut up_min = 1.0f32;
            // 2 s of full push stays on the flat sidewalk (curb at z = 6).
            for _ in 0..120 {
                at = follow(&carry, at);
                carry.update(&mut dynamics, stick(0., 1., 0.), at);
                dynamics.set_held(carry.held());
                dynamics.step(&world, &mut layer, &[]);
                up_min = up_min.min(dynamics.bodies[0].rates.basis.columns[1][1]);
            }
            let z = dynamics.position_of(id).unwrap().z;
            println!("{name}: up_y min {up_min:.4}, z {z:.2}");
            assert_eq!(carry.held(), Some(id), "{name}: the push dropped the prop");
            assert!(z < CURB_Z - half[2], "{name}: reached the curb, test not on flat ground: z {z}");
            assert!(z > -4.0 + 1.0, "{name}: barely moved: z {z}");
            assert!(up_min > 0.99, "{name}: a straight push tipped it: up_y min {up_min}");
        }
    }

    /// The commanded block (82C53EF8): a commanded body switches to it on its
    /// next step and back to its free block on the step after the commands
    /// stop. The block is the body's own material, combined with the other
    /// side by 82763078 (static max, dynamic max, restitution min); nothing
    /// replaces the combined friction. Every command wakes the body, zero or
    /// not (82ADF7B8).
    #[test]
    fn commanded_block_switches_with_the_command_and_zero_commands_wake() {
        let (world, mut layer, mut dynamics) = fixture([0., REST_Y, 1.2]);
        assert!(dynamics.bodies[0].asleep);
        let authored = dynamics.bodies[0].material;
        let low = RetailContactMaterial { static_friction: 0.01, dynamic_friction: 0.005, restitution: 0.9 };
        assert_eq!(dynamics.body_material(0), authored, "free block = authored material by default");
        assert!(dynamics.apply_move_command(7, Vector3::ZERO, 0.0, Vector3::ZERO, simulation().time_step));
        assert!(!dynamics.bodies[0].asleep, "a zero command must still wake the body");
        dynamics.step(&world, &mut layer, &[]);
        let held = dynamics.body_material(0);
        assert_eq!((held.static_friction, held.dynamic_friction, held.restitution), (0.03, 0.02, authored.restitution));
        // Combine, not replace: against a low-friction side the held block wins,
        // against the floor material the higher friction wins (max / max / min).
        let c = dynamics.contact_material(0, low);
        assert_eq!((c.static_friction, c.dynamic_friction, c.restitution), (0.03, 0.02, authored.restitution.min(0.9)));
        let floor = material();
        let c = dynamics.contact_material(0, floor);
        assert_eq!(c, combine_contact_materials(held, floor));
        assert_eq!(c.dynamic_friction, floor.dynamic_friction.max(0.02));
        dynamics.step(&world, &mut layer, &[]);
        assert_eq!(dynamics.body_material(0), authored, "block not restored after commands stopped");
        // Per prop type override (mod) and the old wake rule.
        let mut rules = MoveCommandRules::default();
        rules.by_template.insert(
            "template/crate".into(),
            PropMaterialBlocks { held: Some([0.4, 0.3]), free: Some([0.9, 0.8]), restitution: Some(0.2), ..Default::default() },
        );
        rules.wake_on_command = false;
        dynamics.set_move_rules(rules);
        assert_eq!(dynamics.body_material(0), RetailContactMaterial { static_friction: 0.9, dynamic_friction: 0.8, restitution: 0.2 });
        dynamics.bodies[0].asleep = true;
        dynamics.apply_move_command(7, Vector3::ZERO, 0.0, Vector3::ZERO, simulation().time_step);
        assert!(dynamics.bodies[0].asleep, "wake_on_command off: a zero command leaves the body asleep");
        dynamics.apply_move_command(7, Vector3::new(1.0, 0.0, 0.0), 0.0, Vector3::ZERO, simulation().time_step);
        dynamics.step(&world, &mut layer, &[]);
        assert_eq!(dynamics.body_material(0), RetailContactMaterial { static_friction: 0.4, dynamic_friction: 0.3, restitution: 0.2 });
    }

    /// Per-type DMO data from the vault record (DMO+4380 -> +4 layout of
    /// `livingworld_dynamicobject_characteristics`): +272 restitution, +312
    /// flag, +316 / +324 upright pair, +320 / +328 default pair, inherited from
    /// the parent record like retail's layout.
    #[test]
    fn dmo_type_blocks_read_the_characteristics_record_with_parents() {
        let field = |t: &str, d: &str| serde_json::json!({"type": t, "data": d});
        let float = |v: f32| field("EA::Reflection::Float", &format!("{:08X}", v.to_bits()));
        let vector = |v: [f32; 3]| field("EA::Reflection::Vector3",
            &format!("{:08X}{:08X}{:08X}00000000", v[0].to_bits(), v[1].to_bits(), v[2].to_bits()));
        let json = serde_json::json!({"version": 1, "collections": [
            {"class": DMO_TYPE_CLASS, "key": "default", "parent": "", "source": "", "sha256": "", "fields": {
                "Hash_5CCD5998E03C299B": float(0.5), "Hash_C4D8A03586A31915": field("EA::Reflection::Bool", "00"),
                "Hash_E0101A9DFD63DEE9": float(0.0), "Hash_6E0BB4F5881A4841": float(0.8),
                "Hash_CDA7A31C5EDBEB6E": float(0.0), "Hash_086956BCA2187458": float(0.6),
                "LinearDrag": float(0.0), "AngularDrag": float(0.0),
                "Hash_E5778CDD4576D890": float(100.0), "Hash_4890392C91829954": float(100.0),
                "Hash_BAA01E2BA1237455": float(100.0), "Hash_F4D1C84C36A854AC": vector([1.2; 3]),
                "Hash_D3CDE380DBB3ADC0": vector([0.0; 3]), "Hash_E43445036AAEE7C5": vector([2.5, 15.0, 2.5]),
                "Hash_2114F9329E98F00C": float(5.0)}},
            {"class": DMO_TYPE_CLASS, "key": "cart", "parent": "default", "source": "", "sha256": "", "fields": {
                "Hash_C4D8A03586A31915": field("EA::Reflection::Bool", "01"),
                "Hash_E0101A9DFD63DEE9": float(0.2), "Hash_6E0BB4F5881A4841": float(0.35),
                "Hash_CDA7A31C5EDBEB6E": float(0.175), "Hash_086956BCA2187458": float(0.25),
                "AngularDrag": float(0.35), "Hash_E5778CDD4576D890": float(20.0),
                "Hash_D3CDE380DBB3ADC0": vector([0.0, 0.2, 0.0])}},
        ]});
        let collections: skate_data::collections::Collections = serde_json::from_value(json).unwrap();
        let plain = dmo_type_blocks(&collections, "default").unwrap();
        assert_eq!(plain, PropMaterialBlocks {
            held: None, free: Some([0.8, 0.6]), free_upright: Some([0.0, 0.0]),
            upright_pair: Some(false), restitution: Some(0.5), record_272: Some(false),
            linear_drag: Some(0.0), angular_drag: Some(0.0), mass: Some(100.0),
            maximum_linear_velocity: Some(100.0), maximum_angular_velocity: Some(100.0),
            inertia_scale: Some([1.2; 3]), inertia_offset: Some([0.0; 3]),
            plugin_rotation_tolerance: Some([2.5, 15.0, 2.5]), plugin_max_distance: Some(5.0),
        });
        let cart = dmo_type_blocks(&collections, "cart").unwrap();
        assert_eq!(cart.free, Some([0.35, 0.25]));
        assert_eq!(cart.free_upright, Some([0.2, 0.175]));
        assert_eq!((cart.upright_pair, cart.record_272, cart.restitution), (Some(true), Some(true), Some(0.5)));
        assert_eq!((cart.linear_drag, cart.angular_drag), (Some(0.0), Some(0.35)), "drag inherits per field");
        assert_eq!((cart.mass, cart.maximum_linear_velocity, cart.maximum_angular_velocity), (Some(20.0), Some(100.0), Some(100.0)));
        assert_eq!((cart.inertia_scale, cart.inertia_offset), (Some([1.2; 3]), Some([0.0, 0.2, 0.0])), "vectors inherit per field");
        assert_eq!((cart.plugin_rotation_tolerance, cart.plugin_max_distance), (Some([2.5, 15.0, 2.5]), Some(5.0)), "the plugin check limits inherit");
        assert!(dmo_type_blocks(&collections, "missing").is_err());
    }

    /// The template's authored grab splines become a world-object grab record that moves with the prop.
    #[test]
    fn a_prop_with_grab_splines_is_a_world_object_in_the_grab_scene() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let mut splines = std::collections::BTreeMap::new();
        splines.insert("template".to_owned(), vec![crate::living_world::vehicles::CarGrabSpline { points: vec![[-0.5, 0.3, 0.0], [-0.2, 0.3, 0.0], [0.2, 0.3, 0.0], [0.5, 0.3, 0.0]], direction: [0.0, 0.0, -1.0], flags: 0x3E4 }]);
        assert_eq!(dynamics.set_grab_splines(&splines), 1);
        let objects = dynamics.grab_objects();
        assert_eq!(objects.len(), 1);
        let o = &objects[0];
        assert!(matches!(o.provider, skate_core::player::offboard::grab_scene::Provider::LivingWorld));
        assert_eq!(o.splines[0].descriptor.kind, 2);
        let origin = dynamics.bodies[0].origin();
        let [start, end] = o.record(&o.splines[0]).unwrap().endpoints();
        let scale = dynamics.bodies[0].axis_scale;
        assert!((start[0] - (origin.x - 0.5 * scale.x)).abs() < 1e-5 && (end[1] - (origin.y + 0.3 * scale.y)).abs() < 1e-5, "{start:?} {end:?} {origin:?}");
        assert!(skate_core::player::offboard::grab_scene::Registry::new(objects).is_ok());
    }

    /// Retail type data drives the free block, the upright pair and
    /// record+272; a mod entry keyed by the type's record name (or the MOBJ
    /// template name) overrides single fields; clearing the mod rules
    /// (mod disable) restores the retail values.
    #[test]
    fn retail_type_data_is_the_default_and_mods_override_it() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let authored = dynamics.bodies[0].material;
        let retail = PropMaterialBlocks {
            held: None, free: Some([0.35, 0.25]), free_upright: Some([0.2, 0.175]),
            upright_pair: Some(true), restitution: Some(0.5), record_272: Some(true),
            ..Default::default()
        };
        let mut types = std::collections::BTreeMap::new();
        types.insert("other".to_owned(), DmoType { priority: Some(100), key: "x".into(), blocks: PropMaterialBlocks::default() });
        assert_eq!(dynamics.set_type_data(&types), 0, "no type for this template id");
        assert_eq!(dynamics.body_material(0), authored);
        types.insert("template".to_owned(), DmoType { priority: Some(100), key: "cart".into(), blocks: retail });
        assert_eq!(dynamics.set_type_data(&types), 1);
        assert_eq!(dynamics.type_key(7), Some("cart"));
        // Upright fixture body: the upright pair.
        assert_eq!(dynamics.body_material(0), RetailContactMaterial { static_friction: 0.2, dynamic_friction: 0.175, restitution: 0.5 });
        assert!(dynamics.held_body(7).unwrap().record_272);
        let mut rules = MoveCommandRules::default();
        rules.by_template.insert("cart".into(), PropMaterialBlocks { free_upright: Some([0.9, 0.8]), record_272: Some(false), ..Default::default() });
        dynamics.set_move_rules(rules.clone());
        assert_eq!(dynamics.body_material(0), RetailContactMaterial { static_friction: 0.9, dynamic_friction: 0.8, restitution: 0.5 });
        assert!(!dynamics.held_body(7).unwrap().record_272);
        // The template name entry wins over the type entry.
        rules.by_template.insert("template/crate".into(), PropMaterialBlocks { upright_pair: Some(false), ..Default::default() });
        dynamics.set_move_rules(rules);
        assert_eq!(dynamics.body_material(0), RetailContactMaterial { static_friction: 0.35, dynamic_friction: 0.25, restitution: 0.5 });
        dynamics.set_move_rules(MoveCommandRules::default());
        assert_eq!(dynamics.body_material(0), RetailContactMaterial { static_friction: 0.2, dynamic_friction: 0.175, restitution: 0.5 });
        assert!(dynamics.held_body(7).unwrap().record_272);
    }

    /// Type drag (DMO data +308 / +336 -> Inertia +32 / +36, 82C4E568) is the
    /// body's drag; the integrator 82AE6590 applies it per second
    /// (`v *= max(frequency - drag, 0) dt` each fixed step, so 60 Hz and 30 Hz
    /// steps lose the same speed per second to first order). A mod entry
    /// overrides one field; mod disable restores retail; without type data the
    /// authored damping stays.
    #[test]
    fn type_drag_is_the_body_drag_and_mods_override_it() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let authored = dynamics.bodies[0].inertia;
        assert_eq!(dynamics.body_inertia(0), authored, "no type data: authored damping");
        let mut types = std::collections::BTreeMap::new();
        types.insert("template".to_owned(), DmoType { priority: Some(100), key: "lw_props".into(), blocks: PropMaterialBlocks {
            linear_drag: Some(0.1), angular_drag: Some(0.35), ..Default::default() } });
        assert_eq!(dynamics.set_type_data(&types), 1);
        let inertia = dynamics.body_inertia(0);
        assert_eq!((inertia.linear_drag, inertia.angular_drag), (0.1, 0.35));
        assert_eq!(inertia.inverse_mass, authored.inverse_mass, "drag only");
        // Applied by the retail integrator: one free step scales v and w by
        // (frequency - drag) dt.
        let simulation = dynamics.simulation;
        let mut rates = dynamics.bodies[0].rates;
        rates.linear_velocity = Vector3::new(2.0, 0.0, -1.0);
        rates.angular_velocity = Vector3::new(0.0, 3.0, 0.0);
        rates.force_acceleration = Vector3::ZERO;
        let step = integrate_body_rates(rates, inertia, simulation, RetailReactionCorrections::default());
        let k = |drag: f32| (simulation.frequency - drag) * simulation.time_step;
        let v = step.state.linear_velocity;
        assert!((v.x - 2.0 * k(0.1)).abs() < 1e-5 && (v.z + k(0.1)).abs() < 1e-5, "{v:?}");
        assert!((step.state.angular_velocity.y - 3.0 * k(0.35)).abs() < 1e-5);
        // Mod override, field-wise, by type record name; reset on disable.
        let mut rules = MoveCommandRules::default();
        rules.by_template.insert("lw_props".into(), PropMaterialBlocks { angular_drag: Some(5.0), ..Default::default() });
        dynamics.set_move_rules(rules);
        let modded = dynamics.body_inertia(0);
        assert_eq!((modded.linear_drag, modded.angular_drag), (0.1, 5.0));
        dynamics.set_move_rules(MoveCommandRules::default());
        assert_eq!(dynamics.body_inertia(0).angular_drag, 0.35);
    }

    /// Type body data (82C4E568): mass +304 -> inverse mass, caps +292 / +296,
    /// box inertia from the AABB half extents x +16 + +32 (82C47FC8); the
    /// integrator 82AE6590 shortens faster velocities to the caps. A mod entry
    /// overrides one field (mass alone keeps the type's box); mod disable
    /// restores retail; without type data the authored mass stays.
    #[test]
    fn type_body_data_sets_mass_inertia_and_caps() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let authored = dynamics.bodies[0].inertia;
        let h = dynamics.bodies[0].authored_half_extents;
        let mut types = std::collections::BTreeMap::new();
        types.insert("template".to_owned(), DmoType { priority: Some(100), key: "dt_keg".into(), blocks: PropMaterialBlocks {
            mass: Some(150.0), maximum_linear_velocity: Some(20.0), maximum_angular_velocity: Some(10.0),
            inertia_scale: Some([1.2; 3]), inertia_offset: Some([0.0, 0.5, 0.0]), ..Default::default() } });
        assert_eq!(dynamics.set_type_data(&types), 1);
        let inertia = dynamics.bodies[0].inertia;
        assert_eq!(inertia, dynamics.body_inertia(0));
        assert_eq!(inertia.inverse_mass, 1.0 / 150.0);
        assert_eq!((inertia.maximum_linear_velocity, inertia.maximum_angular_velocity), (20.0, 10.0));
        let b = Vector3::new(h.x * 1.2, h.y * 1.2 + 0.5, h.z * 1.2);
        let k = 150.0 / 3.0;
        let expect = [(b.y * b.y + b.z * b.z) * k, (b.x * b.x + b.z * b.z) * k, (b.x * b.x + b.y * b.y) * k];
        for (got, want) in [inertia.inverse_tensor.x, inertia.inverse_tensor.y, inertia.inverse_tensor.z].into_iter().zip(expect) {
            assert!((got * want - 1.0).abs() < 1e-5, "{got} vs 1/{want}");
        }
        assert_ne!(inertia.inverse_tensor, authored.inverse_tensor);
        // The world inverse inertia used by contacts follows the new tensor.
        let world = world_inverse_inertia(dynamics.bodies[0].rates.basis, inertia.inverse_tensor);
        assert_eq!(dynamics.bodies[0].rates.world_inverse_inertia, world);
        // Caps in the retail integrator: speed above the cap comes out at it.
        let mut rates = dynamics.bodies[0].rates;
        rates.linear_velocity = Vector3::new(30.0, 0.0, 40.0);
        rates.angular_velocity = Vector3::new(0.0, 25.0, 0.0);
        rates.force_acceleration = Vector3::ZERO;
        let step = integrate_body_rates(rates, inertia, dynamics.simulation, RetailReactionCorrections::default());
        assert!((length(step.state.linear_velocity) - 20.0).abs() < 1e-3, "{:?}", step.state.linear_velocity);
        assert!((length(step.state.angular_velocity) - 10.0).abs() < 1e-3);
        // Mod: mass only, by type record name; the type's box and caps stay.
        let mut rules = MoveCommandRules::default();
        rules.by_template.insert("dt_keg".into(), PropMaterialBlocks { mass: Some(300.0), ..Default::default() });
        dynamics.set_move_rules(rules);
        let modded = dynamics.bodies[0].inertia;
        assert_eq!(modded.inverse_mass, 1.0 / 300.0);
        assert_eq!(modded.maximum_linear_velocity, 20.0);
        assert!((modded.inverse_tensor.x * 2.0 - inertia.inverse_tensor.x).abs() < 1e-6);
        dynamics.set_move_rules(MoveCommandRules::default());
        assert_eq!(dynamics.bodies[0].inertia, inertia, "mod disable restores retail");
        dynamics.set_type_data(&std::collections::BTreeMap::new());
        assert_eq!(dynamics.bodies[0].inertia, authored, "no type data: authored");
    }

    /// Free pair choice (82C53EF8 / 82C54BF0 with 82C54B00): only a type with
    /// the upright flag (data +312 bit 0) uses the upright pair, and only while
    /// its up axis y > 0.65; tipped (or exactly 0.65) it uses the default pair.
    /// The held block ignores the upright test.
    #[test]
    fn free_block_follows_the_upright_test() {
        let authored = RetailContactMaterial { static_friction: 0.5, dynamic_friction: 0.5, restitution: 0.1 };
        let blocks = PropMaterialBlocks {
            free: Some([0.6, 0.5]),
            free_upright: Some([0.9, 0.7]),
            restitution: Some(0.3),
            ..Default::default()
        };
        let mut rules = MoveCommandRules::default();
        rules.by_template.insert("t".into(), blocks);
        let pair = |r: &MoveCommandRules, commanded, up_y| {
            let m = r.body_material("t", authored, commanded, up_y);
            [m.static_friction, m.dynamic_friction, m.restitution]
        };
        // Flag off: the default pair whatever the pose.
        assert_eq!(pair(&rules, false, 1.0), [0.6, 0.5, 0.3]);
        assert_eq!(pair(&rules, false, 0.0), [0.6, 0.5, 0.3]);
        rules.by_template.get_mut("t").unwrap().upright_pair = Some(true);
        assert_eq!(pair(&rules, false, 1.0), [0.9, 0.7, 0.3]);
        assert_eq!(pair(&rules, false, 0.66), [0.9, 0.7, 0.3]);
        assert_eq!(pair(&rules, false, 0.65), [0.6, 0.5, 0.3], "strict > 0.65");
        assert_eq!(pair(&rules, false, -1.0), [0.6, 0.5, 0.3]);
        assert_eq!(pair(&rules, true, 1.0), [0.03, 0.02, 0.3]);
        assert_eq!(pair(&rules, true, 0.0), [0.03, 0.02, 0.3]);
        // Threshold is data.
        rules.upright_cos = 0.9;
        assert_eq!(pair(&rules, false, 0.8), [0.6, 0.5, 0.3]);
        // No type data at all: authored material, held keeps the authored restitution.
        let plain = MoveCommandRules::default();
        assert_eq!(plain.body_material("other", authored, false, 1.0), authored);
        assert_eq!(pair(&plain, true, 1.0)[2], 0.1);
        // Deterministic: same inputs, same bits.
        assert_eq!(pair(&rules, false, 0.95).map(f32::to_bits), pair(&rules, false, 0.95).map(f32::to_bits));
    }

    /// Slot 9 sinks (82D9CC78 / 82D9CCF0): the linear command adds L dt at the
    /// centre of mass with no mass factor and no torque, the vertical part is
    /// dropped, and the yaw command adds (0, Y, 0) dt with no inertia factor.
    #[test]
    fn move_command_is_an_acceleration_at_the_centre_of_mass() {
        let (_, _, mut dynamics) = fixture([0., REST_Y, 1.2]);
        let dt = simulation().time_step;
        let grip = Vector3::new(0.4, REST_Y, 0.7);
        dynamics.apply_move_command(7, Vector3::new(6.0, 5.0, -12.0), 3.0, grip, dt);
        let r = dynamics.bodies[0].rates;
        assert!((r.linear_velocity.x - 6.0 * dt).abs() < 1e-6 && (r.linear_velocity.z + 12.0 * dt).abs() < 1e-6);
        assert_eq!(r.linear_velocity.y, 0.0, "vertical command must be dropped");
        assert_eq!((r.angular_velocity.x, r.angular_velocity.z), (0.0, 0.0), "no lever torque at the centre of mass");
        assert!((r.angular_velocity.y - 3.0 * dt).abs() < 1e-6);
        assert_eq!(r.torque_acceleration, Vector3::ZERO);
    }

    /// Contact gap (2026-10-08 street trace): a box lying tilted on the tiled
    /// floor, a few cm into it, must still produce floor manifolds; the
    /// dragged bin lost every floor contact in this pose and fell through.
    /// Cause: the SAT axis is the tilted box face (1-8 deg off the floor),
    /// which triangle fixup sees as an edge region on a welded flat edge;
    /// the GP pair query (is_object false) rejects it on every tile, the
    /// object world query (the production path, `world_query_for`) accepts
    /// it while the tilt stays within acos(1 - convexity_epsilon) = 8.1 deg. The pair query is still checked to keep the repro honest.
    #[test]
    fn lying_tilted_box_keeps_floor_contacts() {
        let world = street_world();
        let half = [0.28f32, 0.41, 0.27];
        let (_, _, dynamics) = box_fixture([0., SIDEWALK_Y + half[1], 0.], half);
        let pair = dynamics.pair_for(0);
        let query = dynamics.world_query_for(0);
        let mut misses = Vec::new();
        let mut pair_misses = 0;
        let mut lowest_up = 1.0f32;
        for step in 0..36 {
            let roll = std::f32::consts::FRAC_PI_2 + (step as f32 - 18.0).to_radians();
            let (s, c) = roll.sin_cos();
            // Rows = local axes in world space: roll about world Z.
            let basis = Basis3 { columns: [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]] };
            let down = half[0] * s.abs() + half[1] * c.abs();
            for depth in [0.0f32, 0.02, 0.05] {
                let center = Vector3::new(0.37, SIDEWALK_Y + down - depth, 2.41);
                let primitive = ContactPrimitive::RoundedBox { center, basis, half_extents: Vector3::new(half[0], half[1], half[2]), radius: 0.0 };
                let bounds = skate_core::physics::board_world::query_metadata::Bounds {
                    min: Vector3::new(center.x - 0.6, center.y - 0.6, center.z - 0.6),
                    max: Vector3::new(center.x + 0.6, center.y + 0.6, center.z + 0.6),
                };
                let mut pair_manifolds = 0;
                let mut candidates = Vec::new();
                for range in world.candidate_ranges(Some(bounds)) {
                    for triangle in &world.triangles()[range] {
                        if primitive_pair_contacts(primitive, ContactPrimitive::Triangle(triangle.triangle), pair).is_some() {
                            pair_manifolds += 1;
                        }
                        candidates.push(triangle.triangle);
                    }
                }
                let mut manifolds = 0;
                for triangle in candidates {
                    if let Some(manifold) = primitive_triangle_world_contacts(primitive, triangle, Vector3::ZERO, query) {
                        manifolds += 1;
                        lowest_up = lowest_up.min(manifold.normal.y);
                    }
                }
                if manifolds == 0 {
                    misses.push((step as i32 - 18, depth));
                }
                if pair_manifolds == 0 {
                    pair_misses += 1;
                }
            }
        }
        println!("tilted box: pair-query misses {pair_misses}, lowest floor normal up {lowest_up:.4}");
        assert!(misses.is_empty(), "no floor contact for (roll offset deg, depth m): {misses:?}");
        assert!(pair_misses > 0, "the GP pair query no longer drops these contacts: re-check the repro");
        assert!(lowest_up > 0.0, "a floor manifold pushes the box down: {lowest_up}");
    }

    /// PROP_BELOW_GROUND's test: a body pushed under the floor face is
    /// detected (the probe starts above its last rest height), one resting
    /// on it is not.
    #[test]
    fn ground_probe_flags_a_body_under_the_floor() {
        let world = street_world();
        let (_, _, mut dynamics) = box_fixture([0., SIDEWALK_Y + 0.4, 0.], [0.3, 0.4, 0.3]);
        let id = dynamics.bodies[0].id;
        let resting = dynamics.ground_probe(id, &world).unwrap();
        assert!(!resting.below_ground() && resting.gap().unwrap().abs() < 1e-3, "{resting:?}");
        dynamics.bodies[0].rates.position.y = SIDEWALK_Y - 0.6;
        let sunk = dynamics.ground_probe(id, &world).unwrap();
        assert!(sunk.below_ground(), "sunk body not flagged: {sunk:?}");
    }

    /// Real DownTown repro (private assets): drag the props the user moved on
    /// 2026-10-07 and release them. `SKATE3_ASSET_ROOT` = assets root,
    /// `SKATE3_MAP` = DownTown.skate. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn downtown_dragged_props_rest_on_the_floor() {
        // skate_vita: the interim carry this test was written for (upstream runs it with prop grab off, the switch's
        // default); a test that turned the switch on before it made the carry use the authored records, under which
        // newspaper box 2198011218 sinks in upstream too (`fad5aae`).
        set_prop_grab_enabled(false);
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT"));
        let map_path = std::path::PathBuf::from(std::env::var_os("SKATE3_MAP").expect("set SKATE3_MAP"));
        let map = skate_data::skate_map::SkateMap::parse(&std::fs::read(&map_path).unwrap()).unwrap();
        let world = crate::skate_world::collision_world(&map, floor_material()).unwrap();
        let ids = [3417526289u32, 880096370, 44597382, 3160070536, 788476715, 2198011218, 3116907260, 206220507];
        let dt = simulation().time_step;
        let mut failures = Vec::new();
        for id in ids {
            let (mut layer, mut dynamics) = crate::skate_world::load_prop_layer(&root, "DownTown", floor_material(), simulation()).expect("DownTown props");
            let Some(spawn) = dynamics.position_of(id) else { println!("{id}: not in the package"); continue };
            let index = dynamics.by_id[&id];
            let half = dynamics.bodies[index].half_extents;
            let ground = dynamics.ground_probe(id, &world).unwrap();
            let floor = ground.ground.unwrap_or(spawn.y - half.y);
            let forward = Vector3::new(0., 0., 1.);
            let reach = dynamics.bodies[index].bounds();
            let start_z = reach.min.z - 0.6;
            let at = |state, z: f32| crate::physics::prop_carry::Carrier {
                state,
                position: Vector3::new(spawn.x, floor + 0.9, z),
                forward,
                time_step: dt,
                skeleton: None,
                ground_grab: None,
            };
            use skate_core::player::state::PhysicalStateId::{BipedGround, OffBoardPushing};
            let mut carry = crate::physics::prop_carry::PropCarry::default();
            carry.update(&mut dynamics, tick(), at(BipedGround, start_z));
            if carry.held() != Some(id) {
                println!("{id}: grabbed {:?} instead", carry.held());
                continue;
            }
            dynamics.set_held(carry.held());
            let mut worst = f32::INFINITY;
            let mut up_min = 1.0f32;
            let mut skater = at(OffBoardPushing, start_z);
            for _ in 1..=180 {
                skater = follow(&carry, skater);
                carry.update(&mut dynamics, stick(0., 1., 0.), skater);
                dynamics.set_held(carry.held());
                dynamics.step(&world, &mut layer, &[]);
                worst = worst.min(dynamics.ground_probe(id, &world).unwrap().gap().unwrap_or(0.));
                up_min = up_min.min(dynamics.bodies[index].rates.basis.columns[1][1]);
            }
            carry.update(&mut dynamics, release(), skater);
            dynamics.set_held(carry.held());
            for _ in 0..180 {
                dynamics.step(&world, &mut layer, &[]);
                worst = worst.min(dynamics.ground_probe(id, &world).unwrap().gap().unwrap_or(0.));
            }
            let end = dynamics.ground_probe(id, &world).unwrap();
            println!(
                "{id} {} type {:?} mass {:.1}: spawn {spawn:?} floor {floor:.2} worst gap {worst:.3} held up_y min {up_min:.4} end {end:?} asleep {}",
                dynamics.bodies[index].template,
                dynamics.type_key(id),
                1.0 / dynamics.bodies[index].inertia.inverse_mass,
                dynamics.bodies[index].asleep
            );
            if end.below_ground() || worst < -0.1 {
                failures.push(id);
            }
        }
        assert!(failures.is_empty(), "props sank: {failures:?}");
    }
}

/// Retail collision types of DMO query meshes (the query mesh's rejection flag is `1 << type`; b221-b223 and the trace
/// in .local/research/prop-types): a held prop (a ped's hand prop switches on pickup, slot 32 `82C56B00`).
pub(crate) const HELD_COLLISION_TYPE: u32 = 13;
/// A prop at least `walkable_mass` heavy.
pub(crate) const HEAVY_COLLISION_TYPE: u32 = 12;
/// A lighter prop.
pub(crate) const LIGHT_COLLISION_TYPE: u32 = 14;

/// The collision type of a prop of `mass` kg (13 while held). NOT RETAIL YET for the player's own grab: the ped's
/// pickup switches to 13 in the trace; that the player's grab does too is not traced.
pub(crate) fn collision_type(mass: f32, held: bool, walkable_mass: f32) -> u32 {
    if held {
        HELD_COLLISION_TYPE
    } else if mass >= walkable_mass {
        HEAVY_COLLISION_TYPE
    } else {
        LIGHT_COLLISION_TYPE
    }
}

impl PropDynamics {
    /// Each body's collision instance and retail collision type (template mass, not a carry's effective mass).
    pub(crate) fn for_each_collision_type(&self, held: Option<u32>, mut f: impl FnMut(usize, u32)) {
        let walkable = self.tuning.solver.walkable_mass;
        for body in &self.bodies {
            let w = body.authored_inertia.inverse_mass;
            let mass = if w > 0.0 { 1.0 / w } else { f32::INFINITY };
            f(body.instance, collision_type(mass, held == Some(body.id), walkable));
        }
    }
}

#[cfg(test)]
mod collision_type_tests {
    use super::*;

    #[test]
    fn props_get_retails_collision_types_by_mass() {
        // b223: the flat bench is 50 kg, the cone 1.75 kg; 5.5 kg itself is heavy (`>=`).
        assert_eq!(collision_type(50.0, false, 5.5), HEAVY_COLLISION_TYPE);
        assert_eq!(collision_type(1.75, false, 5.5), LIGHT_COLLISION_TYPE);
        assert_eq!(collision_type(5.5, false, 5.5), HEAVY_COLLISION_TYPE);
        assert_eq!(collision_type(50.0, true, 5.5), HELD_COLLISION_TYPE);
        // The walker's mask (0x6000) skips 13 and 14, so only the heavy type is ground for it.
        let mask = skate_core::player::offboard::air_selector::MESH_REJECT_MASK;
        assert_eq!(mask & (1 << HEAVY_COLLISION_TYPE), 0);
        assert_ne!(mask & (1 << LIGHT_COLLISION_TYPE), 0);
        assert_ne!(mask & (1 << HELD_COLLISION_TYPE), 0);
    }
}
