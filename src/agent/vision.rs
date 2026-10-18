//! Waste photo → items (FR-33, FR-34).
//!
//! Two engines behind one door:
//! - [`VisionEngine::Stub`] (default): a deterministic stand-in that "sees"
//!   the shop's at-risk lots. It does not look at the pixels — it exists so the
//!   whole capture → confirm → destination flow runs without a cloud account,
//!   and it says so in the thread's source line.
//! - [`VisionEngine::Gemini`]: Gemini structured output, enabled by
//!   `SELLER_SVC_GEMINI_API_KEY`. Written to the PRD's contract (§9 "vision
//!   output") but **not exercised in tests** — no key in CI.
//!
//! Either way the result is a *proposal*. [`ground`] then keeps only candidate
//! ids the shop supplied (the model may not invent products), and anything
//! below the threshold becomes a "which one is it?" question. Vision never
//! writes; the merchant's tap does.

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::json;

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
}

/// Perishable products with stock, nearest expiry first (FR-34).
pub fn candidates(state: &State) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = state
        .data
        .products
        .iter()
        .filter(|p| p.perishable && p.track_stock)
        .filter_map(|p| {
            let lot = state.data.lots_of(&p.id).into_iter().next()?;
            Some(Candidate {
                product_id: p.id.clone(),
                name: p.name.clone(),
                unit: p.unit.clone(),
                lot_id: Some(lot.id.clone()),
                lot_qty: lot.qty,
                expiry: lot.expiry,
            })
        })
        .collect();
    out.sort_by_key(|c| (c.expiry.is_none(), c.expiry));
    out
}

pub enum VisionEngine {
    Stub,
    Gemini(Gemini),
}

impl VisionEngine {
    pub fn from_config(api_key: Option<&str>, model: &str) -> Self {
        match api_key {
            Some(k) if !k.is_empty() => VisionEngine::Gemini(Gemini {
                http: reqwest::Client::new(),
                key: k.to_owned(),
                model: model.to_owned(),
            }),
            _ => VisionEngine::Stub,
        }
    }

    pub fn source(&self) -> String {
        match self {
            VisionEngine::Stub => "Vision stand-in (no Gemini key)".into(),
            VisionEngine::Gemini(g) => format!("Gemini vision · {}", g.model),
        }
    }

    pub async fn analyze(
        &self,
        photo: &[u8],
        content_type: &str,
        candidates: &[Candidate],
        today: NaiveDate,
    ) -> Vec<Detection> {
        match self {
            VisionEngine::Stub => stub(candidates, today),
            VisionEngine::Gemini(g) => match g.analyze(photo, content_type, candidates).await {
                Ok(d) => d,
                Err(e) => {
                    // A failed call must not lose the photo: fall back to the
                    // stand-in so the merchant can still confirm by hand.
                    tracing::error!(error = %e, "gemini vision failed; using stand-in");
                    stub(candidates, today)
                }
            },
        }
    }
}

const BOXES: [[f32; 4]; 3] = [
    [5.0, 15.0, 41.0, 47.0],
    [53.0, 20.0, 39.0, 57.0],
    [12.0, 67.0, 32.0, 23.0],
];

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
                qty: matched.map_or(d.count, |c| d.count.min(c.lot_qty.max(1))),
                condition,
                confidence: d.confidence,
            }
        })
        .collect()
}

pub struct Gemini {
    http: reqwest::Client,
    key: String,
    model: String,
}

impl Gemini {
    async fn analyze(
        &self,
        photo: &[u8],
        content_type: &str,
        candidates: &[Candidate],
    ) -> Result<Vec<Detection>, String> {
        let list: Vec<_> = candidates
            .iter()
            .map(|c| json!({ "id": c.product_id, "name": c.name, "unit": c.unit }))
            .collect();
        let prompt = format!(
            "You are reading a photo of a café's leftovers at closing. Identify each distinct item, count it, and judge its condition. \
             You may only match items to these candidate products by id: {}. \
             If unsure, set candidate_id to null and list up to 3 likely ids in top_candidates. \
             bbox is [x, y, w, h] in percent of the image. Do not invent products.",
            serde_json::Value::Array(list)
        );
        let schema = json!({
            "type": "OBJECT",
            "properties": { "items": { "type": "ARRAY", "items": {
                "type": "OBJECT",
                "properties": {
                    "label": {"type": "STRING"},
                    "count": {"type": "INTEGER"},
                    "unit": {"type": "STRING"},
                    "condition": {"type": "STRING", "enum": ["expired", "damaged", "spoiled", "production_leftover"]},
                    "confidence": {"type": "NUMBER"},
                    "bbox": {"type": "ARRAY", "items": {"type": "NUMBER"}},
                    "candidate_id": {"type": "STRING", "nullable": true},
                    "top_candidates": {"type": "ARRAY", "items": {"type": "STRING"}}
                },
                "required": ["label", "count", "condition", "confidence", "bbox"]
            }}},
            "required": ["items"]
        });
        let body = json!({
            "contents": [{ "parts": [
                { "text": prompt },
                { "inline_data": { "mime_type": content_type, "data": base64(photo) } }
            ]}],
            "generationConfig": { "responseMimeType": "application/json", "responseSchema": schema }
        });
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );
        let res: serde_json::Value = self
            .http
            .post(url)
            .header("x-goog-api-key", &self.key)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let text = res["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or("no text in response")?;
        #[derive(Deserialize)]
        struct Out {
            items: Vec<Detection>,
        }
        serde_json::from_str::<Out>(text)
            .map(|o| o.items)
            .map_err(|e| e.to_string())
    }
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
        let g = ground(&[det(Some("p999"), 0.99, &[])], &c, today(), ids());
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
            ids(),
        );
        assert_eq!(g[0].product_id, None);
        assert_eq!(g[0].candidates.len(), 3);
        assert!(!g[0].candidates.contains(&"zzz".to_string()));
    }

    #[test]
    fn the_ledger_date_overrides_the_models_freshness() {
        let c = [cand("p4", -1, "L")];
        let g = ground(&[det(Some("p4"), 0.95, &[])], &c, today(), ids());
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

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
        assert_eq!(base64(b"ab"), "YWI=");
    }
}
