//! skate_vita addition: IMA ADPCM channels that stay in a file on the card and
//! are read ahead of the playhead, for the ambience beds (22 beds, 137 MB: not resident).
//!
//! A [`StreamSource`] is one channel's blocks at an offset of a file. Its window holds the chunk
//! the reader is in and the two after it (chunks wrap: the beds loop); a prefetch thread keeps the
//! window filled, so the mixer never waits on the card: a block whose chunk is not resident reads
//! as silence (an underrun, counted). A new source loads its first chunks at once, on the opening
//! thread.
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use crate::adpcm::{BLOCK_BYTES, BLOCK_FRAMES, Samples, decode_block_bytes};

/// Blocks per chunk: 36 KiB of ADPCM, 65,536 frames (1.4 s at 48 kHz).
pub const CHUNK_BLOCKS: usize = 1024;
/// Chunks kept resident per channel: the playhead's and the next two.
const AHEAD: usize = 3;

/// One channel of an ADPCM file on the card.
pub struct StreamSource {
    path: PathBuf,
    /// Where the channel's blocks start in the file.
    offset: u64,
    pub frames: usize,
    chunks: usize,
    window: Mutex<Vec<(usize, Arc<[u8]>)>>,
    /// The chunk the reader asked for last.
    playhead: AtomicUsize,
    /// Blocks asked for while their chunk was not resident.
    underruns: AtomicU32,
}

impl std::fmt::Debug for StreamSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StreamSource({}, {} frames)", self.path.display(), self.frames)
    }
}

type Init = fn();
static THREAD_INIT: OnceLock<Init> = OnceLock::new();
static POOL: Mutex<Vec<Weak<StreamSource>>> = Mutex::new(Vec::new());
static UNDERRUNS: AtomicU32 = AtomicU32::new(0);

/// Blocks of every stream that read as silence because the card was too slow (diagnostics).
pub fn underruns_total() -> u32 {
    UNDERRUNS.load(Ordering::Relaxed)
}
static THREAD: OnceLock<()> = OnceLock::new();

/// Runs on the prefetch thread when it starts (the Vita pins it and lowers its priority).
pub fn set_thread_init(init: Init) {
    let _ = THREAD_INIT.set(init);
}

impl StreamSource {
    /// The channel of `frames` frames whose blocks start at `offset` of `path`: its first chunks
    /// are read now, and the prefetch thread starts keeping it filled.
    pub fn open(path: &Path, offset: u64, frames: usize) -> Result<Arc<Self>, String> {
        let chunks = frames.div_ceil(BLOCK_FRAMES).div_ceil(CHUNK_BLOCKS).max(1);
        let source = Arc::new(Self {
            path: path.to_owned(),
            offset,
            frames,
            chunks,
            window: Mutex::new(Vec::new()),
            playhead: AtomicUsize::new(0),
            underruns: AtomicU32::new(0),
        });
        source.fill()?;
        let mut pool = POOL.lock().map_err(|_| "stream pool poisoned")?;
        pool.retain(|w| w.strong_count() > 0);
        pool.push(Arc::downgrade(&source));
        drop(pool);
        THREAD.get_or_init(|| {
            let spawned = std::thread::Builder::new().name("stream-prefetch".into()).stack_size(128 * 1024).spawn(|| {
                if let Some(init) = THREAD_INIT.get() {
                    init();
                }
                loop {
                    std::thread::sleep(Duration::from_millis(20));
                    let sources: Vec<Arc<StreamSource>> = match POOL.lock() {
                        Ok(mut pool) => {
                            pool.retain(|w| w.strong_count() > 0);
                            pool.iter().filter_map(Weak::upgrade).collect()
                        }
                        Err(_) => return,
                    };
                    for s in sources {
                        let _ = s.fill();
                    }
                }
            });
            if spawned.is_err() {
                eprintln!("stream: the prefetch thread did not start");
            }
        });
        Ok(source)
    }

    /// Chunks to keep: the playhead's and the next ones, wrapping.
    fn wanted(&self) -> Vec<usize> {
        let p = self.playhead.load(Ordering::Relaxed) % self.chunks;
        let mut w: Vec<usize> = (0..AHEAD.min(self.chunks)).map(|k| (p + k) % self.chunks).collect();
        w.dedup();
        w
    }

    /// Reads the wanted chunks that are not resident and drops the others.
    fn fill(&self) -> Result<(), String> {
        let wanted = self.wanted();
        let missing: Vec<usize> = {
            let window = self.window.lock().map_err(|_| "stream window poisoned")?;
            wanted.iter().copied().filter(|c| !window.iter().any(|(r, _)| r == c)).collect()
        };
        let mut loaded = Vec::with_capacity(missing.len());
        for c in missing {
            loaded.push((c, self.read_chunk(c)?));
        }
        let mut window = self.window.lock().map_err(|_| "stream window poisoned")?;
        window.retain(|(c, _)| wanted.contains(c));
        window.extend(loaded);
        Ok(())
    }

    fn read_chunk(&self, c: usize) -> Result<Arc<[u8]>, String> {
        let blocks = self.frames.div_ceil(BLOCK_FRAMES);
        let first = c * CHUNK_BLOCKS;
        let n = CHUNK_BLOCKS.min(blocks - first);
        let mut bytes = vec![0u8; n * BLOCK_BYTES];
        let err = |e: std::io::Error| format!("{}: {e}", self.path.display());
        let mut file = std::fs::File::open(&self.path).map_err(err)?;
        file.seek(SeekFrom::Start(self.offset + (first * BLOCK_BYTES) as u64)).map_err(err)?;
        file.read_exact(&mut bytes).map_err(err)?;
        Ok(bytes.into())
    }

    /// The resident chunk `c`.
    fn chunk(&self, c: usize) -> Option<Arc<[u8]>> {
        self.playhead.store(c, Ordering::Relaxed);
        let window = self.window.lock().ok()?;
        window.iter().find(|(r, _)| *r == c).map(|(_, b)| b.clone())
    }

    /// Blocks that read as silence because the card was too slow.
    pub fn underruns(&self) -> u32 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Decodes block `b` into `out` through the reader's cached chunk; false (silence) when the
    /// chunk is not resident.
    pub(crate) fn decode(&self, b: usize, cache: &mut Option<(usize, Arc<[u8]>)>, out: &mut [f32; BLOCK_FRAMES]) -> bool {
        let c = b / CHUNK_BLOCKS;
        if cache.as_ref().is_none_or(|(cc, _)| *cc != c) {
            *cache = self.chunk(c).map(|bytes| (c, bytes));
        }
        let Some((_, bytes)) = cache else {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            UNDERRUNS.fetch_add(1, Ordering::Relaxed);
            *out = [0.0; BLOCK_FRAMES];
            return false;
        };
        let at = (b % CHUNK_BLOCKS) * BLOCK_BYTES;
        match bytes.get(at..at + BLOCK_BYTES) {
            Some(block) => {
                decode_block_bytes(block, out);
                true
            }
            None => {
                *out = [0.0; BLOCK_FRAMES];
                false
            }
        }
    }
}

/// The streaming channels of the `.ima` file of `len` bytes at `offset` of `path`, as a sample
/// ([`crate::adpcm::pcm_from_ima`] without reading the blocks).
pub fn pcm_from_ima_file(path: &Path, offset: u64, len: usize) -> Result<crate::mixer::Pcm, String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(err)?;
    file.seek(SeekFrom::Start(offset)).map_err(err)?;
    let mut head = [0u8; 20];
    file.read_exact(&mut head).map_err(err)?;
    let word = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
    if &head[..4] != b"SKIM" || word(4) != 1 {
        return Err("not an .ima file of this version".into());
    }
    let (rate, count, frames) = (word(8), word(12) as usize, word(16) as usize);
    let mut at = 20usize;
    let mut channels = Vec::with_capacity(count);
    for _ in 0..count {
        let mut w = [0u8; 4];
        file.seek(SeekFrom::Start(offset + at as u64)).map_err(err)?;
        file.read_exact(&mut w).map_err(err)?;
        let bytes = u32::from_le_bytes(w) as usize;
        if bytes != frames.div_ceil(BLOCK_FRAMES) * BLOCK_BYTES || at + 4 + bytes > len {
            return Err("an .ima channel's size does not match its frames".into());
        }
        channels.push(Samples::Stream(StreamSource::open(path, offset + at as u64 + 4, frames)?));
        at += 4 + bytes;
    }
    Ok(crate::mixer::Pcm { rate, channels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(n: usize) -> Vec<i16> {
        (0..n).map(|i| ((i as f32 * 0.02).sin() * 12000.0) as i16).collect()
    }

    /// A stream reads the same samples as the in-memory ADPCM, through chunk changes and the loop
    /// wrap, from an `.ima` that sits at an offset of a larger file.
    #[test]
    fn a_stream_decodes_like_the_in_memory_samples() {
        let frames = CHUNK_BLOCKS * BLOCK_FRAMES * 2 + 5000;
        let (left, right) = (wave(frames), wave(frames).iter().map(|x| x / 2).collect::<Vec<_>>());
        let ima = crate::adpcm::to_ima(48_000, &[left, right]);
        let path = std::env::temp_dir().join(format!("skate-stream-test-{}.bin", std::process::id()));
        let mut file = vec![7u8; 1000];
        file.extend_from_slice(&ima);
        file.extend_from_slice(&[9u8; 300]);
        std::fs::write(&path, &file).unwrap();
        let streamed = pcm_from_ima_file(&path, 1000, ima.len()).unwrap();
        let memory = crate::adpcm::pcm_from_ima(&ima).unwrap();
        assert_eq!((streamed.rate, streamed.channels.len()), (48_000, 2));
        for (s, m) in streamed.channels.iter().zip(&memory.channels) {
            assert_eq!(s.len(), m.len());
            let (mut rs, mut rm) = (s.reader(), m.reader());
            // Two laps (the second wraps to chunk 0); the prefetch thread keeps up between laps.
            for lap in 0..2 {
                for i in 0..frames {
                    assert_eq!(rs.get(i), rm.get(i), "lap {lap} frame {i}");
                    if i % 20_000 == 0 {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                }
            }
            let Samples::Stream(source) = s else { panic!("not a stream") };
            assert_eq!(source.underruns(), 0);
        }
        let _ = std::fs::remove_file(path);
    }
}
