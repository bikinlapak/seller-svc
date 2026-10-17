//! Sample data for Jasa Coffee Shop.
//!
//! This is a **stand-in**, not a source of truth. worksh.io owns products,
//! stock, lots and POs, and the `passtok` schema owns threads and
//! suggestions; neither is wired up yet (see docs/plan/05-phases.md, Phases 2
//! and 6). Until then this module fills the exact same response shapes, so
//! seller-app can be built and tested against the real contract.
//!
//! The content mirrors the design canvas.

use crate::AppError;
use crate::contract::*;
use crate::token::{ActionKeys, suggestion_claims};

/// Demo tenant. Will come from the logged-in merchant's session.
pub const TENANT: &str = "jasa-coffee-shop";

pub fn shop() -> Shop {
    Shop {
        name: "Jasa Coffee Shop".into(),
        owner: "Rima".into(),
        greeting: "Morning, Rima.".into(),
    }
}

pub fn home() -> AssistantHome {
    AssistantHome {
        shop: shop(),
        attention: vec![
            Attention {
                id: "a1".into(),
                title: "Arabica beans run out Thursday".into(),
                detail: "Suggested: order 12 kg".into(),
                icon: AttentionIcon::Stock,
                href: "/assistant/reorder".into(),
            },
            Attention {
                id: "a2".into(),
                title: "14 croissants expire tomorrow".into(),
                detail: "8 are unlikely to sell".into(),
                icon: AttentionIcon::Expiry,
                href: "/assistant/expiry".into(),
            },
            Attention {
                id: "a3".into(),
                title: "Donation pickup 21.30".into(),
                detail: "[PARTNER FOOD BANK]".into(),
                icon: AttentionIcon::Donation,
                href: "/assistant/donation".into(),
            },
        ],
        commands: vec![
            command("Add a product from a photo", "product"),
            command("How much oat milk is left?", "stock"),
            command("Photograph today's leftovers", "waste"),
            command("Menu ideas from stock", "special"),
            command("This week's report", "report"),
            command("Change opening hours", "shop"),
        ],
    }
}

fn command(label: &str, topic: &str) -> QuickCommand {
    QuickCommand {
        label: label.into(),
        href: format!("/assistant/{topic}"),
    }
}

/// Known thread topics. Also used to validate the path.
pub const TOPICS: [&str; 9] = [
    "reorder", "waste", "product", "stock", "expiry", "donation", "special", "report", "shop",
];

pub fn thread(topic: &str, keys: &ActionKeys) -> Result<Option<Thread>, AppError> {
    let t = match topic {
        "reorder" => reorder_thread(keys)?,
        "waste" => waste_thread(),
        "product" => brief(
            "Add a product from a photo",
            "Send me a photo and I'll draft the name, category and description. Nothing is saved until you approve it.",
        ),
        "stock" => brief(
            "Check and adjust stock",
            "Oat Milk 1 L is down to 5 L in the main store, across 2 lots. Want me to record an adjustment?",
        ),
        "expiry" => brief(
            "Expiring soon",
            "14 croissants expire tomorrow and 8 are unlikely to sell. Shall I set a stepped discount from 17.00?",
        ),
        "donation" => brief(
            "Donation",
            "[PARTNER FOOD BANK] can collect at 21.30. Before we go on, there's a food-safety checklist for you to tick.",
        ),
        "special" => brief(
            "Today's special from stock",
            "From what's on hand: croissants and oat milk make 6 portions of pudding. Price Rp18,000, recovering Rp108,000.",
        ),
        "report" => brief(
            "Reports and impact",
            "This week: Rp12.4M revenue, Rp3.1M profit. Across 9 waste photos, 78% was diverted — up from 61% last week.",
        ),
        "shop" => brief(
            "Shop settings",
            "You're open 07.00–21.00. Which day would you like to change?",
        ),
        _ => return Ok(None),
    };
    Ok(Some(t))
}

fn brief(topic: &str, opening: &str) -> Thread {
    Thread {
        topic: topic.into(),
        shop: shop(),
        source: None,
        messages: vec![Message::Text {
            id: "a1".into(),
            author: Author::Agent,
            time: "Just now".into(),
            text: opening.into(),
            actions: None,
        }],
    }
}

/// B3 · Reorder suggestion → PO. The demo path.
///
/// Tokens are signed here because the agent does not exist yet. Once Phase 6
/// lands, the agent signs once when it writes the suggestion and the token is
/// stored with the message — not minted afresh on every read of the thread.
fn reorder_thread(keys: &ActionKeys) -> Result<Thread, AppError> {
    let full = keys.sign(&suggestion_claims(TENANT, "sug-0771", 12, "kg"))?;
    let partial = keys.sign(&suggestion_claims(TENANT, "sug-0771", 9, "kg"))?;

    Ok(Thread {
        topic: "Reorder suggestion".into(),
        shop: shop(),
        source: None,
        messages: vec![
            Message::SuggestionCard {
                id: "m1".into(),
                author: Author::Agent,
                time: "Monday 07.00".into(),
                card: SuggestionCard {
                    label: "Reorder suggestion".into(),
                    confidence: "High confidence".into(),
                    title: "Arabica Beans 1 kg · order 12 kg".into(),
                    reason: "4 kg on hand, selling 1.6 kg a day, out by Thursday. Sumber Kopi delivers in 2 days, arriving Wednesday."
                        .into(),
                    quantity: 12,
                    unit: "kg".into(),
                    token: full,
                },
            },
            Message::Text {
                id: "m2".into(),
                author: Author::Staff,
                time: "07.03".into(),
                text: "why 12? we're closed two days next week".into(),
                actions: None,
            },
            Message::Text {
                id: "m3".into(),
                author: Author::Agent,
                time: "07.04".into(),
                text: "12 kg covers 7 days plus a buffer. Closed two days, 9 kg is enough. I've noted the closure for next time."
                    .into(),
                actions: Some(vec![
                    InlineAction {
                        label: "Order 9 kg".into(),
                        token: Some(partial),
                        primary: true,
                    },
                    InlineAction {
                        label: "Keep 12 kg".into(),
                        token: None,
                        primary: false,
                    },
                ]),
            },
        ],
    })
}

/// B5 · Waste photo → AI vision → destination.
fn waste_thread() -> Thread {
    Thread {
        topic: "Waste photo".into(),
        shop: shop(),
        source: Some("Gemini vision".into()),
        messages: vec![
            Message::Photo {
                id: "w1".into(),
                author: Author::Staff,
                time: "21.10".into(),
                photo_label: "[PHOTO OF LEFTOVER TRAY]".into(),
                boxes: vec![
                    DetectionBox {
                        x: 5.0,
                        y: 15.0,
                        w: 41.0,
                        h: 47.0,
                        confident: true,
                    },
                    DetectionBox {
                        x: 53.0,
                        y: 20.0,
                        w: 39.0,
                        h: 57.0,
                        confident: true,
                    },
                    DetectionBox {
                        x: 12.0,
                        y: 67.0,
                        w: 32.0,
                        h: 23.0,
                        confident: false,
                    },
                ],
            },
            Message::WasteCard {
                id: "w2".into(),
                author: Author::Agent,
                time: "21.11".into(),
                card: WasteCard {
                    label: "Today's leftovers · 3 items".into(),
                    value: "~Rp167k".into(),
                    items: vec![
                        WasteItem {
                            id: "i1".into(),
                            name: "Butter Croissant".into(),
                            quantity: 3,
                            unit: "pcs".into(),
                            severity: Severity::Ok,
                            severity_label: "still good · 96%".into(),
                            destinations: vec![
                                destination("donation", "Donate", true),
                                destination("special", "Today's special", false),
                                destination("staff", "Staff meal", false),
                            ],
                            warning: None,
                        },
                        WasteItem {
                            id: "i2".into(),
                            name: "Oat Milk 1 L".into(),
                            quantity: 2,
                            unit: "L".into(),
                            severity: Severity::Critical,
                            severity_label: "past date · 92%".into(),
                            // Past its date: selling and donating are not
                            // offered at all, rather than merely disabled.
                            destinations: vec![
                                destination("compost", "Compost / larvae", true),
                                destination("discard", "Discard", false),
                            ],
                            warning: Some("Past its date: it cannot be sold or donated.".into()),
                        },
                    ],
                    uncertain: Some(UncertainItem {
                        confidence: 58,
                        candidates: vec![
                            Candidate {
                                id: "c1".into(),
                                name: "Wholemeal Bread".into(),
                            },
                            Candidate {
                                id: "c2".into(),
                                name: "Milk Bun".into(),
                            },
                        ],
                    }),
                    note: "Photo kept as evidence · location data removed".into(),
                },
            },
        ],
    }
}

fn destination(id: &str, label: &str, selected: bool) -> Destination {
    Destination {
        id: id.into(),
        label: label.into(),
        selected,
    }
}

pub fn products() -> ProductList {
    ProductList {
        total: 24,
        categories: ["All", "Drinks", "Bakery", "Ingredients", "Drafts"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        products: vec![
            Product {
                id: "p1".into(),
                name: "Palm Sugar Latte 250 ml".into(),
                price: Some(35_000),
                price_label: Some("Rp35,000".into()),
                subtitle: None,
                category: "Drinks".into(),
                stock_label: "20 in stock".into(),
                stock_severity: Severity::Ok,
                status: "New".into(),
                status_important: false,
                photo_colour: "#EAD9C2".into(),
            },
            Product {
                id: "p2".into(),
                name: "Butter Croissant (box of 4)".into(),
                price: Some(60_000),
                price_label: Some("Rp60,000".into()),
                subtitle: None,
                category: "Bakery".into(),
                stock_label: "14 · exp. tomorrow".into(),
                stock_severity: Severity::Warn,
                status: "Discount 17.00".into(),
                status_important: true,
                photo_colour: "#E8DCC8".into(),
            },
            Product {
                id: "p3".into(),
                name: "Arabica Beans 1 kg".into(),
                price: Some(210_000),
                price_label: Some("Rp210,000".into()),
                subtitle: None,
                category: "Ingredients".into(),
                stock_label: "4 kg".into(),
                stock_severity: Severity::Critical,
                status: "PO sent".into(),
                status_important: false,
                photo_colour: "#DCCFBD".into(),
            },
            Product {
                id: "p4".into(),
                name: "Oat Milk 1 L".into(),
                price: None,
                price_label: None,
                subtitle: Some("Ingredient · not for sale".into()),
                category: "Ingredients".into(),
                stock_label: "5 L".into(),
                stock_severity: Severity::Warn,
                status: "2 lots".into(),
                status_important: false,
                photo_colour: "#E6E2D6".into(),
            },
            Product {
                id: "p5".into(),
                name: "Palm Sugar Doughnut".into(),
                price: Some(15_000),
                price_label: Some("Rp15,000".into()),
                subtitle: None,
                category: "Bakery".into(),
                stock_label: "12 in stock".into(),
                stock_severity: Severity::Ok,
                status: "Active".into(),
                status_important: false,
                photo_colour: "#EFE0CB".into(),
            },
        ],
    }
}

pub fn stock() -> StockList {
    StockList {
        location: "Main store".into(),
        summary: StockSummary {
            value: "Rp6.8M".into(),
            low_count: 3,
            expiring_count: 2,
        },
        rows: vec![
            StockRow {
                id: "s1".into(),
                name: "Arabica Beans 1 kg".into(),
                quantity_label: "4 kg".into(),
                percent: 18,
                severity: Severity::Critical,
                left: "Out Thursday · 2.5 days".into(),
                right: "PO-0771 arrives Wednesday".into(),
                right_important: true,
            },
            StockRow {
                id: "s2".into(),
                name: "Oat Milk 1 L".into(),
                quantity_label: "5 L".into(),
                percent: 30,
                severity: Severity::Warn,
                left: "3 L expire Thursday".into(),
                right: "2 lots".into(),
                right_important: false,
            },
            StockRow {
                id: "s3".into(),
                name: "Butter Croissant".into(),
                quantity_label: "14 boxes".into(),
                percent: 70,
                severity: Severity::Warn,
                left: "Expires tomorrow · discount 17.00".into(),
                right: "lot L-889".into(),
                right_important: false,
            },
            StockRow {
                id: "s4".into(),
                name: "Palm Sugar 1 kg".into(),
                quantity_label: "9 kg".into(),
                percent: 62,
                severity: Severity::Ok,
                left: "Fine · 11 days".into(),
                right: "auto-approved".into(),
                right_important: false,
            },
        ],
    }
}

pub fn ai_settings() -> AiSettings {
    AiSettings {
        modes: vec![
            AssistantMode {
                id: "frugal".into(),
                label: "Frugal".into(),
                note: "short answers, small model".into(),
            },
            AssistantMode {
                id: "balanced".into(),
                label: "Balanced".into(),
                note: "default".into(),
            },
            AssistantMode {
                id: "thorough".into(),
                label: "Thorough".into(),
                note: "longer reasoning".into(),
            },
        ],
        selected_mode: "balanced".into(),
        models: vec![
            model("Chat and explanations", None, "Gemini Flash", false),
            model("Reading product and waste photos", None, "Gemini Pro", false),
            model("Menu ideas and listings", None, "Gemini Flash", false),
            // Marked as a system number: it comes from TimesFM via BigQuery,
            // not from a language model. The merchant deserves to know which
            // figures a chat model did not invent.
            model(
                "Stock forecasting",
                Some("computed by the system, not by chat"),
                "TimesFM",
                true,
            ),
        ],
        autonomy: vec![
            Autonomy {
                id: "small-po".into(),
                label: "Auto-approve small orders".into(),
                note: Some("≤Rp500k, after 3 approvals with no edits".into()),
                enabled: true,
            },
            Autonomy {
                id: "markdown".into(),
                label: "Auto-discount near expiry".into(),
                note: None,
                enabled: false,
            },
        ],
        language: LanguageSetting {
            options: ["English", "Indonesia · santai", "Indonesia · formal"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            selected: "English".into(),
        },
        budget: Budget {
            cap_label: "Rp150k/mo".into(),
            used_percent: 25,
            note: "25% used this month. If it runs out, the assistant drops to Frugal.".into(),
        },
        privacy: "Your shop's data is not used to train models. Waste photos are kept for 90 days as evidence."
            .into(),
    }
}

fn model(task: &str, note: Option<&str>, name: &str, system: bool) -> TaskModel {
    TaskModel {
        task: task.into(),
        note: note.map(Into::into),
        name: name.into(),
        system,
    }
}
