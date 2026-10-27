//! skate_vita addition (`std::fs` behind a trait): file reads of the
//! asset loaders go through here. Files registered with [`mount`] (the contents of
//! the baked `sim.pak` / `anim.pak` on the Vita) are served from memory; any other
//! path falls through to `std::fs`, so with nothing mounted every loader behaves
//! exactly as before (errors included: an absent file is still `NotFound`).
use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// A mounted file: a range of a shared buffer (a whole file, or one file of a pack kept in one
/// piece), or a range of a pack file on the card, read when a loader asks for it (skate_vita:
/// the Vita keeps sim.pak, anim.pak and ui.pak on the card instead of 38 MB of RAM).
#[derive(Clone)]
enum Entry {
    Memory(Arc<[u8]>, Range<usize>),
    Disk(Arc<PathBuf>, u64, usize),
}

static FILES: RwLock<Option<HashMap<PathBuf, Entry>>> = RwLock::new(None);
/// With strict on, unmounted paths are `NotFound` instead of read from disk: a run
/// from packs then proves the packs hold every file the loaders read.
static STRICT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_strict(on: bool) {
    STRICT.store(on, std::sync::atomic::Ordering::Relaxed);
}
fn strict() -> bool {
    STRICT.load(std::sync::atomic::Ordering::Relaxed)
}
fn not_found(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{} is not in the mounted packs", path.display()))
}

fn insert(path: PathBuf, entry: Entry) {
    FILES.write().unwrap().get_or_insert_with(HashMap::new).insert(path, entry);
}

/// Registers `bytes` as the content of `path` (replacing an earlier registration).
pub fn mount(path: impl Into<PathBuf>, bytes: Arc<[u8]>) {
    let range = 0..bytes.len();
    mount_range(path, bytes, range);
}

/// Registers `bytes[range]` as the content of `path` (skate_vita retail audio: the files of
/// sound.pak stay in the pack's one buffer, so 3000 files are not 3000 allocations).
pub fn mount_range(path: impl Into<PathBuf>, bytes: Arc<[u8]>, range: Range<usize>) {
    assert!(range.end <= bytes.len());
    insert(path.into(), Entry::Memory(bytes, range));
}

/// Registers `len` bytes at `offset` of the file `pack` as the content of `path`: read from
/// there on every access.
pub fn mount_disk(path: impl Into<PathBuf>, pack: Arc<PathBuf>, offset: u64, len: usize) {
    insert(path.into(), Entry::Disk(pack, offset, len));
}

/// Where a file mounted with [`mount_disk`] lives: its pack file, the offset in it and its length
/// (skate_vita retail audio: the ambience beds stream from there).
pub fn disk_location(path: &Path) -> Option<(Arc<PathBuf>, u64, usize)> {
    match lookup(path)? {
        Entry::Disk(pack, offset, len) => Some((pack, offset, len)),
        Entry::Memory(..) => None,
    }
}

/// Forgets one registered file (its buffer is freed with its last user).
pub fn unmount(path: &Path) {
    if let Some(files) = FILES.write().unwrap().as_mut() {
        files.remove(path);
    }
}

/// Forgets every registered file.
pub fn unmount_all() {
    *FILES.write().unwrap() = None;
}

fn lookup(path: &Path) -> Option<Entry> {
    FILES.read().unwrap().as_ref()?.get(path).cloned()
}

fn read_disk(pack: &Path, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let _foreground = crate::io_gate::foreground();
    let started = std::time::Instant::now();
    let mut file = std::fs::File::open(pack)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes)?;
    let source = pack.file_name().map_or(std::borrow::Cow::Borrowed("?"), |n| n.to_string_lossy());
    crate::io_gate::account(&source, len, started.elapsed().as_micros() as u64);
    Ok(bytes)
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match lookup(path) {
        Some(Entry::Memory(bytes, range)) => Ok(bytes[range].to_vec()),
        Some(Entry::Disk(pack, offset, len)) => read_disk(&pack, offset, len),
        None if strict() => Err(not_found(path)),
        None => std::fs::read(path),
    }
}

/// [`read`] without a copy for a file in memory: the buffer and the file's range in it
/// (skate_vita retail audio: the samples keep it).
pub fn read_shared(path: &Path) -> io::Result<(Arc<[u8]>, Range<usize>)> {
    let whole = |v: Vec<u8>| {
        let len = v.len();
        (Arc::from(v), 0..len)
    };
    match lookup(path) {
        Some(Entry::Memory(bytes, range)) => Ok((bytes, range)),
        Some(Entry::Disk(pack, offset, len)) => read_disk(&pack, offset, len).map(whole),
        None if strict() => Err(not_found(path)),
        None => std::fs::read(path).map(whole),
    }
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    if lookup(path).is_none() {
        return if strict() { Err(not_found(path)) } else { std::fs::read_to_string(path) };
    }
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn is_file(path: &Path) -> bool {
    lookup(path).is_some() || (!strict() && path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounted_files_shadow_the_disk_and_others_fall_through() {
        let path = Path::new("vfs-test:/assets/private/game.json");
        assert_eq!(read(path).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(!is_file(path));
        mount(path, Arc::from(&b"{}"[..]));
        assert_eq!(read(path).unwrap(), b"{}");
        assert_eq!(read_to_string(path).unwrap(), "{}");
        assert!(is_file(path));
        let cargo = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(read(&cargo).is_ok() && is_file(&cargo));
        // A file on disk by offset reads the same bytes.
        let on_disk = Path::new("vfs-test:/assets/private/disk.toml");
        mount_disk(on_disk, Arc::new(cargo.clone()), 2, 5);
        assert_eq!(read(on_disk).unwrap(), std::fs::read(&cargo).unwrap()[2..7]);
        assert_eq!(read_shared(on_disk).unwrap().0.len(), 5);
    }
}
