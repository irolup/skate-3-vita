use super::*;
use skate_data::trigger_volumes::parse;

fn record_file(volumes: &[(&str, [f32; 3], [f32; 3])]) -> Vec<TriggerVolumeRecord> {
    let rows: Vec<_> = volumes.iter().map(|(id, c, h)| serde_json::json!({
        "id": id, "name": format!("{id}_name"), "instance_id": "2c70170600250000",
        "shape": {"kind": "box", "center": c, "half_extents": h}})).collect();
    let doc = serde_json::json!({"format": "skate3rust-trigger-volumes", "version": 1, "volumes": rows});
    parse(&serde_json::to_vec(&doc).unwrap()).unwrap().volumes
}

/// A standing skater as the recomp measured one: feet on the ground, head 1.62 m
/// and hips 0.98 m above them.
fn standing(x: f32, z: f32) -> Cylinder {
    QueryShape::RETAIL.cylinder([x, 0., z], [x, 1.62, z], [x, 0.98, z]).unwrap()
}

fn app(volumes: TriggerVolumes) -> App {
    let mut app = App::new();
    app.insert_resource(volumes)
        .init_resource::<TriggerBodies>()
        .insert_resource(TriggerState::default()) // player not tracked: no SkaterRuntime here
        .add_message::<TriggerEntered>()
        .add_message::<TriggerExited>()
        .add_systems(Update, update_bodies_only);
    app
}

/// `update` without the player (tests have no skater runtime).
fn update_bodies_only(
    volumes: Res<TriggerVolumes>,
    bodies: Res<TriggerBodies>,
    mut state: ResMut<TriggerState>,
    entered: MessageWriter<TriggerEntered>,
    exited: MessageWriter<TriggerExited>,
) {
    step(&volumes, &bodies, &mut state, None, entered, exited);
}

fn drain<M: Message + Clone>(app: &mut App) -> Vec<M> {
    app.world_mut().resource_mut::<Messages<M>>().drain().collect()
}

#[test]
fn bodies_entering_and_leaving_publish_messages_and_mod_events() {
    let volumes = TriggerVolumes::from_records(&record_file(&[
        ("gate", [0., 1., 0.], [2., 1., 2.]),
        ("far", [50., 1., 0.], [2., 1., 2.]),
    ]), "test".into()).unwrap();
    let mut app = app(volumes);
    app.world_mut().resource_mut::<TriggerBodies>().bodies.insert("mod:demo:ball".into(), standing(10., 0.));
    app.update();
    assert!(drain::<TriggerEntered>(&mut app).is_empty());
    app.world_mut().resource_mut::<TriggerBodies>().bodies.insert("mod:demo:ball".into(), standing(1., 1.));
    app.update();
    assert_eq!(drain::<TriggerEntered>(&mut app), vec![TriggerEntered { body: "mod:demo:ball".into(), volume: "gate".into() }]);
    app.world_mut().resource_mut::<TriggerBodies>().bodies.insert("mod:demo:ball".into(), standing(50., 0.));
    app.update();
    assert_eq!(drain::<TriggerEntered>(&mut app), vec![TriggerEntered { body: "mod:demo:ball".into(), volume: "far".into() }]);
    assert_eq!(drain::<TriggerExited>(&mut app), vec![TriggerExited { body: "mod:demo:ball".into(), volume: "gate".into() }]);
    // No longer tracked while inside: exit (retail entity removal 82DD6D20).
    app.world_mut().resource_mut::<TriggerBodies>().bodies.clear();
    app.update();
    assert_eq!(drain::<TriggerExited>(&mut app), vec![TriggerExited { body: "mod:demo:ball".into(), volume: "far".into() }]);
    let queue = std::mem::take(&mut app.world_mut().resource_mut::<TriggerState>().mod_queue);
    assert_eq!(queue.iter().map(|e| (e.volume.as_str(), e.entered)).collect::<Vec<_>>(),
        vec![("gate", true), ("far", true), ("gate", false), ("far", false)]);
}

#[test]
fn mods_add_volumes_and_switch_map_volumes_off_until_they_go_away() {
    let mut volumes = TriggerVolumes::from_records(&record_file(&[("reset", [0., 0., 0.], [100., 50., 100.])]), "test".into()).unwrap();
    let id = TriggerVolumes::mod_id("demo", "goal");
    volumes.mods.insert(("demo".into(), "goal".into()), TriggerVolume {
        id: id.clone(), name: "goal".into(), full_name: None, group: TriggerGroup::Challenge,
        shape: OrientedBox::axis_aligned([5., 0., 5.], [7., 2., 7.]), instance_id: None, link_guid: None,
        source: VolumeSource::Mod("demo".into()),
    });
    volumes.disabled.insert("reset".into(), "demo".into());
    assert_eq!(volumes.active().iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), vec![id.as_str()]);
    let mut app = app(volumes);
    app.world_mut().resource_mut::<TriggerBodies>().bodies.insert("player".into(), standing(6., 6.));
    app.update();
    assert_eq!(drain::<TriggerEntered>(&mut app).iter().map(|e| e.volume.clone()).collect::<Vec<_>>(), vec![id.clone()]);
    // The mod unloads: its volume and its switch go; the map volume is live again.
    app.world_mut().resource_mut::<TriggerVolumes>().clear_owner("demo");
    app.update();
    assert_eq!(drain::<TriggerEntered>(&mut app).iter().map(|e| e.volume.clone()).collect::<Vec<_>>(), vec!["reset".to_string()]);
    // A removed volume is forgotten without an exit (retail RemoveVolume 82DD7018).
    assert!(drain::<TriggerExited>(&mut app).is_empty());
}

#[test]
fn only_challenge_volumes_track_bodies() {
    // Retail's Stairs and Camera groups are built without entity slots: their
    // volumes are listed but never post enter/exit.
    let rows: Vec<_> = [("challenge", "a"), ("stairs", "b"), ("camera", "c")].iter().map(|(group, id)| serde_json::json!({
        "id": id, "name": id, "group": group,
        "shape": {"kind": "box", "center": [0., 1., 0.], "half_extents": [2., 1., 2.]}})).collect();
    let doc = serde_json::json!({"format": "skate3rust-trigger-volumes", "version": 1, "volumes": rows});
    let records = parse(&serde_json::to_vec(&doc).unwrap()).unwrap().volumes;
    let volumes = TriggerVolumes::from_records(&records, "test".into()).unwrap();
    assert_eq!(volumes.map.len(), 3);
    let mut app = app(volumes);
    app.world_mut().resource_mut::<TriggerBodies>().bodies.insert(PLAYER.into(), standing(0., 0.));
    app.update();
    assert_eq!(drain::<TriggerEntered>(&mut app), vec![TriggerEntered { body: PLAYER.into(), volume: "a".into() }]);
}

#[test]
fn map_ids_cannot_use_the_mod_prefix() {
    assert!(TriggerVolumes::from_records(&record_file(&[("mod:x:y", [0.; 3], [1.; 3])]), "t".into()).is_err());
}

#[test]
fn missing_sidecar_means_no_volumes_and_check_passes() {
    let path = std::env::temp_dir().join("skate-no-such-map-for-triggers.skate");
    assert_eq!(check(Some(&path), None), Ok(0));
    assert_eq!(TriggerVolumes::load_or_warn(Some(&path), None).map.len(), 0);
}

/// Data-gated: `SKATE3_MAPS_DIR` = maps directory with converted `.triggers`.
/// The SkateSchool authored start lies in `tut_sksc_inthehub_vol_02` and the
/// map-wide `tut_sksc_reset_vol01`, but not in `inthehub_vol_01`'s area.
#[test]
fn skateschool_start_is_inside_the_hub_and_reset_volumes() {
    let Some(dir) = std::env::var_os("SKATE3_MAPS_DIR").map(std::path::PathBuf::from) else {
        eprintln!("skipped: set SKATE3_MAPS_DIR");
        return;
    };
    let volumes = TriggerVolumes::load(Some(&dir.join("SkateSchool.skate")), None).unwrap();
    assert_eq!(volumes.map.len(), 6);
    let mut app = app(volumes);
    app.world_mut().resource_mut::<TriggerBodies>().bodies
        .insert(PLAYER.into(), QueryShape::RETAIL.cylinder([-31.2822, 0.112, -31.5035], [-31.2822, 1.732, -31.5035], [-31.2822, 1.092, -31.5035]).unwrap());
    app.update();
    let names: Vec<String> = drain::<TriggerEntered>(&mut app).iter()
        .map(|e| app.world().resource::<TriggerVolumes>().get(&e.volume).unwrap().name.clone()).collect();
    assert_eq!(names, vec!["tut_sksc_inthehub_vol_02", "tut_sksc_reset_vol01"]);
    // A corner of the 45 degree hub square's bounds is outside the square itself
    // (retail tests the oriented box, not the bounds); the map-wide box still holds it.
    app.world_mut().resource_mut::<TriggerBodies>().bodies
        .insert(PLAYER.into(), standing(8.7, 44.5));
    app.update();
    let names: Vec<String> = drain::<TriggerExited>(&mut app).iter()
        .map(|e| app.world().resource::<TriggerVolumes>().get(&e.volume).unwrap().name.clone()).collect();
    assert_eq!(names, vec!["tut_sksc_inthehub_vol_02"]);
}
