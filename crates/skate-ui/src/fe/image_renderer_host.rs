//! The front end's image slot manager (`*(FUN_824AD2F0() + 0x120)`, research b258): 40 global slots the movies
//! reserve, load a named runtime image into and draw through an `ImageRenderComponent` clip (the Challenge Map
//! pictures). The `Game.ImageRenderer_*` natives live here; the draw resolves `slot/<n>` texture keys through
//! [`ImageSlots::texture`]. Mods reach the pictures through the image names (`RetailMenuMovies::set_named_image`).

use crate::fe::apt_vm::{to_integer, Value};
use crate::fe::retail_menu_movies::{image_key, NAMED_IMAGE_PREFIX};

/// Retail's slot count (b258 section 0: 40 slot pointers, 40 reserved flags).
pub const SLOTS: usize = 40;
/// Texture key prefix the draw gives an `ImageRenderComponent` quad: `slot/<n>`.
pub const SLOT_TEXTURE_PREFIX: &str = "slot/";
/// Texture key prefix of `LoadAssetByName` images: a texture another system keeps resident under that name (the
/// player's park screenshot); nothing supplies them yet, so they draw nothing.
pub const ASSET_TEXTURE_PREFIX: &str = "asset/";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slot {
    pub reserved: bool,
    /// The loaded texture key (`image/<key>` or `asset/<name>`), None when unloaded.
    pub image: Option<String>,
    /// LockAspectRatio (loader +0x1C); reset by every load.
    pub lock: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageSlots {
    pub slots: Vec<Slot>,
}

impl Default for ImageSlots {
    fn default() -> Self {
        Self { slots: vec![Slot::default(); SLOTS] }
    }
}

impl ImageSlots {
    /// One `Game.ImageRenderer_*` native; None = not ours.
    pub fn call(&mut self, object: &str, method: &str, args: &[Value]) -> Option<Value> {
        let method = method.strip_prefix("ImageRenderer_").filter(|_| object == "Game")?;
        let index = |i: usize| args.get(i).map(to_integer).filter(|n| (0..SLOTS as i64).contains(&(*n as i64))).map(|n| n as usize);
        let text = |i: usize| match args.get(i) {
            Some(Value::Text(t)) => Some(t.clone()),
            _ => None,
        };
        match method {
            // 825A5E28: the first slot not reserved, else -1.
            "ReserveNextFreeIndex" => {
                let free = self.slots.iter().position(|s| !s.reserved);
                if let Some(i) = free {
                    self.slots[i].reserved = true;
                }
                return Some(Value::Number(free.map_or(-1.0, |i| i as f64)));
            }
            // 825A5E88: clears the reserved flag only; the image stays until the next load or unload.
            "FreeImageIndex" => {
                if let Some(i) = index(0) {
                    self.slots[i].reserved = false;
                }
            }
            // 825A5D08: only with a string name; replaces the slot's image, the aspect lock resets.
            "Load" => {
                if let (Some(name), Some(i)) = (text(0), index(1)) {
                    self.slots[i].image = Some(format!("{NAMED_IMAGE_PREFIX}{}", image_key(&name)));
                    self.slots[i].lock = false;
                }
            }
            // 825A59B8: a resident texture by name; no file is requested.
            "LoadAssetByName" => {
                if let (Some(name), Some(i)) = (text(0), index(1)) {
                    self.slots[i].image = Some(format!("{ASSET_TEXTURE_PREFIX}{name}"));
                    self.slots[i].lock = false;
                }
            }
            // 825A5D80.
            "Unload" => {
                if let Some(i) = index(0) {
                    self.slots[i].image = None;
                }
            }
            // 825A5ED8: number + bool, only on a loaded slot.
            "LockAspectRatio" => {
                if let (Some(i), Some(Value::Bool(lock))) = (index(0), args.get(1)) {
                    if self.slots[i].image.is_some() {
                        self.slots[i].lock = *lock;
                    }
                }
            }
            // LoadAsset / MemcardLoad / OnlineLoad / LoadTeamLogo: online and memory card sources, not ported; the
            // slot keeps what it has (draws nothing new).
            _ => {}
        }
        Some(Value::Undefined)
    }

    /// The texture key a `slot/<n>` draw shows, None when the slot is empty.
    pub fn texture(&self, key: &str) -> Option<&str> {
        let slot: usize = key.strip_prefix(SLOT_TEXTURE_PREFIX)?.parse().ok()?;
        self.slots.get(slot)?.image.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: f64) -> Value {
        Value::Number(v)
    }

    #[test]
    fn slots_reserve_load_free_and_unload_like_retail() {
        let mut s = ImageSlots::default();
        for i in 0..SLOTS {
            assert_eq!(s.call("Game", "ImageRenderer_ReserveNextFreeIndex", &[]), Some(n(i as f64)));
        }
        assert_eq!(s.call("Game", "ImageRenderer_ReserveNextFreeIndex", &[]), Some(n(-1.0)), "all 40 reserved");
        s.call("Game", "ImageRenderer_Load", &[Value::Text("fe\\source\\images\\map\\map1024".into()), n(3.0)]);
        assert_eq!(s.texture("slot/3"), Some("image/fe/source/images/map/map1024"));
        s.call("Game", "ImageRenderer_LockAspectRatio", &[n(3.0), Value::Bool(true)]);
        assert!(s.slots[3].lock);
        s.call("Game", "ImageRenderer_Load", &[Value::Text("fe/source/images/map/map512".into()), n(3.0)]);
        assert!(!s.slots[3].lock, "a load resets the lock");
        s.call("Game", "ImageRenderer_FreeImageIndex", &[n(3.0)]);
        assert!(!s.slots[3].reserved);
        assert_eq!(s.texture("slot/3"), Some("image/fe/source/images/map/map512"), "free keeps the image");
        assert_eq!(s.call("Game", "ImageRenderer_ReserveNextFreeIndex", &[]), Some(n(3.0)));
        s.call("Game", "ImageRenderer_Unload", &[n(3.0)]);
        assert_eq!(s.texture("slot/3"), None);
        s.call("Game", "ImageRenderer_Load", &[n(5.0), n(4.0)]);
        assert_eq!(s.texture("slot/4"), None, "a number name loads nothing");
        assert_eq!(s.call("Game", "OnMenuSelect", &[]), None);
        assert_eq!(s.call("FEChallengeMap", "ImageRenderer_Load", &[]), None);
    }
}
