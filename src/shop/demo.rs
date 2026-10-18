//! Hangat Bakery: a deterministic demo tenant.
//!
//! 90 days of demand with the patterns a judge would expect from a real
//! bakery (busier weekends, a payday bump, one closed day, a pastry promo
//! week, a few butter stockout days), and today's lots arranged so the
//! morning run produces the demo's stories: bread flour runs out in two days,
//! butter and milk go on one grouped order, this morning's croissants and
//! baguettes are baked beyond what will sell, and a lot of milk is already
//! past its date. Same seed, same numbers, every run — PRD §14 "demo data looks
//! fake" is answered by generating it from realistic patterns and saying so.

use chrono::{Datelike, Duration, NaiveDate};

use super::*;
use crate::engine::waste::{Destination, WasteRecord};

pub const TENANT: &str = "hangat-bakery";
const HISTORY_DAYS: i64 = 90;

/// splitmix64: tiny, deterministic, good enough for demo noise.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct Spec {
    id: &'static str,
    name: &'static str,
    category: Category,
    unit: &'static str,
    sell_price: Option<Idr>,
    cost: Idr,
    track_stock: bool,
    shelf_life_days: Option<u32>,
    supplier: Option<&'static str>,
    pack: i64,
    moq: i64,
    weight_gram: i64,
    /// Mean daily demand on an ordinary weekday.
    base: f64,
    /// Round demand to whole units (bakery, bottles).
    whole: bool,
    colour: &'static str,
    description: &'static str,
}

const SPECS: &[Spec] = &[
    Spec {
        id: "p1",
        name: "Butter Croissant",
        category: Category::Pastries,
        unit: "pcs",
        sell_price: Some(18_000),
        cost: 7_000,
        track_stock: true,
        shelf_life_days: Some(2),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 70,
        base: 20.0,
        whole: true,
        colour: "#E8D3AE",
        description: "Laminated with cultured butter, baked every morning.",
    },
    Spec {
        id: "p2",
        name: "Chocolate Croissant",
        category: Category::Pastries,
        unit: "pcs",
        sell_price: Some(22_000),
        cost: 9_000,
        track_stock: true,
        shelf_life_days: Some(2),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 80,
        base: 14.0,
        whole: true,
        colour: "#D9BFA0",
        description: "Croissant dough around two batons of dark chocolate.",
    },
    Spec {
        id: "p3",
        name: "Cinnamon Roll",
        category: Category::Pastries,
        unit: "pcs",
        sell_price: Some(20_000),
        cost: 7_000,
        track_stock: true,
        shelf_life_days: Some(2),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 110,
        base: 12.0,
        whole: true,
        colour: "#E3C9A4",
        description: "Soft roll with cinnamon sugar and a cream-cheese glaze.",
    },
    Spec {
        id: "p4",
        name: "Sourdough Loaf",
        category: Category::Bread,
        unit: "loaf",
        sell_price: Some(45_000),
        cost: 15_000,
        track_stock: true,
        shelf_life_days: Some(3),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 800,
        base: 10.0,
        whole: true,
        colour: "#D6BC94",
        description: "48-hour naturally leavened country loaf.",
    },
    Spec {
        id: "p5",
        name: "Milk Bread",
        category: Category::Bread,
        unit: "loaf",
        sell_price: Some(30_000),
        cost: 10_000,
        track_stock: true,
        shelf_life_days: Some(3),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 450,
        base: 9.0,
        whole: true,
        colour: "#F0E2C8",
        description: "Soft Hokkaido-style pull-apart loaf (roti sobek).",
    },
    Spec {
        id: "p6",
        name: "Baguette",
        category: Category::Bread,
        unit: "pcs",
        sell_price: Some(25_000),
        cost: 8_000,
        track_stock: true,
        shelf_life_days: Some(1),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 250,
        base: 9.0,
        whole: true,
        colour: "#DDBF8E",
        description: "Crisp French baguette, best on the day it is baked.",
    },
    Spec {
        id: "p7",
        name: "Bread Flour",
        category: Category::Ingredients,
        unit: "kg",
        sell_price: None,
        cost: 14_000,
        track_stock: true,
        shelf_life_days: Some(180),
        supplier: Some("sup-tepung"),
        pack: 25,
        moq: 25,
        weight_gram: 1000,
        base: 22.0,
        whole: false,
        colour: "#EFE9DD",
        description: "High-protein bread flour, delivered in 25 kg sacks.",
    },
    Spec {
        id: "p8",
        name: "Butter",
        category: Category::Ingredients,
        unit: "kg",
        sell_price: None,
        cost: 140_000,
        track_stock: true,
        shelf_life_days: Some(30),
        supplier: Some("sup-dairy"),
        pack: 5,
        moq: 5,
        weight_gram: 1000,
        base: 4.5,
        whole: false,
        colour: "#F2E3B3",
        description: "Unsalted butter for lamination, 1 kg blocks in cases of 5.",
    },
    Spec {
        id: "p9",
        name: "Fresh Milk 1 L",
        category: Category::Ingredients,
        unit: "L",
        sell_price: None,
        cost: 22_000,
        track_stock: true,
        shelf_life_days: Some(7),
        supplier: Some("sup-dairy"),
        pack: 6,
        moq: 6,
        weight_gram: 1030,
        base: 6.0,
        whole: false,
        colour: "#F1ECE2",
        description: "Full-cream fresh milk for doughs and custards.",
    },
    Spec {
        id: "p10",
        name: "Eggs (tray of 30)",
        category: Category::Ingredients,
        unit: "tray",
        sell_price: None,
        cost: 60_000,
        track_stock: true,
        shelf_life_days: Some(21),
        supplier: Some("sup-telur"),
        pack: 1,
        moq: 2,
        weight_gram: 1800,
        base: 2.2,
        whole: false,
        colour: "#EBD9B9",
        description: "Free-range eggs, trays of 30.",
    },
    Spec {
        id: "p11",
        name: "Sugar",
        category: Category::Ingredients,
        unit: "kg",
        sell_price: None,
        cost: 17_000,
        track_stock: true,
        shelf_life_days: Some(365),
        supplier: Some("sup-tepung"),
        pack: 1,
        moq: 10,
        weight_gram: 1000,
        base: 3.0,
        whole: false,
        colour: "#F4F1EA",
        description: "Fine white sugar.",
    },
    Spec {
        id: "p12",
        name: "Dark Chocolate",
        category: Category::Ingredients,
        unit: "kg",
        sell_price: None,
        cost: 180_000,
        track_stock: true,
        shelf_life_days: Some(180),
        supplier: Some("sup-kakao"),
        pack: 1,
        moq: 2,
        weight_gram: 1000,
        base: 0.6,
        whole: false,
        colour: "#7A5A43",
        description: "55% dark chocolate batons for pain au chocolat.",
    },
    Spec {
        id: "p13",
        name: "Instant Yeast 500 g",
        category: Category::Ingredients,
        unit: "pack",
        sell_price: None,
        cost: 35_000,
        track_stock: true,
        shelf_life_days: Some(120),
        supplier: Some("sup-tepung"),
        pack: 1,
        moq: 4,
        weight_gram: 500,
        base: 0.4,
        whole: false,
        colour: "#E9E2CF",
        description: "Instant dry yeast, 500 g vacuum packs.",
    },
    Spec {
        id: "p14",
        name: "Paper Bag (pack of 100)",
        category: Category::Supplies,
        unit: "pack",
        sell_price: None,
        cost: 50_000,
        track_stock: true,
        shelf_life_days: None,
        supplier: Some("sup-kemas"),
        pack: 1,
        moq: 2,
        weight_gram: 900,
        base: 0.6,
        whole: false,
        colour: "#EDE7DC",
        description: "Brown paper bags for takeaway bread.",
    },
    Spec {
        id: "p15",
        name: "Hot Chocolate",
        category: Category::Drinks,
        unit: "cup",
        sell_price: Some(28_000),
        cost: 9_000,
        track_stock: false,
        shelf_life_days: None,
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 0,
        base: 15.0,
        whole: true,
        colour: "#C9A98A",
        description: "Made to order with our dark chocolate.",
    },
    Spec {
        id: "p16",
        name: "Cheese Danish",
        category: Category::Pastries,
        unit: "pcs",
        sell_price: Some(24_000),
        cost: 9_000,
        track_stock: false,
        shelf_life_days: Some(2),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 90,
        base: 0.0,
        whole: true,
        colour: "#EAD6A8",
        description: "Draft: recipe still being tested.",
    },
];

fn suppliers() -> Vec<Supplier> {
    let s = |id: &str, name: &str, wa: &str, lead: u32| Supplier {
        id: id.into(),
        name: name.into(),
        whatsapp: wa.into(),
        lead_time_days: lead,
    };
    vec![
        s("sup-tepung", "Tepung Jaya", "6281200000001", 2),
        s("sup-dairy", "Dairy Prima", "6281200000002", 1),
        s("sup-telur", "Telur Segar Farm", "6281200000003", 1),
        s("sup-kakao", "Kakao Nusantara", "6281200000004", 3),
        s("sup-kemas", "Kemasan Jaya", "6281200000005", 4),
    ]
}

/// A bakery is busier at the weekend.
fn weekday_factor(d: NaiveDate) -> f64 {
    [0.9, 0.9, 0.95, 1.0, 1.1, 1.3, 1.2][d.weekday().num_days_from_monday() as usize]
}

/// Indonesian payday: the last week of the month and the 1st.
fn is_payday(d: NaiveDate) -> bool {
    d.day() >= 25 || d.day() == 1
}

pub fn build(today: NaiveDate) -> ShopData {
    let mut rng = Rng(0x00B4_CE11);
    let start = today - Duration::days(HISTORY_DAYS);
    let closed_day = today - Duration::days(40);
    let promo = (today - Duration::days(60))..=(today - Duration::days(54));
    let butter_stockout = (today - Duration::days(20))..=(today - Duration::days(18));

    let products: Vec<Product> = SPECS
        .iter()
        .map(|s| Product {
            id: s.id.into(),
            name: s.name.into(),
            category: s.category,
            unit: s.unit.into(),
            sell_price: s.sell_price,
            cost: s.cost,
            track_stock: s.track_stock,
            perishable: s.shelf_life_days.is_some_and(|d| d <= 30),
            shelf_life_days: s.shelf_life_days,
            supplier_id: s.supplier.map(Into::into),
            pack_size: s.pack,
            min_order_qty: s.moq,
            weight_gram: s.weight_gram,
            draft: s.id == "p16",
            colour: s.colour.into(),
            description: s.description.into(),
        })
        .collect();

    let history = SPECS
        .iter()
        .map(|s| {
            let days = (0..HISTORY_DAYS)
                .map(|i| {
                    let date = start + Duration::days(i);
                    let mut mean = s.base * weekday_factor(date);
                    if is_payday(date) {
                        mean *= 1.15;
                    }
                    if promo.contains(&date) && matches!(s.id, "p1" | "p3") {
                        mean *= 1.4;
                    }
                    let noise = 0.75 + 0.5 * rng.unit();
                    let mut qty = mean * noise;
                    qty = if s.whole {
                        qty.round()
                    } else {
                        (qty * 10.0).round() / 10.0
                    };
                    let closed = date == closed_day;
                    let stockout = s.id == "p8" && butter_stockout.contains(&date);
                    if closed {
                        qty = 0.0;
                    } else if stockout {
                        // Only what was left got sold before the shelf ran dry.
                        qty = (qty * 0.3 * 10.0).round() / 10.0;
                    }
                    DayDemand {
                        date,
                        qty,
                        stockout,
                        closed,
                    }
                })
                .collect();
            (s.id.to_owned(), days)
        })
        .collect();

    let d = |n: i64| today + Duration::days(n);
    let lot = |id: &str, product: &str, qty: i64, expiry: Option<NaiveDate>, received: i64| Lot {
        id: id.into(),
        product_id: product.into(),
        qty,
        expiry,
        received: d(received),
    };
    let lots = vec![
        // Ingredients, bought in.
        lot("L-101", "p7", 40, Some(d(150)), -12),
        lot("L-102", "p8", 6, Some(d(20)), -6),
        lot("L-103", "p9", 2, Some(d(-1)), -8),
        lot("L-104", "p9", 5, Some(d(4)), -3),
        lot("L-105", "p10", 4, Some(d(14)), -5),
        lot("L-106", "p11", 30, Some(d(300)), -20),
        lot("L-107", "p12", 5, Some(d(150)), -15),
        lot("L-108", "p13", 3, Some(d(90)), -10),
        lot("L-109", "p14", 4, None, -25),
        // This morning's bake (and yesterday's loaves).
        lot("B-201", "p1", 50, Some(d(1)), 0),
        lot("B-202", "p2", 20, Some(d(1)), 0),
        lot("B-203", "p3", 16, Some(d(1)), 0),
        lot("B-204", "p4", 8, Some(d(2)), -1),
        lot("B-205", "p5", 10, Some(d(2)), -1),
        lot("B-206", "p6", 14, Some(d(0)), 0),
    ];

    ShopData {
        tenant: TENANT.into(),
        name: "Hangat Bakery".into(),
        owner: "Rima".into(),
        location: "Main store".into(),
        products,
        suppliers: suppliers(),
        lots,
        history,
        closures: Vec::new(),
    }
}

/// Four weeks of past write-offs, recorded before and after the shop started
/// giving every leftover a destination. Older weeks lean on the bin; recent
/// ones on donation and compost — which is the story the diversion score tells.
pub fn past_waste(data: &ShopData, today: NaiveDate) -> Vec<WasteRecord> {
    let mut rng = Rng(0x0005_7A5E);
    let mut out = Vec::new();
    let items = ["p1", "p2", "p3", "p4", "p5", "p6", "p9"];
    for week in 1..=4i64 {
        // Week 4 is the oldest. Landfill share falls from ~70% to ~20%.
        let landfill_share = 0.1 + 0.15 * week as f64;
        for day in 0..7 {
            let date = today - Duration::days(week * 7 - day);
            for pid in items {
                if rng.unit() < 0.55 {
                    continue;
                }
                let Some(p) = data.product(pid) else { continue };
                let qty = 1 + (rng.unit() * 3.0) as i64;
                let roll = rng.unit();
                let dest = if roll < landfill_share {
                    Destination::Landfill
                } else if p.sell_price.is_none() {
                    Destination::BsfOrCompost
                } else if roll < landfill_share + 0.35 {
                    Destination::Donated
                } else if roll < landfill_share + 0.55 {
                    Destination::StaffMeal
                } else {
                    Destination::BsfOrCompost
                };
                out.push(WasteRecord {
                    id: format!("w-{}-{pid}", date.format("%m%d")),
                    date,
                    product_id: pid.into(),
                    product_name: p.name.clone(),
                    lot_id: None,
                    qty,
                    unit: p.unit.clone(),
                    destination: dest,
                    reason: "waste:production_leftover".into(),
                    value_at_cost: qty * p.cost,
                    grams: qty * p.weight_gram,
                    photo_id: (rng.unit() < 0.6)
                        .then(|| format!("ph-past-{}", date.format("%m%d"))),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
    }

    #[test]
    fn same_seed_same_data() {
        let a = build(today());
        let b = build(today());
        assert_eq!(a.history("p7"), b.history("p7"));
    }

    #[test]
    fn history_is_ninety_days_ending_yesterday() {
        let data = build(today());
        let h = data.history("p7");
        assert_eq!(h.len(), 90);
        assert_eq!(h.last().unwrap().date, today() - Duration::days(1));
    }

    #[test]
    fn butter_has_flagged_stockout_days() {
        let data = build(today());
        assert_eq!(data.history("p8").iter().filter(|d| d.stockout).count(), 3);
    }

    #[test]
    fn an_expired_lot_is_not_usable() {
        let data = build(today());
        assert_eq!(data.total_on_hand("p9"), 7);
        assert_eq!(data.usable_on_hand("p9", today()), 5);
    }
}
