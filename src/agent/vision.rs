//! Waste photo → items (FR-33, FR-34).
//!
//! Two engines behind one door:
//! - [`VisionEngine::Stub`] (default): a deterministic stand-in that "sees"
//!   the shop's at-risk lots. It does not look at the pixels — it exists so the
//!   whole capture → confirm → destination flow runs without a cloud account,
//!   and it says so in the thread's source line.
//! - [`VisionEngine::Gemini`]: Gemini structured output through
//!   [`crate::llm`] (Vertex AI or the Gemini API), to the PRD's contract (§9
//!   "vision output"). Not exercised in CI — no credentials there; any
//!   failure falls back to the stand-in so the photo is never lost.
//!
//! Either way the result is a *proposal*. [`ground`] then keeps only candidate
//! ids the shop supplied (the model may not invent products), and anything
//! below the threshold becomes a "which one is it?" question. Vision never
//! writes; the merchant's tap does.

use std::sync::Arc;

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::json;

use crate::llm::Gemini;

use crate::engine::waste::{Condition, condition_for};
use crate::store::{DetectedItem, State};

/// Below this, the merchant must pick the product.
pub const CONFIDENCE_THRESHOLD: f64 = 0.7;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub product_id: String,
    pub name: String,
    pub unit: String,
    pub lot_id: Option<String>,
    pub lot_qty: i64,
    pub expiry: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Detection {
    pub label: String,
    pub count: i64,
    #[serde(default)]
    pub unit: String,
    pub condition: String,
    pub confidence: f64,
    /// Percentages of the photo: x, y, w, h.
    pub bbox: [f32; 4],
    pub candidate_id: Option<String>,
    #[serde(default)]
    pub top_candidates: Vec<String>,
    /// The individual items behind this detection, one box each, so the
    /// photo shows exactly what was counted. Empty = just `bbox`.
    #[serde(default)]
    pub parts: Vec<[f32; 4]>,
}

impl Detection {
    /// Boxes to draw on the photo.
    pub fn boxes(&self) -> Vec<[f32; 4]> {
        if self.parts.is_empty() {
            vec![self.bbox]
        } else {
            self.parts.clone()
        }
    }
}

/// What a photo is for: a tray of leftovers at closing, or a shelf or
/// storeroom being counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Waste,
    Stock,
}

/// The products the model may match against (FR-34).
///
/// Waste: every tracked product with stock, perishable and nearest expiry
/// first (that is what usually ends up on the tray). Stock: every tracked
/// product, with zero stock too — a delivery may not be booked yet. Offering
/// only perishables made a photo of flour sacks or paper bags match nothing.
pub fn candidates(state: &State, purpose: Purpose) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = state
        .data
        .products
        .iter()
        .filter(|p| p.track_stock)
        .filter_map(|p| {
            let lot = state.data.lots_of(&p.id).into_iter().next();
            if purpose == Purpose::Waste && lot.is_none() {
                return None;
            }
            Some(Candidate {
                product_id: p.id.clone(),
                name: p.name.clone(),
                unit: p.unit.clone(),
                lot_id: lot.map(|l| l.id.clone()),
                lot_qty: match purpose {
                    Purpose::Waste => lot.map_or(0, |l| l.qty),
                    Purpose::Stock => state.data.total_on_hand(&p.id),
                },
                expiry: lot.and_then(|l| l.expiry),
            })
        })
        .collect();
    out.sort_by_key(|c| (c.expiry.is_none(), c.expiry));
    out
}

pub enum VisionEngine {
    Stub,
    Gemini(Arc<Gemini>),
}

impl VisionEngine {
    pub fn new(llm: Option<Arc<Gemini>>) -> Self {
        match llm {
            Some(g) => VisionEngine::Gemini(g),
            None => VisionEngine::Stub,
        }
    }

    pub fn source(&self) -> String {
        match self {
            VisionEngine::Stub => "Vision stand-in (no Gemini configured)".into(),
            VisionEngine::Gemini(g) => format!("Gemini vision · {}", g.model),
        }
    }

    pub async fn analyze(
        &self,
        photo: &[u8],
        content_type: &str,
        candidates: &[Candidate],
        today: NaiveDate,
        purpose: Purpose,
    ) -> Vec<Detection> {
        let fallback = || match purpose {
            Purpose::Waste => stub(candidates, today),
            Purpose::Stock => stub_count(candidates),
        };
        match self {
            VisionEngine::Stub => fallback(),
            VisionEngine::Gemini(g) => {
                match gemini(g, photo, content_type, candidates, purpose).await {
                    Ok(d) => {
                        // What the model saw, so a "nothing found" can be
                        // told apart from "found things we don't sell".
                        tracing::info!(
                            ?purpose,
                            items = d.len(),
                            seen = ?d.iter().map(|x| format!("{}×{} → {}", x.label, x.count, x.candidate_id.as_deref().unwrap_or("?"))).collect::<Vec<_>>(),
                            "gemini vision"
                        );
                        d
                    }
                    Err(e) => {
                        // A failed call must not lose the photo: fall back to
                        // the stand-in so the merchant can still confirm.
                        tracing::error!(error = %e, "gemini vision failed; using stand-in");
                        fallback()
                    }
                }
            }
        }
    }
}

const BOXES: [[f32; 4]; 3] = [
    [5.0, 15.0, 41.0, 47.0],
    [53.0, 20.0, 39.0, 57.0],
    [12.0, 67.0, 32.0, 23.0],
];

/// Stock stand-in: "counts" the three products with the most stock, one of
/// them a unit short, so the variance path is visible without a model.
fn stub_count(candidates: &[Candidate]) -> Vec<Detection> {
    let mut by_stock: Vec<&Candidate> = candidates.iter().filter(|c| c.lot_qty > 0).collect();
    by_stock.sort_by_key(|c| std::cmp::Reverse(c.lot_qty));
    by_stock
        .into_iter()
        .take(3)
        .enumerate()
        .map(|(i, c)| Detection {
            label: c.name.to_lowercase(),
            count: if i == 1 {
                (c.lot_qty - 1).max(0)
            } else {
                c.lot_qty
            },
            unit: c.unit.clone(),
            condition: "production_leftover".into(),
            confidence: 0.9,
            bbox: BOXES[i],
            candidate_id: Some(c.product_id.clone()),
            top_candidates: Vec::new(),
            parts: Vec::new(),
        })
        .collect()
}

/// The stand-in: the two most urgent lots, seen confidently, and one bakery
/// item it is unsure about.
fn stub(candidates: &[Candidate], today: NaiveDate) -> Vec<Detection> {
    let mut out = Vec::new();
    let urgent: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.expiry.is_some_and(|e| (e - today).num_days() <= 1))
        .collect();
    for (i, c) in urgent.iter().take(2).enumerate() {
        let past = c.expiry.is_some_and(|e| e < today);
        out.push(Detection {
            label: c.name.to_lowercase(),
            count: c.lot_qty.min(3),
            unit: c.unit.clone(),
            condition: if past {
                "expired"
            } else {
                "production_leftover"
            }
            .into(),
            confidence: [0.96, 0.92][i],
            bbox: BOXES[i],
            candidate_id: Some(c.product_id.clone()),
            top_candidates: Vec::new(),
            parts: Vec::new(),
        });
    }
    let taken: Vec<&str> = out
        .iter()
        .filter_map(|d| d.candidate_id.as_deref())
        .collect();
    let unsure: Vec<String> = candidates
        .iter()
        .filter(|c| !taken.contains(&c.product_id.as_str()))
        .filter(|c| matches!(c.unit.as_str(), "loaf" | "pcs" | "box"))
        .take(3)
        .map(|c| c.product_id.clone())
        .collect();
    if !unsure.is_empty() {
        out.push(Detection {
            label: "bread".into(),
            count: 1,
            unit: "pcs".into(),
            condition: "production_leftover".into(),
            confidence: 0.58,
            bbox: BOXES[2],
            candidate_id: None,
            top_candidates: unsure,
            parts: Vec::new(),
        });
    }
    out
}

fn parse_condition(s: &str) -> Condition {
    match s {
        "expired" => Condition::PastDate,
        "spoiled" => Condition::Spoiled,
        "damaged" => Condition::Damaged,
        _ => Condition::Good,
    }
}

/// Keep only what the shop supplied. Anything else, or anything below the
/// threshold, becomes a question with at most three candidates.
pub fn ground(
    detections: &[Detection],
    candidates: &[Candidate],
    today: NaiveDate,
    purpose: Purpose,
    mut next_id: impl FnMut() -> String,
) -> Vec<DetectedItem> {
    let known = |id: &str| candidates.iter().find(|c| c.product_id == id);
    detections
        .iter()
        .filter(|d| d.count > 0)
        .map(|d| {
            let matched = d
                .candidate_id
                .as_deref()
                .and_then(known)
                .filter(|_| d.confidence >= CONFIDENCE_THRESHOLD);
            let mut condition = parse_condition(&d.condition);
            if let Some(c) = matched {
                // The ledger's date beats the model's opinion of freshness:
                // past its date is past its date, however good it looks.
                if condition_for(c.expiry, today) == Condition::PastDate {
                    condition = Condition::PastDate;
                }
            }
            let tops: Vec<String> = d
                .top_candidates
                .iter()
                .chain(d.candidate_id.iter())
                .filter(|id| known(id).is_some())
                .fold(Vec::new(), |mut v, id| {
                    if !v.contains(id) {
                        v.push(id.clone());
                    }
                    v
                })
                .into_iter()
                .take(3)
                .collect();
            DetectedItem {
                item_id: next_id(),
                label: d.label.clone(),
                product_id: matched.map(|c| c.product_id.clone()),
                candidates: if matched.is_some() { Vec::new() } else { tops },
                lot_id: matched.and_then(|c| c.lot_id.clone()),
                // Waste can't exceed the lot; a stock count is the physical
                // truth, so it is never capped by the ledger.
                qty: match (purpose, matched) {
                    (Purpose::Waste, Some(c)) => d.count.min(c.lot_qty.max(1)),
                    _ => d.count,
                },
                condition,
                confidence: d.confidence,
            }
        })
        .collect()
}

async fn gemini(
    g: &Gemini,
    photo: &[u8],
    content_type: &str,
    candidates: &[Candidate],
    purpose: Purpose,
) -> Result<Vec<Detection>, crate::llm::LlmError> {
    let list: Vec<_> = candidates
        .iter()
        .map(|c| json!({ "id": c.product_id, "name": c.name, "stock_unit": c.unit }))
        .collect();
    let task = match purpose {
        Purpose::Waste => {
            "You look at a photo of a bakery's leftovers or waste: food or ingredients being \
             thrown away, on a tray, a counter or in a bin. Judge each item's condition."
        }
        Purpose::Stock => {
            "You look at a photo of a bakery's stock: shelves, a fridge or a storeroom. \
             condition is production_leftover unless an item is visibly damaged or spoiled."
        }
    };
    // Counting is done in code from the per-item list: models enumerate
    // objects with boxes far more consistently than they report totals.
    let system = format!(
        "{task} List EVERY individual physical item you can see as its own entry with its \
         own tight box: each sack, bag, carton, block, tray, pastry, loaf or stack is one \
         entry. Never return one entry for a group of items, and never list an item twice. \
         In a stack (egg trays, boxes, sacks, bread crates) every layer is its own entry: \
         count the layers from their visible edges. \
         stock_units is how many of the matched candidate's stock_unit that ONE item is: a \
         25 kg flour sack is 25 when the stock_unit is kg, a 1 kg butter block is 1, a 1 L \
         carton is 1, a tray of eggs is 1 when the stock_unit is tray, a croissant is 1 when \
         the stock_unit is pcs. Match each item to a candidate product id; never \
         invent an id. If an item matches no candidate, still list it with candidate_id null \
         and empty top_candidates. If unsure between candidates, set candidate_id null, \
         confidence below 0.7 and up to 3 ids in top_candidates. box_2d is [ymin, xmin, ymax, \
         xmax] normalised to 0-1000. Only list what is visible; an empty list is right only \
         for a photo with no products at all."
    );
    let prompt = format!(
        "Candidate products at this shop: {}",
        serde_json::Value::Array(list)
    );
    let schema = json!({
        "type": "OBJECT",
        "properties": { "items": { "type": "ARRAY", "items": {
            "type": "OBJECT",
            "properties": {
                "label": {"type": "STRING"},
                "stock_units": {"type": "NUMBER"},
                "condition": {"type": "STRING", "enum": ["expired", "damaged", "spoiled", "production_leftover"]},
                "confidence": {"type": "NUMBER"},
                "box_2d": {"type": "ARRAY", "items": {"type": "INTEGER"}},
                "candidate_id": {"type": "STRING", "nullable": true},
                "top_candidates": {"type": "ARRAY", "items": {"type": "STRING"}}
            },
            "required": ["label", "stock_units", "condition", "confidence", "box_2d"]
        }}},
        "required": ["items"]
    });
    #[derive(Deserialize)]
    struct Out {
        items: Vec<Raw>,
    }
    #[derive(Deserialize)]
    struct Raw {
        label: String,
        #[serde(default = "one")]
        stock_units: f64,
        condition: String,
        confidence: f64,
        #[serde(default)]
        box_2d: Vec<f32>,
        candidate_id: Option<String>,
        #[serde(default)]
        top_candidates: Vec<String>,
    }
    fn one() -> f64 {
        1.0
    }
    let out: Out = g
        .json(
            &system,
            vec![
                json!({ "text": prompt }),
                json!({ "inlineData": { "mimeType": content_type, "data": base64(photo) } }),
            ],
            schema,
        )
        .await?;
    let items: Vec<Item> = out
        .items
        .into_iter()
        .take(80)
        .map(|r| Item {
            label: r.label,
            units: if r.stock_units.is_finite() {
                r.stock_units.clamp(0.0, 50.0)
            } else {
                1.0
            },
            condition: r.condition,
            confidence: r.confidence.clamp(0.0, 1.0),
            bbox: box_2d_to_percent(&r.box_2d),
            candidate_id: r.candidate_id,
            top_candidates: r.top_candidates,
        })
        .collect();
    Ok(aggregate(items))
}

/// One physical item as the model reported it.
#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub units: f64,
    pub condition: String,
    pub confidence: f64,
    pub bbox: [f32; 4],
    pub candidate_id: Option<String>,
    pub top_candidates: Vec<String>,
}

/// From items to one detection per product: the same item listed twice
/// (boxes for the same product overlapping by more than half) counts once;
/// the rest add up, and the total is rounded up to whole stock units.
/// Unmatched items are grouped by their label.
pub fn aggregate(items: Vec<Item>) -> Vec<Detection> {
    let mut unique: Vec<Item> = Vec::new();
    for it in items {
        let repeat = unique.iter().any(|u| {
            u.candidate_id == it.candidate_id
                && (u.candidate_id.is_some() || u.label.eq_ignore_ascii_case(&it.label))
                && iou(&u.bbox, &it.bbox) > 0.5
        });
        if !repeat {
            unique.push(it);
        }
    }
    let rank = |c: &str| match c {
        "expired" => 3,
        "spoiled" => 2,
        "damaged" => 1,
        _ => 0,
    };
    let mut groups: Vec<(String, Vec<Item>)> = Vec::new();
    for it in unique {
        let key = it
            .candidate_id
            .clone()
            .unwrap_or_else(|| format!("?{}", it.label.to_lowercase()));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(it),
            None => groups.push((key, vec![it])),
        }
    }
    groups
        .into_iter()
        .map(|(_, v)| {
            let units: f64 = v.iter().map(|i| i.units).sum();
            let parts: Vec<[f32; 4]> = v.iter().map(|i| i.bbox).collect();
            let worst = v
                .iter()
                .max_by_key(|i| rank(&i.condition))
                .map_or("production_leftover".into(), |i| i.condition.clone());
            let confidence = v.iter().map(|i| i.confidence).sum::<f64>() / v.len() as f64;
            let mut tops: Vec<String> = Vec::new();
            for t in v.iter().flat_map(|i| i.top_candidates.iter()) {
                if !tops.contains(t) {
                    tops.push(t.clone());
                }
            }
            Detection {
                label: v[0].label.clone(),
                count: (units - 1e-6).ceil().max(1.0) as i64,
                unit: String::new(),
                condition: worst,
                confidence,
                bbox: union(&parts),
                candidate_id: v[0].candidate_id.clone(),
                top_candidates: tops,
                parts,
            }
        })
        .collect()
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let ix = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let iy = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    let inter = ix * iy;
    inter / (a[2] * a[3] + b[2] * b[3] - inter)
}

fn union(boxes: &[[f32; 4]]) -> [f32; 4] {
    let x0 = boxes.iter().map(|b| b[0]).fold(f32::MAX, f32::min);
    let y0 = boxes.iter().map(|b| b[1]).fold(f32::MAX, f32::min);
    let x1 = boxes.iter().map(|b| b[0] + b[2]).fold(0.0, f32::max);
    let y1 = boxes.iter().map(|b| b[1] + b[3]).fold(0.0, f32::max);
    [x0, y0, x1 - x0, y1 - y0]
}

/// Gemini's native box, `[ymin, xmin, ymax, xmax]` on 0-1000, to the
/// contract's `[x, y, w, h]` in percent. Anything malformed becomes a box
/// covering the photo rather than a sliver in a corner.
pub fn box_2d_to_percent(b: &[f32]) -> [f32; 4] {
    let [ymin, xmin, ymax, xmax] = match b {
        [a, b, c, d] => [*a, *b, *c, *d].map(|v| (v / 10.0).clamp(0.0, 100.0)),
        _ => return [0.0, 0.0, 100.0, 100.0],
    };
    let (x0, x1) = (xmin.min(xmax), xmin.max(xmax));
    let (y0, y1) = (ymin.min(ymax), ymin.max(ymax));
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
        return [0.0, 0.0, 100.0, 100.0];
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
    }

    fn cand(id: &str, days: i64, unit: &str) -> Candidate {
        Candidate {
            product_id: id.into(),
            name: id.into(),
            unit: unit.into(),
            lot_id: Some(format!("L-{id}")),
            lot_qty: 5,
            expiry: Some(today() + chrono::Duration::days(days)),
        }
    }

    fn det(candidate: Option<&str>, confidence: f64, tops: &[&str]) -> Detection {
        Detection {
            label: "x".into(),
            count: 2,
            unit: "pcs".into(),
            condition: "production_leftover".into(),
            confidence,
            bbox: [0.0; 4],
            candidate_id: candidate.map(Into::into),
            top_candidates: tops.iter().map(|s| (*s).into()).collect(),
            parts: Vec::new(),
        }
    }

    fn ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("i{n}")
        }
    }

    #[test]
    fn an_invented_product_is_never_accepted() {
        let c = [cand("p2", 1, "box")];
        let g = ground(
            &[det(Some("p999"), 0.99, &[])],
            &c,
            today(),
            Purpose::Waste,
            ids(),
        );
        assert_eq!(g[0].product_id, None);
        assert!(g[0].candidates.is_empty());
    }

    #[test]
    fn low_confidence_becomes_a_question_with_at_most_three() {
        let c = [
            cand("a", 1, "pcs"),
            cand("b", 1, "pcs"),
            cand("c", 1, "pcs"),
            cand("d", 1, "pcs"),
        ];
        let g = ground(
            &[det(Some("a"), 0.5, &["b", "c", "d", "zzz"])],
            &c,
            today(),
            Purpose::Waste,
            ids(),
        );
        assert_eq!(g[0].product_id, None);
        assert_eq!(g[0].candidates.len(), 3);
        assert!(!g[0].candidates.contains(&"zzz".to_string()));
    }

    #[test]
    fn the_ledger_date_overrides_the_models_freshness() {
        let c = [cand("p4", -1, "L")];
        let g = ground(
            &[det(Some("p4"), 0.95, &[])],
            &c,
            today(),
            Purpose::Waste,
            ids(),
        );
        assert_eq!(g[0].condition, Condition::PastDate);
    }

    #[test]
    fn stub_sees_the_most_urgent_lots() {
        let c = [
            cand("old", -1, "L"),
            cand("soon", 1, "box"),
            cand("bun", 1, "pcs"),
        ];
        let d = stub(&c, today());
        assert_eq!(d[0].candidate_id.as_deref(), Some("old"));
        assert_eq!(d[0].condition, "expired");
        assert!(d.last().unwrap().confidence < CONFIDENCE_THRESHOLD);
    }

    fn item(id: Option<&str>, units: f64, bbox: [f32; 4]) -> Item {
        Item {
            label: id.unwrap_or("thing").into(),
            units,
            condition: "production_leftover".into(),
            confidence: 0.9,
            bbox,
            candidate_id: id.map(Into::into),
            top_candidates: Vec::new(),
        }
    }

    #[test]
    fn items_are_counted_per_product() {
        // Three flour sacks side by side, four milk cartons.
        let mut items: Vec<Item> = (0..3)
            .map(|i| item(Some("p3"), 1.0, [25.0 + 15.0 * i as f32, 0.0, 14.0, 30.0]))
            .collect();
        items.extend(
            (0..4).map(|i| item(Some("p4"), 1.0, [20.0 + 7.0 * i as f32, 38.0, 6.0, 25.0])),
        );
        let d = aggregate(items);
        let count = |id: &str| {
            d.iter()
                .find(|x| x.candidate_id.as_deref() == Some(id))
                .unwrap()
                .count
        };
        assert_eq!(count("p3"), 3);
        assert_eq!(count("p4"), 4);
        assert_eq!(d[0].parts.len(), 3, "one box per bag on the photo");
    }

    #[test]
    fn the_same_item_listed_twice_counts_once() {
        let d = aggregate(vec![
            item(Some("p3"), 1.0, [25.0, 0.0, 14.0, 30.0]),
            item(Some("p3"), 1.0, [26.0, 1.0, 14.0, 29.0]),
        ]);
        assert_eq!(d[0].count, 1);
    }

    #[test]
    fn fractions_round_up_to_whole_stock_units() {
        // Three loose croissants of a box-of-4 product → 0.75 → 1 box.
        let d = aggregate(
            (0..3)
                .map(|i| item(Some("p2"), 0.25, [10.0 * i as f32, 50.0, 9.0, 10.0]))
                .collect(),
        );
        assert_eq!(d[0].count, 1);
    }

    #[test]
    fn unmatched_items_are_grouped_by_label() {
        let d = aggregate(vec![
            item(None, 1.0, [0.0, 0.0, 10.0, 10.0]),
            item(None, 1.0, [50.0, 50.0, 10.0, 10.0]),
        ]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].count, 2);
    }

    #[test]
    fn gemini_boxes_become_percent_boxes() {
        // ymin 150, xmin 50, ymax 620, xmax 460 → x 5, y 15, w 41, h 47.
        assert_eq!(
            box_2d_to_percent(&[150.0, 50.0, 620.0, 460.0]),
            [5.0, 15.0, 41.0, 47.0]
        );
        assert_eq!(box_2d_to_percent(&[1.0, 2.0]), [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(
            box_2d_to_percent(&[500.0, 500.0, 500.0, 500.0]),
            [0.0, 0.0, 100.0, 100.0]
        );
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
        assert_eq!(base64(b"ab"), "YWI=");
    }
}
