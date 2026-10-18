//! Run the engine over the shop: forecast, reorder plan and expiry risk per
//! product. Everything the agent says starts here.

use chrono::Duration;
use serde_json::json;

use crate::engine::expiry::{self, LotRisk};
use crate::engine::forecast::{self, Forecast};
use crate::engine::reorder::{self, REVIEW_DAYS, ReorderInput, ReorderPlan};
use crate::store::State;

pub const HORIZON: usize = 30;
/// Wasted this many times in 7 days → next suggestion drops (FR-36).
pub const WASTE_REPEAT: usize = 3;
pub const WASTE_CUT: f64 = -0.2;

#[derive(Debug, Clone)]
pub struct Analysis {
    pub product_id: String,
    pub forecast: Forecast,
    pub usable: i64,
    pub on_order: i64,
    pub cover: f64,
    pub input: Option<ReorderInput>,
    pub plan: Option<ReorderPlan>,
    pub risks: Vec<LotRisk>,
    /// Times wasted in the last 7 days.
    pub wasted_recently: usize,
}

impl Analysis {
    pub fn at_risk_total(&self) -> i64 {
        self.risks.iter().map(|r| r.at_risk_qty).sum()
    }

    /// The engine inputs behind a reorder line, stored with the suggestion.
    pub fn inputs_json(&self, supplier: &str, lead: u32) -> serde_json::Value {
        let plan = self.plan.as_ref();
        json!({
            "product_id": self.product_id,
            "on_hand": self.usable,
            "on_order": self.on_order,
            "forecast_daily": round2(self.forecast.daily_rate()),
            "forecast_method": self.forecast.method,
            "confidence": self.forecast.confidence.label(),
            "days_of_cover": round2(self.cover),
            "lead_time_days": lead,
            "review_days": REVIEW_DAYS,
            "shelf_life_days": self.input.and_then(|i| i.shelf_life_days),
            "pack_size": self.input.map(|i| i.pack_size),
            "min_order_qty": self.input.map(|i| i.min_order_qty),
            "adjust_pct": self.input.map(|i| i.adjust_pct),
            "safety_stock": plan.map(|p| round2(p.safety_stock)),
            "reorder_point": plan.map(|p| round2(p.reorder_point)),
            "raw_qty": plan.map(|p| round2(p.raw_qty)),
            "expiry_cap": plan.and_then(|p| p.cap).map(round2),
            "cap_reason": plan.map(|p| p.cap_reason.as_str()),
            "suggested_qty": plan.map(|p| p.qty),
            "supplier_id": supplier,
            "wasted_last_7_days": self.wasted_recently,
        })
    }
}

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn analyze(state: &State, product_id: &str) -> Option<Analysis> {
    let p = state.data.product(product_id)?;
    if !p.track_stock {
        return None;
    }
    let today = state.today;
    let fc = forecast::forecast(
        state.data.history(product_id),
        today,
        HORIZON,
        &state.data.closures,
    );
    let usable = state.data.usable_on_hand(product_id, today);
    let on_order = state.on_order(product_id);
    let lots = state.data.lots_of(product_id);
    // Cover counts only stock that can still be sold on each day.
    let cover = expiry::cover_days(&lots, &fc, today);

    let week_ago = today - Duration::days(7);
    let wasted_recently = state
        .waste
        .iter()
        .filter(|w| w.product_id == product_id && w.date >= week_ago)
        .count();

    let supplier = p
        .supplier_id
        .as_deref()
        .and_then(|s| state.data.supplier(s));
    let risks = expiry::simulate(&lots, &fc, today);
    // Stock expected to expire unsold cannot serve future demand, so it does
    // not count toward the reorder position (lots already past their date
    // are excluded from `usable` anyway).
    let doomed: i64 = risks
        .iter()
        .filter(|r| r.expiry >= today)
        .map(|r| r.at_risk_qty)
        .sum();

    let (input, plan) = match supplier {
        Some(s) => {
            let mut adjust = state.adjust.get(product_id).copied().unwrap_or(0.0);
            if wasted_recently >= WASTE_REPEAT {
                adjust += WASTE_CUT;
            }
            let input = ReorderInput {
                usable_on_hand: (usable - doomed).max(0) as f64,
                on_order: on_order as f64,
                lead_time_days: s.lead_time_days,
                review_days: REVIEW_DAYS,
                shelf_life_days: p.shelf_life_days,
                pack_size: p.pack_size,
                min_order_qty: p.min_order_qty,
                adjust_pct: adjust,
            };
            (Some(input), Some(reorder::plan(&input, &fc)))
        }
        None => (None, None),
    };

    Some(Analysis {
        product_id: product_id.into(),
        forecast: fc,
        usable,
        on_order,
        cover,
        input,
        plan,
        risks,
        wasted_recently,
    })
}

pub fn analyze_all(state: &State) -> Vec<Analysis> {
    state
        .data
        .products
        .iter()
        .filter_map(|p| analyze(state, &p.id))
        .collect()
}

/// Reorder lines grouped by supplier (FR-13), the order with the soonest
/// run-out first.
pub fn reorder_groups(all: &[Analysis], state: &State) -> Vec<(String, Vec<Analysis>)> {
    let mut groups: Vec<(String, Vec<Analysis>)> = Vec::new();
    for a in all {
        if !a.plan.is_some_and(|p| p.needed) {
            continue;
        }
        let Some(sid) = state
            .data
            .product(&a.product_id)
            .and_then(|p| p.supplier_id.clone())
        else {
            continue;
        };
        match groups.iter_mut().find(|(s, _)| *s == sid) {
            Some((_, v)) => v.push(a.clone()),
            None => groups.push((sid, vec![a.clone()])),
        }
    }
    let soonest = |v: &[Analysis]| v.iter().map(|a| a.cover).fold(f64::INFINITY, f64::min);
    groups.sort_by(|a, b| soonest(&a.1).total_cmp(&soonest(&b.1)));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shop::demo;
    use chrono::NaiveDate;

    fn state() -> State {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        State::new(demo::build(today), today)
    }

    #[test]
    fn the_demo_morning_has_its_stories() {
        let s = state();
        let all = analyze_all(&s);
        let get = |id: &str| all.iter().find(|a| a.product_id == id).unwrap();

        // Arabica runs short and gets an order of about 12 kg.
        let arabica = get("p3");
        assert!(arabica.cover < 3.5, "cover {}", arabica.cover);
        let q = arabica.plan.unwrap().qty;
        assert!((10..=15).contains(&q), "arabica qty {q}");

        // Croissants: a lot expiring tomorrow with more than will sell.
        let croissant = get("p2");
        assert!(croissant.at_risk_total() >= 5, "{:?}", croissant.risks);

        // Doughnuts: the expiry cap bites.
        let donut = get("p5").plan.unwrap();
        assert!(donut.cap.is_some());
        assert!(donut.qty as f64 <= donut.cap.unwrap());

        // Milk from one supplier is grouped into one order.
        let groups = reorder_groups(&all, &s);
        let susu = groups.iter().find(|(sid, _)| sid == "sup-susu");
        assert!(susu.is_some_and(|(_, v)| v.len() == 2), "{groups:?}");
    }

    #[test]
    fn repeated_waste_lowers_the_suggestion() {
        let mut s = state();
        let before = analyze(&s, "p5").unwrap().plan.unwrap().raw_qty;
        let records = demo::past_waste(&s.data, s.today);
        let mut r = records[0].clone();
        r.product_id = "p5".into();
        for i in 0..WASTE_REPEAT {
            let mut x = r.clone();
            x.id = format!("x{i}");
            x.date = s.today;
            s.waste.push(x);
        }
        let after = analyze(&s, "p5").unwrap().plan.unwrap().raw_qty;
        assert!(after < before, "{after} !< {before}");
    }
}
