//! Stock count from a photo (stock opname by camera).
//!
//! Gemini counts what is on the shelf; everything after that is the engine:
//! the variance against the system, how long the counted stock lasts against
//! the forecast, and the order it implies (same reorder rule, expiry cap and
//! pack rounding as the morning run). The merchant confirms the counts, which
//! adjusts stock and refreshes the reorder suggestions for those products.

use std::collections::BTreeMap;

use chrono::Duration;

use crate::AppError;
use crate::agent::analysis::analyze;
use crate::agent::{agent_text, sign, vision};
use crate::clock;
use crate::contract::*;
use crate::engine::reorder::{self, days_of_cover};
use crate::fmt::{qty, trim};
use crate::store::{State, WasteSession};
use crate::token::ActionKeys;

/// One product's numbers at a given counted quantity. Used when the card is
/// drawn and again when the count is confirmed.
pub fn count_line(state: &State, product_id: &str, counted: i64) -> Option<CountLine> {
    let p = state.data.product(product_id)?;
    let a = analyze(state, product_id)?;
    let system = state.data.total_on_hand(product_id);
    let cover = days_of_cover(counted as f64, &a.forecast);
    let until = clock::relative_day(
        state.today + Duration::days(cover.floor() as i64),
        state.today,
    );
    let plan = a.input.map(|mut i| {
        i.usable_on_hand = counted as f64;
        reorder::plan(&i, &a.forecast)
    });
    let diff = counted - system;
    let difference_label = match diff {
        0 => "matches the system".to_owned(),
        d if d < 0 => format!("{} fewer than the system", qty(-d, &p.unit)),
        d => format!("{} more than the system", qty(d, &p.unit)),
    };
    let (need_label, severity) = match plan {
        Some(pl) if pl.needed => (
            format!("Order {} · runs out {until}", qty(pl.qty, &p.unit)),
            Severity::Critical,
        ),
        None => (format!("Made in house · lasts until {until}"), Severity::Ok),
        Some(_) if cover < 7.0 => (
            format!("Enough for now · lasts until {until}"),
            Severity::Warn,
        ),
        Some(_) => (format!("Enough · lasts until {until}"), Severity::Ok),
    };
    Some(CountLine {
        product_id: p.id.clone(),
        name: p.name.clone(),
        unit: p.unit.clone(),
        counted,
        system,
        difference_label,
        cover_label: format!(
            "{} days at {} {} a day",
            trim(cover.min(99.0), 1),
            trim(a.forecast.daily_rate(), 1),
            crate::fmt::plural(&p.unit, 2)
        ),
        need_label,
        severity,
    })
}

/// A grounded photo becomes a count session, the photo, and the count card —
/// or a plain answer saying what was (not) recognised.
pub fn post_count_session(
    state: &mut State,
    keys: &ActionKeys,
    photo_id: &str,
    detections: &[vision::Detection],
) -> Result<String, AppError> {
    let candidates = vision::candidates(state, vision::Purpose::Stock);
    let session_id = state.next_id("sc");
    let mut n = 0;
    let items = vision::ground(
        detections,
        &candidates,
        state.today,
        vision::Purpose::Stock,
        || {
            n += 1;
            format!("{session_id}:{n}")
        },
    );

    // One line per product: the model may report a product twice.
    let mut counted: BTreeMap<String, i64> = BTreeMap::new();
    let mut unsure = Vec::new();
    let mut unmatched = Vec::new();
    for it in &items {
        match &it.product_id {
            Some(pid) => *counted.entry(pid.clone()).or_default() += it.qty,
            None if !it.candidates.is_empty() => unsure.push(UnsureCount {
                item_id: it.item_id.clone(),
                label: it.label.clone(),
                quantity: it.qty,
                candidates: it
                    .candidates
                    .iter()
                    .filter_map(|c| {
                        state.data.product(c).map(|p| Candidate {
                            id: p.id.clone(),
                            name: p.name.clone(),
                        })
                    })
                    .collect(),
            }),
            None => unmatched.push(format!("{} ×{}", it.label, it.qty)),
        }
    }
    let lines: Vec<CountLine> = counted
        .iter()
        .filter_map(|(pid, n)| count_line(state, pid, *n))
        .collect();

    let photo_msg = state.next_id("m");
    state.push(
        "stock",
        Message::Photo {
            id: photo_msg,
            author: Author::Staff,
            time: clock::hhmm(),
            photo_label: "Stock photo".into(),
            photo_url: Some(format!("/photos/{photo_id}")),
            boxes: detections
                .iter()
                .flat_map(|d| {
                    let confident =
                        d.confidence >= vision::CONFIDENCE_THRESHOLD && d.candidate_id.is_some();
                    d.boxes().into_iter().map(move |b| DetectionBox {
                        x: b[0],
                        y: b[1],
                        w: b[2],
                        h: b[3],
                        confident,
                    })
                })
                .collect(),
        },
    );

    if lines.is_empty() && unsure.is_empty() {
        let id = state.next_id("m");
        let text = if unmatched.is_empty() {
            "I couldn't see any products in this photo. Try again closer, with the shelf or \
             fridge filling the frame and the labels facing the camera."
                .to_owned()
        } else {
            format!(
                "I could see {}, but none of it matches a product in your catalog, so there's \
                 nothing to count. Add it as a product in worksh.io first.",
                unmatched.join(", ")
            )
        };
        state.push("stock", agent_text(id, text));
        return Ok(session_id);
    }

    let token = sign(state, keys, "stock_count", &session_id, 1, "count")?;
    let card_msg = state.next_id("m");
    state.sessions.insert(
        session_id.clone(),
        WasteSession {
            id: session_id.clone(),
            photo_id: photo_id.into(),
            message_id: card_msg.clone(),
            items,
            state: CardState::Open,
        },
    );
    let total = lines.len() + unsure.len();
    state.push(
        "stock",
        Message::CountCard {
            id: card_msg,
            author: Author::Agent,
            time: clock::hhmm(),
            card: CountCard {
                label: format!(
                    "Stock count · {total} {}",
                    crate::fmt::plural("product", total as i64)
                ),
                lines,
                unsure,
                unmatched,
                note: "Check each count, then confirm: stock is adjusted and orders recalculated."
                    .into(),
                token,
                state: CardState::Open,
            },
        },
    );
    Ok(session_id)
}
