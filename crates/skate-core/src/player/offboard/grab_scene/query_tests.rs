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

/// b334: the registry's duplicate-id check by set reports the same error as the old scan of the earlier ids, at the same
/// object, also when another check fails first (an id 0 or a wrong record kind later or earlier in the list).
#[test]
fn registry_errors_match_the_old_duplicate_scan() {
    fn old(objects: &[Object]) -> Result<(), &'static str> {
        for (index, object) in objects.iter().enumerate() {
            if object.id == 0 {
                return Err("Interactable object ID zero is reserved for none");
            }
            for spline in &object.splines {
                if matches!(object.provider, Provider::Vehicle) != (spline.descriptor.kind == 1) {
                    return Err("Car grab records are type 1, others type 2");
                }
            }
            if objects[..index].iter().any(|prior| prior.id == object.id) {
                return Err("Duplicate authored interactable object ID");
            }
        }
        Ok(())
    }
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..2000 {
        let n = (next() % 8) as usize;
        let objects: Vec<Object> = (0..n)
            .map(|_| {
                let id = (next() % 6) as u32;
                // One in eight a car with a type 2 record (a wrong kind).
                if next() % 8 == 0 { object(id.max(1), Provider::Vehicle, 2, 0.0) } else { object(id, Provider::LivingWorld, 2, 0.0) }
            })
            .collect();
        assert_eq!(Registry::new(objects.clone()).map(|_| ()), old(&objects), "{:?}", objects.iter().map(|o| o.id).collect::<Vec<_>>());
    }
}
