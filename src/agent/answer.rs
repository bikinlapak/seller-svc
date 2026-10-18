//! Q&A in a thread (FR-20, FR-31).
//!
//! Grounded by construction: each answer is computed from the suggestion's
//! stored inputs or the engine, never generated. Recognised:
//!
//! - "why 12?" / "kenapa" → the arithmetic behind the open suggestion;
//! - "what if I order 8" / a bare number → how long that lasts, plus a button;
//! - "we're closed two days next week" / "libur 2 hari" → the closure is saved
//!   for future forecasts and the order is recomputed;
//! - "how much butter is left?" → stock, lots and run-out day;
//! - anything else → what the assistant can help with (it stays in scope:
//!   stock and procurement only, decision L28).
//!
//! Understanding and answering are separate steps. [`Intent`] is what the
//! merchant asked; it comes from Gemini when configured ([`llm_intent`]) and
//! from keyword rules otherwise ([`rules`]). [`answer_intent`] then computes
//! the answer from the engine — the same code whichever understood the
//! question, so the model can misread a question but cannot change a number.
//! [`localise`] optionally rewords the computed answer into the merchant's
//! language, and is rejected if it introduces any number.

use std::sync::Arc;

use chrono::{Datelike, Duration, NaiveDate};
use serde::Deserialize;
use serde_json::json;

use crate::AppError;
use crate::agent::analysis::{Analysis, analyze};
use crate::agent::{agent_text, explain, sign};
use crate::contract::{Author, CardState, InlineAction, Message};
use crate::engine::reorder::days_of_cover;
use crate::fmt::{qty, trim};
use crate::shop::Product;
use crate::store::{State, SuggestionKind};
use crate::token::ActionKeys;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    Why,
    WhatIf,
    Closure,
    Stock,
    Other,
}

/// What the merchant asked, with the slots the engine needs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Intent {
    pub kind: IntentKind,
    #[serde(default)]
    pub quantity: Option<i64>,
    #[serde(default)]
    pub closure_days: Option<i64>,
    #[serde(default)]
    pub next_week: bool,
    /// Product words as the merchant wrote them ("butter").
    #[serde(default)]
    pub product: Option<String>,
}

/// Keyword understanding: the fallback, and the only path without Gemini.
pub fn rules(text: &str) -> Intent {
    let lower = text.to_lowercase();
    let next_week = lower.contains("next week") || lower.contains("minggu depan");
    let base = |kind| Intent {
        kind,
        quantity: None,
        closure_days: None,
        next_week,
        product: None,
    };
    if let Some(days) = closure_days(&lower) {
        return Intent {
            closure_days: Some(days),
            ..base(IntentKind::Closure)
        };
    }
    if is_stock_question(&lower) {
        return Intent {
            product: Some(lower.clone()),
            ..base(IntentKind::Stock)
        };
    }
    if is_why(&lower) {
        return base(IntentKind::Why);
    }
    if let Some(n) = first_number(&lower) {
        return Intent {
            quantity: Some(n),
            ..base(IntentKind::WhatIf)
        };
    }
    base(IntentKind::Other)
}

/// Push the merchant's message. The answer follows separately, because
/// understanding it may take a model call made outside the state lock.
pub fn push_staff(state: &mut State, topic: &str, text: &str) {
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
}

/// Rules only, in one step — what the keyword path and tests use.
pub fn reply(
    state: &mut State,
    keys: &ActionKeys,
    topic: &str,
    text: &str,
) -> Result<(), AppError> {
    push_staff(state, topic, text);
    let answer = answer_intent(state, keys, topic, &rules(text))?;
    state.push(topic, answer);
    Ok(())
}

/// Compute the answer to an understood question. Every figure comes from the
/// engine or the stored suggestion inputs.
pub fn answer_intent(
    state: &mut State,
    keys: &ActionKeys,
    topic: &str,
    intent: &Intent,
) -> Result<Message, AppError> {
    let id = state.next_id("m");

    if intent.kind == IntentKind::Closure
        && let Some(days) = intent.closure_days.filter(|d| (1..=14).contains(d))
    {
        return closure(state, keys, topic, intent.next_week, days, id);
    }

    // Product questions work from any thread.
    let product_words = intent.product.clone().unwrap_or_default().to_lowercase();
    if intent.kind == IntentKind::Stock
        && let Some(p) = find_product(state, &product_words).cloned()
    {
        let msg = match analyze(state, &p.id) {
            Some(a) => explain::stock_answer(&a, &p, state),
            None => format!("{} is made to order, so there's no stock to count.", p.name),
        };
        return Ok(agent_text(id, msg));
    }

    let open = open_reorder(state, topic);

    if intent.kind == IntentKind::Why {
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

    if intent.kind == IntentKind::WhatIf
        && let Some(n) = intent.quantity
        && let Some((sug_id, product_id)) = open
    {
        return what_if(state, keys, &sug_id, &product_id, n, id);
    }

    Ok(agent_text(
        id,
        "I can help with stock, orders, expiring bakes and leftovers. Try \"why so much flour?\", \"what if I order 100?\", \"we're closed 2 days next week\" or \"how much butter is left?\"",
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
    next_week: bool,
    days: i64,
    id: String,
) -> Result<Message, AppError> {
    let start = if next_week {
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
    // "tomorrow and Tuesday", "Monday to Wednesday": unambiguous in English
    // and in translation.
    let first = crate::clock::relative_day(dates[0], state.today);
    let last = crate::clock::relative_day(*dates.last().unwrap_or(&start), state.today);
    let span = match dates.len() {
        1 => first,
        2 => format!("{first} and {last}"),
        _ => format!("{first} to {last}"),
    };
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

/* ---------- Gemini: understanding and wording ---------- */

/// Understand a message with Gemini. `None` on any failure: the caller uses
/// [`rules`]. The model only classifies and extracts slots; it never sees
/// or produces an answer.
pub async fn llm_intent(
    g: &Arc<crate::llm::Gemini>,
    text: &str,
    products: &[String],
    open_suggestion: Option<&str>,
) -> Option<Intent> {
    let system = "You classify one message a bakery owner sends to their stock assistant. \
        kind: why = asks the reason behind a suggestion; what_if = asks about ordering a \
        different quantity (put it in quantity); closure = says the shop will be closed for \
        some days (closure_days, and next_week=true if they mean next week); stock = asks how \
        much of a product is left (product = the product words); other = anything else. \
        Messages may be English or Indonesian. Extract only what is written; never guess a \
        number.";
    let prompt = format!(
        "Products: {}\nOpen suggestion in this thread: {}\nMessage: {text}",
        products.join(", "),
        open_suggestion.unwrap_or("none")
    );
    let schema = json!({
        "type": "OBJECT",
        "properties": {
            "kind": {"type": "STRING", "enum": ["why", "what_if", "closure", "stock", "other"]},
            "quantity": {"type": "INTEGER", "nullable": true},
            "closure_days": {"type": "INTEGER", "nullable": true},
            "next_week": {"type": "BOOLEAN"},
            "product": {"type": "STRING", "nullable": true}
        },
        "required": ["kind", "next_week"]
    });
    match g
        .json::<Intent>(system, vec![json!({ "text": prompt })], schema)
        .await
    {
        Ok(mut i) => {
            // A number the merchant did not write is dropped, not trusted.
            let written = crate::llm::numbers_in(text);
            let keep = |n: Option<i64>| n.filter(|v| written.contains(&v.to_string()) || *v <= 10);
            i.quantity = i
                .quantity
                .filter(|v| written.contains(&v.to_string()))
                .or_else(|| first_number(&text.to_lowercase()));
            i.closure_days = keep(i.closure_days);
            Some(i)
        }
        Err(e) => {
            tracing::warn!(error = %e, "gemini intent failed; using rules");
            None
        }
    }
}

/// Day words and their Indonesian forms. Numbers are checked by
/// [`crate::llm::grounded`]; days are the other fact a translation can
/// quietly get wrong ("closed Tuesday" → "open again Tuesday").
const DAYS: [(&str, &str); 10] = [
    ("today", "hari ini"),
    ("tomorrow", "besok"),
    ("monday", "senin"),
    ("tuesday", "selasa"),
    ("wednesday", "rabu"),
    ("thursday", "kamis"),
    ("friday", "jumat"),
    ("saturday", "sabtu"),
    ("sunday", "minggu"),
    ("yesterday", "kemarin"),
];

/// Every day named in the English answer is named in the translation.
pub fn days_kept(source: &str, translated: &str) -> bool {
    let src = source.to_lowercase();
    let out = translated.to_lowercase().replace('\'', "");
    DAYS.iter()
        .filter(|(en, _)| src.contains(en))
        .all(|(_, id)| out.contains(id))
}

/// Reword a computed answer (and its button labels) into the merchant's
/// language. Rejected — the English stays — if the result contains any
/// number the original did not.
pub async fn localise(
    g: &Arc<crate::llm::Gemini>,
    language: &str,
    text: &str,
    labels: &[String],
) -> Option<(String, Vec<String>)> {
    let tone = match language {
        "Indonesia · formal" => "Indonesian, polite and formal (Anda)",
        "Indonesia · santai" => "Indonesian, warm and casual like a helpful shop assistant (kamu)",
        _ => return None,
    };
    let system = format!(
        "Translate a bakery stock assistant's reply, and each of its button labels, into \
         {tone}. Keep the meaning exactly: every number, unit, product name, day and rupiah \
         amount as given, and which days are closed or open. Do not add, round or compute \
         anything; do not add advice. Keep it as short as the original."
    );
    let schema = json!({
        "type": "OBJECT",
        "properties": {
            "text": {"type": "STRING"},
            "labels": {"type": "ARRAY", "items": {"type": "STRING"}}
        },
        "required": ["text", "labels"]
    });
    let prompt = json!({ "text": text, "labels": labels }).to_string();
    #[derive(Deserialize)]
    struct Out {
        text: String,
        labels: Vec<String>,
    }
    let out: Out = match g
        .json(&system, vec![json!({ "text": prompt })], schema)
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "gemini localise failed; keeping English");
            return None;
        }
    };
    let source = format!("{text} {}", labels.join(" "));
    let candidate = format!("{} {}", out.text, out.labels.join(" "));
    if out.labels.len() != labels.len() || !crate::llm::grounded(&candidate, &source) {
        tracing::warn!("gemini localise changed a number or a label count; keeping English");
        return None;
    }
    if !days_kept(&source, &candidate) {
        tracing::warn!("gemini localise dropped or changed a day; keeping English");
        return None;
    }
    Some((out.text, out.labels))
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
    fn a_translation_must_keep_the_days() {
        let en = "Closed tomorrow and Tuesday: the 4 kg lasts until Thursday.";
        assert!(days_kept(
            en,
            "Tutup besok dan Selasa: 4 kg cukup sampai Kamis."
        ));
        assert!(!days_kept(
            en,
            "Buka lagi Selasa depan: 4 kg cukup sampai Kamis."
        ));
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
