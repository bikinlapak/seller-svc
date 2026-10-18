//! Words for the engine's numbers.
//!
//! **Template stand-in for Gemini explanations (FR-17).** Every sentence is
//! built from stored inputs only, so it cannot invent a number — the property
//! the PRD asks of the model, guaranteed here by construction. Swapping in
//! Gemini changes the phrasing, not the figures it is given.

use chrono::Duration;

use crate::agent::analysis::Analysis;
use crate::clock::{relative_day, weekday_name};
use crate::engine::expiry::{LotRisk, RiskAction};
use crate::engine::reorder::CapReason;
use crate::engine::special::Special;
use crate::fmt::{idr, idr_short, plural, qty, trim};
use crate::shop::{Product, Supplier};
use crate::store::State;

pub fn run_out_label(a: &Analysis, state: &State) -> String {
    if a.cover >= 60.0 {
        return "plenty in stock".into();
    }
    let day = state.today + Duration::days(a.cover.floor() as i64);
    format!("out by {}", relative_day(day, state.today))
}

/// "4 kg on hand, selling 1.6 kg a day, out by Thursday. Sumber Kopi delivers
/// in 2 days, arriving Wednesday."
pub fn reorder_line(a: &Analysis, p: &Product, s: &Supplier, state: &State) -> String {
    let rate = a.forecast.daily_rate();
    let mut out = format!(
        "{} on hand, using {} {} a day, {}.",
        qty(a.usable, &p.unit),
        trim(rate, 1),
        plural(&p.unit, 2),
        run_out_label(a, state),
    );
    if a.on_order > 0 {
        out.push_str(&format!(" {} already on order.", qty(a.on_order, &p.unit)));
    }
    if let Some(plan) = a.plan {
        match plan.cap_reason {
            CapReason::Expiry => out.push_str(&format!(
                " Capped at {}: with a {}-day shelf life, more would expire before it sells.",
                qty(plan.qty, &p.unit),
                p.shelf_life_days.unwrap_or_default()
            )),
            CapReason::MoqAboveCap => {}
            CapReason::None => {}
        }
    }
    if a.wasted_recently >= crate::agent::analysis::WASTE_REPEAT {
        out.push_str(&format!(
            " Wasted {} times this week, so I suggested 20% less.",
            a.wasted_recently
        ));
    }
    out.push(' ');
    out.push_str(&delivery_line(s, state));
    out
}

pub fn delivery_line(s: &Supplier, state: &State) -> String {
    let arrive = state.today + Duration::days(i64::from(s.lead_time_days));
    format!(
        "{} delivers in {} {}, arriving {}.",
        s.name,
        s.lead_time_days,
        if s.lead_time_days == 1 { "day" } else { "days" },
        relative_day(arrive, state.today)
    )
}

/// Q&A "why": the arithmetic, from stored inputs.
pub fn why_reorder(a: &Analysis, p: &Product, s: &Supplier) -> String {
    let Some(plan) = a.plan else {
        return format!("{} doesn't need an order right now.", p.name);
    };
    let days = s.lead_time_days + crate::engine::reorder::REVIEW_DAYS;
    let demand = a.forecast.demand(0, days as usize);
    // The position the rule used: stock expiring unsold does not count.
    let sellable = a.input.map_or(a.usable, |i| i.usable_on_hand as i64);
    let mut out = format!(
        "{} covers the next {days} days ({} until delivery + {} until the next order): about {} {} of demand at {} a day, plus {} safety stock for a busy day, minus the {} you have",
        qty(plan.qty, &p.unit),
        s.lead_time_days,
        crate::engine::reorder::REVIEW_DAYS,
        trim(demand, 1),
        plural(&p.unit, 2),
        trim(a.forecast.daily_rate(), 1),
        trim(plan.safety_stock, 1),
        qty(sellable, &p.unit),
    );
    if sellable < a.usable {
        out.push_str(&format!(
            " that will sell (the other {} expire first)",
            qty(a.usable - sellable, &p.unit)
        ));
    }
    if a.on_order > 0 {
        out.push_str(&format!(" and the {} on order", qty(a.on_order, &p.unit)));
    }
    out.push('.');
    if p.pack_size > 1 {
        out.push_str(&format!(" Rounded to packs of {}.", p.pack_size));
    }
    if plan.cap_reason == CapReason::Expiry {
        out.push_str(&format!(
            " The expiry cap held it at {}: that's all that sells before it goes off.",
            qty(plan.qty, &p.unit)
        ));
    }
    out.push_str(&format!(
        " Forecast: {} ({}).",
        a.forecast.method,
        a.forecast.confidence.label().to_lowercase()
    ));
    out
}

/// "14 boxes expire tomorrow; about 6 will sell at full price. 30% off from
/// 17.00 should recover ~Rp235k; bundling with a latte ~Rp168k."
pub fn expiry_reason(p: &Product, r: &LotRisk, actions: &[RiskAction], state: &State) -> String {
    let mut out = format!(
        "{} expire {}; about {} will sell at full price.",
        qty(r.qty_remaining, &p.unit),
        relative_day(r.expiry, state.today),
        trim(r.expected_sold.floor(), 0),
    );
    let ranked: Vec<String> = actions
        .iter()
        .filter(|a| a.expected_recovered > 0)
        .enumerate()
        .map(|(i, a)| {
            let label = if i == 0 {
                a.label.clone()
            } else {
                lower_first(&a.label)
            };
            format!("{label} recovers ~{}", idr_short(a.expected_recovered))
        })
        .collect();
    if !ranked.is_empty() {
        out.push(' ');
        out.push_str(&ranked.join("; "));
        out.push('.');
    }
    out
}

pub fn special_reason(s: &Special, state: &State) -> String {
    let uses: Vec<String> = s
        .uses
        .iter()
        .filter_map(|(pid, n)| {
            let p = state.data.product(pid)?;
            Some(format!("{} ({})", p.name, qty(*n, &p.unit)))
        })
        .collect();
    format!(
        "Uses {}. {} at {} each recovers about {}. {} Check taste and food safety before it goes on the menu.",
        uses.join(", "),
        qty(s.portions, "portion"),
        idr(s.price),
        idr(s.expected_recovered),
        s.method,
    )
}

/// "Thursday" for a run-out N days from today.
pub fn run_out_day(a: &Analysis, state: &State) -> String {
    let day = state.today + Duration::days(a.cover.floor() as i64);
    weekday_name(day).into()
}

/// "3 L usable across 1 lot (2 L past its date). Lasts until Thursday."
pub fn stock_answer(a: &Analysis, p: &Product, state: &State) -> String {
    let lots = state.data.lots_of(&p.id);
    let expired: i64 = lots
        .iter()
        .filter(|l| l.expiry.is_some_and(|e| e < state.today))
        .map(|l| l.qty)
        .sum();
    let usable_lots = lots.len()
        - lots
            .iter()
            .filter(|l| l.expiry.is_some_and(|e| e < state.today))
            .count();
    let mut out = format!(
        "{}: {} usable across {} {}",
        p.name,
        qty(a.usable, &p.unit),
        usable_lots,
        plural("lot", usable_lots as i64)
    );
    if expired > 0 {
        out.push_str(&format!(" ({} past its date)", qty(expired, &p.unit)));
    }
    out.push_str(&format!(
        ". At about {} {} a day that's {}.",
        trim(a.forecast.daily_rate(), 1),
        plural(&p.unit, 2),
        run_out_label(a, state)
    ));
    if a.on_order > 0 {
        out.push_str(&format!(" {} on order.", qty(a.on_order, &p.unit)));
    }
    out
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
