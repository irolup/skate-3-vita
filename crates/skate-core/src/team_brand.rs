//! The team brand: retail's `Progression::TeamBrandManagerData` (0x40 bytes at `[[TheSim+0xE0]+0x48]`, saved raw by
//! 827CE580 and read back by 827C7770; TU3, research of 2 Nov 2026, docs/customization-survey.md). It holds the
//! career's board sales and cash: the CAC shop spends the cash, and the board-sales total opens the
//! `cac_unlock_bs_group*` clothing groups (824CF350, 824D0888, 825AF758 compare it with vault thresholds).
//!
//! Fields (offsets in the record):
//! - +0x04, +0x08: board sales a challenge awards (827CDDD8 adds both when a challenge pays out);
//! - +0x0C: board sales added by 827CEC78 (from 826FD028);
//! - +0x10: board sales set by 827CECE0 (from 825F23B0 and its twin);
//! - +0x14: cash; +0x18: cash earned in total.
//!
//! Earning (827CED48) adds to the cash and to the total, then records the running total in career stats row 62
//! (0x3E); spending (827CEE00, the CAC checkout 82604xxx) takes the price off the cash, never below 0, and the checkout
//! records the price in row 63 (0x3F). Both are skipped when the progression manager's vt+8 check fails (not read).

use crate::career_stats::CareerStats;

/// Career stats rows the cash moves write.
pub const ROW_CASH_EARNED: usize = 62;
pub const ROW_CASH_SPENT: usize = 63;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TeamBrand {
    /// Board sales at +0x04, +0x08, +0x0C, +0x10.
    pub sales: [u32; 4],
    pub cash: u32,
    pub cash_earned: u32,
}

impl TeamBrand {
    /// Boards sold: the four counters summed, as every board-sales check does.
    pub fn board_sales(&self) -> u64 {
        self.sales.iter().map(|&s| u64::from(s)).sum()
    }

    /// Earn cash (827CED48). Records row 62 with the new running total (retail's setter adds it to the row; the row's
    /// stats gate, 8250EB08, is not modelled: our stats record every row).
    pub fn earn(&mut self, amount: u32, stats: &mut CareerStats) {
        self.cash = self.cash.wrapping_add(amount);
        self.cash_earned = self.cash_earned.wrapping_add(amount);
        stats.record(ROW_CASH_EARNED, self.cash_earned as f32);
    }

    /// Spend cash (827CEE00): never below 0. The CAC checkout then records the price in row 63.
    pub fn spend(&mut self, price: u32, stats: &mut CareerStats) {
        self.cash = self.cash.saturating_sub(price);
        stats.record(ROW_CASH_SPENT, price as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earning_and_spending_follow_retail() {
        let mut stats = CareerStats::default();
        let mut brand = TeamBrand::default();
        brand.earn(100, &mut stats);
        brand.earn(50, &mut stats);
        assert_eq!((brand.cash, brand.cash_earned), (150, 150));
        // The row takes the running total each time: 100, then 150.
        assert_eq!(stats.value(ROW_CASH_EARNED), 250.0);
        brand.spend(60, &mut stats);
        assert_eq!(brand.cash, 90);
        brand.spend(500, &mut stats);
        assert_eq!(brand.cash, 0);
        assert_eq!(brand.cash_earned, 150);
        assert_eq!(stats.value(ROW_CASH_SPENT), 560.0);
    }

    #[test]
    fn board_sales_sum_the_four_counters() {
        let brand = TeamBrand { sales: [u32::MAX, 1, 2, 3], ..Default::default() };
        assert_eq!(brand.board_sales(), u64::from(u32::MAX) + 6);
    }
}
