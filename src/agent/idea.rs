//! Today's special from Gemini (FR-46), replacing the recipe-library idea
//! when a model is configured.
//!
//! The model proposes a dish from the shop's own stock: a name, a method, a
//! price and per-portion ingredient lines, using only product ids it was
//! given. Everything it says is then checked and sized by the engine
//! ([`special::evaluate`]): unknown products, an empty main ingredient, a
//! silly quantity or price, and the idea is discarded and the library one
//! stays. Portions, ingredients used and rupiah recovered are never the
//! model's numbers.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;

use crate::AppError;
use crate::agent::{analysis, at_risk_map, explain, sign};
use crate::contract::{CardState, Message};
use crate::engine::special::{self, Special};
use crate::llm::Gemini;
use crate::store::{Store, SuggestionKind};
use crate::token::ActionKeys;

/// (product id, name, unit) of everything in stock.
type Catalog = Vec<(String, String, String)>;

#[derive(Debug, Deserialize)]
struct Idea {
    name: String,
    method: String,
    price_idr: i64,
    lines: Vec<IdeaLine>,
}

#[derive(Debug, Deserialize)]
struct IdeaLine {
    product_id: String,
    per_portion: f64,
}

/// Replace the open special card's idea with Gemini's, if it survives the
/// checks. Returns whether the card changed.
pub async fn refresh_special(
    store: &Store,
    keys: &ActionKeys,
    g: &Gemini,
) -> Result<bool, AppError> {
    // 1. Gather the inputs under the lock.
    let (sug_id, available, at_risk, catalog) =
        {
            let s = store.lock()?;
            let Some(sug) = s.suggestions.values().find(|x| {
                x.state == CardState::Open && matches!(x.kind, SuggestionKind::Special(_))
            }) else {
                return Ok(false);
            };
            let all = analysis::analyze_all(&s);
            let available: HashMap<String, i64> = all
                .iter()
                .map(|a| (a.product_id.clone(), a.usable))
                .collect();
            let at_risk = at_risk_map(&all, s.today);
            let catalog: Catalog = s
                .data
                .products
                .iter()
                .filter(|p| available.get(&p.id).is_some_and(|n| *n > 0))
                .map(|p| (p.id.clone(), p.name.clone(), p.unit.clone()))
                .collect();
            (sug.id.clone(), available, at_risk, catalog)
        };

    let risky: Vec<_> = catalog
        .iter()
        .filter(|(id, _, _)| at_risk.get(id).is_some_and(|n| *n > 0))
        .map(|(id, name, unit)| json!({"id": id, "name": name, "unit": unit, "at_risk": at_risk[id]}))
        .collect();
    if risky.is_empty() {
        return Ok(false);
    }
    let stock: Vec<_> = catalog
        .iter()
        .map(|(id, name, unit)| json!({"id": id, "name": name, "unit": unit, "available": available[id]}))
        .collect();

    // 2. Ask, outside the lock.
    let system = "You are a baker proposing ONE simple bake or dish for today's menu that uses up \
        ingredients about to expire. Use only the product ids given. The first line must be one \
        of the at-risk items. per_portion is in that product's own unit (e.g. 1 croissant, \
        0.1 of a litre of milk). Suggest a fair bakery price in rupiah. method: one or two \
        short sentences. Do not mention quantities of portions or money in the name or method.";
    let prompt = json!({ "at_risk": risky, "in_stock": stock }).to_string();
    let schema = json!({
        "type": "OBJECT",
        "properties": {
            "name": {"type": "STRING"},
            "method": {"type": "STRING"},
            "price_idr": {"type": "INTEGER"},
            "lines": {"type": "ARRAY", "items": {
                "type": "OBJECT",
                "properties": {"product_id": {"type": "STRING"}, "per_portion": {"type": "NUMBER"}},
                "required": ["product_id", "per_portion"]
            }}
        },
        "required": ["name", "method", "price_idr", "lines"]
    });
    let idea: Idea = match g
        .json(system, vec![json!({ "text": prompt })], schema)
        .await
    {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "gemini special failed; keeping the library idea");
            return Ok(false);
        }
    };

    // 3. Check it, then size it with the engine.
    let Some(sp) = check(&idea, &catalog, &available, &at_risk) else {
        tracing::warn!(?idea, "gemini special rejected by the checks");
        return Ok(false);
    };

    // 4. Swap it into the card if the card is still open.
    let mut s = store.lock()?;
    let Some(sug) = s.suggestions.get(&sug_id).cloned() else {
        return Ok(false);
    };
    if sug.state != CardState::Open {
        return Ok(false);
    }
    let token = sign(&s, keys, "create_special", &sug.id, sp.portions, "portions")?;
    let reason = explain::special_reason(&sp, &s);
    let title = format!("{} · {}", sp.name, crate::fmt::qty(sp.portions, "portion"));
    if let Some(msgs) = s.threads.get_mut(&sug.topic) {
        for m in msgs.iter_mut() {
            if let Message::SuggestionCard { id, card, .. } = m
                && *id == sug.message_id
            {
                card.title = title.clone();
                card.reason = reason.clone();
                card.quantity = sp.portions;
                card.token = token.clone();
                card.confidence = format!("Gemini idea · {}", g.model);
            }
        }
    }
    if let Some(x) = s.suggestions.get_mut(&sug.id) {
        x.kind = SuggestionKind::Special(sp.clone());
    }
    tracing::info!(name = %sp.name, portions = sp.portions, "gemini special adopted");
    drop(s);
    store.touched("special");
    Ok(true)
}

fn check(
    idea: &Idea,
    catalog: &Catalog,
    available: &HashMap<String, i64>,
    at_risk: &HashMap<String, i64>,
) -> Option<Special> {
    let name = idea.name.trim();
    let method = idea.method.trim();
    if name.is_empty() || name.chars().count() > 60 || method.chars().count() > 300 {
        return None;
    }
    if !(5_000..=100_000).contains(&idea.price_idr) || idea.lines.is_empty() || idea.lines.len() > 6
    {
        return None;
    }
    let known = |id: &str| catalog.iter().any(|(c, _, _)| c == id);
    let mut lines: Vec<(String, f64)> = Vec::new();
    for l in &idea.lines {
        // Unknown product, duplicate, or a per-portion amount no bakery uses.
        if !known(&l.product_id)
            || lines.iter().any(|(p, _)| *p == l.product_id)
            || !(l.per_portion > 0.0 && l.per_portion <= 2.0)
        {
            return None;
        }
        lines.push((l.product_id.clone(), l.per_portion));
    }
    // Round the price to Rp1,000 like a menu would.
    let price = (idea.price_idr + 500) / 1_000 * 1_000;
    special::evaluate(name, &lines, price, method, available, at_risk)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps() -> (Catalog, HashMap<String, i64>, HashMap<String, i64>) {
        let catalog: Catalog = vec![
            (
                "p2".into(),
                "Butter Croissant (box of 4)".into(),
                "box".into(),
            ),
            ("p6".into(), "Fresh Milk 1 L".into(), "L".into()),
        ];
        let available = HashMap::from([("p2".into(), 14), ("p6".into(), 4)]);
        let at_risk = HashMap::from([("p2".into(), 3)]);
        (catalog, available, at_risk)
    }

    fn idea(lines: Vec<(&str, f64)>, price: i64) -> Idea {
        Idea {
            name: "Croissant Pudding".into(),
            method: "Bake it.".into(),
            price_idr: price,
            lines: lines
                .into_iter()
                .map(|(p, per)| IdeaLine {
                    product_id: p.into(),
                    per_portion: per,
                })
                .collect(),
        }
    }

    #[test]
    fn a_sound_idea_is_sized_by_the_engine() {
        let (c, a, r) = maps();
        let sp = check(&idea(vec![("p2", 0.25), ("p6", 0.1)], 17_600), &c, &a, &r).unwrap();
        assert_eq!(sp.portions, special::MAX_PORTIONS);
        assert_eq!(sp.price, 18_000, "rounded to the nearest thousand");
        assert_eq!(sp.expected_recovered, sp.portions * 18_000);
    }

    #[test]
    fn an_invented_product_sinks_the_idea() {
        let (c, a, r) = maps();
        assert!(check(&idea(vec![("p2", 0.25), ("p99", 0.1)], 18_000), &c, &a, &r).is_none());
    }

    #[test]
    fn the_main_ingredient_must_be_at_risk() {
        let (c, a, r) = maps();
        assert!(check(&idea(vec![("p6", 0.1), ("p2", 0.25)], 18_000), &c, &a, &r).is_none());
    }

    #[test]
    fn silly_prices_and_amounts_are_refused() {
        let (c, a, r) = maps();
        assert!(check(&idea(vec![("p2", 0.25)], 900_000), &c, &a, &r).is_none());
        assert!(check(&idea(vec![("p2", 5.0)], 18_000), &c, &a, &r).is_none());
        assert!(check(&idea(vec![("p2", 0.0)], 18_000), &c, &a, &r).is_none());
    }
}
