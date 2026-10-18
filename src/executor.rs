//! Action executor (FR-18, FR-19, FR-27, FR-35, FR-41, FR-48, FR-49).
//!
//! The only code that writes on the assistant's behalf, and only after a
//! merchant's tap carrying a signed token. Order of operations, for every act:
//!
//! 1. the route has verified signature, audience, issuer and expiry;
//! 2. **validate** the request against the stored suggestion — nothing is
//!    spent yet, so a bad request does not burn the token;
//! 3. **claim** the token: tenant, quantity cap, single use;
//! 4. **apply**: mutate, write the audit entry and the outbox event, post the
//!    result into the thread.
//!
//! No LLM is involved in any of it. Writes go to the in-memory ledger for the
//! hackathon build; each one is marked with the worksh.io mutation it becomes
//! (`createPurchaseOrder`, `adjustStock`, `createWorkOrder`).

use std::collections::HashMap;

use chrono::Duration;
use serde::Deserialize;
use serde_json::json;

use crate::AppError;
use crate::agent::{self, note, sign};
use crate::clock;
use crate::contract::*;
use crate::engine::waste::{Condition, Destination, WasteRecord, allowed, diversion};
use crate::fmt::{idr, qty};
use crate::shop::{Category, Product};
use crate::store::*;
use crate::token::{ActionClaims, ActionKeys, TokenGuard};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionBody {
    pub quantity: Option<i64>,
    #[serde(default)]
    pub lines: Vec<LineBody>,
    #[serde(default)]
    pub items: Vec<WasteLineBody>,
    #[serde(default)]
    pub checklist: HashMap<String, bool>,
    pub recipient_id: Option<String>,
    pub pickup_window: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineBody {
    pub product_id: String,
    pub quantity: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WasteLineBody {
    pub item_id: String,
    pub quantity: i64,
    pub destination: String,
    /// For an item vision was unsure about: the merchant's pick, or "none".
    pub product_id: Option<String>,
}

pub const CHECKLIST: [(&str, &str); 4] = [
    ("before_expiry", "Every item is before its expiry date"),
    ("storage_ok", "Kept at the right temperature"),
    ("sealed", "Packed and sealed for transport"),
    ("allergen_label", "Allergens labelled (milk, gluten, egg)"),
];

pub const PICKUP_WINDOWS: [&str; 3] = ["21.30–22.00", "22.00–22.30", "Tomorrow 07.00–07.30"];

pub fn execute(
    state: &mut State,
    keys: &ActionKeys,
    guard: &TokenGuard,
    claims: &ActionClaims,
    body: &ActionBody,
) -> Result<ResultCard, AppError> {
    let tenant = state.data.tenant.clone();
    match claims.act.as_str() {
        "create_po" => {
            let plan = validate_po(state, claims, body)?;
            guard.claim(claims, &tenant, plan.iter().map(|(_, q)| q).sum())?;
            Ok(apply_po(state, claims, plan))
        }
        "markdown" => {
            let n = body.quantity.unwrap_or(0);
            open_suggestion(state, &claims.sug)?;
            guard.claim(claims, &tenant, n)?;
            apply_markdown(state, claims, n)
        }
        "create_special" => {
            let n = body.quantity.unwrap_or(0);
            open_suggestion(state, &claims.sug)?;
            guard.claim(claims, &tenant, n)?;
            apply_special(state, claims, n)
        }
        "record_waste" => {
            let plan = validate_waste(state, claims, body)?;
            guard.claim(claims, &tenant, 1)?;
            apply_waste(state, keys, claims, plan)
        }
        "donate" => {
            let plan = validate_donation(state, claims, body)?;
            guard.claim(claims, &tenant, 1)?;
            Ok(apply_donation(state, claims, plan))
        }
        _ => Err(AppError::TokenInvalid),
    }
}

/// Reject from the card. Spends the token like an approval would.
pub fn reject(
    state: &mut State,
    guard: &TokenGuard,
    claims: &ActionClaims,
) -> Result<ResultCard, AppError> {
    let s = open_suggestion(state, &claims.sug)?.clone();
    guard.claim(claims, &state.data.tenant.clone(), 1)?;
    if let Some(x) = state.suggestions.get_mut(&s.id) {
        x.state = CardState::Rejected;
    }
    agent::set_card_state(state, &s.topic, &s.message_id, CardState::Rejected);
    agent::retire_inline_actions(state, &s.topic);
    let id = state.next_id("m");
    state.push(&s.topic, note(id, "You rejected this suggestion"));
    state.emit(
        "passtok.decision.recorded",
        json!({ "suggestion_id": s.id, "decision": "rejected", "actor": state.data.owner }),
    );
    state.record("reject", &s.id, json!({}), "rejected");
    Ok(ResultCard {
        title: "Suggestion rejected".into(),
        detail: "Nothing was ordered or changed.".into(),
        wa_url: None,
        links: Vec::new(),
    })
}

fn open_suggestion<'a>(state: &'a State, id: &str) -> Result<&'a Suggestion, AppError> {
    let s = state.suggestions.get(id).ok_or(AppError::TokenInvalid)?;
    if s.state != CardState::Open {
        // Handled through another button (or another device).
        return Err(AppError::TokenUsed);
    }
    Ok(s)
}

/* ---------- Purchase orders ---------- */

fn validate_po(
    state: &State,
    claims: &ActionClaims,
    body: &ActionBody,
) -> Result<Vec<(String, i64)>, AppError> {
    let s = open_suggestion(state, &claims.sug)?;
    let SuggestionKind::Reorder { lines, .. } = &s.kind else {
        return Err(AppError::TokenInvalid);
    };
    let plan: Vec<(String, i64)> = if body.lines.is_empty() {
        // Single-line card: the stepper's quantity.
        let [line] = lines.as_slice() else {
            return Err(AppError::BadRequest("this order has several lines".into()));
        };
        vec![(line.product_id.clone(), body.quantity.unwrap_or(0))]
    } else {
        body.lines
            .iter()
            .map(|l| {
                let cap = lines
                    .iter()
                    .find(|s| s.product_id == l.product_id)
                    .ok_or_else(|| AppError::BadRequest("unknown line".into()))?
                    .qty;
                if l.quantity < 0 {
                    return Err(AppError::BadRequest("quantity cannot be negative".into()));
                }
                if l.quantity > cap {
                    return Err(AppError::QtyAboveCap {
                        requested: l.quantity,
                        cap,
                    });
                }
                Ok((l.product_id.clone(), l.quantity))
            })
            .collect::<Result<_, _>>()?
    };
    let plan: Vec<_> = plan.into_iter().filter(|(_, q)| *q != 0).collect();
    if plan.is_empty() {
        return Err(AppError::BadRequest("quantity must be at least 1".into()));
    }
    Ok(plan)
}

fn apply_po(state: &mut State, claims: &ActionClaims, plan: Vec<(String, i64)>) -> ResultCard {
    let s = state.suggestions[&claims.sug].clone();
    let SuggestionKind::Reorder { supplier_id, lines } = &s.kind else {
        unreachable!("validated as a reorder");
    };
    let supplier = state.data.supplier(supplier_id).cloned();
    let lead = supplier.as_ref().map_or(1, |x| x.lead_time_days);
    let expected = state.today + Duration::days(i64::from(lead));

    // TODO(worksh.io): createPurchaseOrder → submitPurchaseOrder through
    // state.works as the merchant; approve/issue stay with the human.
    state.po_seq += 1;
    let number = format!("PO-{:04}", state.po_seq);
    state.orders.push(PurchaseOrder {
        number: number.clone(),
        supplier_id: supplier_id.clone(),
        lines: plan.clone(),
        expected,
        suggestion_id: s.id.clone(),
        status: "submitted",
    });

    // FR-14: an edit down teaches the next suggestion a smaller buffer.
    let mut edited = false;
    for (pid, q) in &plan {
        if let Some(sug) = lines.iter().find(|l| &l.product_id == pid)
            && *q != sug.qty
            && sug.qty > 0
        {
            edited = true;
            let ratio = *q as f64 / sug.qty as f64 - 1.0;
            let e = state.adjust.entry(pid.clone()).or_insert(0.0);
            *e = (*e * 0.5 + ratio * 0.5).clamp(-0.5, 0.3);
        }
    }

    if let Some(x) = state.suggestions.get_mut(&s.id) {
        x.state = CardState::Approved;
    }
    agent::set_card_state(state, &s.topic, &s.message_id, CardState::Approved);
    agent::retire_inline_actions(state, &s.topic);

    let described: Vec<String> = plan
        .iter()
        .filter_map(|(pid, q)| {
            let p = state.data.product(pid)?;
            Some(format!("{} {}", qty(*q, &p.unit), p.name))
        })
        .collect();
    let supplier_name = supplier
        .as_ref()
        .map_or("the supplier".into(), |x| x.name.clone());
    let detail = format!(
        "{} · {} · arrives {}",
        described.join(", "),
        supplier_name,
        clock::relative_day(expected, state.today)
    );
    let message = format!(
        "Hello {supplier_name}, {} would like to order {} ({number}), for delivery {}. Thank you!",
        state.data.name,
        described.join(", "),
        clock::relative_day(expected, state.today)
    );
    let wa_url = supplier
        .as_ref()
        .map(|x| format!("https://wa.me/{}?text={}", x.whatsapp, urlencode(&message)));
    let links = match plan.as_slice() {
        [(pid, _)] => vec![Link {
            label: "View product".into(),
            href: format!("/products/{pid}"),
        }],
        _ => vec![Link {
            label: "View stock".into(),
            href: "/stock".into(),
        }],
    };
    let card = ResultCard {
        title: format!("{number} submitted"),
        detail,
        wa_url,
        links,
    };

    let total: i64 = plan.iter().map(|(_, q)| q).sum();
    let unit = if plan.len() == 1 {
        state
            .data
            .product(&plan[0].0)
            .map(|p| crate::fmt::plural(&p.unit, total))
            .unwrap_or_default()
    } else {
        "items".into()
    };
    let n = state.next_id("m");
    state.push(&s.topic, note(n, format!("You approved {total} {unit}")));
    let r = state.next_id("m");
    state.push(
        &s.topic,
        Message::ResultCard {
            id: r,
            author: Author::Agent,
            time: "Just now".into(),
            card: card.clone(),
        },
    );
    let inputs = json!(lines.iter().map(|l| l.inputs.clone()).collect::<Vec<_>>());
    state.emit(
        "passtok.decision.recorded",
        json!({
            "suggestion_id": s.id,
            "decision": if edited { "edited" } else { "approved" },
            "final_lines": plan.iter().map(|(p, q)| json!({"product_id": p, "qty": q})).collect::<Vec<_>>(),
            "actor": state.data.owner,
            "po_id": number,
        }),
    );
    state.record("create_po", &s.id, inputs, &number);
    card
}

/* ---------- Markdown ---------- */

fn apply_markdown(
    state: &mut State,
    claims: &ActionClaims,
    n: i64,
) -> Result<ResultCard, AppError> {
    let s = state.suggestions[&claims.sug].clone();
    let SuggestionKind::Markdown {
        product_id,
        lot_id,
        pct,
        ..
    } = &s.kind
    else {
        return Err(AppError::TokenInvalid);
    };
    let p = state
        .data
        .product(product_id)
        .cloned()
        .ok_or(AppError::NotFound)?;
    // TODO(bikinlapak + worksh.io): time-boxed price override on lapak and
    // POS — the PRD §10 gap. Recorded here and shown on the product list.
    state.markdowns.push(Markdown {
        product_id: p.id.clone(),
        lot_id: lot_id.clone(),
        qty: n,
        pct: *pct,
        starts: "17.00".into(),
        ends: "21.30".into(),
    });
    if let Some(x) = state.suggestions.get_mut(&s.id) {
        x.state = CardState::Approved;
    }
    agent::set_card_state(state, &s.topic, &s.message_id, CardState::Approved);
    let price = p.sell_price.unwrap_or(0) * (100 - pct) / 100;
    let card = ResultCard {
        title: format!("{pct}% off from 17.00"),
        detail: format!(
            "{} of {} at {} · until 21.30 · storefront and POS",
            qty(n, &p.unit),
            p.name,
            idr(price)
        ),
        wa_url: None,
        links: vec![Link {
            label: "View product".into(),
            href: format!("/products/{}", p.id),
        }],
    };
    let id = state.next_id("m");
    state.push(
        &s.topic,
        note(
            id,
            format!("You started a {pct}% discount on {}", qty(n, &p.unit)),
        ),
    );
    let r = state.next_id("m");
    state.push(
        &s.topic,
        Message::ResultCard {
            id: r,
            author: Author::Agent,
            time: "Just now".into(),
            card: card.clone(),
        },
    );
    state.emit(
        "passtok.markdown.scheduled",
        json!({ "product_id": p.id, "lot_id": lot_id, "steps": [{"at": "17:00", "pct": pct}], "channels": ["lapak", "pos"], "ends_at": "21:30", "then": "donated", "qty": n }),
    );
    state.record(
        "markdown",
        &s.id,
        json!({ "qty": n, "pct": pct }),
        "scheduled",
    );
    Ok(card)
}

/* ---------- Daily special ---------- */

fn apply_special(
    state: &mut State,
    claims: &ActionClaims,
    portions: i64,
) -> Result<ResultCard, AppError> {
    let s = state.suggestions[&claims.sug].clone();
    let SuggestionKind::Special(sp) = &s.kind else {
        return Err(AppError::TokenInvalid);
    };

    // TODO(worksh.io): createWorkOrder for the recipe; adjustStock per
    // component with reason dest:repurposed.
    let today = state.today;
    let mut used = Vec::new();
    for (i, (pid, per)) in sp.lines.iter().enumerate() {
        let need = (per * portions as f64).ceil() as i64;
        let taken = state.data.take(pid, None, need);
        used.push((pid.clone(), taken));
        // The at-risk item counts as rescued; the others are ordinary usage.
        if i == 0
            && let Some(p) = state.data.product(pid).cloned()
        {
            let id = state.next_id("w");
            state.waste.push(WasteRecord {
                id,
                date: today,
                product_id: p.id.clone(),
                product_name: p.name.clone(),
                lot_id: None,
                qty: taken,
                unit: p.unit.clone(),
                destination: Destination::Repurposed,
                reason: "dest:repurposed".into(),
                value_at_cost: taken * p.cost,
                grams: taken * p.weight_gram,
                photo_id: None,
            });
        }
    }
    let wo = format!("WO-{:04}", 30 + state.work_orders.len() + 1);
    state.work_orders.push(WorkOrder {
        number: wo.clone(),
        product_name: sp.name.clone(),
        portions,
    });
    let pid = format!("sp-{}", state.work_orders.len());
    state.data.products.push(Product {
        id: pid.clone(),
        name: format!("{} (today)", sp.name),
        category: Category::Specials,
        unit: "portion".into(),
        sell_price: Some(sp.price),
        cost: 0,
        track_stock: false,
        perishable: true,
        shelf_life_days: Some(1),
        supplier_id: None,
        pack_size: 1,
        min_order_qty: 1,
        weight_gram: 150,
        draft: false,
        colour: "#E9D3B4".into(),
        description: format!(
            "Today's special, {portions} portions from stock at risk. {}",
            sp.method
        ),
    });

    if let Some(x) = state.suggestions.get_mut(&s.id) {
        x.state = CardState::Approved;
    }
    agent::set_card_state(state, &s.topic, &s.message_id, CardState::Approved);
    let card = ResultCard {
        title: format!("{wo} created"),
        detail: format!(
            "{} of {} at {} · listed for today",
            qty(portions, "portion"),
            sp.name,
            idr(sp.price)
        ),
        wa_url: None,
        links: vec![Link {
            label: "View listing".into(),
            href: format!("/products/{pid}"),
        }],
    };
    let id = state.next_id("m");
    state.push(
        &s.topic,
        note(id, format!("You approved {}", qty(portions, "portion"))),
    );
    let r = state.next_id("m");
    state.push(
        &s.topic,
        Message::ResultCard {
            id: r,
            author: Author::Agent,
            time: "Just now".into(),
            card: card.clone(),
        },
    );
    state.emit(
        "passtok.special.created",
        json!({ "work_order": wo, "name": sp.name, "portions": portions, "uses": used }),
    );
    state.record(
        "create_special",
        &s.id,
        json!({ "portions": portions, "uses": used }),
        &wo,
    );
    Ok(card)
}

/* ---------- Waste write-off ---------- */

pub struct WastePlan {
    session_id: String,
    lines: Vec<(DetectedItem, String, i64, Destination)>,
}

fn validate_waste(
    state: &State,
    claims: &ActionClaims,
    body: &ActionBody,
) -> Result<WastePlan, AppError> {
    let session = state
        .sessions
        .get(&claims.sug)
        .ok_or(AppError::TokenInvalid)?;
    if session.state != CardState::Open {
        return Err(AppError::TokenUsed);
    }
    let mut lines = Vec::new();
    for l in &body.items {
        let item = session
            .items
            .iter()
            .find(|i| i.item_id == l.item_id)
            .ok_or_else(|| AppError::BadRequest(format!("unknown item {}", l.item_id)))?;
        let product_id = match (&item.product_id, l.product_id.as_deref()) {
            (Some(p), _) => p.clone(),
            (None, Some("none")) | (None, None) => continue, // "Not one of our products"
            (None, Some(pick)) if item.candidates.iter().any(|c| c == pick) => pick.to_owned(),
            (None, Some(_)) => {
                return Err(AppError::BadRequest(
                    "pick one of the suggested products".into(),
                ));
            }
        };
        let dest = Destination::parse(&l.destination).ok_or_else(|| {
            AppError::BadRequest(format!("unknown destination {}", l.destination))
        })?;
        if !allowed(dest, item.condition) {
            return Err(AppError::BadRequest(format!(
                "{} can't go to {}: it is past its date or spoiled",
                product_id,
                dest.label()
            )));
        }
        let on_hand = state.data.total_on_hand(&product_id);
        if l.quantity < 1 || l.quantity > on_hand.max(1) {
            return Err(AppError::BadRequest(format!(
                "quantity for {product_id} must be between 1 and {}",
                on_hand.max(1)
            )));
        }
        lines.push((item.clone(), product_id, l.quantity, dest));
    }
    if lines.is_empty() {
        return Err(AppError::BadRequest("nothing to record".into()));
    }
    Ok(WastePlan {
        session_id: session.id.clone(),
        lines,
    })
}

fn apply_waste(
    state: &mut State,
    keys: &ActionKeys,
    claims: &ActionClaims,
    plan: WastePlan,
) -> Result<ResultCard, AppError> {
    let today = state.today;
    let photo_id = state.sessions[&plan.session_id].photo_id.clone();
    let mut to_donate = Vec::new();
    let mut event_lines = Vec::new();
    let mut recorded_value = 0;

    for (item, pid, n, dest) in &plan.lines {
        let Some(p) = state.data.product(pid).cloned() else {
            continue;
        };
        let lot = item
            .lot_id
            .clone()
            .filter(|_| item.product_id.as_deref() == Some(pid));
        if *dest == Destination::Donated {
            let expiry = lot
                .as_deref()
                .and_then(|l| state.data.lots.iter().find(|x| x.id == l))
                .and_then(|l| l.expiry);
            to_donate.push(DonationItem {
                product_id: pid.clone(),
                lot_id: lot,
                qty: *n,
                expiry,
            });
            continue;
        }
        // TODO(worksh.io): adjustStock(direction out, reason) per item; lot-
        // accurate once works-core accepts lotId (PRD §10 gap).
        let taken = state.data.take(pid, lot.as_deref(), *n);
        let taken = if taken < *n {
            taken + state.data.take(pid, None, n - taken)
        } else {
            taken
        };
        let id = state.next_id("w");
        let value = taken * p.cost;
        recorded_value += value;
        state.waste.push(WasteRecord {
            id,
            date: today,
            product_id: p.id.clone(),
            product_name: p.name.clone(),
            lot_id: lot.clone(),
            qty: taken,
            unit: p.unit.clone(),
            destination: *dest,
            reason: format!("waste:{} dest:{}", item.condition.id(), dest.id()),
            value_at_cost: value,
            grams: taken * p.weight_gram,
            photo_id: Some(photo_id.clone()),
        });
        event_lines.push(json!({
            "product_id": p.id, "lot_id": lot, "qty": taken, "reason": format!("waste:{}", item.condition.id()),
            "destination": dest.id(), "value_at_cost_idr": value, "vision_confidence": item.confidence,
            "edited_by_merchant": taken != item.qty,
        }));
    }

    if let Some(sess) = state.sessions.get_mut(&plan.session_id) {
        sess.state = CardState::Approved;
    }
    let card_msg = state.sessions[&plan.session_id].message_id.clone();
    agent::set_card_state(state, "waste", &card_msg, CardState::Approved);

    let mut links = Vec::new();
    let donation_note = if to_donate.is_empty() {
        String::new()
    } else {
        let id = create_donation(state, keys, to_donate, Some(photo_id.clone()))?;
        links.push(Link {
            label: "Continue to donation".into(),
            href: "/assistant/donation".into(),
        });
        format!(" Donation {id} is waiting for the safety checklist.")
    };
    let today_div = diversion(state.waste.iter().filter(|w| w.date == today));
    let card = ResultCard {
        title: format!(
            "Recorded {} {}",
            plan.lines.len(),
            crate::fmt::plural("item", plan.lines.len() as i64)
        ),
        detail: format!(
            "{} at cost · {}% of today's leftovers kept out of landfill.{donation_note}",
            idr(recorded_value),
            today_div.score.map_or(100, crate::fmt::percent)
        ),
        wa_url: None,
        links: {
            links.push(Link {
                label: "Waste report".into(),
                href: "/reports".into(),
            });
            links
        },
    };
    let r = state.next_id("m");
    state.push(
        "waste",
        Message::ResultCard {
            id: r,
            author: Author::Agent,
            time: "Just now".into(),
            card: card.clone(),
        },
    );
    state.emit(
        "passtok.waste.recorded",
        json!({ "location_id": "loc_main", "photo_ids": [photo_id], "lines": event_lines, "actor": state.data.owner }),
    );
    state.record(
        "record_waste",
        &claims.sug,
        json!({ "lines": plan.lines.len() }),
        "recorded",
    );
    Ok(card)
}

/* ---------- Donation ---------- */

/// Open a donation and post its card, with the checklist, into the
/// donation thread.
fn create_donation(
    state: &mut State,
    keys: &ActionKeys,
    items: Vec<DonationItem>,
    photo_id: Option<String>,
) -> Result<String, AppError> {
    state.don_seq += 1;
    let id = format!("DON-{}-{:04}", state.today.format("%Y"), state.don_seq);
    let msg_id = state.next_id("m");
    let token = sign(state, keys, "donate", &id, 1, "donation")?;
    let lines: Vec<DonationLine> = items
        .iter()
        .filter_map(|i| {
            let p = state.data.product(&i.product_id)?;
            Some(DonationLine {
                name: p.name.clone(),
                quantity: i.qty,
                unit: p.unit.clone(),
                expiry: i
                    .expiry
                    .map(|e| format!("best before {}", clock::relative_day(e, state.today)))
                    .unwrap_or_else(|| "no expiry".into()),
                value: idr(i.qty * p.cost),
            })
        })
        .collect();
    let card = DonationCard {
        donation_id: id.clone(),
        title: format!("Donation {id}"),
        lines,
        recipients: state
            .recipients
            .iter()
            .map(|r| crate::contract::Recipient {
                id: r.id.clone(),
                name: r.name.clone(),
                kind: if r.kind == "partner_food_bank" {
                    "Partner food bank".into()
                } else {
                    "Your list".into()
                },
            })
            .collect(),
        checklist: CHECKLIST
            .iter()
            .map(|(id, label)| ChecklistItem {
                id: (*id).into(),
                label: (*label).into(),
            })
            .collect(),
        pickup_windows: PICKUP_WINDOWS.iter().map(|w| (*w).into()).collect(),
        token,
        state: CardState::Open,
        receipt_href: None,
    };
    // Replace the "no donation pending" opener with the real card.
    state.threads.insert("donation".into(), Vec::new());
    state.push(
        "donation",
        Message::DonationCard {
            id: msg_id.clone(),
            author: Author::Agent,
            time: "Just now".into(),
            card,
        },
    );
    state.donations.insert(
        id.clone(),
        Donation {
            id: id.clone(),
            items,
            photo_id,
            state: CardState::Open,
            recipient_id: None,
            pickup_window: None,
            completed_on: None,
            message_id: msg_id,
        },
    );
    Ok(id)
}

pub struct DonationPlan {
    id: String,
    recipient: crate::store::Recipient,
    window: String,
}

fn validate_donation(
    state: &State,
    claims: &ActionClaims,
    body: &ActionBody,
) -> Result<DonationPlan, AppError> {
    let d = state
        .donations
        .get(&claims.sug)
        .ok_or(AppError::TokenInvalid)?;
    if d.state != CardState::Open {
        return Err(AppError::TokenUsed);
    }
    // FR-48: the checklist is a hard gate, every box.
    let missing: Vec<&str> = CHECKLIST
        .iter()
        .filter(|(k, _)| !body.checklist.get(*k).copied().unwrap_or(false))
        .map(|(_, label)| *label)
        .collect();
    if !missing.is_empty() {
        return Err(AppError::BadRequest(format!(
            "food-safety checklist incomplete: {}",
            missing.join("; ")
        )));
    }
    // UU 8/1999: nothing past its date leaves as a donation, checked against
    // the ledger at the moment of donating, not when the photo was taken.
    for i in &d.items {
        if i.expiry.is_some_and(|e| e < state.today) {
            return Err(AppError::BadRequest(format!(
                "{} is past its date and can't be donated",
                i.product_id
            )));
        }
    }
    let recipient = state
        .recipients
        .iter()
        .find(|r| Some(&r.id) == body.recipient_id.as_ref())
        .cloned()
        .ok_or_else(|| AppError::BadRequest("choose a recipient".into()))?;
    let window = body
        .pickup_window
        .as_deref()
        .filter(|w| PICKUP_WINDOWS.contains(w))
        .ok_or_else(|| AppError::BadRequest("choose a pickup window".into()))?;
    Ok(DonationPlan {
        id: d.id.clone(),
        recipient,
        window: window.to_owned(),
    })
}

fn apply_donation(state: &mut State, _claims: &ActionClaims, plan: DonationPlan) -> ResultCard {
    let today = state.today;
    let d = state.donations[&plan.id].clone();
    let mut lines = Vec::new();
    let mut value = 0;
    for i in &d.items {
        let Some(p) = state.data.product(&i.product_id).cloned() else {
            continue;
        };
        // TODO(worksh.io): adjustStock with reason dest:donated.
        let taken = state.data.take(&p.id, i.lot_id.as_deref(), i.qty);
        let v = taken * p.cost;
        value += v;
        let id = state.next_id("w");
        state.waste.push(WasteRecord {
            id,
            date: today,
            product_id: p.id.clone(),
            product_name: p.name.clone(),
            lot_id: i.lot_id.clone(),
            qty: taken,
            unit: p.unit.clone(),
            destination: Destination::Donated,
            reason: "dest:donated".into(),
            value_at_cost: v,
            grams: taken * p.weight_gram,
            photo_id: d.photo_id.clone(),
        });
        lines.push(json!({ "product_id": p.id, "lot_id": i.lot_id, "qty": taken, "expiry_date": i.expiry.map(|e| e.to_string()), "value_at_cost_idr": v }));
    }
    if let Some(x) = state.donations.get_mut(&plan.id) {
        x.state = CardState::Approved;
        x.recipient_id = Some(plan.recipient.id.clone());
        x.pickup_window = Some(plan.window.clone());
        x.completed_on = Some(today);
    }
    let receipt = format!("/donations/{}/receipt.pdf", plan.id);
    if let Some(msgs) = state.threads.get_mut("donation") {
        for m in msgs.iter_mut() {
            if let Message::DonationCard { card, .. } = m
                && card.donation_id == plan.id
            {
                card.state = CardState::Approved;
                card.receipt_href = Some(receipt.clone());
            }
        }
    }
    let message = format!(
        "Hello {}, {} has a donation ready ({}), pickup {}. Items: {}.",
        plan.recipient.name,
        state.data.name,
        plan.id,
        plan.window,
        d.items
            .iter()
            .filter_map(|i| state
                .data
                .product(&i.product_id)
                .map(|p| qty(i.qty, &p.unit) + " " + &p.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let card = ResultCard {
        title: format!("{} confirmed", plan.id),
        detail: format!(
            "{} · pickup {} · {} at cost",
            plan.recipient.name,
            plan.window,
            idr(value)
        ),
        wa_url: Some(format!(
            "https://wa.me/{}?text={}",
            plan.recipient.whatsapp,
            urlencode(&message)
        )),
        links: vec![Link {
            label: "Receipt (PDF)".into(),
            href: receipt,
        }],
    };
    let n = state.next_id("m");
    state.push("donation", note(n, "Checklist passed"));
    let r = state.next_id("m");
    state.push(
        "donation",
        Message::ResultCard {
            id: r,
            author: Author::Agent,
            time: "Just now".into(),
            card: card.clone(),
        },
    );
    state.emit(
        "passtok.donation.completed",
        json!({
            "donation_id": plan.id,
            "recipient": { "type": plan.recipient.kind, "id": plan.recipient.id },
            "lines": lines,
            "safety_checklist": { "before_expiry": true, "storage_ok": true, "sealed": true, "allergen_label": true },
            "pickup_window": plan.window,
            "receipt_uri": format!("receipts/{}.pdf", plan.id),
        }),
    );
    state.record(
        "donate",
        &plan.id,
        json!({ "recipient": plan.recipient.id }),
        "completed",
    );
    card
}

/// Percent-encoding for a query string.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn condition_label(c: Condition) -> &'static str {
    match c {
        Condition::Good => "still good",
        Condition::PastDate => "past date",
        Condition::Spoiled => "spoiled",
        Condition::Damaged => "damaged",
    }
}
