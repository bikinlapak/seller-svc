//! Reorder rule (FR-10 to FR-14). Deterministic: the language model explains
//! these numbers, it never produces them.
//!
//! Policy, per product × location:
//! - **days of cover** = how long usable stock lasts against the forecast;
//! - **reorder point** = demand over (lead time + one check interval) +
//!   safety stock, where safety stock is the gap between the upper prediction
//!   bound and the point forecast over the lead time. The check interval is
//!   there because the agent looks once a day: stock that would cross the
//!   line before tomorrow's run has to be ordered today;
//! - when stock position (usable + on order) is at or below the reorder point,
//!   order up to demand over (lead time + review period) + safety stock;
//! - **expiry cap** (FR-12): never more than will sell before it expires —
//!   demand over (shelf life − lead time) from the day it arrives;
//! - round to the pack size and respect the minimum order, without ever
//!   rounding *past* the cap (FR-13).
//!
//! The cap is what turns a forecast into waste reduction, so it is checked
//! last and nothing after it may raise the quantity.

use crate::engine::forecast::Forecast;

/// Days between two orders: the shop reviews stock weekly.
pub const REVIEW_DAYS: u32 = 7;
/// Days between two runs of the agent.
pub const CHECK_DAYS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReorderInput {
    pub usable_on_hand: f64,
    pub on_order: f64,
    pub lead_time_days: u32,
    pub review_days: u32,
    pub shelf_life_days: Option<u32>,
    pub pack_size: i64,
    pub min_order_qty: i64,
    /// Learned from the owner's edits (FR-14), e.g. 0.1 for +10%. Negative
    /// values come from repeated waste (FR-36).
    pub adjust_pct: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapReason {
    None,
    /// The expiry cap cut the order.
    Expiry,
    /// The minimum order would exceed what can sell before expiry.
    MoqAboveCap,
}

impl CapReason {
    pub fn as_str(self) -> &'static str {
        match self {
            CapReason::None => "none",
            CapReason::Expiry => "expiry",
            CapReason::MoqAboveCap => "moq_above_cap",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReorderPlan {
    pub days_of_cover: f64,
    pub reorder_point: f64,
    pub safety_stock: f64,
    /// Order-up-to target minus position, before cap and rounding.
    pub raw_qty: f64,
    /// Expiry cap in stock units, if the product has a shelf life.
    pub cap: Option<f64>,
    pub cap_reason: CapReason,
    /// Final suggestion. 0 = nothing to order.
    pub qty: i64,
    pub needed: bool,
}

/// Days until usable stock runs out, walking the daily forecast. Fractional,
/// so "2.5 days" reads naturally. Capped at 365.
pub fn days_of_cover(on_hand: f64, fc: &Forecast) -> f64 {
    let mut left = on_hand;
    for i in 0..365 {
        let need = fc.mean_at(i);
        if need <= 0.0 {
            continue;
        }
        if left < need {
            return i as f64 + left / need;
        }
        left -= need;
    }
    365.0
}

pub fn plan(input: &ReorderInput, fc: &Forecast) -> ReorderPlan {
    let lead = input.lead_time_days as usize;
    let review = input.review_days as usize;

    let lead_demand = fc.demand(0, lead + CHECK_DAYS);
    let safety: f64 = (0..lead.max(1))
        .map(|i| (fc.upper_at(i) - fc.mean_at(i)).max(0.0))
        .sum();
    let reorder_point = lead_demand + safety;
    let target = fc.demand(0, lead + review) + safety;
    let position = input.usable_on_hand + input.on_order;
    let cover = days_of_cover(input.usable_on_hand, fc);

    let needed = position <= reorder_point;
    let raw_qty = if needed {
        ((target - position) * (1.0 + input.adjust_pct)).max(0.0)
    } else {
        0.0
    };

    let cap = input.shelf_life_days.map(|shelf| {
        let sellable_days = (shelf as usize).saturating_sub(lead);
        fc.demand(lead, sellable_days)
    });

    let mut cap_reason = CapReason::None;
    let mut want = raw_qty;
    if let Some(c) = cap
        && want > c
    {
        want = c;
        cap_reason = CapReason::Expiry;
    }

    let qty = if want <= 0.0 {
        0
    } else {
        round_to_pack(want, input.pack_size, cap)
    };

    // The minimum order: raise to it only if that still fits under the cap.
    let qty = if qty > 0 && qty < input.min_order_qty {
        let raised = round_up(input.min_order_qty, input.pack_size);
        match cap {
            Some(c) if raised as f64 > c + 1e-9 => {
                cap_reason = CapReason::MoqAboveCap;
                0
            }
            _ => raised,
        }
    } else {
        qty
    };

    ReorderPlan {
        days_of_cover: cover,
        reorder_point,
        safety_stock: safety,
        raw_qty,
        cap,
        cap_reason,
        qty,
        needed: needed && qty > 0,
    }
}

/// Round up to whole packs, unless that would cross the cap — then round
/// down. The cap wins over convenience.
fn round_to_pack(qty: f64, pack: i64, cap: Option<f64>) -> i64 {
    let pack = pack.max(1);
    let up = round_up(qty.ceil() as i64, pack);
    match cap {
        Some(c) if up as f64 > c + 1e-9 => (c.floor() as i64 / pack) * pack,
        _ => up,
    }
}

fn round_up(qty: i64, pack: i64) -> i64 {
    let pack = pack.max(1);
    (qty + pack - 1) / pack * pack
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::forecast::Confidence;
    use chrono::NaiveDate;

    fn flat(rate: f64, band: f64) -> Forecast {
        Forecast {
            start: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            mean: vec![rate; 14],
            upper: vec![rate + band; 14],
            confidence: Confidence::High,
            method: "test",
        }
    }

    fn input(on_hand: f64) -> ReorderInput {
        ReorderInput {
            usable_on_hand: on_hand,
            on_order: 0.0,
            lead_time_days: 2,
            review_days: REVIEW_DAYS,
            shelf_life_days: None,
            pack_size: 1,
            min_order_qty: 1,
            adjust_pct: 0.0,
        }
    }

    #[test]
    fn cover_is_fractional_days() {
        assert!((days_of_cover(4.0, &flat(1.6, 0.0)) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn orders_up_to_lead_plus_review_plus_safety() {
        // 4 on hand, 1.6/day, band 0.4: ROP = 1.6 × 3 + 0.8 = 5.6 → reorder.
        // Target = 1.6 × 9 + 0.8 = 15.2; need 11.2 → 12.
        let p = plan(&input(4.0), &flat(1.6, 0.4));
        assert!(p.needed);
        assert_eq!(p.qty, 12);
        assert_eq!(p.cap_reason, CapReason::None);
    }

    #[test]
    fn no_order_above_the_reorder_point() {
        let p = plan(&input(20.0), &flat(1.6, 0.4));
        assert!(!p.needed);
        assert_eq!(p.qty, 0);
    }

    #[test]
    fn stock_on_order_counts_toward_position() {
        let mut i = input(4.0);
        i.on_order = 12.0;
        assert!(!plan(&i, &flat(1.6, 0.4)).needed);
    }

    #[test]
    fn expiry_cap_limits_the_order() {
        // Doughnuts: 8/day, 2-day shelf life, 1-day lead → sellable 1 day → cap 8.
        let mut i = input(4.0);
        i.lead_time_days = 1;
        i.shelf_life_days = Some(2);
        let p = plan(&i, &flat(8.0, 2.0));
        assert_eq!(p.cap, Some(8.0));
        assert_eq!(p.qty, 8);
        assert_eq!(p.cap_reason, CapReason::Expiry);
    }

    #[test]
    fn pack_rounding_never_crosses_the_cap() {
        // Cap 8, pack 6: rounding 8 up to 12 would cross it → 6.
        let mut i = input(4.0);
        i.lead_time_days = 1;
        i.shelf_life_days = Some(2);
        i.pack_size = 6;
        let p = plan(&i, &flat(8.0, 2.0));
        assert_eq!(p.qty, 6);
        assert!(p.qty as f64 <= p.cap.unwrap());
    }

    #[test]
    fn moq_above_the_cap_means_no_order() {
        let mut i = input(0.5);
        i.lead_time_days = 1;
        i.shelf_life_days = Some(2);
        i.min_order_qty = 6;
        let p = plan(&i, &flat(1.0, 0.2));
        assert_eq!(p.qty, 0);
        assert_eq!(p.cap_reason, CapReason::MoqAboveCap);
        assert!(!p.needed);
    }

    #[test]
    fn learned_buffer_raises_the_order_but_not_past_the_cap() {
        let mut i = input(4.0);
        i.adjust_pct = 0.1;
        assert_eq!(plan(&i, &flat(1.6, 0.4)).qty, 13);

        i.shelf_life_days = Some(9);
        // Cap = 1.6 × 7 = 11.2 → 11, buffer or not.
        assert_eq!(plan(&i, &flat(1.6, 0.4)).qty, 11);
    }

    /// The property the PRD cares about most, over a grid of inputs: the final
    /// quantity never exceeds the expiry cap.
    #[test]
    fn never_exceeds_the_cap_across_a_grid() {
        for rate in [0.5, 1.0, 2.6, 8.0, 20.0] {
            for shelf in [1, 2, 3, 7, 30] {
                for lead in [0, 1, 2, 4] {
                    for pack in [1, 4, 6, 12] {
                        for moq in [1, 6, 24] {
                            let i = ReorderInput {
                                usable_on_hand: 0.0,
                                on_order: 0.0,
                                lead_time_days: lead,
                                review_days: REVIEW_DAYS,
                                shelf_life_days: Some(shelf),
                                pack_size: pack,
                                min_order_qty: moq,
                                adjust_pct: 0.2,
                            };
                            let p = plan(&i, &flat(rate, rate * 0.3));
                            assert!(
                                p.qty as f64 <= p.cap.unwrap() + 1e-9,
                                "rate {rate} shelf {shelf} lead {lead} pack {pack} moq {moq}: {p:?}"
                            );
                            assert_eq!(p.qty % pack.max(1), 0, "not a whole pack: {p:?}");
                        }
                    }
                }
            }
        }
    }
}
