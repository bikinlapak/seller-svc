//! Expiry risk (FR-15, FR-16).
//!
//! Simulate sell-through day by day against the forecast, consuming lots in
//! FEFO order. Whatever is still on a lot after its expiry date is
//! `at_risk_qty`. Each at-risk lot then gets ranked actions with the rupiah
//! each is expected to recover — numbers from here, words from the agent.

use chrono::NaiveDate;

use crate::contract::Idr;
use crate::engine::forecast::Forecast;
use crate::shop::Lot;

#[derive(Debug, Clone, PartialEq)]
pub struct LotRisk {
    pub lot_id: String,
    pub expiry: NaiveDate,
    pub qty_remaining: i64,
    pub expected_sold: f64,
    /// Whole units expected to be left on the expiry date.
    pub at_risk_qty: i64,
}

/// `lots` in FEFO order. Lots already past their date are all at risk; lots
/// without an expiry are ignored here.
pub fn simulate(lots: &[&Lot], fc: &Forecast, today: NaiveDate) -> Vec<LotRisk> {
    let mut left: Vec<f64> = lots.iter().map(|l| l.qty as f64).collect();
    let mut sold = vec![0.0; lots.len()];

    let last = lots.iter().filter_map(|l| l.expiry).max();
    if let Some(last) = last {
        let days = (last - today).num_days();
        for day in 0..=days.max(-1) {
            let date = today + chrono::Duration::days(day);
            let mut demand = fc.mean_at(day as usize);
            for (i, lot) in lots.iter().enumerate() {
                if demand <= 0.0 {
                    break;
                }
                // A lot is sellable through its expiry date, not after.
                if lot.expiry.is_some_and(|e| e < date) || left[i] <= 0.0 {
                    continue;
                }
                let n = left[i].min(demand);
                left[i] -= n;
                sold[i] += n;
                demand -= n;
            }
        }
    }

    lots.iter()
        .enumerate()
        .filter_map(|(i, lot)| {
            let expiry = lot.expiry?;
            // Round in the shop's favour: 7.6 left is 7 at risk, not 8, so we
            // never tell the owner to discount stock that will probably sell.
            let at_risk = left[i].floor() as i64;
            Some(LotRisk {
                lot_id: lot.id.clone(),
                expiry,
                qty_remaining: lot.qty,
                expected_sold: sold[i],
                at_risk_qty: at_risk.max(0),
            })
        })
        .collect()
}

/// Days until sellable stock runs out, consuming lots FEFO and dropping each
/// lot on its expiry date. Unlike plain on-hand ÷ demand, a lot that will be
/// thrown away tomorrow does not count as cover for Thursday. Capped at 365.
pub fn cover_days(lots: &[&Lot], fc: &Forecast, today: NaiveDate) -> f64 {
    let mut left: Vec<f64> = lots.iter().map(|l| l.qty as f64).collect();
    for day in 0..365i64 {
        let date = today + chrono::Duration::days(day);
        let need = fc.mean_at(day as usize);
        let sellable: f64 = lots
            .iter()
            .zip(&left)
            .filter(|(l, _)| l.expiry.is_none_or(|e| e >= date))
            .map(|(_, q)| *q)
            .sum();
        if need <= 0.0 {
            continue;
        }
        if sellable < need {
            return day as f64 + sellable / need;
        }
        let mut demand = need;
        for (i, lot) in lots.iter().enumerate() {
            if demand <= 0.0 {
                break;
            }
            if lot.expiry.is_some_and(|e| e < date) {
                continue;
            }
            let n = left[i].min(demand);
            left[i] -= n;
            demand -= n;
        }
    }
    365.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Markdown,
    Bundle,
    DailySpecial,
    Donate,
    WriteOff,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RiskAction {
    pub kind: ActionKind,
    pub label: String,
    pub expected_recovered: Idr,
}

/// Share of the at-risk quantity a 30% markdown is expected to clear.
const MARKDOWN_SELL_THROUGH: f64 = 0.7;
const MARKDOWN_PCT: i64 = 30;
const BUNDLE_SELL_THROUGH: f64 = 0.5;
/// Buy two, get one free: a third off each item that goes.
const BUNDLE_DISCOUNT: f64 = 1.0 / 3.0;

/// Ranked by rupiah recovered, highest first. Only products that are sold
/// on their own get markdown and bundle; past-date stock gets nothing that
/// sells or donates it (UU 8/1999 Art. 8).
pub fn actions(
    at_risk: i64,
    sell_price: Option<Idr>,
    cost: Idr,
    past_date: bool,
) -> Vec<RiskAction> {
    let mut out = Vec::new();
    if at_risk <= 0 {
        return out;
    }
    if !past_date {
        if let Some(price) = sell_price {
            let md = (at_risk as f64
                * MARKDOWN_SELL_THROUGH
                * price as f64
                * (100 - MARKDOWN_PCT) as f64
                / 100.0)
                .round() as Idr;
            out.push(RiskAction {
                kind: ActionKind::Markdown,
                label: format!("{MARKDOWN_PCT}% off from 17.00"),
                expected_recovered: md,
            });
            let bundle =
                (at_risk as f64 * BUNDLE_SELL_THROUGH * price as f64 * (1.0 - BUNDLE_DISCOUNT))
                    .round() as Idr;
            out.push(RiskAction {
                kind: ActionKind::Bundle,
                label: "Bundle: buy 2, get 1 free".into(),
                expected_recovered: bundle,
            });
        } else {
            // An ingredient: use it in today's special at roughly cost.
            out.push(RiskAction {
                kind: ActionKind::DailySpecial,
                label: "Use in today's special".into(),
                expected_recovered: (at_risk as f64 * cost as f64 * 0.8).round() as Idr,
            });
        }
        out.push(RiskAction {
            kind: ActionKind::Donate,
            label: "Donate to the food bank".into(),
            expected_recovered: 0,
        });
    }
    out.push(RiskAction {
        kind: ActionKind::WriteOff,
        label: "Write off".into(),
        expected_recovered: 0,
    });
    out.sort_by_key(|a| std::cmp::Reverse(a.expected_recovered));
    out
}

pub const fn markdown_pct() -> i64 {
    MARKDOWN_PCT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::forecast::Confidence;
    use chrono::Duration;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
    }

    fn fc(rate: f64) -> Forecast {
        Forecast {
            start: today(),
            mean: vec![rate; 14],
            upper: vec![rate; 14],
            confidence: Confidence::High,
            method: "test",
        }
    }

    fn lot(id: &str, qty: i64, days: i64) -> Lot {
        Lot {
            id: id.into(),
            product_id: "p".into(),
            qty,
            expiry: Some(today() + Duration::days(days)),
            received: today(),
        }
    }

    #[test]
    fn croissants_expiring_tomorrow() {
        // 14 boxes, 3 a day, sellable today and tomorrow → 6 sold, 8 at risk.
        let l = lot("L-889", 14, 1);
        let r = simulate(&[&l], &fc(3.0), today());
        assert_eq!(r[0].at_risk_qty, 8);
        assert!((r[0].expected_sold - 6.0).abs() < 1e-9);
    }

    #[test]
    fn fefo_sells_the_older_lot_first() {
        let old = lot("old", 4, 1);
        let new = lot("new", 4, 5);
        let r = simulate(&[&old, &new], &fc(2.0), today());
        assert_eq!(r[0].at_risk_qty, 0, "the old lot sells through first");
        assert_eq!(r[1].at_risk_qty, 0);
    }

    #[test]
    fn cover_does_not_count_stock_that_expires_first() {
        // 14 boxes, 3 a day, 14 expire tomorrow: sellable today and tomorrow
        // only, so cover is 2 days — not 14 ÷ 3 ≈ 4.7.
        let l = lot("L-889", 14, 1);
        assert!((cover_days(&[&l], &fc(3.0), today()) - 2.0).abs() < 1e-9);
        // Without expiry it is the plain ratio.
        let mut forever = lot("x", 6, 0);
        forever.expiry = None;
        assert!((cover_days(&[&forever], &fc(3.0), today()) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_lot_past_its_date_is_entirely_at_risk() {
        let l = lot("gone", 2, -1);
        let r = simulate(&[&l], &fc(5.0), today());
        assert_eq!(r[0].at_risk_qty, 2);
    }

    #[test]
    fn markdown_ranks_above_donation_for_sellable_stock() {
        let a = actions(8, Some(60_000), 36_000, false);
        assert_eq!(a[0].kind, ActionKind::Markdown);
        // 8 × 0.7 × 60,000 × 0.7 = 235,200
        assert_eq!(a[0].expected_recovered, 235_200);
        assert_eq!(a.last().unwrap().expected_recovered, 0);
    }

    #[test]
    fn past_date_stock_is_never_sold_or_donated() {
        let a = actions(2, Some(60_000), 36_000, true);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].kind, ActionKind::WriteOff);
    }
}
