//! Response shapes consumed by seller-app.
//!
//! This is the Rust half of `seller-app/src/lib/types/api.ts`. Both are
//! hand-written because the two repos cannot share a schema at compile time;
//! `tests/http.rs` keeps them honest.
//!
//! Everything serialises as camelCase so no mapping is needed in SvelteKit.

use serde::{Deserialize, Serialize};

/// Whole rupiah, no cents. Never a float.
pub type Idr = i64;

/// How urgent something is. Used for both stock cover and expiry, so it is a
/// severity rather than a date state: `Critical` means "act today", whether
/// that is four kilos left or milk past its date.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Ok,
    Warn,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct Shop {
    pub name: String,
    pub owner: String,
    /// Greeting follows local time; the server decides, not the browser.
    pub greeting: String,
}

/* ---------- Assistant home ---------- */

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttentionIcon {
    Stock,
    Expiry,
    Donation,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attention {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub icon: AttentionIcon,
    pub href: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuickCommand {
    pub label: String,
    pub href: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantHome {
    pub shop: Shop,
    pub attention: Vec<Attention>,
    pub commands: Vec<QuickCommand>,
}

/* ---------- Thread ---------- */

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Author {
    Agent,
    Staff,
    System,
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectionBox {
    /// Percentages of the photo, not pixels, so they survive any screen width.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub confident: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InlineAction {
    pub label: String,
    /// `None` = a secondary button with no write behind it.
    pub token: Option<String>,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionCard {
    pub label: String,
    pub confidence: String,
    pub title: String,
    pub reason: String,
    /// The proposed quantity. The action token carries the same cap: the
    /// merchant may revise down, never up.
    pub quantity: i64,
    pub unit: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultCard {
    pub title: String,
    pub detail: String,
    pub wa_url: String,
    pub po_href: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Destination {
    pub id: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasteItem {
    pub id: String,
    pub name: String,
    pub quantity: i64,
    pub unit: String,
    pub severity: Severity,
    pub severity_label: String,
    pub destinations: Vec<Destination>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UncertainItem {
    pub confidence: i64,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasteCard {
    pub label: String,
    pub value: String,
    pub items: Vec<WasteItem>,
    pub uncertain: Option<UncertainItem>,
    pub note: String,
}

/// One line in a thread. `type` picks the component that renders it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Text {
        id: String,
        author: Author,
        time: String,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        actions: Option<Vec<InlineAction>>,
    },
    #[serde(rename_all = "camelCase")]
    Photo {
        id: String,
        author: Author,
        time: String,
        photo_label: String,
        boxes: Vec<DetectionBox>,
    },
    Note {
        id: String,
        author: Author,
        time: String,
        text: String,
    },
    SuggestionCard {
        id: String,
        author: Author,
        time: String,
        card: SuggestionCard,
    },
    ResultCard {
        id: String,
        author: Author,
        time: String,
        card: ResultCard,
    },
    WasteCard {
        id: String,
        author: Author,
        time: String,
        card: WasteCard,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub topic: String,
    pub shop: Shop,
    /// Shown on the agent's meta line, e.g. "Gemini vision".
    pub source: Option<String>,
    pub messages: Vec<Message>,
}

/* ---------- Products ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub id: String,
    pub name: String,
    pub price: Option<Idr>,
    pub price_label: Option<String>,
    pub subtitle: Option<String>,
    pub category: String,
    pub stock_label: String,
    pub stock_severity: Severity,
    pub status: String,
    pub status_important: bool,
    pub photo_colour: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProductList {
    pub total: usize,
    pub categories: Vec<String>,
    pub products: Vec<Product>,
}

/* ---------- Stock ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StockSummary {
    pub value: String,
    pub low_count: usize,
    pub expiring_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRow {
    pub id: String,
    pub name: String,
    pub quantity_label: String,
    /// 0-100. Computed server-side from on-hand against the reorder point.
    pub percent: i64,
    pub severity: Severity,
    pub left: String,
    pub right: String,
    pub right_important: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StockList {
    pub location: String,
    pub summary: StockSummary,
    pub rows: Vec<StockRow>,
}

/* ---------- AI settings ---------- */

#[derive(Debug, Clone, Serialize)]
pub struct AssistantMode {
    pub id: String,
    pub label: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskModel {
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub name: String,
    /// `true` = the number is computed by the system, not by a language model.
    pub system: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Autonomy {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageSetting {
    pub options: Vec<String>,
    pub selected: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub cap_label: String,
    pub used_percent: i64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub modes: Vec<AssistantMode>,
    pub selected_mode: String,
    pub models: Vec<TaskModel>,
    pub autonomy: Vec<Autonomy>,
    pub language: LanguageSetting,
    pub budget: Budget,
    pub privacy: String,
}
