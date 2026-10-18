//! The PasStok agent: turns engine output into cards in the Shop Assistant
//! threads, answers questions, and reads waste photos.
//!
//! It holds no write path. It *signs* proposals (action tokens); a write
//! happens only when the merchant taps and the executor verifies the token
//! (FR-18, FR-28).
//!
//! The production agent is ADK + Gemini consuming Pub/Sub events. Here the
//! "morning run" is called at start-up (and by the demo kit), and the words
//! come from [`explain`] templates — see that module for why that is safe.

pub mod analysis;
pub mod answer;
pub mod explain;
pub mod idea;
pub mod vision;

use std::collections::HashMap;

use serde_json::json;

use crate::AppError;
use crate::clock;
use crate::contract::*;
use crate::engine::expiry;
use crate::engine::special;
use crate::engine::waste::diversion;
use crate::fmt::{idr, idr_short, percent, qty};
use crate::store::{State, SuggestedLine, Suggestion, SuggestionKind};
use crate::token::{ActionKeys, action_claims};

use analysis::{Analysis, analyze, analyze_all, reorder_groups};

/// Thread topics, in the order the app links them.
pub const TOPICS: [&str; 9] = [
    "reorder", "expiry", "waste", "donation", "special", "report", "stock", "product", "shop",
];

pub fn topic_title(topic: &str) -> Option<&'static str> {
    Some(match topic {
        "reorder" => "Reorder suggestions",
        "expiry" => "Expiring soon",
        "waste" => "Waste photo",
        "donation" => "Donation",
        "special" => "Today's special from stock",
        "report" => "Reports and impact",
        "stock" => "Check stock",
        "product" => "Add a product from a photo",
        "shop" => "Shop settings",
        _ => return None,
    })
}

pub fn shop(state: &State) -> Shop {
    Shop {
        name: state.data.name.clone(),
        owner: state.data.owner.clone(),
        greeting: clock::greeting(&state.data.owner),
    }
}

pub fn sign(
    state: &State,
    keys: &ActionKeys,
    act: &str,
    sug: &str,
    max: i64,
    unit: &str,
) -> Result<String, AppError> {
    keys.sign(&action_claims(&state.data.tenant, act, sug, max, unit))
}

pub fn agent_text(id: String, text: impl Into<String>) -> Message {
    Message::Text {
        id,
        author: Author::Agent,
        time: clock::hhmm(),
        text: text.into(),
        actions: None,
    }
}

pub fn note(id: String, text: impl Into<String>) -> Message {
    Message::Note {
        id,
        author: Author::System,
        time: String::new(),
        text: text.into(),
    }
}

/// Rebuild every thread from the engine: the morning digest. Called on start
/// and by the demo kit; tokens are signed once here and stored with the
/// message, not minted on every read.
pub fn morning_run(state: &mut State, keys: &ActionKeys) -> Result<(), AppError> {
    state.threads.clear();
    state.suggestions.retain(|_, s| s.state != CardState::Open);

    let all = analyze_all(state);
    post_reorders(state, keys, &all)?;
    post_expiry(state, keys, &all)?;
    post_special(state, keys, &all)?;

    let waste_opener = state.next_id("m");
    state.push(
        "waste",
        agent_text(
            waste_opener,
            "Anything thrown away today? Take a photo of the tray. I'll outline the items and match them to your products; nothing is recorded until you tap.",
        ),
    );
    let d = state.next_id("m");
    state.push(
        "donation",
        agent_text(
            d,
            "No donation pending. When you give leftovers the \"Donate\" destination, the pickup and the food-safety checklist appear here.",
        ),
    );
    let r = state.next_id("m");
    let digest = digest_text(state);
    state.push("report", agent_text(r, digest));
    let s = state.next_id("m");
    let stock = stock_summary(state, &all);
    state.push("stock", agent_text(s, stock));
    let p = state.next_id("m");
    state.push(
        "product",
        agent_text(
            p,
            "Photo-to-listing isn't in this build yet. Add or edit products in worksh.io for now; they show up here straight away.",
        ),
    );
    let h = state.next_id("m");
    state.push(
        "shop",
        agent_text(
            h,
            "You're open 07.00–21.00 every day. Tell me about closures (\"we're closed 2 days next week\") and I'll plan orders around them.",
        ),
    );
    Ok(())
}

fn post_reorders(state: &mut State, keys: &ActionKeys, all: &[Analysis]) -> Result<(), AppError> {
    let groups = reorder_groups(all, state);
    if groups.is_empty() {
        let id = state.next_id("m");
        state.push("reorder", agent_text(id, "Nothing needs ordering today."));
        return Ok(());
    }
    for (sid, lines) in groups {
        let Some(supplier) = state.data.supplier(&sid).cloned() else {
            continue;
        };
        let sug_id = state.next_id("sug");
        let msg_id = state.next_id("m");

        let mut card_lines = Vec::new();
        let mut stored = Vec::new();
        let mut total = 0;
        let mut low_confidence = false;
        for a in &lines {
            let (Some(p), Some(plan)) = (state.data.product(&a.product_id), a.plan) else {
                continue;
            };
            total += plan.qty;
            low_confidence |= a.forecast.confidence == crate::engine::forecast::Confidence::Low;
            card_lines.push(CardLine {
                product_id: p.id.clone(),
                name: p.name.clone(),
                quantity: plan.qty,
                unit: p.unit.clone(),
                note: explain::reorder_line(a, p, &supplier, state).replace(
                    &format!(" {}", explain::delivery_line(&supplier, state)),
                    "",
                ),
            });
            stored.push(SuggestedLine {
                product_id: p.id.clone(),
                qty: plan.qty,
                inputs: a.inputs_json(&sid, supplier.lead_time_days),
            });
        }
        if stored.is_empty() {
            continue;
        }

        let single = card_lines.len() == 1;
        let (title, reason, unit) = if single {
            let l = &card_lines[0];
            let a = &lines[0];
            let p = state
                .data
                .product(&l.product_id)
                .expect("line product exists");
            (
                format!("{} · order {}", l.name, qty(l.quantity, &l.unit)),
                explain::reorder_line(a, p, &supplier, state),
                l.unit.clone(),
            )
        } else {
            (
                format!("{} · {} products", supplier.name, card_lines.len()),
                format!(
                    "Grouped into one purchase order. {}",
                    explain::delivery_line(&supplier, state)
                ),
                "items".into(),
            )
        };

        let token = sign(state, keys, "create_po", &sug_id, total, &unit)?;
        let event_lines: Vec<_> = stored.iter().map(|l| l.inputs.clone()).collect();
        state.emit(
            "passtok.reorder.suggested",
            json!({ "supplier_id": sid, "location_id": "loc_main", "suggestion_id": sug_id, "lines": event_lines }),
        );
        state.suggestions.insert(
            sug_id.clone(),
            Suggestion {
                id: sug_id.clone(),
                topic: "reorder".into(),
                message_id: msg_id.clone(),
                kind: SuggestionKind::Reorder {
                    supplier_id: sid.clone(),
                    lines: stored,
                },
                state: CardState::Open,
                event_id: format!("evt-{sug_id}"),
            },
        );
        state.push(
            "reorder",
            Message::SuggestionCard {
                id: msg_id,
                author: Author::Agent,
                time: "Today 07.00".into(),
                card: SuggestionCard {
                    label: "Reorder suggestion".into(),
                    confidence: if low_confidence {
                        "Low confidence".into()
                    } else {
                        "High confidence".into()
                    },
                    title,
                    reason,
                    quantity: total,
                    unit,
                    token,
                    state: CardState::Open,
                    approve_label: "Approve".into(),
                    editable: true,
                    lines: if single { Vec::new() } else { card_lines },
                },
            },
        );
    }
    Ok(())
}

fn post_expiry(state: &mut State, keys: &ActionKeys, all: &[Analysis]) -> Result<(), AppError> {
    let today = state.today;
    let mut any = false;
    for a in all {
        let Some(p) = state.data.product(&a.product_id).cloned() else {
            continue;
        };
        for r in &a.risks {
            if r.at_risk_qty <= 0 {
                continue;
            }
            let past = r.expiry < today;
            if past {
                any = true;
                let id = state.next_id("m");
                state.push(
                    "expiry",
                    agent_text(
                        id,
                        format!(
                            "{} of {} (lot {}) passed its date {}. It can't be sold or donated; photograph it at closing and it'll go to compost.",
                            qty(r.at_risk_qty, &p.unit),
                            p.name,
                            r.lot_id,
                            clock::relative_day(r.expiry, today)
                        ),
                    ),
                );
                continue;
            }
            let actions = expiry::actions(r.at_risk_qty, p.sell_price, p.cost, false);
            state.emit(
                "passtok.lot.expiry_risk",
                json!({
                    "lot_id": r.lot_id, "product_id": p.id, "expiry_date": r.expiry.to_string(),
                    "qty_remaining": r.qty_remaining, "expected_sold_before_expiry": analysis::round2(r.expected_sold),
                    "at_risk_qty": r.at_risk_qty,
                    "actions": actions.iter().map(|x| json!({"type": format!("{:?}", x.kind).to_lowercase(), "expected_recovered_idr": x.expected_recovered})).collect::<Vec<_>>(),
                }),
            );
            if p.sell_price.is_none() {
                // Ingredients go to the special, not to a markdown.
                continue;
            }
            any = true;
            let sug_id = state.next_id("sug");
            let msg_id = state.next_id("m");
            let pct = expiry::markdown_pct();
            let token = sign(state, keys, "markdown", &sug_id, r.at_risk_qty, &p.unit)?;
            state.suggestions.insert(
                sug_id.clone(),
                Suggestion {
                    id: sug_id.clone(),
                    topic: "expiry".into(),
                    message_id: msg_id.clone(),
                    kind: SuggestionKind::Markdown {
                        product_id: p.id.clone(),
                        lot_id: r.lot_id.clone(),
                        qty: r.at_risk_qty,
                        pct,
                    },
                    state: CardState::Open,
                    event_id: format!("evt-{sug_id}"),
                },
            );
            state.push(
                "expiry",
                Message::SuggestionCard {
                    id: msg_id,
                    author: Author::Agent,
                    time: "Today 07.00".into(),
                    card: SuggestionCard {
                        label: "Expiring soon".into(),
                        confidence: a.forecast.confidence.label().into(),
                        title: format!(
                            "{} · {} of {} at risk",
                            p.name,
                            r.at_risk_qty,
                            qty(r.qty_remaining, &p.unit)
                        ),
                        reason: explain::expiry_reason(&p, r, &actions, state),
                        quantity: r.at_risk_qty,
                        unit: p.unit.clone(),
                        token,
                        state: CardState::Open,
                        approve_label: format!("Start {pct}% discount"),
                        editable: true,
                        lines: Vec::new(),
                    },
                },
            );
        }
    }
    if !any {
        let id = state.next_id("m");
        state.push(
            "expiry",
            agent_text(id, "Nothing at risk of expiring unsold."),
        );
    }
    Ok(())
}

pub fn at_risk_map(all: &[Analysis], today: chrono::NaiveDate) -> HashMap<String, i64> {
    all.iter()
        .map(|a| {
            let n = a
                .risks
                .iter()
                .filter(|r| r.expiry >= today)
                .map(|r| r.at_risk_qty)
                .sum();
            (a.product_id.clone(), n)
        })
        .collect()
}

fn post_special(state: &mut State, keys: &ActionKeys, all: &[Analysis]) -> Result<(), AppError> {
    let available: HashMap<String, i64> = all
        .iter()
        .map(|a| (a.product_id.clone(), a.usable))
        .collect();
    let cost: HashMap<String, i64> = state
        .data
        .products
        .iter()
        .map(|p| (p.id.clone(), p.cost))
        .collect();
    let at_risk = at_risk_map(all, state.today);
    let Some(sp) = special::propose(&available, &at_risk, &cost) else {
        let id = state.next_id("m");
        state.push(
            "special",
            agent_text(id, "Nothing near its date that a special could use today."),
        );
        return Ok(());
    };
    let sug_id = state.next_id("sug");
    let msg_id = state.next_id("m");
    let token = sign(
        state,
        keys,
        "create_special",
        &sug_id,
        sp.portions,
        "portions",
    )?;
    let reason = explain::special_reason(&sp, state);
    state.suggestions.insert(
        sug_id.clone(),
        Suggestion {
            id: sug_id.clone(),
            topic: "special".into(),
            message_id: msg_id.clone(),
            kind: SuggestionKind::Special(sp.clone()),
            state: CardState::Open,
            event_id: format!("evt-{sug_id}"),
        },
    );
    state.push(
        "special",
        Message::SuggestionCard {
            id: msg_id,
            author: Author::Agent,
            time: "Today 07.00".into(),
            card: SuggestionCard {
                label: "Today's special".into(),
                confidence: "AI idea · from your stock".into(),
                title: format!("{} · {}", sp.name, qty(sp.portions, "portion")),
                reason,
                quantity: sp.portions,
                unit: "portions".into(),
                token,
                state: CardState::Open,
                approve_label: "Make it today".into(),
                editable: true,
                lines: Vec::new(),
            },
        },
    );
    Ok(())
}

fn stock_summary(state: &State, all: &[Analysis]) -> String {
    let mut low: Vec<String> = all
        .iter()
        .filter(|a| a.cover < 4.0)
        .filter_map(|a| {
            let p = state.data.product(&a.product_id)?;
            Some(format!(
                "{} ({}, {})",
                p.name,
                qty(a.usable, &p.unit),
                explain::run_out_label(a, state)
            ))
        })
        .collect();
    if low.is_empty() {
        return "Everything has at least four days of cover. Ask about any product, e.g. \"how much oat milk is left?\"".into();
    }
    low.truncate(4);
    format!(
        "Running low: {}. Ask about any product, e.g. \"how much oat milk is left?\"",
        low.join("; ")
    )
}

pub fn digest_text(state: &State) -> String {
    let today = state.today;
    let this_week = state
        .waste
        .iter()
        .filter(|w| (today - w.date).num_days() < 7);
    let last_week = state.waste.iter().filter(|w| {
        let d = (today - w.date).num_days();
        (7..14).contains(&d)
    });
    let now = diversion(this_week);
    let before = diversion(last_week);
    let po = state.orders.len();
    match (now.score, before.score) {
        (Some(n), Some(b)) => format!(
            "This week: {} of leftovers recorded across {} photos, {}% kept out of landfill (last week {}%). {} purchase orders from suggestions.",
            idr(now.total_value),
            now.photos,
            percent(n),
            percent(b),
            po
        ),
        (Some(n), None) => format!(
            "This week: {} of leftovers recorded, {}% kept out of landfill.",
            idr(now.total_value),
            percent(n)
        ),
        _ => "No leftovers recorded this week yet. Photograph the tray at closing and it shows up here.".into(),
    }
}

pub fn home(state: &State) -> AssistantHome {
    let mut attention = Vec::new();
    let mut open: Vec<&Suggestion> = state
        .suggestions
        .values()
        .filter(|s| s.state == CardState::Open)
        .collect();
    open.sort_by(|a, b| a.id.cmp(&b.id));

    for s in open {
        match &s.kind {
            SuggestionKind::Reorder { supplier_id, lines } => {
                let supplier = state
                    .data
                    .supplier(supplier_id)
                    .map(|x| x.name.clone())
                    .unwrap_or_default();
                if let [l] = lines.as_slice()
                    && let (Some(p), Some(a)) = (
                        state.data.product(&l.product_id),
                        analyze(state, &l.product_id),
                    )
                {
                    attention.push(Attention {
                        id: s.id.clone(),
                        title: format!("{} runs out {}", p.name, explain::run_out_day(&a, state)),
                        detail: format!("Suggested: order {}", qty(l.qty, &p.unit)),
                        icon: AttentionIcon::Stock,
                        href: "/assistant/reorder".into(),
                    });
                } else {
                    attention.push(Attention {
                        id: s.id.clone(),
                        title: format!("Order from {supplier}"),
                        detail: format!("{} products running low", lines.len()),
                        icon: AttentionIcon::Stock,
                        href: "/assistant/reorder".into(),
                    });
                }
            }
            SuggestionKind::Markdown {
                product_id,
                lot_id,
                qty: n,
                ..
            } => {
                let Some(p) = state.data.product(product_id) else {
                    continue;
                };
                let lot = state.data.lots.iter().find(|l| &l.id == lot_id);
                let when = lot
                    .and_then(|l| l.expiry)
                    .map(|e| clock::relative_day(e, state.today))
                    .unwrap_or_default();
                attention.push(Attention {
                    id: s.id.clone(),
                    title: format!(
                        "{} {} expire {when}",
                        lot.map_or(0, |l| l.qty),
                        crate::fmt::plural(&p.unit, lot.map_or(0, |l| l.qty)) + " of " + &p.name
                    ),
                    detail: format!("{n} are unlikely to sell"),
                    icon: AttentionIcon::Expiry,
                    href: "/assistant/expiry".into(),
                });
            }
            SuggestionKind::Special(sp) => attention.push(Attention {
                id: s.id.clone(),
                title: format!("Idea: {}", sp.name),
                detail: format!(
                    "Uses stock at risk · recovers {}",
                    idr_short(sp.expected_recovered)
                ),
                icon: AttentionIcon::Special,
                href: "/assistant/special".into(),
            }),
        }
    }
    if let Some(d) = state
        .donations
        .values()
        .find(|d| d.state == CardState::Open)
    {
        attention.insert(
            0,
            Attention {
                id: d.id.clone(),
                title: "Donation waiting for the checklist".into(),
                detail: format!("{} · pickup from 21.30", d.id),
                icon: AttentionIcon::Donation,
                href: "/assistant/donation".into(),
            },
        );
    }
    // End-of-day nudge (FR-32).
    if clock::now()
        .format("%H")
        .to_string()
        .parse::<u32>()
        .unwrap_or(0)
        >= 17
    {
        attention.push(Attention {
            id: "nudge-waste".into(),
            title: "Anything thrown away today?".into(),
            detail: "Snap a photo of the tray".into(),
            icon: AttentionIcon::Waste,
            href: "/assistant/waste".into(),
        });
    }

    let command = |label: &str, topic: &str| QuickCommand {
        label: label.into(),
        href: format!("/assistant/{topic}"),
    };
    AssistantHome {
        shop: shop(state),
        attention,
        commands: vec![
            command("Photograph today's leftovers", "waste"),
            command("How much oat milk is left?", "stock"),
            command("Menu ideas from stock", "special"),
            command("This week's report", "report"),
            command("What's expiring?", "expiry"),
            command("Change opening hours", "shop"),
        ],
    }
}

pub fn thread(state: &State, topic: &str, vision_source: &str) -> Option<Thread> {
    let title = topic_title(topic)?;
    Some(Thread {
        topic: title.into(),
        shop: shop(state),
        source: (topic == "waste").then(|| vision_source.to_owned()),
        messages: state.threads.get(topic).cloned().unwrap_or_default(),
    })
}

/// Flip the state of the card a suggestion was posted as.
pub fn set_card_state(state: &mut State, topic: &str, message_id: &str, new: CardState) {
    if let Some(msgs) = state.threads.get_mut(topic) {
        for m in msgs.iter_mut() {
            match m {
                Message::SuggestionCard { id, card, .. } if id == message_id => card.state = new,
                Message::WasteCard { id, card, .. } if id == message_id => card.state = new,
                Message::DonationCard { id, card, .. } if id == message_id => card.state = new,
                Message::Text { id, actions, .. } if id == message_id => *actions = None,
                _ => {}
            }
        }
    }
}

/// Inline actions offered in Q&A answers point at a suggestion too; once it
/// is handled they must disappear everywhere.
pub fn retire_inline_actions(state: &mut State, topic: &str) {
    if let Some(msgs) = state.threads.get_mut(topic) {
        for m in msgs.iter_mut() {
            if let Message::Text { actions, .. } = m {
                *actions = None;
            }
        }
    }
}

pub fn value_label(v: crate::contract::Idr) -> String {
    format!("~{}", idr_short(v))
}

/// A grounded waste photo becomes a session plus two messages: the photo with
/// its boxes, and the card where the merchant confirms counts, picks the
/// unsure item and chooses destinations (FR-33 to FR-35, FR-40).
pub fn post_waste_session(
    state: &mut State,
    keys: &ActionKeys,
    photo_id: &str,
    detections: &[vision::Detection],
) -> Result<String, AppError> {
    use crate::engine::waste::{Condition, options};

    let candidates = vision::candidates(state);
    let session_id = state.next_id("ws");
    let mut n = 0;
    let items = vision::ground(detections, &candidates, state.today, || {
        n += 1;
        format!("{session_id}:{n}")
    });

    let boxes = detections
        .iter()
        .map(|d| DetectionBox {
            x: d.bbox[0],
            y: d.bbox[1],
            w: d.bbox[2],
            h: d.bbox[3],
            confident: d.confidence >= vision::CONFIDENCE_THRESHOLD && d.candidate_id.is_some(),
        })
        .collect();

    let destination_list = |p: &crate::shop::Product, q: i64, c: Condition| {
        options(p, q, c)
            .into_iter()
            .map(|o| Destination {
                id: o.destination.id().into(),
                label: o.destination.label().into(),
                selected: o.selected,
            })
            .collect::<Vec<_>>()
    };

    let mut card_items = Vec::new();
    let mut value = 0;
    let mut uncertain = None;
    for it in &items {
        match &it.product_id {
            Some(pid) => {
                let Some(p) = state.data.product(pid).cloned() else {
                    continue;
                };
                value += it.qty * p.cost;
                let past = !it.condition.can_be_eaten();
                card_items.push(WasteItem {
                    id: it.item_id.clone(),
                    product_id: p.id.clone(),
                    name: p.name.clone(),
                    quantity: it.qty,
                    max_quantity: state.data.total_on_hand(&p.id).max(it.qty),
                    unit: p.unit.clone(),
                    severity: if past {
                        Severity::Critical
                    } else {
                        Severity::Ok
                    },
                    severity_label: format!(
                        "{} · {}%",
                        crate::executor::condition_label(it.condition),
                        percent(it.confidence)
                    ),
                    destinations: destination_list(&p, it.qty, it.condition),
                    warning: past.then(|| "Past its date: it can't be sold or donated.".to_owned()),
                });
            }
            None if uncertain.is_none() && !it.candidates.is_empty() => {
                let first = state.data.product(&it.candidates[0]).cloned();
                uncertain = Some(UncertainItem {
                    item_id: it.item_id.clone(),
                    label: it.label.clone(),
                    quantity: it.qty,
                    unit: first.as_ref().map(|p| p.unit.clone()).unwrap_or_default(),
                    confidence: percent(it.confidence),
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
                    destinations: first
                        .map(|p| destination_list(&p, it.qty, it.condition))
                        .unwrap_or_default(),
                });
            }
            None => {}
        }
    }

    let total = card_items.len() + usize::from(uncertain.is_some());
    let token = sign(state, keys, "record_waste", &session_id, 1, "photo")?;
    let photo_msg = state.next_id("m");
    let card_msg = state.next_id("m");
    state.sessions.insert(
        session_id.clone(),
        crate::store::WasteSession {
            id: session_id.clone(),
            photo_id: photo_id.into(),
            message_id: card_msg.clone(),
            items,
            state: CardState::Open,
        },
    );
    state.push(
        "waste",
        Message::Photo {
            id: photo_msg,
            author: Author::Staff,
            time: clock::hhmm(),
            photo_label: "Leftover tray".into(),
            photo_url: Some(format!("/photos/{photo_id}")),
            boxes,
        },
    );
    state.push(
        "waste",
        Message::WasteCard {
            id: card_msg,
            author: Author::Agent,
            time: clock::hhmm(),
            card: WasteCard {
                label: format!(
                    "Today's leftovers · {total} {}",
                    crate::fmt::plural("item", total as i64)
                ),
                value: value_label(value),
                items: card_items,
                uncertain,
                note: "Photo kept as evidence · location data removed".into(),
                token,
                state: CardState::Open,
            },
        },
    );
    Ok(session_id)
}
