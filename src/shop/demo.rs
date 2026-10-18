//! Jasa Coffee Shop: a deterministic demo tenant.
//!
//! 90 days of demand with the patterns a judge would expect from a real café
//! (busier weekends, a payday bump, one closed day, a promo week, a few
//! oat-milk stockout days), and today's lots arranged so the morning run
//! produces the demo's stories: Arabica runs out Thursday, a croissant lot
//! expires tomorrow with more than will sell, a lot of oat milk is already past
//! its date. Same seed, same numbers, every run — PRD §14 "demo data looks
//! fake" is answered by generating it from realistic patterns and saying so.

use chrono::{Datelike, Duration, NaiveDate};

use super::*;
use crate::engine::waste::{Destination, WasteRecord};

pub const TENANT: &str = "jasa-coffee-shop";
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
        name: "Palm Sugar Latte 250 ml",
        category: Category::Drinks,
        unit: "cup",
        sell_price: Some(35_000),
        cost: 11_000,
        track_stock: false,
        shelf_life_days: None,
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 0,
        base: 38.0,
        whole: true,
        colour: "#EAD9C2",
        description: "Espresso, fresh milk and Banten palm sugar. Made to order.",
    },
    Spec {
        id: "p2",
        name: "Butter Croissant (box of 4)",
        category: Category::Bakery,
        unit: "box",
        sell_price: Some(60_000),
        cost: 36_000,
        track_stock: true,
        shelf_life_days: Some(3),
        supplier: Some("sup-roti"),
        pack: 1,
        moq: 4,
        weight_gram: 240,
        base: 3.0,
        whole: true,
        colour: "#E8DCC8",
        description: "Four all-butter croissants from Roti Pagi, delivered every morning.",
    },
    Spec {
        id: "p3",
        name: "Arabica Beans 1 kg",
        category: Category::Ingredients,
        unit: "kg",
        sell_price: None,
        cost: 210_000,
        track_stock: true,
        shelf_life_days: Some(60),
        supplier: Some("sup-kopi"),
        pack: 1,
        moq: 2,
        weight_gram: 1000,
        base: 1.6,
        whole: false,
        colour: "#DCCFBD",
        description: "Single-origin Gayo arabica, medium roast. The house espresso.",
    },
    Spec {
        id: "p4",
        name: "Oat Milk 1 L",
        category: Category::Ingredients,
        unit: "L",
        sell_price: None,
        cost: 32_000,
        track_stock: true,
        shelf_life_days: Some(10),
        supplier: Some("sup-susu"),
        pack: 6,
        moq: 6,
        weight_gram: 1030,
        base: 1.2,
        whole: false,
        colour: "#E6E2D6",
        description: "Barista oat milk for plant-based drinks.",
    },
    Spec {
        id: "p5",
        name: "Palm Sugar Doughnut",
        category: Category::Bakery,
        unit: "pcs",
        sell_price: Some(15_000),
        cost: 6_000,
        track_stock: true,
        shelf_life_days: Some(2),
        supplier: Some("sup-roti"),
        pack: 6,
        moq: 6,
        weight_gram: 70,
        base: 8.0,
        whole: true,
        colour: "#EFE0CB",
        description: "Brioche doughnut glazed with palm sugar caramel.",
    },
    Spec {
        id: "p6",
        name: "Fresh Milk 1 L",
        category: Category::Ingredients,
        unit: "L",
        sell_price: None,
        cost: 22_000,
        track_stock: true,
        shelf_life_days: Some(7),
        supplier: Some("sup-susu"),
        pack: 6,
        moq: 6,
        weight_gram: 1030,
        base: 2.6,
        whole: false,
        colour: "#F1ECE2",
        description: "Full-cream fresh milk for lattes.",
    },
    Spec {
        id: "p7",
        name: "Palm Sugar 1 kg",
        category: Category::Ingredients,
        unit: "kg",
        sell_price: None,
        cost: 28_000,
        track_stock: true,
        shelf_life_days: Some(180),
        supplier: Some("sup-gula"),
        pack: 1,
        moq: 5,
        weight_gram: 1000,
        base: 0.8,
        whole: false,
        colour: "#D9C3A5",
        description: "Banten palm sugar block, for syrups and glaze.",
    },
    Spec {
        id: "p8",
        name: "Wholemeal Bread",
        category: Category::Bakery,
        unit: "loaf",
        sell_price: Some(25_000),
        cost: 12_000,
        track_stock: true,
        shelf_life_days: Some(3),
        supplier: Some("sup-roti"),
        pack: 1,
        moq: 2,
        weight_gram: 400,
        base: 2.0,
        whole: true,
        colour: "#D8C6A8",
        description: "Wholemeal sourdough loaf.",
    },
    Spec {
        id: "p9",
        name: "Milk Bun",
        category: Category::Bakery,
        unit: "pcs",
        sell_price: Some(12_000),
        cost: 5_000,
        track_stock: true,
        shelf_life_days: Some(2),
        supplier: Some("sup-roti"),
        pack: 6,
        moq: 6,
        weight_gram: 60,
        base: 3.0,
        whole: true,
        colour: "#F2E6D3",
        description: "Soft Hokkaido-style milk bun.",
    },
    Spec {
        id: "p10",
        name: "Cold Brew 250 ml",
        category: Category::Drinks,
        unit: "bottle",
        sell_price: Some(30_000),
        cost: 12_000,
        track_stock: true,
        shelf_life_days: Some(5),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 280,
        base: 4.0,
        whole: true,
        colour: "#CDB89C",
        description: "18-hour cold brew, bottled in house.",
    },
    Spec {
        id: "p11",
        name: "Iced Americano",
        category: Category::Drinks,
        unit: "cup",
        sell_price: Some(25_000),
        cost: 7_000,
        track_stock: false,
        shelf_life_days: None,
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 0,
        base: 22.0,
        whole: true,
        colour: "#E1D2BC",
        description: "Double shot over ice. Made to order.",
    },
    Spec {
        id: "p12",
        name: "Banana Bread Slice",
        category: Category::Bakery,
        unit: "slice",
        sell_price: Some(20_000),
        cost: 8_000,
        track_stock: false,
        shelf_life_days: Some(3),
        supplier: None,
        pack: 1,
        moq: 1,
        weight_gram: 90,
        base: 0.0,
        whole: true,
        colour: "#E4D0B0",
        description: "Draft: recipe still being tested.",
    },
    Spec {
        id: "p13",
        name: "Paper Cup 12 oz (pack of 50)",
        category: Category::Supplies,
        unit: "pack",
        sell_price: None,
        cost: 45_000,
        track_stock: true,
        shelf_life_days: None,
        supplier: Some("sup-kemas"),
        pack: 1,
        moq: 2,
        weight_gram: 600,
        base: 0.5,
        whole: false,
        colour: "#EDE7DC",
        description: "Takeaway cups, recyclable.",
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
        s("sup-kopi", "Sumber Kopi", "6281200000001", 2),
        s("sup-roti", "Roti Pagi", "6281200000002", 1),
        s("sup-susu", "Susu Nusantara", "6281200000003", 1),
        s("sup-gula", "Gula Aren Banten", "6281200000004", 3),
        s("sup-kemas", "Kemasan Jaya", "6281200000005", 4),
    ]
}

/// A café is busier at the weekend.
fn weekday_factor(d: NaiveDate) -> f64 {
    [0.9, 0.9, 0.95, 1.0, 1.1, 1.3, 1.2][d.weekday().num_days_from_monday() as usize]
}

/// Indonesian payday: the last week of the month and the 1st.
fn is_payday(d: NaiveDate) -> bool {
    d.day() >= 25 || d.day() == 1
}

pub fn build(today: NaiveDate) -> ShopData {
    let mut rng = Rng(0x00C0_FFEE);
    let start = today - Duration::days(HISTORY_DAYS);
    let closed_day = today - Duration::days(40);
    let promo = (today - Duration::days(60))..=(today - Duration::days(54));
    let oat_stockout = (today - Duration::days(20))..=(today - Duration::days(18));

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
            draft: s.id == "p12",
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
                    if promo.contains(&date) && matches!(s.id, "p1" | "p5") {
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
                    let stockout = s.id == "p4" && oat_stockout.contains(&date);
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
        lot("L-771", "p3", 4, Some(d(50)), -10),
        lot("L-889", "p2", 14, Some(d(1)), -1),
        lot("L-901", "p4", 2, Some(d(-1)), -9),
        lot("L-902", "p4", 2, Some(d(3)), -4),
        lot("L-930", "p5", 12, Some(d(1)), 0),
        lot("L-911", "p6", 4, Some(d(4)), -3),
        lot("L-640", "p7", 9, Some(d(120)), -30),
        lot("L-925", "p8", 4, Some(d(0)), -2),
        lot("L-931", "p9", 6, Some(d(1)), 0),
        lot("L-920", "p10", 10, Some(d(3)), -2),
        lot("L-512", "p13", 3, None, -20),
    ];

    ShopData {
        tenant: TENANT.into(),
        name: "Jasa Coffee Shop".into(),
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
    let items = ["p2", "p5", "p8", "p9", "p4", "p6"];
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
        assert_eq!(a.history("p3"), b.history("p3"));
    }

    #[test]
    fn history_is_ninety_days_ending_yesterday() {
        let data = build(today());
        let h = data.history("p3");
        assert_eq!(h.len(), 90);
        assert_eq!(h.last().unwrap().date, today() - Duration::days(1));
    }

    #[test]
    fn oat_milk_has_flagged_stockout_days() {
        let data = build(today());
        assert_eq!(data.history("p4").iter().filter(|d| d.stockout).count(), 3);
    }

    #[test]
    fn an_expired_lot_is_not_usable() {
        let data = build(today());
        assert_eq!(data.total_on_hand("p4"), 4);
        assert_eq!(data.usable_on_hand("p4", today()), 2);
    }
}
