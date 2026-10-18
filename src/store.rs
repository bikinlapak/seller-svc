//! PasStok state for the tenant: threads, suggestions, purchase orders, the
//! waste ledger, donations, settings, the audit log and the event outbox.
//!
//! **In memory, by decision for the hackathon build.** The PRD puts this in
//! Postgres next to the tenant in worksh.io (decision L28); the shapes here
//! are the tables that migration will create (`threads`, `messages`,
//! `suggestions`, `action_tokens`, `outbox`). A restart resets the demo,
//! which is also what the demo kit wants.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use chrono::NaiveDate;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::AppError;
use crate::contract::{CardState, Idr, Message, ResultCard};
use crate::engine::special::Special;
use crate::engine::waste::{Condition, WasteRecord};
use crate::shop::ShopData;

#[derive(Debug, Clone)]
pub struct SuggestedLine {
    pub product_id: String,
    pub qty: i64,
    /// The engine's inputs, kept so any answer can be traced (§11
    /// Explainability) and Q&A is grounded on them, never on the model.
    pub inputs: Value,
}

#[derive(Debug, Clone)]
pub enum SuggestionKind {
    Reorder {
        supplier_id: String,
        lines: Vec<SuggestedLine>,
    },
    Markdown {
        product_id: String,
        lot_id: String,
        qty: i64,
        pct: i64,
    },
    Special(Special),
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub id: String,
    pub topic: String,
    pub message_id: String,
    pub kind: SuggestionKind,
    pub state: CardState,
    /// Event id that produced it: the idempotency key of FR-19.
    pub event_id: String,
}

#[derive(Debug, Clone)]
pub struct PurchaseOrder {
    pub number: String,
    pub supplier_id: String,
    pub lines: Vec<(String, i64)>,
    pub expected: NaiveDate,
    pub suggestion_id: String,
    /// worksh.io lifecycle: created → submitted → approved → issued.
    pub status: &'static str,
}

#[derive(Debug, Clone)]
pub struct Markdown {
    pub product_id: String,
    pub lot_id: String,
    pub qty: i64,
    pub pct: i64,
    pub starts: String,
    pub ends: String,
}

#[derive(Debug, Clone)]
pub struct WorkOrder {
    pub number: String,
    pub product_name: String,
    pub portions: i64,
}

/// One item vision proposed, after grounding against the catalog.
#[derive(Debug, Clone)]
pub struct DetectedItem {
    pub item_id: String,
    pub label: String,
    /// `None` = below the confidence threshold: the merchant must pick.
    pub product_id: Option<String>,
    pub candidates: Vec<String>,
    pub lot_id: Option<String>,
    pub qty: i64,
    pub condition: Condition,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct WasteSession {
    pub id: String,
    pub photo_id: String,
    /// The waste card this session was posted as.
    pub message_id: String,
    pub items: Vec<DetectedItem>,
    pub state: CardState,
}

#[derive(Debug, Clone)]
pub struct DonationItem {
    pub product_id: String,
    pub lot_id: Option<String>,
    pub qty: i64,
    pub expiry: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct Donation {
    pub id: String,
    pub items: Vec<DonationItem>,
    pub photo_id: Option<String>,
    pub state: CardState,
    pub recipient_id: Option<String>,
    pub pickup_window: Option<String>,
    pub completed_on: Option<NaiveDate>,
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct Recipient {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub whatsapp: String,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub mode: String,
    pub autonomy: HashMap<String, bool>,
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: "balanced".into(),
            autonomy: HashMap::from([("small-po".into(), true), ("markdown".into(), false)]),
            language: "English".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub at: String,
    pub actor: String,
    pub action: String,
    pub suggestion: String,
    pub inputs: Value,
    pub outcome: String,
}

#[derive(Debug, Clone)]
pub struct Photo {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

#[derive(Debug, Clone)]
pub struct ModerationReport {
    pub message_id: String,
    pub topic: String,
    pub reason: String,
}

pub struct State {
    pub data: ShopData,
    pub today: NaiveDate,
    pub threads: HashMap<String, Vec<Message>>,
    pub suggestions: HashMap<String, Suggestion>,
    pub orders: Vec<PurchaseOrder>,
    pub markdowns: Vec<Markdown>,
    pub work_orders: Vec<WorkOrder>,
    pub waste: Vec<WasteRecord>,
    pub sessions: HashMap<String, WasteSession>,
    pub donations: HashMap<String, Donation>,
    pub recipients: Vec<Recipient>,
    pub settings: Settings,
    pub audit: Vec<AuditEntry>,
    /// Events as they would go to Pub/Sub (PRD §9 contracts).
    pub outbox: Vec<Value>,
    /// Idempotency-Key → the response it produced (FR-19).
    pub idempotent: HashMap<String, ResultCard>,
    pub photos: HashMap<String, Photo>,
    pub moderation: Vec<ModerationReport>,
    /// Learned order adjustment per product (FR-14, FR-36).
    pub adjust: HashMap<String, f64>,
    seq: u64,
    pub po_seq: u32,
    pub don_seq: u32,
}

impl State {
    pub fn new(data: ShopData, today: NaiveDate) -> Self {
        Self {
            data,
            today,
            threads: HashMap::new(),
            suggestions: HashMap::new(),
            orders: Vec::new(),
            markdowns: Vec::new(),
            work_orders: Vec::new(),
            waste: Vec::new(),
            sessions: HashMap::new(),
            donations: HashMap::new(),
            recipients: vec![
                Recipient {
                    id: "rcp-garda".into(),
                    name: "Garda Pangan food bank".into(),
                    kind: "partner_food_bank",
                    whatsapp: "6281200000100".into(),
                },
                Recipient {
                    id: "rcp-masjid".into(),
                    name: "Al-Ikhlas mosque kitchen".into(),
                    kind: "own_list",
                    whatsapp: "6281200000101".into(),
                },
            ],
            settings: Settings::default(),
            audit: Vec::new(),
            outbox: Vec::new(),
            idempotent: HashMap::new(),
            photos: HashMap::new(),
            moderation: Vec::new(),
            adjust: HashMap::new(),
            seq: 0,
            po_seq: 771,
            don_seq: 13,
        }
    }

    /// Sortable, unique within the process.
    pub fn next_id(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}-{:05}", self.seq)
    }

    pub fn thread_mut(&mut self, topic: &str) -> &mut Vec<Message> {
        self.threads.entry(topic.to_owned()).or_default()
    }

    pub fn push(&mut self, topic: &str, message: Message) {
        self.thread_mut(topic).push(message);
    }

    /// Quantity already ordered and not yet received, per product.
    pub fn on_order(&self, product_id: &str) -> i64 {
        self.orders
            .iter()
            .filter(|o| o.status != "received")
            .flat_map(|o| o.lines.iter())
            .filter(|(p, _)| p == product_id)
            .map(|(_, q)| q)
            .sum()
    }

    pub fn emit(&mut self, topic: &str, mut event: Value) {
        let id = self.next_id("evt");
        if let Some(obj) = event.as_object_mut() {
            obj.insert("event_id".into(), Value::String(id));
            obj.insert("tenant_id".into(), Value::String(self.data.tenant.clone()));
            obj.insert(
                "occurred_at".into(),
                Value::String(crate::clock::now().to_rfc3339()),
            );
            obj.insert("topic".into(), Value::String(topic.to_owned()));
        }
        tracing::info!(topic, "event");
        self.outbox.push(event);
    }

    pub fn record(&mut self, action: &str, suggestion: &str, inputs: Value, outcome: &str) {
        self.audit.push(AuditEntry {
            at: crate::clock::now().to_rfc3339(),
            actor: self.data.owner.clone(),
            action: action.into(),
            suggestion: suggestion.into(),
            inputs,
            outcome: outcome.into(),
        });
    }

    pub fn value_at_cost(&self, product_id: &str, qty: i64) -> Idr {
        self.data.product(product_id).map_or(0, |p| p.cost * qty)
    }
}

/// The state behind one lock, plus a channel announcing which thread changed
/// (feeds the SSE stream, FR-25).
pub struct Store {
    state: Mutex<State>,
    pub changes: broadcast::Sender<String>,
}

impl Store {
    pub fn new(state: State) -> Self {
        let (changes, _) = broadcast::channel(64);
        Self {
            state: Mutex::new(state),
            changes,
        }
    }

    pub fn lock(&self) -> Result<MutexGuard<'_, State>, AppError> {
        self.state
            .lock()
            .map_err(|_| AppError::Config("state lock poisoned".into()))
    }

    pub fn replace(&self, state: State) -> Result<(), AppError> {
        *self.lock()? = state;
        let _ = self.changes.send("*".into());
        Ok(())
    }

    /// Tell stream subscribers a thread changed. No receivers is fine.
    pub fn touched(&self, topic: &str) {
        let _ = self.changes.send(topic.to_owned());
    }
}
