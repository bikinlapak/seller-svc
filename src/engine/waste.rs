//! Waste by destination (FR-40, FR-41, FR-54; decision L30).
//!
//! Every item that will not sell at full price gets a destination from the
//! food recovery hierarchy. Which destinations are *offered* depends on the
//! item's state, because UU 8/1999 Art. 8 forbids trading goods past their
//! date: before the date anything goes; past it, or spoiled, only compost or
//! disposal. Landfill is always offered explicitly and never preselected.

use chrono::NaiveDate;

use crate::contract::Idr;
use crate::shop::Product;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Destination {
    SoldMarkdown,
    Bundled,
    Repurposed,
    StaffMeal,
    Donated,
    BsfOrCompost,
    Landfill,
}

impl Destination {
    pub const ALL: [Destination; 7] = [
        Destination::SoldMarkdown,
        Destination::Bundled,
        Destination::Repurposed,
        Destination::StaffMeal,
        Destination::Donated,
        Destination::BsfOrCompost,
        Destination::Landfill,
    ];

    /// Wire id, as in the PRD (`dest:<id>`).
    pub fn id(self) -> &'static str {
        match self {
            Destination::SoldMarkdown => "sold_markdown",
            Destination::Bundled => "bundled",
            Destination::Repurposed => "repurposed",
            Destination::StaffMeal => "staff_meal",
            Destination::Donated => "donated",
            Destination::BsfOrCompost => "bsf_or_compost",
            Destination::Landfill => "landfill",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Destination::SoldMarkdown => "Sold at a discount",
            Destination::Bundled => "Bundled",
            Destination::Repurposed => "Today's special",
            Destination::StaffMeal => "Staff meal",
            Destination::Donated => "Donate",
            Destination::BsfOrCompost => "Compost / larvae",
            Destination::Landfill => "Discard",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.id() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// Still before its date and fit to eat.
    Good,
    PastDate,
    Spoiled,
    Damaged,
}

impl Condition {
    pub fn id(self) -> &'static str {
        match self {
            Condition::Good => "production_leftover",
            Condition::PastDate => "expired",
            Condition::Spoiled => "spoiled",
            Condition::Damaged => "damaged",
        }
    }

    pub fn can_be_eaten(self) -> bool {
        matches!(self, Condition::Good | Condition::Damaged)
    }
}

/// The condition the ledger implies for a lot, before vision has a say.
pub fn condition_for(expiry: Option<NaiveDate>, today: NaiveDate) -> Condition {
    match expiry {
        Some(e) if e < today => Condition::PastDate,
        _ => Condition::Good,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DestinationOption {
    pub destination: Destination,
    pub expected_recovered: Idr,
    pub selected: bool,
}

/// Destinations offered at closing for a leftover item, best recovery first,
/// the highest preselected. Landfill is last and never preselected.
pub fn options(product: &Product, qty: i64, condition: Condition) -> Vec<DestinationOption> {
    let mut opts: Vec<(Destination, Idr)> = Vec::new();
    if condition.can_be_eaten() {
        // A special sells the item again at roughly half its usual value.
        let special = match product.sell_price {
            Some(p) => qty * p / 2,
            None => qty * product.cost * 4 / 5,
        };
        opts.push((Destination::Repurposed, special));
        opts.push((Destination::Donated, 0));
        opts.push((Destination::StaffMeal, 0));
    }
    opts.push((Destination::BsfOrCompost, 0));
    opts.push((Destination::Landfill, 0));

    // Stable sort keeps the hierarchy order among equal recoveries.
    opts.sort_by_key(|o| std::cmp::Reverse(o.1));
    opts.into_iter()
        .enumerate()
        .map(|(i, (destination, expected_recovered))| DestinationOption {
            destination,
            expected_recovered,
            selected: i == 0 && destination != Destination::Landfill,
        })
        .collect()
}

/// Whether `d` may be chosen for an item in `condition` — checked again on
/// write, not just when the options are drawn.
pub fn allowed(d: Destination, condition: Condition) -> bool {
    match d {
        Destination::BsfOrCompost | Destination::Landfill => true,
        _ => condition.can_be_eaten(),
    }
}

/// One stock-out that did not sell at full price.
#[derive(Debug, Clone, PartialEq)]
pub struct WasteRecord {
    pub id: String,
    pub date: NaiveDate,
    pub product_id: String,
    pub product_name: String,
    pub lot_id: Option<String>,
    pub qty: i64,
    pub unit: String,
    pub destination: Destination,
    /// `waste:<condition>` or `dest:<destination>`.
    pub reason: String,
    pub value_at_cost: Idr,
    pub grams: i64,
    pub photo_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DestinationTotal {
    pub destination: Destination,
    pub value: Idr,
    pub grams: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diversion {
    pub total_value: Idr,
    pub total_grams: i64,
    /// 1 − landfill ÷ total, at cost. `None` when nothing was recorded.
    pub score: Option<f64>,
    pub score_by_weight: Option<f64>,
    pub by_destination: Vec<DestinationTotal>,
    pub photos: usize,
}

pub fn diversion<'a>(records: impl IntoIterator<Item = &'a WasteRecord>) -> Diversion {
    let mut by: Vec<DestinationTotal> = Destination::ALL
        .iter()
        .map(|d| DestinationTotal {
            destination: *d,
            value: 0,
            grams: 0,
        })
        .collect();
    let mut photos = std::collections::BTreeSet::new();
    for r in records {
        if let Some(t) = by.iter_mut().find(|t| t.destination == r.destination) {
            t.value += r.value_at_cost;
            t.grams += r.grams;
        }
        if let Some(p) = &r.photo_id {
            photos.insert(p.clone());
        }
    }
    let total_value: Idr = by.iter().map(|t| t.value).sum();
    let total_grams: i64 = by.iter().map(|t| t.grams).sum();
    let landfill = by
        .iter()
        .find(|t| t.destination == Destination::Landfill)
        .cloned()
        .unwrap_or(DestinationTotal {
            destination: Destination::Landfill,
            value: 0,
            grams: 0,
        });
    let ratio = |part: i64, total: i64| (total > 0).then(|| 1.0 - part as f64 / total as f64);
    by.retain(|t| t.value > 0 || t.grams > 0);
    by.sort_by_key(|t| std::cmp::Reverse(t.value));
    Diversion {
        total_value,
        total_grams,
        score: ratio(landfill.value, total_value),
        score_by_weight: ratio(landfill.grams, total_grams),
        by_destination: by,
        photos: photos.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shop::Category;

    fn croissant() -> Product {
        Product {
            id: "p2".into(),
            name: "Butter Croissant (box of 4)".into(),
            category: Category::Pastries,
            unit: "box".into(),
            sell_price: Some(60_000),
            cost: 36_000,
            track_stock: true,
            perishable: true,
            shelf_life_days: Some(3),
            supplier_id: None,
            pack_size: 1,
            min_order_qty: 1,
            weight_gram: 240,
            draft: false,
            colour: String::new(),
            description: String::new(),
        }
    }

    #[test]
    fn past_date_offers_only_compost_and_discard() {
        let ids: Vec<_> = options(&croissant(), 2, Condition::PastDate)
            .iter()
            .map(|o| o.destination)
            .collect();
        assert_eq!(ids, vec![Destination::BsfOrCompost, Destination::Landfill]);
    }

    #[test]
    fn landfill_is_never_preselected() {
        for c in [
            Condition::Good,
            Condition::PastDate,
            Condition::Spoiled,
            Condition::Damaged,
        ] {
            let opts = options(&croissant(), 3, c);
            let land = opts
                .iter()
                .find(|o| o.destination == Destination::Landfill)
                .unwrap();
            assert!(!land.selected, "{c:?}");
            assert_eq!(opts.iter().filter(|o| o.selected).count(), 1, "{c:?}");
        }
    }

    #[test]
    fn highest_recovery_is_preselected() {
        let opts = options(&croissant(), 3, Condition::Good);
        assert_eq!(opts[0].destination, Destination::Repurposed);
        assert!(opts[0].selected);
        assert_eq!(opts[0].expected_recovered, 90_000);
    }

    #[test]
    fn selling_or_donating_spoiled_food_is_refused_on_write() {
        assert!(!allowed(Destination::Donated, Condition::Spoiled));
        assert!(!allowed(Destination::Repurposed, Condition::PastDate));
        assert!(allowed(Destination::BsfOrCompost, Condition::PastDate));
    }

    fn rec(dest: Destination, value: Idr) -> WasteRecord {
        WasteRecord {
            id: "x".into(),
            date: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            product_id: "p2".into(),
            product_name: "x".into(),
            lot_id: None,
            qty: 1,
            unit: "box".into(),
            destination: dest,
            reason: String::new(),
            value_at_cost: value,
            grams: value / 100,
            photo_id: None,
        }
    }

    #[test]
    fn diversion_is_share_kept_out_of_landfill() {
        let r = [
            rec(Destination::Donated, 60_000),
            rec(Destination::BsfOrCompost, 20_000),
            rec(Destination::Landfill, 20_000),
        ];
        let d = diversion(&r);
        assert_eq!(d.total_value, 100_000);
        assert!((d.score.unwrap() - 0.8).abs() < 1e-9);
        assert_eq!(d.by_destination[0].destination, Destination::Donated);
    }

    #[test]
    fn nothing_recorded_has_no_score() {
        assert_eq!(diversion(&[]).score, None);
    }
}
