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
    Special,
    Waste,
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
    /// What the button submits with its token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// Lifecycle of any card that carries an action token.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CardState {
    Open,
    Approved,
    Rejected,
    /// Superseded by a newer suggestion, e.g. after a stock count.
    Replaced,
}

/// One product on a multi-line card (a PO grouped by supplier, FR-13).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardLine {
    pub product_id: String,
    pub name: String,
    /// Proposed quantity; also the cap for this line.
    pub quantity: i64,
    pub unit: String,
    pub note: String,
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
    pub state: CardState,
    /// Primary button text: "Approve", "Start discount", "Make it".
    pub approve_label: String,
    /// Whether "Change quantity" is offered.
    pub editable: bool,
    /// Several products in one card. Empty = the single `quantity` above.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<CardLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub label: String,
    pub href: String,
}

/// The card shown after the executor has written.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultCard {
    pub title: String,
    pub detail: String,
    /// Prefilled `wa.me` message to the supplier or recipient, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wa_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
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
    pub product_id: String,
    pub name: String,
    pub quantity: i64,
    /// Upper bound for the editable count: what the ledger says is there.
    pub max_quantity: i64,
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
#[serde(rename_all = "camelCase")]
pub struct UncertainItem {
    pub item_id: String,
    /// What the model saw, e.g. "bread loaf".
    pub label: String,
    pub quantity: i64,
    pub unit: String,
    pub confidence: i64,
    pub candidates: Vec<Candidate>,
    /// Offered once the merchant has said what it is.
    pub destinations: Vec<Destination>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasteCard {
    pub label: String,
    pub value: String,
    pub items: Vec<WasteItem>,
    pub uncertain: Option<UncertainItem>,
    pub note: String,
    pub token: String,
    pub state: CardState,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChecklistItem {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DonationLine {
    pub name: String,
    pub quantity: i64,
    pub unit: String,
    pub expiry: String,
    pub value: String,
}

/// Donation with the food-safety checklist as a hard gate (FR-48).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DonationCard {
    pub donation_id: String,
    pub title: String,
    pub lines: Vec<DonationLine>,
    pub recipients: Vec<Recipient>,
    pub checklist: Vec<ChecklistItem>,
    pub pickup_windows: Vec<String>,
    pub token: String,
    pub state: CardState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_href: Option<String>,
}

/// One product in a stock count: what the photo shows against the system,
/// and what that means for ordering. All figures from the engine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountLine {
    pub product_id: String,
    pub name: String,
    pub unit: String,
    /// Counted from the photo; the merchant can correct it.
    pub counted: i64,
    /// What the ledger says is on hand.
    pub system: i64,
    pub difference_label: String,
    pub cover_label: String,
    /// "Order 12 kg · runs out Tuesday" / "Enough · lasts until Friday".
    pub need_label: String,
    pub severity: Severity,
}

/// Something the model saw but could not place with confidence.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsureCount {
    pub item_id: String,
    pub label: String,
    pub quantity: i64,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountCard {
    pub label: String,
    pub lines: Vec<CountLine>,
    pub unsure: Vec<UnsureCount>,
    /// Seen but not in the catalog, e.g. "stand mixer ×1".
    pub unmatched: Vec<String>,
    pub note: String,
    pub token: String,
    pub state: CardState,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        photo_url: Option<String>,
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
    DonationCard {
        id: String,
        author: Author,
        time: String,
        card: DonationCard,
    },
    CountCard {
        id: String,
        author: Author,
        time: String,
        card: CountCard,
    },
}

impl Message {
    pub fn id(&self) -> &str {
        match self {
            Message::Text { id, .. }
            | Message::Photo { id, .. }
            | Message::Note { id, .. }
            | Message::SuggestionCard { id, .. }
            | Message::ResultCard { id, .. }
            | Message::WasteCard { id, .. }
            | Message::DonationCard { id, .. }
            | Message::CountCard { id, .. } => id,
        }
    }
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
    /// `low`, `expiring`, `on_order`: what the filters and the summary
    /// tiles count, so the two can never disagree.
    pub tags: Vec<String>,
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

/* ---------- Product detail ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LotView {
    pub id: String,
    pub quantity: i64,
    pub unit: String,
    /// "expires tomorrow", "past date", "no expiry".
    pub expiry_label: String,
    pub severity: Severity,
    pub at_risk: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPoint {
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forecast: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper: Option<f64>,
    /// Stockout or closed day: not real demand.
    pub censored: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDetail {
    pub product: Product,
    pub description: String,
    pub unit: String,
    pub facts: Vec<Fact>,
    pub lots: Vec<LotView>,
    /// Last 28 days actual, next 14 days forecast with upper bound.
    pub series: Vec<SeriesPoint>,
    pub forecast_note: String,
}

/* ---------- Dashboard (FR-23, FR-24) ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    pub label: String,
    pub value: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtRiskLot {
    pub product_id: String,
    pub name: String,
    pub lot_id: String,
    pub at_risk: String,
    pub expiry_label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductSeries {
    pub product_id: String,
    pub name: String,
    pub unit: String,
    pub accuracy: String,
    pub series: Vec<SeriesPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub impact: Vec<Stat>,
    pub open_suggestions: usize,
    pub at_risk: Vec<AtRiskLot>,
    pub series: Vec<ProductSeries>,
    pub method_note: String,
}

/* ---------- Waste report (FR-54) ---------- */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationShare {
    pub id: String,
    pub label: String,
    pub value: String,
    pub weight: String,
    /// 0-100 share of total value.
    pub percent: i64,
    pub landfill: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WasteReport {
    pub period: String,
    /// 0-100, `None` when nothing was recorded.
    pub diversion_percent: Option<i64>,
    pub previous_percent: Option<i64>,
    pub total_value: String,
    pub total_weight: String,
    pub photos: usize,
    pub by_destination: Vec<DestinationShare>,
    pub recent: Vec<Fact>,
    pub method: String,
}

/* ---------- Settings update ---------- */

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdate {
    pub selected_mode: Option<String>,
    #[serde(default)]
    pub autonomy: std::collections::HashMap<String, bool>,
    pub language: Option<String>,
}
