//! Cross-movie imports of APT movies, independent of Bevy.
//!
//! [data] Each movie's root character (type 9) holds an import table of 16-byte records
//! (source movie path, symbol name, local character id, runtime slot) and an export table of
//! 8-byte records (symbol name, character id). A character id listed in the import table has
//! no local definition: it is the source movie's exported character with that name. The
//! source path is relative to `data/fe/` (`source/controls/button_item2`).
//!
//! Retail [code, TU3], doc 31 Milestone 3b:
//! - Loading: the APT load queue sub_82E7E9F8 calls the host callback in AptInitParms+36
//!   (parms 0x830CE9C8, set by sub_825D3840) = sub_82CA4138, which prefixes `data\fe\`
//!   (0x820AED74) and opens the movie through sub_82CA3428 (`/` -> `\`, `big:%s`, `.apt`,
//!   `.const`). A loaded movie is linked only once every movie it imports is available:
//!   sub_82E7E8C0 walks the import list (16-byte stride) and asks sub_82E7E6E0 for each source
//!   by name; until all are there the movie stays waiting. So sources load first, on demand,
//!   and chained imports are already linked in the source when the importer links.
//! - Linking sub_82E76070: for each import record in table order, scan the source movie's
//!   export list from the start and take the first export whose name equals the record's
//!   name byte for byte (case-sensitive); store the source's character-table entry for that
//!   export into this movie's character table at the record's character id, and give the
//!   character a reference to its source movie (record+12). No export of that name: the slot
//!   is set to null.
//! Ours follows the same order and rules ([`resolve`]); a chained import is followed to the
//! defining movie, which is what the pre-linked source table gives retail.
//!
//! A mod can supply or replace a library movie by key through [`OverrideTable`]; entries
//! are owned by the mod and removed with [`OverrideTable::clear_owner`] when it is disabled.
//!
//! NOT RETAIL YET (not decoded): what retail does when a source movie file is missing (the
//! queue entry never reaches the loaded state; whether it waits forever or fails was not
//! traced) and with import cycles (each movie would wait for the other). Ours reports both
//! as [`Failure`]s and links nothing for them. NOT RETAIL (safety): the chain-depth and
//! movie-count limits; retail movies never reach them.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Longest import chain followed (A imports B's export, which B imports from C, ...).
pub const MAX_CHAIN: usize = 16;
/// Most distinct movies one resolution loads.
pub const MAX_MOVIES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub file: String,
    pub name: String,
    pub character_id: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    pub name: String,
    pub character_id: i32,
}
/// The import-relevant part of one movie.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovieDecl {
    #[serde(default)]
    pub imports: Vec<Import>,
    #[serde(default)]
    pub exports: Vec<Export>,
    /// Locally defined character ids.
    #[serde(default)]
    pub characters: BTreeSet<i32>,
}
impl MovieDecl {
    pub fn from_json(json: &serde_json::Value) -> Result<Self, String> {
        let field = |name: &str| {
            json.get(name)
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![]))
        };
        Ok(MovieDecl {
            imports: serde_json::from_value(field("imports")).map_err(|e| e.to_string())?,
            exports: serde_json::from_value(field("exports")).map_err(|e| e.to_string())?,
            // Either plain ids or the pipeline's character objects with an `id`.
            characters: field("characters")
                .as_array()
                .ok_or("APT characters must be a list")?
                .iter()
                .filter_map(|c| c.as_i64().or_else(|| c["id"].as_i64()))
                .map(|i| i as i32)
                .collect(),
        })
    }
}

/// Stable library key for an import path: `/` separators, no `data/fe/` prefix, no extension.
pub fn library_key(file: &str) -> String {
    let path = file.replace('\\', "/");
    let path = path.trim_start_matches('/');
    let path = path.strip_prefix("data/fe/").unwrap_or(path);
    path.strip_suffix(".apt").unwrap_or(path).to_string()
}

/// The exported menu movie set for data-gated tests (`SKATE3_MENU_MOVIES`, else the main checkout's installed
/// set), or None (skip) when it is absent or older than this engine reads (re-run the hud setup group).
#[cfg(test)]
pub(crate) fn test_menu_set() -> Option<std::path::PathBuf> {
    let root = std::env::var("SKATE3_MENU_MOVIES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| main_checkout().join("assets/private/menu-movies"));
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("manifest.json")).ok()?).ok()?;
    if manifest["version"] != crate::fe::retail_menu_movies::VERSION {
        eprintln!("skipped: menu movie set in {} is version {}, not {}", root.display(), manifest["version"], crate::fe::retail_menu_movies::VERSION);
        return None;
    }
    Some(root)
}

/// The main checkout's root, also from a git worktree (`.git` is then a file pointing at
/// `<main>/.git/worktrees/<name>`), so data-gated tests find the local research data and the
/// installed assets. `SKATE3_MAIN_CHECKOUT` overrides it.
#[cfg(test)]
pub(crate) fn main_checkout() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("SKATE3_MAIN_CHECKOUT") {
        return path.into();
    }
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(here.join(".git"))
        .ok()
        .and_then(|text| {
            let dir = std::path::PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
            // <main>/.git/worktrees/<name> -> <main>
            Some(dir.parent()?.parent()?.parent()?.to_path_buf())
        })
        .unwrap_or(here)
}

/// Supplies movies by library key (user's assets, test data).
pub trait MovieSource {
    fn movie(&mut self, key: &str) -> Option<MovieDecl>;
}
impl MovieSource for BTreeMap<String, MovieDecl> {
    fn movie(&mut self, key: &str) -> Option<MovieDecl> {
        self.get(key).cloned()
    }
}

/// Library movies supplied by mods, keyed by library key; the newest owner's entry wins.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OverrideTable {
    entries: BTreeMap<String, Vec<(String, MovieDecl)>>,
}
impl OverrideTable {
    pub fn set(&mut self, owner: &str, file: &str, movie: MovieDecl) {
        let list = self.entries.entry(library_key(file)).or_default();
        list.retain(|(o, _)| o != owner);
        list.push((owner.to_string(), movie));
    }
    pub fn clear_owner(&mut self, owner: &str) {
        for list in self.entries.values_mut() {
            list.retain(|(o, _)| o != owner);
        }
        self.entries.retain(|_, l| !l.is_empty());
    }
    /// Order of `owner`'s entry for `key` (higher = newer), if it has one.
    pub fn position(&self, owner: &str, key: &str) -> Option<usize> {
        self.entries.get(key)?.iter().position(|(o, _)| o == owner)
    }
    pub fn get(&self, key: &str) -> Option<&MovieDecl> {
        self.entries.get(key).and_then(|l| l.last()).map(|(_, m)| m)
    }
    /// A source that consults the overrides before `base`.
    pub fn over<'a>(&'a self, base: &'a mut dyn MovieSource) -> Layered<'a> {
        Layered {
            overrides: self,
            base,
        }
    }
}
pub struct Layered<'a> {
    overrides: &'a OverrideTable,
    base: &'a mut dyn MovieSource,
}
impl MovieSource for Layered<'_> {
    fn movie(&mut self, key: &str) -> Option<MovieDecl> {
        self.overrides
            .get(key)
            .cloned()
            .or_else(|| self.base.movie(key))
    }
}

/// An imported character id bound to the defining movie's local character.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub movie: String,
    pub character_id: i32,
    pub source: String,
    pub source_character: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Failure {
    MissingMovie(String),
    MissingExport {
        movie: String,
        name: String,
    },
    /// The export names an id that is neither local nor imported.
    UndefinedExport {
        movie: String,
        name: String,
    },
    Cycle,
    ChainLimit,
    MovieLimit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unresolved {
    pub movie: String,
    pub character_id: i32,
    pub failure: Failure,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Resolution {
    /// Sorted by (movie, character_id).
    pub bindings: Vec<Binding>,
    pub unresolved: Vec<Unresolved>,
    /// Every movie reached, by key (the root included).
    pub movies: BTreeMap<String, MovieDecl>,
}
impl Resolution {
    pub fn binding(&self, movie: &str, character_id: i32) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.movie == movie && b.character_id == character_id)
    }
}

struct Resolver<'a> {
    source: &'a mut dyn MovieSource,
    movies: BTreeMap<String, Option<MovieDecl>>,
}
impl Resolver<'_> {
    fn load(&mut self, key: &str) -> Result<Option<&MovieDecl>, Failure> {
        if !self.movies.contains_key(key) {
            if self.movies.len() >= MAX_MOVIES {
                return Err(Failure::MovieLimit);
            }
            let movie = self.source.movie(key);
            self.movies.insert(key.to_string(), movie);
        }
        Ok(self.movies[key].as_ref())
    }
    /// Follows one import to the movie that defines the character.
    fn follow(&mut self, import: &Import) -> Result<(String, i32), Failure> {
        let mut seen = BTreeSet::new();
        let mut current = import.clone();
        for _ in 0..MAX_CHAIN {
            let key = library_key(&current.file);
            if !seen.insert((key.clone(), current.name.clone())) {
                return Err(Failure::Cycle);
            }
            let movie = self
                .load(&key)?
                .ok_or_else(|| Failure::MissingMovie(key.clone()))?;
            let export = movie
                .exports
                .iter()
                .find(|e| e.name == current.name)
                .ok_or_else(|| Failure::MissingExport {
                    movie: key.clone(),
                    name: current.name.clone(),
                })?;
            let id = export.character_id;
            if movie.characters.contains(&id) {
                return Ok((key, id));
            }
            current = movie
                .imports
                .iter()
                .find(|i| i.character_id == id)
                .cloned()
                .ok_or_else(|| Failure::UndefinedExport {
                    movie: key.clone(),
                    name: current.name.clone(),
                })?;
        }
        Err(Failure::ChainLimit)
    }
}

/// Resolves every import of `root` and of every library movie it reaches. Never panics;
/// deterministic for the same source contents.
pub fn resolve(root: &str, root_movie: MovieDecl, source: &mut dyn MovieSource) -> Resolution {
    let root = library_key(root);
    let mut r = Resolver {
        source,
        movies: BTreeMap::new(),
    };
    r.movies.insert(root.clone(), Some(root_movie));
    let mut out = Resolution::default();
    let mut queue = vec![root];
    let mut done = BTreeSet::new();
    while let Some(key) = queue.pop() {
        if !done.insert(key.clone()) {
            continue;
        }
        let Some(Some(movie)) = r.movies.get(&key).cloned() else {
            continue;
        };
        for import in &movie.imports {
            match r.follow(import) {
                Ok((source, source_character)) => out.bindings.push(Binding {
                    movie: key.clone(),
                    character_id: import.character_id,
                    source,
                    source_character,
                }),
                Err(failure) => out.unresolved.push(Unresolved {
                    movie: key.clone(),
                    character_id: import.character_id,
                    failure,
                }),
            }
            let next = library_key(&import.file);
            if !done.contains(&next) && r.movies.get(&next).is_some_and(|m| m.is_some()) {
                queue.push(next);
            }
        }
        queue.sort();
        queue.dedup();
        queue.reverse();
    }
    out.bindings
        .sort_by(|a, b| (&a.movie, a.character_id).cmp(&(&b.movie, b.character_id)));
    out.unresolved
        .sort_by(|a, b| (&a.movie, a.character_id).cmp(&(&b.movie, b.character_id)));
    out.movies = r
        .movies
        .into_iter()
        .filter_map(|(k, m)| m.map(|m| (k, m)))
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(chars: &[i32], imports: &[(&str, &str, i32)], exports: &[(&str, i32)]) -> MovieDecl {
        MovieDecl {
            characters: chars.iter().copied().collect(),
            imports: imports
                .iter()
                .map(|(f, n, c)| Import {
                    file: f.to_string(),
                    name: n.to_string(),
                    character_id: *c,
                })
                .collect(),
            exports: exports
                .iter()
                .map(|(n, c)| Export {
                    name: n.to_string(),
                    character_id: *c,
                })
                .collect(),
        }
    }

    #[test]
    fn apt_imports_resolve_through_chains() {
        let mut lib = BTreeMap::new();
        lib.insert(
            "source/controls/b".into(),
            decl(
                &[1, 2],
                &[("source/controls/c", "glow", 5)],
                &[("Button", 2), ("Glow", 5)],
            ),
        );
        lib.insert("source/controls/c".into(), decl(&[1], &[], &[("glow", 1)]));
        let root = decl(
            &[0],
            &[
                ("source/controls/b", "Button", 40),
                ("source/controls/b.apt", "Glow", 41),
            ],
            &[],
        );
        let r = resolve("data/fe/source/screens/main/a.apt", root, &mut lib);
        assert!(r.unresolved.is_empty(), "{:?}", r.unresolved);
        let b = r.binding("source/screens/main/a", 40).unwrap();
        assert_eq!(
            (b.source.as_str(), b.source_character),
            ("source/controls/b", 2)
        );
        let g = r.binding("source/screens/main/a", 41).unwrap();
        assert_eq!(
            (g.source.as_str(), g.source_character),
            ("source/controls/c", 1)
        );
        // b's own import is resolved as well.
        assert!(r.binding("source/controls/b", 5).is_some());
        assert_eq!(r.movies.len(), 3);
    }

    #[test]
    fn apt_imports_missing_movie_and_export_are_reported() {
        let mut lib = BTreeMap::new();
        lib.insert("x".into(), decl(&[1], &[], &[("a", 1), ("bad", 9)]));
        let root = decl(&[], &[("y", "a", 1), ("x", "b", 2), ("x", "bad", 3)], &[]);
        let r = resolve("root", root, &mut lib);
        assert!(r.bindings.is_empty());
        let f: Vec<_> = r.unresolved.iter().map(|u| u.failure.clone()).collect();
        assert_eq!(f[0], Failure::MissingMovie("y".into()));
        assert_eq!(
            f[1],
            Failure::MissingExport {
                movie: "x".into(),
                name: "b".into()
            }
        );
        assert_eq!(
            f[2],
            Failure::UndefinedExport {
                movie: "x".into(),
                name: "bad".into()
            }
        );
    }

    #[test]
    fn apt_imports_cycles_and_long_chains_stop() {
        let mut lib = BTreeMap::new();
        lib.insert("p".into(), decl(&[], &[("q", "s", 1)], &[("s", 1)]));
        lib.insert("q".into(), decl(&[], &[("p", "s", 1)], &[("s", 1)]));
        for i in 0..40 {
            lib.insert(
                format!("l{i}"),
                decl(&[], &[(&format!("l{}", i + 1), "s", 1)], &[("s", 1)]),
            );
        }
        let root = decl(&[], &[("p", "s", 1), ("l0", "s", 2)], &[]);
        let r = resolve("root", root, &mut lib);
        let f: Vec<_> = r
            .unresolved
            .iter()
            .filter(|u| u.movie == "root")
            .map(|u| u.failure.clone())
            .collect();
        assert_eq!(f, vec![Failure::Cycle, Failure::ChainLimit]);
        // Deterministic.
        let again = resolve(
            "root",
            decl(&[], &[("p", "s", 1), ("l0", "s", 2)], &[]),
            &mut lib,
        );
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            serde_json::to_string(&again).unwrap()
        );
    }

    #[test]
    fn apt_imports_mod_override_wins_and_reverts() {
        let mut lib = BTreeMap::new();
        lib.insert(
            "source/controls/b".into(),
            decl(&[2], &[], &[("Button", 2)]),
        );
        let mut mods = OverrideTable::default();
        mods.set(
            "my_mod",
            "data/fe/source/controls/b.apt",
            decl(&[7], &[], &[("Button", 7)]),
        );
        mods.set(
            "my_mod",
            "source/controls/new_lib",
            decl(&[3], &[], &[("Extra", 3)]),
        );
        let root = || {
            decl(
                &[],
                &[
                    ("source/controls/b", "Button", 1),
                    ("source/controls/new_lib", "Extra", 2),
                ],
                &[],
            )
        };
        let r = resolve("root", root(), &mut mods.over(&mut lib));
        assert_eq!(r.binding("root", 1).unwrap().source_character, 7);
        assert_eq!(r.binding("root", 2).unwrap().source_character, 3);
        mods.clear_owner("my_mod");
        let r = resolve("root", root(), &mut mods.over(&mut lib));
        assert_eq!(r.binding("root", 1).unwrap().source_character, 2);
        assert_eq!(r.unresolved.len(), 1);
    }

    /// Loads the decoded retail menu movies (local, never committed; see
    /// `.claude/skills/recomp-research/tools/apt_actions_json.py`) and resolves every import.
    #[test]
    fn retail_menu_movies_resolve_every_import() {
        let dir = std::env::var("SKATE3_FE_ACTIONS").unwrap_or_else(|_| {
            main_checkout()
                .join(".local/research/fe-menus/actions")
                .to_string_lossy()
                .into_owned()
        });
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipped: no decoded menu movies in {dir}");
            return;
        };
        let mut lib = BTreeMap::new();
        for entry in entries.flatten() {
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
            if json.get("imports").is_none() {
                eprintln!(
                    "skipped: {:?} predates the imports field, re-run apt_actions_json.py",
                    entry.path()
                );
                return;
            }
            let key = library_key(json["movie"].as_str().unwrap());
            lib.insert(key, MovieDecl::from_json(&json).unwrap());
        }
        let total: usize = lib.values().map(|m| m.imports.len()).sum();
        let mut resolved = 0;
        for (key, movie) in lib.clone() {
            let r = resolve(&key, movie.clone(), &mut lib);
            assert!(r.unresolved.is_empty(), "{key}: {:?}", r.unresolved);
            resolved += r.bindings.iter().filter(|b| b.movie == key).count();
        }
        eprintln!(
            "{} movies, {resolved} of {total} imports resolved",
            lib.len()
        );
        assert_eq!(resolved, total);
        assert!(total > 0);
    }
}
