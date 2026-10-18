//! Dynamic clips of the front-end screens (skate_vita, M9): ActionScript's
//! `attachMovie` and `removeMovieClip`, which the trick display never uses. The linked
//! screen (`tools/link_ui_screen.py`) carries an export table (linkage name → character).
//! Attached clips live at `ATTACH_DEPTH + depth` among their parent's children, so they
//! are drawn and removed with it and survive the parent's timeline seeks.
use std::collections::BTreeMap;

use crate::apt_movie::Movie;
use crate::apt_vm::{Value, Vm};

pub const ATTACH_DEPTH: i32 = 16384;

/// `parent.attachMovie(linkage, name, depth, initObject)`: returns the new clip.
pub fn attach_movie(
    movie: &mut Movie,
    vm: &mut Vm,
    exports: &BTreeMap<String, i32>,
    parent: usize,
    args: &[Value],
) -> Result<Value, String> {
    let linkage = args.first().ok_or("attachMovie: linkage name")?.text();
    let name = args.get(1).ok_or("attachMovie: instance name")?.text();
    let depth = args.get(2).map_or(0.0, Value::number);
    if !depth.is_finite() || !(0.0..1_048_576.0).contains(&depth) {
        return Err(format!("attachMovie: invalid depth {depth}"));
    }
    let key = ATTACH_DEPTH + depth as i32;
    // An unknown linkage name attaches nothing and returns undefined, as in Flash (M9 edit).
    let Some(&character) = exports.get(&linkage) else {
        return Ok(Value::Undefined);
    };
    // A clip already at that depth is replaced.
    if let Some(old) = movie.instances.get_mut(&parent).and_then(|i| i.children.remove(&key)) {
        movie.remove(vm, old);
    }
    let id = movie.create(vm, character, Some(parent), 1)?;
    vm.set(parent, &name, Value::Object(id))?;
    if let Some(Value::Object(init)) = args.get(3) {
        let fields: Vec<(String, Value)> = vm.objects[*init].fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in fields {
            vm.set(id, &k, v)?;
        }
    }
    movie.instances.get_mut(&parent).ok_or("attachMovie: parent is not a clip")?.children.insert(key, id);
    Ok(Value::Object(id))
}

/// `clip.removeMovieClip()`: removes an attached clip (a timeline clip is only hidden).
pub fn remove_movie_clip(movie: &mut Movie, vm: &mut Vm, clip: usize) -> Result<(), String> {
    let parent = match vm.get(clip, "_parent") {
        Value::Object(parent) => Some(parent),
        _ => None,
    };
    if let Some(parent) = parent {
        let mut name = None;
        if let Some(instance) = movie.instances.get_mut(&parent) {
            let keys: Vec<i32> = instance.children.iter().filter(|(_, c)| **c == clip).map(|(k, _)| *k).collect();
            for k in keys {
                instance.children.remove(&k);
            }
        }
        for (field, value) in &vm.objects[parent].fields {
            if matches!(value, Value::Object(o) if *o == clip) {
                name = Some(field.clone());
            }
        }
        if let Some(name) = name {
            vm.objects[parent].fields.remove(&name);
        }
    }
    movie.remove(vm, clip);
    Ok(())
}
