//! Saved skate.parks on the memory card (docs/skatepark-survey.md step S3): each park a retail `.park` file
//! (`skate_sim::park_file`) under [`DIR`], listed in `index.json` (name, file, map, origin, when). Retail keeps its parks
//! in its save data (a list of 224-byte records, `82511FC0`); the Vita keeps files.
//!
//! Each park has a picture (retail's browser shows one per park from the memory card): the park as the pause menu
//! froze it, [`PICTURE`] pixels, a PNG beside the park file.
//!
//! Save Park's names compare without case (retail `82512500` finds a taken name; NOT RETAIL YET: its compare's case
//! rule not read).
use serde::{Deserialize, Serialize};

pub const DIR: &str = "ux0:data/SKATEVITA/parks";
const INDEX: &str = "index.json";
/// A park picture's size (16:9, the screen's fifth).
pub const PICTURE: (u32, u32) = (192, 108);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    /// The park file under [`DIR`].
    pub file: String,
    /// The world it was saved on (`packs::WORLDS` name).
    pub map: String,
    /// The template world it started from (the file header's origin, hex).
    pub origin: String,
    /// Seconds since the epoch when it was saved.
    pub saved: u64,
    /// Its picture under [`DIR`] (empty: none).
    #[serde(default)]
    pub picture: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    parks: Vec<Entry>,
}

fn dir() -> std::path::PathBuf {
    std::path::PathBuf::from(DIR)
}

fn index() -> Index {
    std::fs::read(dir().join(INDEX)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The saved parks, newest first.
pub fn list() -> Vec<Entry> {
    let mut parks = index().parks;
    parks.sort_by(|a, b| b.saved.cmp(&a.saved));
    parks
}

/// A park of that name is saved already.
pub fn exists(name: &str) -> bool {
    index().parks.iter().any(|e| e.name.eq_ignore_ascii_case(name))
}

/// The screen (`read_rgba`, 960 x 544 RGBA, bottom row first) as a park picture, top row first: each pixel the
/// average of its block.
pub fn picture(screen: &[u8], width: usize, height: usize) -> Vec<u8> {
    let (w, h) = (PICTURE.0 as usize, PICTURE.1 as usize);
    let (bx, by) = (width / w, height / h);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for yy in y * by..(y + 1) * by {
                for xx in x * bx..(x + 1) * bx {
                    let at = ((height - 1 - yy) * width + xx) * 4;
                    for c in 0..4 {
                        sum[c] += u32::from(screen.get(at + c).copied().unwrap_or(0));
                    }
                }
            }
            out.extend(sum.map(|v| (v / (bx * by).max(1) as u32) as u8));
        }
    }
    out
}

/// Saves `bytes` (a `.park` file) as `name` on `map`, over a park of the same name (Save Park's Yes) or as a new one,
/// with its `picture` ([`PICTURE`] RGBA) when there is one.
pub fn save(name: &str, bytes: &[u8], map: &str, picture: Option<&[u8]>) -> Result<Entry, String> {
    std::fs::create_dir_all(dir()).map_err(|e| format!("{DIR}: {e}"))?;
    let mut index = index();
    let origin = skate_sim::park_file::ParkFile::parse(bytes).map(|f| format!("{:016X}", f.origin)).unwrap_or_default();
    let saved = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let file = match index.parks.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
        Some(e) => e.file.clone(),
        None => {
            let next = (1..).find(|n| !index.parks.iter().any(|e| e.file == format!("park_{n}.park"))).unwrap_or(1);
            format!("park_{next}.park")
        }
    };
    std::fs::write(dir().join(&file), bytes).map_err(|e| format!("{file}: {e}"))?;
    let picture_file = picture.and_then(|rgba| {
        let path = file.replace(".park", ".png");
        write_png(&dir().join(&path), rgba).map_err(|e| println!("park picture {path}: {e}")).ok().map(|_| path)
    });
    let entry = Entry { name: name.to_owned(), file, map: map.to_owned(), origin, saved, picture: picture_file.unwrap_or_default() };
    index.parks.retain(|e| !e.name.eq_ignore_ascii_case(name));
    index.parks.push(entry.clone());
    std::fs::write(dir().join(INDEX), serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?).map_err(|e| format!("{INDEX}: {e}"))?;
    Ok(entry)
}

fn write_png(path: &std::path::Path, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), PICTURE.0, PICTURE.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(rgba)).map_err(|e| e.to_string())
}

/// A saved park's `.park` bytes.
pub fn read(entry: &Entry) -> Result<Vec<u8>, String> {
    std::fs::read(dir().join(&entry.file)).map_err(|e| format!("{}: {e}", entry.file))
}

/// A saved park's picture (width, height, RGBA), None without one.
pub fn read_picture(entry: &Entry) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(dir().join(&entry.picture)).ok().filter(|_| !entry.picture.is_empty())?;
    crate::teleporter_icons::decode_png(&bytes).ok()
}

/// Deletes the park named `name` (its file, picture and index entry).
pub fn delete(name: &str) -> Result<(), String> {
    let mut index = index();
    let Some(k) = index.parks.iter().position(|e| e.name.eq_ignore_ascii_case(name)) else { return Ok(()) };
    let entry = index.parks.remove(k);
    let _ = std::fs::remove_file(dir().join(&entry.file));
    if !entry.picture.is_empty() {
        let _ = std::fs::remove_file(dir().join(&entry.picture));
    }
    std::fs::write(dir().join(INDEX), serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?).map_err(|e| format!("{INDEX}: {e}"))
}

/// A Park Setup lot (the baked catalogue's `park_lots`, in retail's order): its name and templates.
#[derive(Clone, Debug, Default)]
pub struct Lot {
    pub name: String,
    pub templates: Vec<Template>,
}

/// A lot's template: its world row, name, thumbnail and map (`packs::WORLDS` name).
#[derive(Clone, Debug, Default)]
pub struct Template {
    pub world: String,
    pub name: String,
    pub thumb: String,
    pub map: String,
}

/// Park Setup's lots from the catalogue pack (none without it).
pub fn lots() -> Vec<Lot> {
    let path = std::path::Path::new(crate::packs::ROOT).join("private/object-catalog/catalog.json");
    let Some(catalog) = skate_data::vfs::read(&path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return Vec::new() };
    let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_owned();
    catalog["park_lots"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| Lot {
            name: text(&l["name"]),
            templates: l["templates"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| Template { world: text(&t["world"]), name: text(&t["name"]), thumb: text(&t["thumb"]), map: text(&t["map"]) })
                .collect(),
        })
        .collect()
}

/// A save time as the browser's date (year-month-day, UTC).
pub fn date_text(seconds: u64) -> String {
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = (seconds / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("#{year:04}-{month:02}-{day:02}")
}
