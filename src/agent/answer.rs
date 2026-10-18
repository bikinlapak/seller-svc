//! Q&A in a thread (FR-20, FR-31).
//!
//! Grounded by construction: each answer is computed from the suggestion's
//! stored inputs or the engine, never generated. Recognised:
//!
//! - "why 12?" / "kenapa" → the arithmetic behind the open suggestion;
//! - "what if I order 8" / a bare number → how long that lasts, plus a button;
//! - "we're closed two days next week" / "libur 2 hari" → the closure is saved
//!   for future forecasts and the order is recomputed;
//! - "how much oat milk is left?" → stock, lots and run-out day;
//! - anything else → what the assistant can help with (it stays in scope:
//!   stock and procurement only, decision L28).

use chrono::{Datelike, Duration, NaiveDate};

use crate::AppError;
use crate::agent::analysis::{Analysis, analyze};
use crate::agent::{agent_text, explain, sign};
use crate::contract::{Author, CardState, InlineAction, Message};
use crate::engine::reorder::days_of_cover;
use crate::fmt::{qty, trim};
use crate::shop::Product;
use crate::store::{State, SuggestionKind};
use crate::token::ActionKeys;

/// Append the merchant's message and the assistant's answer to `topic`.
pub fn reply(
    state: &mut State,
    keys: &ActionKeys,
    topic: &str,
    text: &str,
) -> Result<(), AppError> {
    let id = state.next_id("m");
    state.push(
        topic,
        Message::Text {
            id,
            author: Author::Staff,
            time: crate::clock::hhmm(),
            text: text.into(),
            actions: None,
        },
    );
    let answer = answer(state, keys, topic, text)?;
    state.push(topic, answer);
    Ok(())
}

fn answer(
    state: &mut State,
    keys: &ActionKeys,
    topic: &str,
    text: &str,
) -> Result<Message, AppError> {
    let lower = text.to_lowercase();
    let id = state.next_id("m");

    if let Some(days) = closure_days(&lower) {
        return closure(state, keys, topic, &lower, days, id);
    }

    // Product questions work from any thread.
    if is_stock_question(&lower)
        && let Some(p) = find_product(state, &lower).cloned()
    {
        let msg = match analyze(state, &p.id) {
            Some(a) => explain::stock_answer(&a, &p, state),
            None => format!("{} is made to order, so there's no stock to count.", p.name),
        };
        return Ok(agent_text(id, msg));
    }

    let open = open_reorder(state, topic);

    if is_why(&lower) {
        if let Some((sug_id, product_id)) = &open {
            let _ = sug_id;
            if let (Some(a), Some(p)) = (analyze(state, product_id), state.data.product(product_id))
                && let Some(s) = p
                    .supplier_id
                    .as_deref()
                    .and_then(|s| state.data.supplier(s))
            {
                return Ok(agent_text(id, explain::why_reorder(&a, p, s)));
            }
        }
        if let Some(why) = why_other(state, topic) {
            return Ok(agent_text(id, why));
        }
    }

    if let Some(n) = first_number(&lower)
        && let Some((sug_id, product_id)) = open
    {
        return what_if(state, keys, &sug_id, &product_id, n, id);
    }

    Ok(agent_text(
        id,
        "I can help with stock, orders, expiring items and leftovers. Try \"why 12?\", \"what if I order 8?\", \"we're closed 2 days next week\" or \"how much oat milk is left?\"",
    ))
}

/// The single-product reorder suggestion open in this thread, if any.
fn open_reorder(state: &State, topic: &str) -> Option<(String, String)> {
    let mut open: Vec<_> = state
        .suggestions
        .values()
        .filter(|s| s.topic == topic && s.state == CardState::Open)
        .collect();
    open.sort_by(|a, b| a.id.cmp(&b.id));
    open.into_iter().find_map(|s| match &s.kind {
        SuggestionKind::Reorder { lines, .. } if lines.len() == 1 => {
            Some((s.id.clone(), lines[0].product_id.clone()))
        }
        _ => None,
    })
}

fn why_other(state: &State, topic: &str) -> Option<String> {
    let s = state
        .suggestions
        .values()
        .find(|s| s.topic == topic && s.state == CardState::Open)?;
    match &s.kind {
        SuggestionKind::Markdown {
            product_id,
            qty: n,
            pct,
            ..
        } => {
            let p = state.data.product(product_id)?;
            Some(format!(
                "The forecast sells about {} {} a day, so {} of this lot will still be on the shelf when it expires. {pct}% off in the evening is the cheapest way to clear them; past the date they can only go to compost.",
                trim(analyze(state, product_id)?.forecast.daily_rate(), 1),
                crate::fmt::plural(&p.unit, 2),
                qty(*n, &p.unit)
            ))
        }
        SuggestionKind::Special(sp) => Some(format!(
            "{} uses stock that won't sell before its date. The portions are limited by what's on hand and by what a day can sell (at most {}).",
            sp.name,
            crate::engine::special::MAX_PORTIONS
        )),
        SuggestionKind::Reorder { .. } => None,
    }
}

fn what_if(
    state: &mut State,
    keys: &ActionKeys,
    sug_id: &str,
    product_id: &str,
    n: i64,
    id: String,
) -> Result<Message, AppError> {
    let (Some(a), Some(p)) = (
        analyze(state, product_id),
        state.data.product(product_id).cloned(),
    ) else {
        return Ok(agent_text(id, "I couldn't find that product any more."));
    };
    let cap = suggested_qty(state, sug_id).unwrap_or(0);
    if n < 1 {
        return Ok(agent_text(id, "An order needs at least 1."));
    }
    let lasts = days_of_cover((a.usable + a.on_order + n) as f64, &a.forecast);
    let until = state.today + Duration::days(lasts.floor() as i64);
    let mut text = format!(
        "{} plus the {} you have lasts about {} days, until {}.",
        qty(n, &p.unit),
        qty(a.usable, &p.unit),
        trim(lasts, 1),
        crate::clock::relative_day(until, state.today),
    );
    if n > cap {
        text.push_str(&format!(
            " That's more than I suggested; the most I can approve here is {}.",
            qty(cap, &p.unit)
        ));
        return Ok(agent_text(id, text));
    }
    let token = sign(state, keys, "create_po", sug_id, n, &p.unit)?;
    Ok(Message::Text {
        id,
        author: Author::Agent,
        time: crate::clock::hhmm(),
        text,
        actions: Some(vec![InlineAction {
            label: format!("Order {}", qty(n, &p.unit)),
            token: Some(token),
            primary: true,
            quantity: Some(n),
            unit: Some(p.unit.clone()),
        }]),
    })
}

fn suggested_qty(state: &State, sug_id: &str) -> Option<i64> {
    match &state.suggestions.get(sug_id)?.kind {
        SuggestionKind::Reorder { lines, .. } => Some(lines.iter().map(|l| l.qty).sum()),
        _ => None,
    }
}

fn closure(
    state: &mut State,
    keys: &ActionKeys,
    topic: &str,
    lower: &str,
    days: i64,
    id: String,
) -> Result<Message, AppError> {
    let start = if lower.contains("next week") || lower.contains("minggu depan") {
        next_monday(state.today)
    } else {
        state.today + Duration::days(1)
    };
    let dates: Vec<NaiveDate> = (0..days).map(|i| start + Duration::days(i)).collect();
    for d in &dates {
        if !state.data.closures.contains(d) {
            state.data.closures.push(*d);
        }
    }
    let span = format!(
        "{} to {}",
        crate::clock::relative_day(dates[0], state.today),
        crate::clock::relative_day(*dates.last().unwrap_or(&start), state.today)
    );
    state.emit(
        "passtok.calendar.override",
        serde_json::json!({ "closed": dates.iter().map(|d| d.to_string()).collect::<Vec<_>>() }),
    );

    let Some((sug_id, product_id)) = open_reorder(state, topic) else {
        return Ok(agent_text(
            id,
            format!("Noted: closed {span}. Future forecasts will expect no sales on those days."),
        ));
    };
    let old = suggested_qty(state, &sug_id).unwrap_or(0);
    let (Some(a), Some(p)) = (
        analyze(state, &product_id),
        state.data.product(&product_id).cloned(),
    ) else {
        return Ok(agent_text(id, "Noted the closure."));
    };
    let new = a.plan.map_or(0, |pl| pl.qty).min(old);
    let text = closure_text(&a, &p, old, new, &span, state);
    if new == old || new == 0 {
        return Ok(agent_text(id, text));
    }
    let lower_token = sign(state, keys, "create_po", &sug_id, new, &p.unit)?;
    let keep_token = sign(state, keys, "create_po", &sug_id, old, &p.unit)?;
    Ok(Message::Text {
        id,
        author: Author::Agent,
        time: crate::clock::hhmm(),
        text,
        actions: Some(vec![
            InlineAction {
                label: format!("Order {}", qty(new, &p.unit)),
                token: Some(lower_token),
                primary: true,
                quantity: Some(new),
                unit: Some(p.unit.clone()),
            },
            InlineAction {
                label: format!("Keep {}", qty(old, &p.unit)),
                token: Some(keep_token),
                primary: false,
                quantity: Some(old),
                unit: Some(p.unit.clone()),
            },
        ]),
    })
}

fn closure_text(
    a: &Analysis,
    p: &Product,
    old: i64,
    new: i64,
    span: &str,
    state: &State,
) -> String {
    if new == 0 {
        // With the closure the stock lasts past the next check: no order is
        // due today. Say so, rather than "0 kg is enough".
        return format!(
            "Closed {span}: the {} you have now lasts {}, so this order can wait. I've saved the closure and will suggest it again when it's due. You can still approve the card if you'd rather order now.",
            qty(a.usable, &p.unit),
            explain::run_out_label(a, state).replace("out by", "until")
        );
    }
    if new >= old {
        return format!(
            "Noted: closed {span}. {} still holds — those days fall after this order runs out.",
            qty(old, &p.unit)
        );
    }
    format!(
        "{} covered a normal week plus a buffer. Closed {span}, {} is enough at about {} {} a day. I've saved the closure for future forecasts.",
        qty(old, &p.unit),
        qty(new, &p.unit),
        trim(a.forecast.daily_rate(), 1),
        crate::fmt::plural(&p.unit, 2)
    )
}

fn next_monday(today: NaiveDate) -> NaiveDate {
    let ahead = 7 - i64::from(today.weekday().num_days_from_monday());
    today + Duration::days(ahead)
}

fn is_why(s: &str) -> bool {
    s.contains("why") || s.contains("kenapa") || s.contains("how come") || s.contains("mengapa")
}

fn is_stock_question(s: &str) -> bool {
    [
        "left", "stock", "sisa", "how much", "how many", "berapa", "on hand",
    ]
    .iter()
    .any(|k| s.contains(k))
}

const NUMBER_WORDS: [(&str, i64); 20] = [
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("satu", 1),
    ("dua", 2),
    ("tiga", 3),
    ("empat", 4),
    ("lima", 5),
    ("enam", 6),
    ("tujuh", 7),
    ("delapan", 8),
    ("sembilan", 9),
    ("sepuluh", 10),
];

/// First number in the text, digits or a small number word.
pub fn first_number(s: &str) -> Option<i64> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .find_map(|w| {
            w.parse::<i64>()
                .ok()
                .or_else(|| NUMBER_WORDS.iter().find(|(k, _)| *k == w).map(|(_, v)| *v))
        })
}

/// "closed two days" / "libur 2 hari" / "tutup 3 hari" → number of days.
/// The number is the one right before "day(s)"/"hari", so "why 12? we're
/// closed two days" reads 2, not 12.
pub fn closure_days(s: &str) -> Option<i64> {
    let closed = ["closed", "closing", "libur", "tutup", "off for"]
        .iter()
        .any(|k| s.contains(k));
    if !closed {
        return None;
    }
    let words: Vec<&str> = s
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words
        .windows(2)
        .find(|w| w[1].starts_with("day") || w[1] == "hari")
        .and_then(|w| first_number(w[0]))
        .filter(|n| (1..=14).contains(n))
}

fn find_product<'a>(state: &'a State, s: &str) -> Option<&'a Product> {
    // Score by how many words of the product name appear in the question.
    state
        .data
        .products
        .iter()
        .map(|p| {
            let name = p.name.to_lowercase();
            let score = name
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 2 && s.contains(w))
                .count();
            (score, p)
        })
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, _)| *score)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_closures_in_both_languages() {
        assert_eq!(
            closure_days("why 12? we're closed two days next week"),
            Some(2)
        );
        assert_eq!(closure_days("minggu depan libur 2 hari"), Some(2));
        assert_eq!(closure_days("tutup tiga hari"), Some(3));
        assert_eq!(closure_days("why 12?"), None);
    }

    #[test]
    fn finds_the_first_number() {
        assert_eq!(first_number("what if I order 8?"), Some(8));
        assert_eq!(first_number("order eight kg"), Some(8));
        assert_eq!(first_number("no number here"), None);
    }

    #[test]
    fn next_monday_is_in_the_future() {
        let mon = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        assert_eq!(
            next_monday(mon),
            NaiveDate::from_ymd_opt(2026, 10, 19).unwrap()
        );
        let sat = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        assert_eq!(
            next_monday(sat),
            NaiveDate::from_ymd_opt(2026, 10, 19).unwrap()
        );
    }
}
