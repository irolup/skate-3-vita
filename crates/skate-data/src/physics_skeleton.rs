//! Lossless stock physics-bone records. Core consumes typed values derived here.
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsBone {
    pub name: String,
    pub source_offset: u64,
    pub words: [u32; 28],
}
impl PhysicsBone {
    pub fn size(&self) -> [f32; 3] {
        [self.words[8], self.words[9], self.words[10]].map(f32::from_bits)
    }
    pub fn rotation(&self) -> [f32; 4] {
        [self.words[12], self.words[13], self.words[14], self.words[15]].map(f32::from_bits)
    }
    pub fn translation(&self) -> [f32; 3] {
        [self.words[16], self.words[17], self.words[18]].map(f32::from_bits)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsSkeleton {
    pub name: String,
    pub source_offset: u64,
    pub bones: Vec<PhysicsBone>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    source_sha256: String,
    skeletons: Vec<PhysicsSkeleton>,
}

/// skate_vita: the binary form of `physics-skeletons.json` (see `crate::binary`).
const BINARY_KIND: &[u8; 4] = b"PSKL";

impl File {
    fn encode(&self) -> Vec<u8> {
        let mut w = crate::binary::Writer::new(BINARY_KIND);
        w.u32(self.version);
        w.str(&self.source_sha256);
        w.u32(self.skeletons.len() as u32);
        for s in &self.skeletons {
            w.str(&s.name);
            w.u64(s.source_offset);
            w.u32(s.bones.len() as u32);
            for b in &s.bones {
                w.str(&b.name);
                w.u64(b.source_offset);
                for word in b.words {
                    w.u32(word);
                }
            }
        }
        w.bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut r = crate::binary::Reader::new(bytes, BINARY_KIND)?;
        let version = r.u32()?;
        let source_sha256 = r.str()?;
        let mut skeletons = Vec::new();
        for _ in 0..r.u32()? {
            let name = r.str()?;
            let source_offset = r.u64()?;
            let mut bones = Vec::new();
            for _ in 0..r.u32()? {
                let name = r.str()?;
                let source_offset = r.u64()?;
                let mut words = [0u32; 28];
                for word in &mut words {
                    *word = r.u32()?;
                }
                bones.push(PhysicsBone { name, source_offset, words });
            }
            skeletons.push(PhysicsSkeleton { name, source_offset, bones });
        }
        r.done()?;
        Ok(Self { version, source_sha256, skeletons })
    }
}

/// skate_vita: the binary form of a `physics-skeletons.json` file, for `sim.pak`.
pub fn encode_json(json: &[u8]) -> Result<Vec<u8>, String> {
    let file: File = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    Ok(file.encode())
}

impl PhysicsSkeleton {
    pub fn load(path: &Path, bank_sha256: &str, name: &str) -> Result<Self, String> {
        let bytes = crate::vfs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        // skate_vita: the baked binary form (sim.pak) or the JSON; same values.
        let file: File = if crate::binary::kind(&bytes).is_some() {
            File::decode(&bytes)?
        } else {
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?
        };
        if file.version != 1 || file.source_sha256 != bank_sha256 {
            return Err("Physics skeleton bank identity or format differs from animation data".into());
        }
        let mut identities = BTreeSet::new();
        if file.skeletons.iter().any(|s| !identities.insert(&s.name)) {
            return Err("Converted physics skeleton has unresolved duplicate names".into());
        }
        let selected = file.skeletons.into_iter().find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("Required stock physics skeleton {name} is absent"))?;
        let mut names = BTreeSet::new();
        if selected.bones.is_empty() || selected.bones.len() > 255 {
            return Err("Invalid stock physical bone count".into());
        }
        for bone in &selected.bones {
            if !names.insert(&bone.name) || bone.size().iter().chain(&bone.rotation()).chain(&bone.translation()).any(|v| !v.is_finite()) {
                return Err(format!("Invalid or repeated stock physics bone {}", bone.name));
            }
        }
        Ok(selected)
    }
}
