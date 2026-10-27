//! The player's career stats file: `ux0:data/SKATEVITA/career-stats.json`, upstream's `settings/career-stats.json`
//! format (row index -> total, rows at 0 left out). The simulation thread counts (`Session::start_career_stats`); the
//! main thread writes the totals, at most every `SAVE_SECONDS` and on quit (a card write on the simulation thread
//! stalls it).
use skate_core::career_stats::CareerStats;
use std::collections::BTreeMap;

pub const PATH: &str = "ux0:data/SKATEVITA/career-stats.json";
/// How often changed totals are written (upstream's period).
pub const SAVE_SECONDS: u64 = 30;

pub fn decode(text: &str) -> CareerStats {
    let mut stats = CareerStats::default();
    if let Ok(saved) = serde_json::from_str::<BTreeMap<String, f32>>(text) {
        for (k, v) in saved {
            if let Ok(i) = k.parse::<usize>() {
                stats.record(i, v);
            }
        }
    }
    stats
}

pub fn encode(stats: &CareerStats) -> Vec<u8> {
    let map: BTreeMap<String, f32> = stats.values.iter().enumerate().filter(|(_, v)| **v != 0.0).map(|(i, v)| (i.to_string(), *v)).collect();
    serde_json::to_vec_pretty(&map).unwrap_or_default()
}

/// The saved totals (none yet, or a damaged file: zeros).
pub fn load() -> CareerStats {
    std::fs::read_to_string(PATH).map(|t| decode(&t)).unwrap_or_default()
}

pub fn save(stats: &CareerStats) {
    let _ = std::fs::create_dir_all("ux0:data/SKATEVITA");
    if let Err(e) = std::fs::write(PATH, encode(stats)) {
        println!("career stats not saved: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::career_stats::row;

    #[test]
    fn totals_survive_a_save_and_load() {
        let mut stats = CareerStats::default();
        stats.record(row::DISTANCE, 12.5);
        stats.record(row::PED_HITS + 1, 1.0);
        let again = decode(std::str::from_utf8(&encode(&stats)).unwrap());
        assert_eq!(again, stats);
        assert_eq!(decode("garbage"), CareerStats::default());
    }
}
