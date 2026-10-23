//! Retail front-end menu movies (APT), loaded and linked (doc 31, menus milestone 3c); their shapes and bitmaps
//! resolve for drawing through `MenuScene` (milestone 4). skate_vita: skate-game `retail_menu_movies.rs` (PR #52
//! `fad5aae`) without Bevy and without mods (no mod movies, bitmaps or images: `set_mod_*`, `clear_*`, `snapshot`
//! and the PNG decoder are not taken); the set is read through a reader (`read_with`), so the Vita reads it from its
//! packs. `relink`, `shapes_of`, `MenuScene`, `image` and the key rules are verbatim.
//!
//! Setup exports the movies from the owned disc (`tools/prepare_menu_screens.py`, the reference's
//! `prepare_menu_movies.py`) into `assets/private/menu-movies/`: `manifest.json`, `shared.json` (language table,
//! fonts, native font layout) and `movies/<key>.json` per movie in the HUD player format plus its import tables.
use crate::fe::{
    apt_imports::{library_key, resolve, MovieDecl},
    apt_movie::Movie,
    apt_scene::{Shape, ShapeSource, Shapes},
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const FORMAT: &str = "skate3-menu-movies";
/// Manifest version setup writes (3: textured units carry their bitmap id, Milestone 4; 4: screens/main; 5: main.apt's
/// layer movies; 6: the challenge map; 7: named runtime images, the challenge map pictures; 8: the Skate Profile, FE 61;
/// 9: Extras, FE 18; 10: the text viewer, FE 3; 11: the replay editor and help, FE 12 / 13).
pub const VERSION: u64 = 11;
/// The oldest version this engine still reads (upstream's rule: 6, 8, 9, 10 and 11 only add a movie and 7 an optional
/// image table).
pub const OLDEST_VERSION: u64 = 5;
/// Texture key prefix of a named runtime image (`Game.ImageRenderer_Load`): `image/<image_key>`.
pub const NAMED_IMAGE_PREFIX: &str = "image/";

/// The table key of a runtime image request: retail builds `data\` + name + `.rx2` (b258 section 2); `\` -> `/`, no
/// leading `/` or `data/`, no `.rx2`, lower case.
pub fn image_key(request: &str) -> String {
    let path = request.replace('\\', "/").to_ascii_lowercase();
    let path = path.trim_start_matches('/');
    let path = path.strip_prefix("data/").unwrap_or(path);
    path.strip_suffix(".rx2").unwrap_or(path).to_owned()
}

/// What a texture key from `MenuScene` draws: a retail payload in the exported set.
#[derive(Debug, PartialEq, Eq)]
pub enum MenuImage<'a> {
    /// RGBA8 file relative to the menu movie set root, with its size.
    Retail { path: &'a str, width: u32, height: u32 },
}

/// Stable bitmap id: movie key plus bitmap character id.
pub fn bitmap_id(movie: &str, bitmap: i32) -> String {
    format!("{movie}#{bitmap}")
}

/// Reads a file of the set by its path relative to the set's root.
pub type Reader<'a> = &'a dyn Fn(&str) -> Result<Vec<u8>, String>;

/// Linked movies plus their sources.
#[derive(Default)]
pub struct RetailMenuMovies {
    /// Retail movie JSON by key, with the shared fields merged in.
    base: BTreeMap<String, Value>,
    /// Shared language / font fields.
    shared: serde_json::Map<String, Value>,
    /// GEO units by movie key, then shape character.
    shapes: BTreeMap<String, Shapes>,
    /// Retail texture payloads (key = path in the set) with their size.
    retail_images: BTreeMap<String, [u32; 2]>,
    /// Named runtime images (manifest `images`, version 7): image key -> payload path, size.
    named_images: BTreeMap<String, (String, [u32; 2])>,
    /// Linked movies by key.
    pub movies: BTreeMap<String, Movie>,
    pub imports_total: usize,
    pub imports_resolved: usize,
    /// Movies that failed to load or link, with the reason.
    pub failures: BTreeMap<String, String>,
    /// skate_vita: the shared text of a lean set ([`Self::read_lean`]).
    text: Option<crate::fe::apt_text::SharedText>,
}

fn merged(mut movie: Value, shared: &serde_json::Map<String, Value>) -> Value {
    if let Some(object) = movie.as_object_mut() {
        for (k, v) in shared {
            object.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    movie
}

fn read_json(read: Reader, path: &str) -> Result<Value, String> {
    serde_json::from_slice(&read(path)?).map_err(|e| format!("{path}: {e}"))
}

impl RetailMenuMovies {
    /// Reads the exported set from a folder; an absent set gives an empty set.
    pub fn read(root: &Path) -> Result<Self, String> {
        if !root.join("manifest.json").is_file() {
            return Ok(Self::default());
        }
        Self::read_with(&|path: &str| std::fs::read(root.join(path)).map_err(|e| format!("{path}: {e}")))
    }

    /// Reads the exported set through `read`.
    pub fn read_with(read: Reader) -> Result<Self, String> {
        let manifest = read_json(read, "manifest.json")?;
        let version = manifest["version"].as_u64().unwrap_or(0);
        if manifest["format"] != FORMAT || !(OLDEST_VERSION..=VERSION).contains(&version) {
            return Err("unsupported menu movie set".into());
        }
        let shared = read_json(read, "shared.json")?.as_object().cloned().ok_or("menu movie shared.json is not an object")?;
        let mut out = Self { shared, ..Self::default() };
        for (key, file) in manifest["movies"].as_object().ok_or("menu movie list missing")? {
            let file = file.as_str().ok_or("menu movie file missing")?;
            if file.contains("..") {
                return Err(format!("menu movie path escapes its folder: {file}"));
            }
            out.base.insert(library_key(key), merged(read_json(read, file)?, &out.shared));
        }
        for (key, image) in manifest["images"].as_object().into_iter().flatten() {
            let path = image["rgba"].as_str().ok_or("menu image path missing")?;
            if path.contains("..") {
                return Err(format!("menu image path escapes its folder: {path}"));
            }
            let size = [image["width"].as_u64().unwrap_or(0) as u32, image["height"].as_u64().unwrap_or(0) as u32];
            out.named_images.insert(image_key(key), (path.to_owned(), size));
        }
        out.relink();
        Ok(out)
    }

    /// The source JSON for a key (retail; no mods here).
    fn source(&self, key: &str) -> Option<&Value> {
        self.base.get(key)
    }

    /// Loads every movie and links its imports (verbatim but the mod sources).
    pub fn relink(&mut self) {
        let mut keys: Vec<String> = self.base.keys().cloned().collect();
        keys.sort();
        keys.dedup();
        self.failures.clear();
        let mut all_shapes = BTreeMap::new();
        let mut images = BTreeMap::new();
        // Font atlases: text draws name them (apt_text), so they resolve like bitmaps.
        for font in self.shared.get("fonts").and_then(Value::as_object).into_iter().flat_map(|f| f.values()) {
            let size = &font["definition"]["textures"][0];
            if let (Some(path), Some(w), Some(h)) = (font["texture"].as_str(), size["width"].as_u64(), size["height"].as_u64()) {
                images.insert(path.to_string(), [w as u32, h as u32]);
            }
        }
        let mut decls = BTreeMap::new();
        let mut libraries = BTreeMap::new();
        for key in &keys {
            let json = self.source(key).expect("listed key");
            match shapes_of(json) {
                Ok(shapes) => {
                    for t in shapes.values().flatten().filter_map(|s| s.texture.as_ref()) {
                        images.insert(t.rgba.clone(), [t.width, t.height]);
                    }
                    all_shapes.insert(key.clone(), shapes);
                }
                Err(e) => {
                    self.failures.insert(key.clone(), e);
                    continue;
                }
            }
            // Import tables resolve even when the player cannot load the movie yet.
            match MovieDecl::from_json(json) {
                Ok(decl) => {
                    decls.insert(key.clone(), decl);
                }
                Err(e) => {
                    self.failures.insert(key.clone(), e);
                    continue;
                }
            }
            match Movie::load(json) {
                Ok(movie) => {
                    libraries.insert(key.clone(), movie);
                }
                Err(e) => {
                    self.failures.insert(key.clone(), e);
                }
            }
        }
        let (mut total, mut resolved) = (0, 0);
        let mut linked = BTreeMap::new();
        for (key, decl) in &decls {
            total += decl.imports.len();
            let resolution = resolve(key, decl.clone(), &mut decls.clone());
            resolved += resolution.bindings.iter().filter(|b| &b.movie == key).count();
            if self.failures.contains_key(key) {
                continue;
            }
            let json = self.source(key).expect("listed key");
            let result = Movie::load(json).and_then(|mut m| m.link_imports(key, &resolution, &libraries).map(|_| m));
            match result {
                Ok(m) => {
                    linked.insert(key.clone(), m);
                }
                Err(e) => {
                    self.failures.insert(key.clone(), e);
                }
            }
        }
        self.movies = linked;
        self.shapes = all_shapes;
        self.retail_images = images;
        self.imports_total = total;
        self.imports_resolved = resolved;
    }

    /// skate_vita (memory): the set as the Vita holds it: the movies reachable from `roots` through their imports
    /// (all of them for `None`), linked as [`Self::relink`] links them, with one [`SharedText`] for all (upstream merged
    /// the shared language and fonts into every movie and kept every movie's JSON). A movie's resolution only follows
    /// its own imports, so a subset links each of its movies as the whole set does.
    pub fn read_lean(read: Reader, roots: Option<&[&str]>) -> Result<Self, String> {
        let manifest = read_json(read, "manifest.json")?;
        let version = manifest["version"].as_u64().unwrap_or(0);
        if manifest["format"] != FORMAT || !(OLDEST_VERSION..=VERSION).contains(&version) {
            return Err("unsupported menu movie set".into());
        }
        let shared_json = read_json(read, "shared.json")?;
        let shared = crate::fe::apt_text::SharedText::from_json(&shared_json)?;
        let mut out = Self::default();
        // The shared fields but the language (the font atlases' sizes for `image`).
        if let Some(object) = shared_json.as_object() {
            out.shared = object.iter().filter(|(k, _)| *k != "language").map(|(k, v)| (k.clone(), v.clone())).collect();
        }
        drop(shared_json);
        let mut files = BTreeMap::new();
        for (key, file) in manifest["movies"].as_object().ok_or("menu movie list missing")? {
            let file = file.as_str().ok_or("menu movie file missing")?;
            if file.contains("..") {
                return Err(format!("menu movie path escapes its folder: {file}"));
            }
            files.insert(library_key(key), file.to_owned());
        }
        for (key, image) in manifest["images"].as_object().into_iter().flatten() {
            let path = image["rgba"].as_str().ok_or("menu image path missing")?;
            if path.contains("..") {
                return Err(format!("menu image path escapes its folder: {path}"));
            }
            let size = [image["width"].as_u64().unwrap_or(0) as u32, image["height"].as_u64().unwrap_or(0) as u32];
            out.named_images.insert(image_key(key), (path.to_owned(), size));
        }
        // The keys to load: the roots and every movie their imports reach.
        let mut queue: Vec<String> = match roots {
            Some(roots) => roots.iter().map(|r| library_key(r)).collect(),
            None => files.keys().cloned().collect(),
        };
        let mut images = BTreeMap::new();
        for font in out.shared.get("fonts").and_then(Value::as_object).into_iter().flat_map(|f| f.values()) {
            let size = &font["definition"]["textures"][0];
            if let (Some(path), Some(w), Some(h)) = (font["texture"].as_str(), size["width"].as_u64(), size["height"].as_u64()) {
                images.insert(path.to_string(), [w as u32, h as u32]);
            }
        }
        let mut decls = BTreeMap::new();
        let mut libraries = BTreeMap::new();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(key) = queue.pop() {
            if !seen.insert(key.clone()) {
                continue;
            }
            let Some(file) = files.get(&key) else { continue };
            let json = read_json(read, file)?;
            match shapes_of(&json) {
                Ok(shapes) => {
                    for t in shapes.values().flatten().filter_map(|s| s.texture.as_ref()) {
                        images.insert(t.rgba.clone(), [t.width, t.height]);
                    }
                    out.shapes.insert(key.clone(), shapes);
                }
                Err(e) => {
                    out.failures.insert(key.clone(), e);
                    continue;
                }
            }
            match MovieDecl::from_json(&json) {
                Ok(decl) => {
                    queue.extend(decl.imports.iter().map(|i| library_key(&i.file)));
                    decls.insert(key.clone(), decl);
                }
                Err(e) => {
                    out.failures.insert(key.clone(), e);
                    continue;
                }
            }
            match Movie::load_shared(&json, &shared) {
                Ok(movie) => {
                    libraries.insert(key.clone(), movie);
                }
                Err(e) => {
                    out.failures.insert(key.clone(), e);
                }
            }
        }
        let (mut total, mut resolved) = (0, 0);
        for (key, decl) in &decls {
            total += decl.imports.len();
            let resolution = resolve(key, decl.clone(), &mut decls.clone());
            resolved += resolution.bindings.iter().filter(|b| &b.movie == key).count();
            if out.failures.contains_key(key) {
                continue;
            }
            // `relink` loads the movie again from its JSON before linking; the library copy is the same load.
            let Some(mut movie) = libraries.get(key).cloned() else { continue };
            match movie.link_imports(key, &resolution, &libraries) {
                Ok(_) => {
                    out.movies.insert(key.clone(), movie);
                }
                Err(e) => {
                    out.failures.insert(key.clone(), e);
                }
            }
        }
        out.retail_images = images;
        out.imports_total = total;
        out.imports_resolved = resolved;
        out.text = Some(shared);
        // The fonts' JSON was only read for the atlas sizes above (the language comes from `text`).
        out.shared = Default::default();
        Ok(out)
    }

    /// Whether a runtime image exists.
    pub fn has_named_image(&self, name: &str) -> bool {
        self.named_images.contains_key(&image_key(name))
    }

    /// Shapes and bitmaps for `apt_scene::draw` of a running instance (a clone of `movies[key]`, which carries the
    /// imported shapes' origins).
    pub fn scene<'a>(&'a self, key: &'a str, movie: &'a Movie) -> MenuScene<'a> {
        MenuScene { set: self, key, movie }
    }

    /// skate_vita: the retail textures the set's shapes and fonts name (key = path in the set), with their size.
    pub fn retail_images(&self) -> impl Iterator<Item = (&str, [u32; 2])> {
        self.retail_images.iter().map(|(k, s)| (k.as_str(), *s))
    }

    /// The image behind a texture key from `MenuScene`.
    pub fn image(&self, texture: &str) -> Option<MenuImage<'_>> {
        if let Some(key) = texture.strip_prefix(NAMED_IMAGE_PREFIX) {
            let (path, size) = self.named_images.get(key)?;
            return Some(MenuImage::Retail { path, width: size[0], height: size[1] });
        }
        let (path, size) = self.retail_images.get_key_value(texture)?;
        Some(MenuImage::Retail { path, width: size[0], height: size[1] })
    }

    /// The shared language table (string id -> text).
    pub fn language(&self) -> std::sync::Arc<BTreeMap<String, String>> {
        match &self.text {
            Some(text) => std::sync::Arc::clone(text.language()),
            None => std::sync::Arc::new(self.shared.get("language").and_then(|l| serde_json::from_value(l.clone()).ok()).unwrap_or_default()),
        }
    }
}

/// GEO units of a movie JSON (`shapes`, absent = none), keyed by shape character id.
fn shapes_of(json: &Value) -> Result<Shapes, String> {
    if json["shapes"].is_null() {
        return Ok(Shapes::new());
    }
    let shapes: Shapes = serde_json::from_value(json["shapes"].clone()).map_err(|e| format!("menu shapes: {e}"))?;
    for unit in shapes.values().flatten() {
        if unit.fill.textured() && (unit.texture.is_none() || unit.bitmap.is_none()) {
            return Err("menu shape textured unit without its bitmap".into());
        }
        let finite = unit.triangles.iter().flatten().all(|v| v.position.iter().chain(&v.uv).all(|x| x.is_finite()));
        if !finite || !unit.color.iter().all(|x| x.is_finite()) {
            return Err("menu shape with a nonfinite value".into());
        }
    }
    Ok(shapes)
}

/// One linked movie's shapes for `apt_scene::draw`: imported shapes read their GEO units from the library that
/// defines them.
pub struct MenuScene<'a> {
    set: &'a RetailMenuMovies,
    key: &'a str,
    movie: &'a Movie,
}
impl MenuScene<'_> {
    fn origin(&self, character: i32) -> (&str, i32) {
        self.movie.shape_origins.get(&character).map_or((self.key, character), |(k, id)| (k.as_str(), *id))
    }
}
impl ShapeSource for MenuScene<'_> {
    fn shape(&self, character: i32) -> Option<&[Shape]> {
        let (key, id) = self.origin(character);
        self.set.shapes.get(key)?.get(&id).map(Vec::as_slice)
    }
    fn texture(&self, _character: i32, shape: &Shape) -> String {
        shape.texture.as_ref().map_or_else(String::new, |t| t.rgba.clone())
    }
    fn retail_unit_colour(&self) -> bool {
        true
    }
}

/// `source/...` style keys: ASCII letters, digits, `_`, `-`, `/`; no empty or dot segments.
pub fn valid_movie_name(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key.split('/').all(|s| {
            !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

#[cfg(test)]
mod memory_tests {
    fn resident_kib() -> u64 {
        let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
        statm.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0) * 4
    }

    /// Diagnostic: the memory the eager set (upstream's `read`: every movie linked, shared fields merged into each)
    /// takes on the desktop.
    #[test]
    #[ignore = "diagnostic; requires the exported set (SKATE3_MENU_MOVIES=<set>)"]
    fn eager_set_memory() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
        let before = resident_kib();
        let set = super::RetailMenuMovies::read(&root).unwrap();
        let after = resident_kib();
        eprintln!("EAGER_SET {} movies, {} failures: +{} KiB resident", set.movies.len(), set.failures.len(), after.saturating_sub(before));
    }

    /// Diagnostic: the lean set (the Vita's), whole and for the pause menu's screens.
    #[test]
    #[ignore = "diagnostic; requires the exported set (SKATE3_MENU_MOVIES=<set>)"]
    fn lean_set_memory() {
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_MENU_MOVIES").unwrap());
        let read = |path: &str| std::fs::read(root.join(path)).map_err(|e| e.to_string());
        let before = resident_kib();
        let set = super::RetailMenuMovies::read_lean(&read, None).unwrap();
        let after = resident_kib();
        eprintln!("LEAN_SET {} movies, {} failures: +{} KiB resident", set.movies.len(), set.failures.len(), after.saturating_sub(before));
        drop(set);
        let roots = ["source/screens/main/FE_root", "source/helper/screenmanager", "source/screens/main/core_menu", "source/screens/options/options", "source/screens/popup/sk8popup"];
        let before = resident_kib();
        let set = super::RetailMenuMovies::read_lean(&read, Some(&roots)).unwrap();
        let after = resident_kib();
        eprintln!("LEAN_PAUSE {} movies: +{} KiB resident; {:?}", set.movies.len(), after.saturating_sub(before), set.movies.keys().collect::<Vec<_>>());
    }
}
