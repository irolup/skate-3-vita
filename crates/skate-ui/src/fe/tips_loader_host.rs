//! FE 52 loading screen's `TipsLoader` natives (registrar sub_825B9C58; research b581) while it is up (skate-game
//! `tips_loader_host.rs`, PR #52 `0947dea5`; doc 31 "Loading screen (FE 52)").
//!
//! skate_vita: the lines come from ui.pak's `private/tips-loader.json` (`vita-bake ui`); the Stats panel's rows from the
//! game's career stats ([`stat_rows`], upstream `CareerStatsHost::panel_rows`, `8df32c51`); no mods' lines.
use crate::fe::apt_vm::{Value, Vm};
use skate_core::tips_loader::{Panel, TipsLoader};

#[derive(Clone, Default)]
pub struct TipsLoaderHost {
    pub tips: TipsLoader,
    /// The `tips_loader/default` lines (string ids).
    pub lines: skate_data::tips_loader::TipsLines,
    /// Stats rows (label, value) for the Stats panel; empty = no Stats panel (retail's rule without stats).
    pub stats: Vec<(String, String)>,
    /// The tips RNG (T+244; retail's seed is open): a per-boot LCG.
    pub seed: u32,
    /// FE 52 is up.
    pub open: bool,
}

/// One draw of the tips RNG.
pub fn lcg(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    *seed >> 16
}

/// The ped type names by type (`ID_LIVINGWORLD_<TYPE>`), bum first (upstream `career_stats_host.rs`).
const TYPE_NAMES: [&str; 14] = [
    "BUM", "JOCK", "TEENGIRL", "TEENGUY", "ADULTMALE", "ADULTFEMALE", "BUSINESSMAN", "BUSINESSWOMAN", "MALESKATER",
    "FEMALESKATER", "MALETOURIST", "FEMALETOURIST", "SECURITYGUARD", "GRANNY",
];

/// The Stats panel rows (label id, value text): the offline fetch's 16 at most (sub_82607840), empty without the
/// table (menu tables `career_stats`).
pub fn stat_rows(
    stats: &skate_core::career_stats::CareerStats,
    table: &[skate_data::menu_tables::StatRow],
    language: &std::collections::BTreeMap<String, String>,
    numbers: &crate::fe::fe_language::NumberFormat,
    imperial: bool,
    rng: &mut dyn FnMut() -> u32,
) -> Vec<(String, String)> {
    let template = |id: &str| crate::fe::fe_language::template(language, id).to_owned();
    let (metres, feet, seconds) = (template("ID_COMMON_METRIC_DISTANCE"), template("ID_COMMON_METRIC_NEW_FEET"), template("ID_COMMON_METRIC_TIME"));
    let int = |n: i64| numbers.int(n);
    // 0x8290E370: the whole part grouped like integers.
    let float = |x: f64, d: usize| {
        let text = numbers.float(x, d);
        let (whole, rest) = text.split_once(numbers.decimal).map_or((text.as_str(), None), |(w, r)| (w, Some(r)));
        let grouped = numbers.int(whole.parse::<i64>().unwrap_or(0));
        let grouped = if whole.starts_with('-') && !grouped.starts_with('-') { format!("-{grouped}") } else { grouped };
        rest.map_or(grouped.clone(), |r| format!("{grouped}{}{r}", numbers.decimal))
    };
    // NOT RETAIL YET: retail's locale currency format (0x8290DCB8); money rows are disabled in the table.
    let money = |x: f64| format!("${}", numbers.int(x as i64));
    let ped_type = |t: usize| TYPE_NAMES.get(t).map(|n| format!("ID_LIVINGWORLD_{n}")).unwrap_or_default();
    let format = skate_core::career_stats::Format { imperial, metres: &metres, feet: &feet, seconds: &seconds, int: &int, float: &float, money: &money, ped_type: &ped_type };
    let enabled: Vec<bool> = table.iter().map(|r| r.enabled).collect();
    stats.fetch(&enabled, rng).into_iter().filter_map(|i| Some((table.get(i)?.label.clone(), stats.format(i, &format)))).collect()
}

impl TipsLoaderHost {
    /// The panels that can show offline: Coach Frank and the hint (with their lines), Stats with rows.
    pub fn available(&self) -> [bool; 6] {
        [!self.lines.coach.is_empty(), !self.stats.is_empty(), !self.lines.hints.is_empty(), false, false, false]
    }

    pub fn call(&mut self, vm: &mut Vm, object: &str, method: &str, args: &[Value]) -> Result<Option<Value>, String> {
        if !self.open || object != "TipsLoader" {
            return Ok(None);
        }
        let enabled = |p: Panel| Value::Bool(self.tips.shown() == Some(p));
        Ok(Some(match method {
            "IsCoachFrankEnabled" => enabled(Panel::CoachFrank),
            "IsStatsEnabled" => enabled(Panel::Stats),
            "IsHintEnabled" => enabled(Panel::Hint),
            "IsLeaderboardEnabled" => enabled(Panel::Leaderboard),
            "IsFeedEnabled" => enabled(Panel::Feed),
            "IsReelEnabled" => enabled(Panel::Reel),
            "GetCoachFrankText" => {
                let mut seed = self.seed;
                let i = self.tips.coach_line(self.lines.coach.len(), &mut || lcg(&mut seed));
                self.seed = seed;
                Value::Text(self.lines.coach.get(i).cloned().unwrap_or_default())
            }
            "GetHint" => {
                let mut seed = self.seed;
                let i = self.tips.hint_line(self.lines.hints.len(), &mut || lcg(&mut seed));
                self.seed = seed;
                let (title, text) = self.lines.hints.get(i).cloned().unwrap_or_default();
                Value::Object(vm.array(vec![Value::Text(title), Value::Text(text)])?)
            }
            // 825BA018: up to n rows of [label, value].
            "GetStats" => {
                let n = args.first().map_or(0, crate::fe::apt_vm::to_integer).max(0) as usize;
                // Shuffled again on every call (sub_826060F0), then up to n rows.
                let mut order: Vec<usize> = (0..self.stats.len()).collect();
                let mut seed = self.seed;
                skate_core::career_stats::shuffle(&mut order, &mut || lcg(&mut seed));
                self.seed = seed;
                let rows = order.into_iter().take(n).map(|i| self.stats[i].clone()).collect::<Vec<_>>();
                let mut out = Vec::new();
                for (label, value) in rows {
                    out.push(Value::Object(vm.array(vec![Value::Text(label), Value::Text(value)])?));
                }
                Value::Object(vm.array(out)?)
            }
            // Online panels: never enabled offline; empty answers.
            "GetLeaderboard" | "GetFeedItems" => Value::Object(vm.array(Vec::new())?),
            "GetReelItem" => Value::Undefined,
            _ => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::career_stats::{row, CareerStats, ROWS};

    #[test]
    fn stat_rows_need_the_table_and_format_by_kind() {
        let mut stats = CareerStats::default();
        stats.record(row::DISTANCE, 1234.5);
        let language = std::collections::BTreeMap::from([("ID_COMMON_METRIC_DISTANCE".to_owned(), "m".to_owned())]);
        let numbers = crate::fe::fe_language::NumberFormat::default();
        assert!(stat_rows(&stats, &[], &language, &numbers, false, &mut || 0).is_empty(), "no table: no Stats panel");
        let table: Vec<_> = (0..ROWS).map(|i| skate_data::menu_tables::StatRow { label: format!("ID_RECORD_{i}"), enabled: true }).collect();
        assert_eq!(stat_rows(&stats, &table, &language, &numbers, false, &mut || 0), [("ID_RECORD_7".to_owned(), "1,234.0 m".to_owned())]);
    }
}
