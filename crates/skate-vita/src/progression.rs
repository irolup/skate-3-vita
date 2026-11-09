//! The player's progression file: `ux0:data/SKATEVITA/progression.json`, our own format for what retail saves as
//! `UnlockablesBackEnd::Data` (the unlock store's granted table) and `Progression::TeamBrandManagerData` (board sales
//! and cash). The main thread writes it when it changed, at most every `career::SAVE_SECONDS` and on quit, like the
//! career stats.
use serde_json::{json, Value};
use skate_core::team_brand::TeamBrand;
use skate_core::unlocks::{Entry, UnlockKind, UnlockStore};

pub const PATH: &str = "ux0:data/SKATEVITA/progression.json";
const VERSION: u64 = 1;
/// The player's name before one is typed. NOT RETAIL: retail shows the profile's (Xbox Live) name.
pub const PLAYER: &str = "Player";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progression {
    pub unlocks: UnlockStore,
    pub brand: TeamBrand,
    /// The career team's name (retail's team record +0x360; empty until named) and the player's name on it (retail:
    /// the profile's name; the Vita's own until changed on the team screen).
    pub team_name: String,
    pub player_name: String,
}

impl Default for Progression {
    /// A fresh profile (retail 825046B0 for the store; the team brand starts at zero).
    fn default() -> Self {
        Self { unlocks: UnlockStore::fresh_profile(), brand: TeamBrand::default(), team_name: String::new(), player_name: PLAYER.to_owned() }
    }
}

pub fn encode(p: &Progression) -> Vec<u8> {
    let unlocks: Vec<Value> = p.unlocks.granted.iter().map(|e| json!([e.kind as u32, e.hash, e.new])).collect();
    let b = &p.brand;
    let doc = json!({
        "version": VERSION,
        "unlocks": unlocks,
        "team_brand": { "sales": b.sales, "cash": b.cash, "cash_earned": b.cash_earned },
        "team": { "name": p.team_name, "player": p.player_name },
    });
    serde_json::to_vec_pretty(&doc).unwrap_or_default()
}

/// A fresh profile with the saved entries loaded on top (retail 82505000); a damaged or newer file gives a fresh
/// profile.
pub fn decode(text: &str) -> Progression {
    let mut p = Progression::default();
    let Ok(doc) = serde_json::from_str::<Value>(text) else { return p };
    if doc["version"].as_u64() != Some(VERSION) {
        return p;
    }
    let saved: Vec<Entry> = doc["unlocks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let kind = UnlockKind::from_index(u32::try_from(e[0].as_u64()?).ok()?)?;
            Some(Entry { kind, hash: u32::try_from(e[1].as_u64()?).ok()?, new: e[2].as_bool()? })
        })
        .collect();
    p.unlocks.load(&saved);
    let u = |v: &Value| v.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    let brand = &doc["team_brand"];
    for (i, s) in p.brand.sales.iter_mut().enumerate() {
        *s = u(&brand["sales"][i]);
    }
    p.brand.cash = u(&brand["cash"]);
    p.brand.cash_earned = u(&brand["cash_earned"]);
    p.team_name = doc["team"]["name"].as_str().unwrap_or("").to_owned();
    p.player_name = doc["team"]["player"].as_str().filter(|n| !n.is_empty()).unwrap_or(PLAYER).to_owned();
    p
}

pub fn load() -> Progression {
    std::fs::read_to_string(PATH).map(|t| decode(&t)).unwrap_or_default()
}

pub fn save(p: &Progression) {
    let _ = std::fs::create_dir_all("ux0:data/SKATEVITA");
    if let Err(e) = std::fs::write(PATH, encode(p)) {
        println!("progression not saved: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::career_stats::CareerStats;

    #[test]
    fn progression_survives_a_save_and_load() {
        let mut p = Progression::default();
        p.unlocks.grant(UnlockKind::ClothingSponsor, "cac_shoe_emerica_promo");
        p.unlocks.grant(UnlockKind::Entitlements, "EMERICA_SHOE_UNLOCK");
        p.unlocks.clear_new(UnlockKind::Entitlements);
        p.brand.sales = [7_500, 1, 2, 3];
        p.brand.earn(250, &mut CareerStats::default());
        let again = decode(std::str::from_utf8(&encode(&p)).unwrap());
        assert_eq!(again, p);
        assert!(again.unlocks.is_new(UnlockKind::ClothingSponsor, "cac_shoe_emerica_promo"));
        assert!(!again.unlocks.is_new(UnlockKind::Entitlements, "EMERICA_SHOE_UNLOCK"));
    }

    #[test]
    fn a_damaged_file_is_a_fresh_profile() {
        assert_eq!(decode("garbage"), Progression::default());
        assert_eq!(decode(r#"{"version": 99}"#), Progression::default());
    }
}
