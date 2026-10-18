use super::*;
use crate::player::offboard::grab_scene::{AssemblyData, Descriptor, Geometry, Object, Spline};
use crate::player::offboard::ground_query::QueryContext;
use crate::player::offboard::ground_sync::{BoardLimits, Bounds};
use std::sync::Arc;

fn object(id: u32, provider: Provider, kind: u32, at: f32) -> Object {
    let geometry = Arc::new(Geometry { id, points: vec![[-1., 0., 0., 1.], [1., 0., 0., 1.]], approach_vectors: vec![[0., 0., 1., 0.]], word_60: 0 });
    Object {
        id,
        provider,
        disabled: false,
        assembly_ready: true,
        assembly: Some(AssemblyData { identity: id, first_part: None }),
        frame: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [at, 0., 0., 1.]],
        object_vector_128: [0.; 4],
        splines: vec![Spline { descriptor: Descriptor { kind, id }, geometry, word_272: 0 }],
    }
}

fn q(mode: u32) -> Query {
    Query {
        position: [0., 0., 1., 0.],
        sort_position: [0., 0., 1., 0.],
        bounds: Bounds { frame: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., -1., 0.], [0.; 4]], extents: [2., 1., 2., 0.] },
        limits: BoardLimits { margin: 0.1, angle_a: 1., angle_b: 1. },
        mode,
        capacity: 32,
        context: QueryContext { selection_flags_2948: 0, matching_id_2952: -1 },
    }
}

#[test]
fn cars_answer_only_the_vehicle_bit_and_world_objects_only_the_other() {
    let registry = Registry::new(vec![object(1, Provider::Vehicle, 1, 0.0), object(2, Provider::LivingWorld, 2, 0.0)]).unwrap();
    let kinds = |mode| query(&registry, &q(mode)).unwrap().iter().map(|r| r.descriptor().kind).collect::<Vec<_>>();
    assert_eq!(kinds(4), vec![2]);
    assert_eq!(kinds(2), vec![1]);
    assert_eq!(kinds(255).len(), 2, "the skitch query's mode 255 reaches both");
    assert!(query(&registry, &q(1)).is_err());
}

#[test]
fn a_car_needs_its_assembly_and_must_be_within_the_sphere() {
    let mut car = object(1, Provider::Vehicle, 1, 0.0);
    car.assembly = None;
    let far = object(2, Provider::Vehicle, 1, 40.0);
    let registry = Registry::new(vec![car, far]).unwrap();
    assert!(query(&registry, &q(255)).unwrap().is_empty());
    assert!(Registry::new(vec![object(3, Provider::Vehicle, 2, 0.0)]).is_err(), "car records are type 1");
}
