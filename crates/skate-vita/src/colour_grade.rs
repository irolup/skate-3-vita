//! The zombie cheat's colour grade (skate-game `colour_matrix.rs`, PR #52 `b110bab`; research b100): the stock
//! `post_fx` record `colour_matrix_zombiemode` from the collections in sim.pak, built into the renderer's near / far
//! matrices (`skate_core::colour_matrix::build`). The base record is the identity (`Grade::IDENTITY`).
use skate_core::colour_matrix::{build, ColourBand, ColourMatrixRecord};
use skate_data::collections::Collections;
use vita_render::scene::Grade;

const CLASS: &str = "post_fx";
const ZOMBIE: &str = "colour_matrix_zombiemode";

fn vec3(c: &Collections, record: &str, name: &str) -> Result<[f32; 3], String> {
    let w = c.words::<3>(CLASS, record, name)?;
    let v = [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2])];
    if v.iter().all(|x| x.is_finite()) { Ok(v) } else { Err(format!("Non-finite {CLASS}/{record}/{name}")) }
}

fn band(c: &Collections, record: &str, side: &str) -> Result<ColourBand, String> {
    let f = |n: &str| c.float(CLASS, record, &format!("{side}_{n}"));
    Ok(ColourBand {
        multiply: vec3(c, record, &format!("{side}_multiply"))?,
        add: vec3(c, record, &format!("{side}_add"))?,
        contrast: f("contrast")?,
        contrast_midpoint: f("contrast_midpoint")?,
        saturation: f("saturation")?,
    })
}

/// One record (parents included).
fn load(c: &Collections, record: &str) -> Result<ColourMatrixRecord, String> {
    Ok(ColourMatrixRecord {
        near: band(c, record, "near")?,
        far: band(c, record, "far")?,
        distance_near: c.float(CLASS, record, "distance_near")?,
        distance_far: c.float(CLASS, record, "distance_far")?,
    })
}

/// The renderer's grade of a record.
pub fn grade(record: &ColourMatrixRecord) -> Grade {
    Grade { near: build(&record.near), far: build(&record.far), distance_near: record.distance_near, distance_far: record.distance_far }
}

/// The zombie grade from the collections under `root` (None: not there; the cheat then draws ungraded).
pub fn zombie(root: &std::path::Path) -> Option<Grade> {
    let record = Collections::load(root).and_then(|c| load(&c, ZOMBIE));
    match record {
        Ok(r) => Some(grade(&r)),
        Err(e) => {
            println!("colour grade {ZOMBIE}: {e}");
            None
        }
    }
}
