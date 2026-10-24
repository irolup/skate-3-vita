//! FE 52 loading screen's `TipsLoader` natives (registrar sub_825B9C58; research b581) while it is up (skate-game
//! `tips_loader_host.rs`, PR #52 `0947dea5`; doc 31 "Loading screen (FE 52)").
//!
//! skate_vita: the lines come from ui.pak's `private/tips-loader.json` (`vita-bake ui`); no Stats panel (retail's rule
//! without stats rows).
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
                let rows = self.stats.iter().take(n).cloned().collect::<Vec<_>>();
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
