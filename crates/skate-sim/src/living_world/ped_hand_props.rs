//! skate_vita (L1e stage 2): skate-game `living_world/ped_hand_props.rs` (upstream PR #52 `618e824`) without Bevy.
//! The records are verbatim; the held object is a [`HeldHandProp`] on the ped entity (its frame computed from the
//! ped's pose when the brain needs it: a throw's release), the released object a runtime prop body whose id the ped
//! keeps while linked ([`PedEntity::linked`](super::peds::PedEntity)); the models are drawn by the renderer from
//! the published poses. The collision of a released object is its model's triangles, baked beside it
//! (`hand_props/<key>.tri`).
//!
//! Upstream's module docs follow.
//!
//! Ped hand props (b87, b89): the `livingworld_handprops` records (offset, carry channel) and their models
//! (`living_world/hand_props.json` + `hand_props/<key>.glb`, setup `tools/asset_pipeline/hand_props.py` from the
//! retail DMO templates), and the held object on the ped.
//!
//! Retail: SpawnInteractionBasedHandProp requests the record (`82E3DDA0`, `brain+3279` 0x01); the ped update
//! creates the DMO (`82E3DE18`) and attaches it (`82E3EC60`: `ped+5920`, `brain+3278` 0x02); every frame
//! `82E3E4F0` / `82E3E1B0` put it at the hand matrix composed with the record's local offset [code].
//! NOT RETAIL YET: the hand bone is RIGHTHANDPROP (rig 26) [inferred: every carry channel is `*RH`; which bone fills
//! the hand matrix at skeleton +19504 is not decoded]; the record fields `Hash_3FE1...` = Euler rotation in degrees
//! and `Hash_DC20...` = translation in metres, applied translation then X, Y, Z [inferred from the value shapes; the
//! record loader is not read].
//!
//! Release (b90, b91): the brain's throw / drop (`skate_core::living_world::peds::hand_prop`) releases the object here:
//! it becomes a prop body created mid-game (`GamePhysics::spawn_runtime_prop`, doc 27 "Props created mid-game") with
//! the release velocity, its model follows the body, and the ped keeps the link until the prop leaves the unlink box.
//! The ped never removes it (b90 §3); the dynamic-object pool (49) and the census cull (100 m) do.

use std::collections::BTreeMap;
use std::path::Path;

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde_json::Value;

/// The rig bone a hand prop follows.
pub(crate) const HAND_PROP_BONE: &str = "RIGHTHANDPROP";

const ROTATION_FIELD: &str = "Hash_3FE10F7B1115B8E6";
const OFFSET_FIELD: &str = "Hash_DC20CCEAB4B92992";
const CHANNEL_FIELD: &str = "Hash_FC1D2C4E5CCA6AED";
/// IsDisposable (record +60), CanSitWithHandProp (+61), CanAttackThrowHandProp (+62) [b90: names from the hashes].
const DISPOSABLE_FIELD: &str = "Hash_D02B4381CF62E28A";
const CAN_SIT_FIELD: &str = "Hash_BCE5969F18AEA914";
const CAN_ATTACK_THROW_FIELD: &str = "Hash_40802A8929B6B25E";

/// One hand prop record.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HandPropRecord {
    /// The model under the asset root (`private/living_world/hand_props/<key>.glb`); `None` when setup found no
    /// template for it (stock: `orange`).
    pub glb: Option<String>,
    pub rotation_degrees: Vec3,
    pub offset: Vec3,
    /// The model's attach locator in its own (glb) space: the point retail puts at the record's offset, so the grip
    /// lands on the hand rather than the model origin. From `hand_props.json` `attach` (setup,
    /// `dynamic_props.template_attach`); identity when absent (a mod model, or an older install).
    pub attach: Mat4,
    pub carry_channel: Option<String>,
    pub disposable: bool,
    pub can_sit: bool,
    pub can_attack_throw: bool,
}

impl HandPropRecord {
    /// The prop's frame in the hand bone's frame, as retail `82E3E1B0` composes it: T(offset) * R * inverse(P).
    /// R turns the record's degrees into Rz * Ry * Rx (research b204: rows [cy cz, cy sz, -sy], ...), so the order
    /// is ZYX, not XYZ; P is the template's attach locator (`template+0x78`, written by `82C4E128`, b204 / b205).
    pub fn local(&self) -> Mat4 {
        let r = self.rotation_degrees * (std::f32::consts::PI / 180.0);
        Mat4::from_translation(self.offset) * Mat4::from_quat(Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x)) * self.attach.inverse()
    }
}

/// Every hand prop by key (`livingworld_handprops`), and the idle actions: each entity's usage per prop
/// (`livingworld_entities` `handprops`: {handprop, f32_24, f32_28, usage}) and each usage's action list
/// (`livingworld_handprops_usagecharacteristics` `actions`) [data, b164].
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HandPropData {
    pub props: BTreeMap<String, HandPropRecord>,
    pub usages: BTreeMap<String, Vec<skate_core::living_world::peds::hand_prop::HandPropAction>>,
    pub entity_usage: BTreeMap<(String, String), String>,
}

fn flag(fields: Option<&Value>, key: &str) -> bool {
    fields.and_then(|f| f.get(key)).and_then(Value::as_bool).unwrap_or(false)
}

fn vec3(v: Option<&Value>) -> Vec3 {
    let f = |k: &str| v.and_then(|v| v.get(k)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    Vec3::new(f("x"), f("y"), f("z"))
}

/// A 4x4 row-vector matrix as setup writes it (rows = basis vectors, translation last), as a glam matrix; `None`
/// unless it is 16 finite numbers with an invertible rotation.
fn attach_matrix(v: &Value) -> Option<Mat4> {
    let rows = v.as_array().filter(|r| r.len() == 4)?;
    let mut cols = [0.0_f32; 16];
    for (i, row) in rows.iter().enumerate() {
        let row = row.as_array().filter(|r| r.len() == 4)?;
        for (j, x) in row.iter().enumerate() {
            cols[i * 4 + j] = x.as_f64()? as f32;
        }
    }
    let m = Mat4::from_cols_array(&cols);
    (m.is_finite() && m.determinant().abs() > 1e-6).then_some(m)
}

impl HandPropData {
    /// From `private/living_world/tables.json` (inheritance already merged by the exporter) and `hand_props.json`.
    pub fn load(root: &Path) -> Result<Self, String> {
        Self::load_with(root, None)
    }

    /// [`Self::load`] with `tables.json` already parsed (`None`: read and parse it here).
    pub fn load_with(root: &Path, parsed: Option<&Value>) -> Result<Self, String> {
        // skate_vita: through `skate_data::vfs`; the packs carry the peds' classes in `ped_tables.json`.
        let read = |p: &str| -> Result<Value, String> { serde_json::from_slice(&skate_data::vfs::read(&root.join(p)).map_err(|e| format!("{p}: {e}"))?).map_err(|e| format!("{p}: {e}")) };
        let own;
        let tables: &Value = match parsed {
            Some(v) => v,
            None => {
                own = read("private/living_world/ped_tables.json").or_else(|_| read("private/living_world/tables.json"))?;
                &own
            }
        };
        let records = tables.pointer("/classes/livingworld_handprops").and_then(Value::as_object).ok_or("tables.json has no livingworld_handprops")?;
        let models = read("private/living_world/hand_props.json").ok();
        let mut props = BTreeMap::new();
        for (key, record) in records {
            let fields = record.get("fields");
            let file = models.as_ref().and_then(|m| m.pointer(&format!("/props/{key}/file"))).and_then(Value::as_str);
            let attach = models.as_ref().and_then(|m| m.pointer(&format!("/props/{key}/attach"))).and_then(attach_matrix);
            props.insert(
                key.clone(),
                HandPropRecord {
                    glb: file.map(|f| format!("private/living_world/{f}")),
                    rotation_degrees: vec3(fields.and_then(|f| f.get(ROTATION_FIELD))),
                    offset: vec3(fields.and_then(|f| f.get(OFFSET_FIELD))),
                    attach: attach.unwrap_or(Mat4::IDENTITY),
                    carry_channel: fields.and_then(|f| f.get(CHANNEL_FIELD)).and_then(Value::as_str).map(str::to_string),
                    disposable: flag(fields, DISPOSABLE_FIELD),
                    can_sit: flag(fields, CAN_SIT_FIELD),
                    can_attack_throw: flag(fields, CAN_ATTACK_THROW_FIELD),
                },
            );
        }
        let key_of = |v: Option<&Value>| v.and_then(|r| r.get("key")).and_then(Value::as_str).map(str::to_string);
        let mut usages = BTreeMap::new();
        if let Some(records) = tables.pointer("/classes/livingworld_handprops_usagecharacteristics").and_then(Value::as_object) {
            for (key, record) in records {
                let list = record.pointer("/fields/actions").and_then(Value::as_array).map_or(Vec::new(), |a| {
                    a.iter()
                        .map(|e| {
                            let f = |k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                            skate_core::living_world::peds::hand_prop::HandPropAction {
                                action: e.get("action").and_then(Value::as_i64).unwrap_or(0) as i32,
                                weight: f("f32_4"),
                                min_seconds: f("f32_8"),
                                max_seconds: f("f32_12"),
                            }
                        })
                        .collect()
                });
                usages.insert(key.clone(), list);
            }
        }
        let mut entity_usage = BTreeMap::new();
        if let Some(records) = tables.pointer("/classes/livingworld_entities").and_then(Value::as_object) {
            for (entity, record) in records {
                for e in record.pointer("/fields/handprops").and_then(Value::as_array).into_iter().flatten() {
                    if let (Some(prop), Some(usage)) = (key_of(e.get("handprop")), key_of(e.get("usage"))) {
                        entity_usage.insert((entity.clone(), prop), usage);
                    }
                }
            }
        }
        Ok(Self { props, usages, entity_usage })
    }

    /// The idle actions of `entity` holding `prop` (empty: the ped never acts with it).
    pub fn actions_for(&self, entity: &str, prop: &str) -> Vec<skate_core::living_world::peds::hand_prop::HandPropAction> {
        self.entity_usage.get(&(entity.to_string(), prop.to_string())).and_then(|u| self.usages.get(u)).cloned().unwrap_or_default()
    }
}

/// The object a ped holds (upstream `HeldHandProp`, the created DMO `ped+5920`): its key and its frame in the hand
/// bone's frame.
#[derive(Clone, Debug)]
pub(crate) struct HeldHandProp {
    pub key: String,
    pub local: Mat4,
}

/// The held object's world frame: the ped's drawn root (position, heading), the hand bone's model-space frame of the
/// ped's pose this tick, the record's offset (upstream: the prop entity's `GlobalTransform`, `present_ped_pose`).
pub(crate) fn held_world(data: &super::peds::PedData, body: &super::peds::PedBody, shown: Vec3, local: Mat4) -> Option<Mat4> {
    let bone = data.rig.names.iter().position(|n| n.eq_ignore_ascii_case(HAND_PROP_BONE))?;
    let globals = super::peds::ped_globals(&data.rig, body, data, 0.0, &[])?;
    Some(Mat4::from_rotation_translation(Quat::from_rotation_y(body.heading), shown) * *globals.get(bone)? * local)
}

/// A hand prop's collision triangles in its model frame (`hand_props/<key>.tri`), read once.
fn triangles(cache: &mut BTreeMap<String, Option<std::sync::Arc<Vec<[Vec3; 3]>>>>, root: &Path, glb: &str) -> Option<std::sync::Arc<Vec<[Vec3; 3]>>> {
    cache
        .entry(glb.to_owned())
        .or_insert_with(|| {
            let path = root.join(glb.strip_suffix(".glb").unwrap_or(glb).to_owned() + ".tri");
            let bytes = skate_data::vfs::read(&path).ok()?;
            let count = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
            let floats: Vec<f32> = bytes.get(4..4 + count * 36)?.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
            Some(std::sync::Arc::new(floats.chunks_exact(9).map(|t| [Vec3::new(t[0], t[1], t[2]), Vec3::new(t[3], t[4], t[5]), Vec3::new(t[6], t[7], t[8])]).collect()))
        })
        .clone()
}

/// Create the requested object (then `holding`, `82E3EC60`; the request bit clears whether or not the create worked,
/// `82E262D0`), release it on the brain's throw / drop, unlink it, cull released props, and remove a held object the
/// brain no longer has. skate_vita: upstream's `sync_hand_props` over the ped index; a ped's scene exists from its
/// spawn here.
pub(crate) fn sync_hand_props(peds: &mut super::peds::PedState, settings: &super::LivingWorldSettings, observers: &[skate_core::living_world::Observer], physics: &mut crate::physics::GamePhysics, asset_root: &Path, tick: u64) {
    let s = settings.ped_brain.values.hand_prop;
    let super::peds::PedState { data, index, released, hand_triangles, .. } = peds;
    for e in index.values_mut() {
        let super::peds::PedEntity { ped, body, mind, held, linked, shown, .. } = e;
        // The tazer's clip-driven states (`82E3ED50`): drawn near the draw clip's end, put away (and the "tazr" prop
        // deleted) near the holster clip's end.
        if mind.brain.tazer_state.is_some_and(|s| s == 1 || s == 3) {
            let left = body.player.channel_remaining(&*data);
            mind.brain.update_tazer_clip(&settings.ped_brain.values, body.plugin_motion.tazer_played, left);
        }
        if let Some(h) = held.clone() {
            // The world frame is needed by a throw's release only (`update_hand_prop_release` reads it then).
            let at = mind.brain.hand_prop.throw.and_then(|_| held_world(data, body, *shown, h.local)).map(|m| m.w_axis.truncate().to_array());
            // A throw's release, or a litter drop (`82E3ED50`: timer 34 after HandPropLitter).
            let thrown = at.and_then(|at| mind.brain.update_hand_prop_release(&s, at)).or_else(|| mind.brain.update_litter());
            if let Some(velocity) = thrown.or(mind.hand_prop_release.take()) {
                *held = None;
                let room = released.len() < s.max_live;
                let model = data.hand_props.props.get(&h.key).and_then(|r| r.glb.clone());
                let tris = model.as_deref().and_then(|g| triangles(hand_triangles, asset_root, g));
                let id = held_world(data, body, *shown, h.local).zip(tris).and_then(|(world, tris)| release(physics, &h, world, &tris, velocity, room));
                match id {
                    Some(id) => {
                        released.push((id, h.key.clone(), None));
                        *linked = Some(id);
                        info!("PED_HAND_PROP ped=#{} {} released body={id} velocity=[{:.2}, {:.2}, {:.2}] tick={tick}", ped.id.serial, h.key, velocity[0], velocity[1], velocity[2]);
                    }
                    None => {
                        mind.brain.hand_prop.clear();
                        info!("PED_HAND_PROP ped=#{} {} released without a body tick={tick}", ped.id.serial, h.key);
                    }
                }
                continue;
            }
            let want = mind.brain.hand_prop.has().then(|| mind.brain.hand_prop.key.clone()).flatten();
            if want.as_deref() != Some(h.key.as_str()) {
                *held = None;
                mind.brain.hand_prop.holding = false;
                info!("PED_HAND_PROP ped=#{} {} removed tick={tick}", ped.id.serial, h.key);
                continue;
            }
            // The idle action (`82E3F470`): the channel cue changes only when the action moves on.
            let left = body.player.channel_remaining(&*data);
            if let Some(cue) = mind.brain.update_hand_prop_action(&s.actions, left) {
                if mind.action_cue != Some(cue) {
                    info!("PED_HAND_PROP_ACTION ped=#{} {} {cue:?} action={} tick={tick}", ped.id.serial, h.key, mind.brain.hand_prop.action);
                    mind.action_cue = Some(cue);
                    mind.action_seq = mind.action_seq.wrapping_add(1);
                }
            }
            continue;
        }
        mind.hand_prop_release = None;
        // The link (`82E3F090`): unlinked once the released prop leaves the box around the ped, or when it is gone.
        if let Some(r) = *linked {
            let at = physics.prop_dynamics().and_then(|d| d.position_of(r));
            let out = match at {
                Some(at) => mind.brain.update_hand_prop_link(&s, [at.x, at.y, at.z], body.position.to_array()),
                None => {
                    mind.brain.hand_prop.clear();
                    true
                }
            };
            if out {
                *linked = None;
                info!("PED_HAND_PROP ped=#{} unlinked body={r} tick={tick}", ped.id.serial);
            }
            continue;
        }
        let want = mind.brain.hand_prop.requested.then(|| mind.brain.hand_prop.key.clone()).flatten();
        let Some(key) = want else { continue };
        mind.brain.hand_prop.requested = false;
        let record = data.hand_props.props.get(&key);
        if record.and_then(|r| r.glb.as_ref()).is_none() {
            info!("PED_HAND_PROP ped=#{} {key} not created (no model) tick={tick}", ped.id.serial);
            continue;
        }
        if released.len() >= s.max_live {
            info!("PED_HAND_PROP ped=#{} {key} not created (pool full) tick={tick}", ped.id.serial);
            continue;
        }
        *held = Some(HeldHandProp { key: key.clone(), local: record.map(HandPropRecord::local).unwrap_or_default() });
        mind.brain.hand_prop.holding = true;
        // The attach (`82E3DE18`): this entity's usage of the prop, then the first pick.
        mind.brain.attach_hand_prop_actions(data.hand_props.actions_for(&ped.entity, &key));
        mind.action_cue = None;
        info!("PED_HAND_PROP ped=#{} {key} created actions={} tick={tick}", ped.id.serial, mind.brain.hand_prop.actions.len());
    }
    // Released props outside every observer's census cull ring go (b91 / dmo-plan: `dynamicobjects` cull 100 m).
    released.retain(|(id, _, _)| {
        let at = physics.prop_dynamics().and_then(|d| d.position_of(*id)).map(|v| Vec3::new(v.x, v.y, v.z));
        let near = at.is_some_and(|p| observers.is_empty() || observers.iter().any(|o| Vec3::from_array(o.position).distance(p) <= s.cull_distance));
        if near {
            return true;
        }
        physics.remove_runtime_prop(*id);
        info!("PED_HAND_PROP culled body={id} tick={tick}");
        false
    });
}

/// The released hand props in the skater's body contact solve (b94): each one is a solid proxy with its box, mass and
/// velocity in contact group 12 (heavy) or 14 (small), so a hit goes through the ordinary skeleton contact: the relative
/// normal speed (halved for small objects) feeds the region forces and the usual wipeout checks (`82BD4A30` ->
/// `82BD88A0`). Retail has no hand-prop hit code (b92 Q2). The prop's own reaction stays the prop step's skater push.
/// skate_vita: no replay check (the Vita's replays do not run the living world).
pub(crate) fn push_hand_prop_proxies(peds: &super::peds::PedState, settings: &super::LivingWorldSettings, physics: &mut crate::physics::GamePhysics, skater: &crate::physics::SkaterRuntime) {
    let s = settings.ped_brain.values.hand_prop;
    if peds.released.is_empty() || !s.skater_contact {
        return;
    }
    let small_mass = skater.collision_feedback.settings.small_object_mass;
    let solids: Vec<_> = {
        let Some(dynamics) = physics.prop_dynamics() else { return };
        peds.released.iter().filter_map(|(id, _, _)| dynamics.body_state(*id).map(|b| hand_prop_proxy(*id, b, small_mass, &s))).collect()
    };
    let mut proxies = std::mem::take(&mut physics.network_proxies);
    for solid in solids {
        proxies.append_solid(solid, physics, skater, false);
    }
    physics.network_proxies = proxies;
}

/// Proxy ids of released hand props: `HAND_PROP_PROXY_TAG | body id`.
pub(crate) const HAND_PROP_PROXY_TAG: u64 = 0x4850_0000_0000_0000;

type BodyState = (skate_core::math::Vector3, skate_core::math::Basis3, skate_core::math::Vector3, skate_core::math::Vector3, skate_core::math::Vector3, skate_core::physics::rigid_body::RetailInertiaDynamics);

fn hand_prop_proxy(id: u32, (c, basis, half, v, w, inertia): BodyState, small_mass: f32, s: &skate_core::living_world::peds::hand_prop::HandPropSettings) -> crate::dynamics::SolidBody {
    use crate::dynamics::rapier3d::prelude::{Pose, Rotation, SharedShape, Vector};
    let q = Quat::from_mat3(&glam::Mat3::from_cols_array_2d(&basis.columns)).normalize();
    let rotation = Rotation::from_xyzw(q.x, q.y, q.z, q.w).normalize();
    let p = |v: skate_core::math::Vector3| Vector::new(v.x, v.y, v.z);
    let mass = if inertia.inverse_mass > 0.0 { 1.0 / inertia.inverse_mass } else { f32::INFINITY };
    crate::dynamics::SolidBody {
        id: HAND_PROP_PROXY_TAG | u64::from(id),
        pose: Pose::from_parts(p(c), rotation),
        center_of_mass: p(c),
        inertia_rotation: rotation,
        inverse_mass: inertia.inverse_mass,
        inverse_inertia: p(inertia.inverse_tensor),
        linvel: p(v),
        angvel: p(w),
        contact_group: if mass >= small_mass { s.heavy_group } else { s.small_group },
        colliders: vec![crate::dynamics::SolidCollider {
            shape: SharedShape::cuboid(half.x.max(0.01), half.y.max(0.01), half.z.max(0.01)),
            pose: Pose::from_parts(p(c), rotation),
            friction: 0.5,
        }],
    }
}

/// Turn the held object into a prop body at its world pose moving at `velocity`: collision from its meshes (in the
/// object's frame, scale folded in), the default physics block (NOT RETAIL YET: the template's
/// `livingworld_dynamicobject_characteristics` record is not resolved for hand prop templates, b91), and its model
/// detached from the hand and following the body. `None` when the pool is full or the model has no triangles.
/// skate_vita: the triangles are the baked model's, the frame is [`held_world`].
fn release(physics: &mut crate::physics::GamePhysics, held: &HeldHandProp, world: Mat4, model: &[[Vec3; 3]], velocity: [f32; 3], room: bool) -> Option<u32> {
    if !room || model.is_empty() {
        return None;
    }
    let (_, rotation, translation) = world.to_scale_rotation_translation();
    let to_local = |p: Vec3| rotation.inverse() * (world.transform_point3(p) - translation);
    let local = model.iter().map(|t| t.map(|p| {
        let q = to_local(p);
        skate_core::math::Vector3::new(q.x, q.y, q.z)
    })).collect();
    let m = glam::Mat3::from_quat(rotation);
    let spec = crate::physics::prop_dynamics::RuntimeProp {
        template: format!("handprop/{}", held.key),
        local,
        physics: Default::default(),
        type_data: None,
        origin: skate_core::math::Vector3::new(translation.x, translation.y, translation.z),
        basis: skate_core::math::Basis3 { columns: [m.x_axis.to_array(), m.y_axis.to_array(), m.z_axis.to_array()] },
        linear_velocity: skate_core::math::Vector3::new(velocity[0], velocity[1], velocity[2]),
        angular_velocity: skate_core::math::Vector3::ZERO,
    };
    physics.spawn_runtime_prop(&spec, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_offset_is_translation_then_rotation() {
        let r = HandPropRecord { rotation_degrees: Vec3::new(90.0, 0.0, 0.0), offset: Vec3::new(0.01, 0.0, 0.01), ..Default::default() };
        let m = r.local();
        assert!(m.w_axis.truncate().distance(Vec3::new(0.01, 0.0, 0.01)) < 1e-6);
        // +Y of the prop turns to +Z of the hand under 90 degrees about X.
        assert!(m.transform_vector3(Vec3::Y).distance(Vec3::Z) < 1e-6);
    }

    /// Retail `82E3E1B0` builds the rotation as Rz * Ry * Rx; its first basis row is [cy cz, cy sz, -sy] (b204).
    #[test]
    fn record_rotation_is_rz_ry_rx_like_retail() {
        // The stock cellphone's angles (15, -10, 15 degrees): with two or more non-zero axes the order matters.
        let d = Vec3::new(15.0, -10.0, 15.0);
        let r = HandPropRecord { rotation_degrees: d, ..Default::default() }.local();
        let (x, y, z) = (d.x.to_radians(), d.y.to_radians(), d.z.to_radians());
        let row0 = Vec3::new(y.cos() * z.cos(), y.cos() * z.sin(), -y.sin());
        assert!(r.transform_vector3(Vec3::X).distance(row0) < 1e-5, "{:?} vs {row0:?}", r.transform_vector3(Vec3::X));
        let rzyx = Mat4::from_rotation_z(z) * Mat4::from_rotation_y(y) * Mat4::from_rotation_x(x);
        assert!(r.abs_diff_eq(rzyx, 1e-5));
    }

    /// Retail places the attach locator P, not the model origin, at the record's offset: World = H * T * R * inv(P).
    #[test]
    fn attach_locator_lands_on_the_offset() {
        // A briefcase-like locator as setup writes it (row vectors): handle 0.343 m up, turned 90 degrees.
        let rows = serde_json::json!([[0.0, -1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [-1.0, 0.0, 0.0, 0.0], [0.0, 0.343, 0.0, 1.0]]);
        let attach = attach_matrix(&rows).unwrap();
        assert!(attach.w_axis.truncate().distance(Vec3::new(0.0, 0.343, 0.0)) < 1e-6);
        assert!(attach.transform_vector3(Vec3::X).distance(Vec3::new(0.0, -1.0, 0.0)) < 1e-6, "row 0 is the image of +x");
        let r = HandPropRecord { rotation_degrees: Vec3::new(15.0, -10.0, 15.0), offset: Vec3::new(0.01, 0.005, 0.0), attach, ..Default::default() };
        let m = r.local();
        // The handle lands on the offset, with the locator's frame turned by R.
        assert!(m.transform_point3(Vec3::new(0.0, 0.343, 0.0)).distance(r.offset) < 1e-5);
        let rot = Mat4::from_quat(Quat::from_euler(EulerRot::ZYX, 15f32.to_radians(), (-10f32).to_radians(), 15f32.to_radians()));
        assert!((m * attach).abs_diff_eq(Mat4::from_translation(r.offset) * rot, 1e-5));
        // Missing or broken: identity (the model origin, as before).
        assert_eq!(HandPropRecord::default().attach, Mat4::IDENTITY);
        assert!(attach_matrix(&serde_json::json!([[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]])).is_none());
        assert!(attach_matrix(&serde_json::json!(null)).is_none());
    }
}
