//! Today's special from near-expiry stock (FR-46).
//!
//! **Local stand-in for the Gemini idea.** The production design asks Gemini
//! for a dish limited to the shop's catalog and current stock; this module
//! picks from a small recipe library under exactly those constraints, so the
//! rest of the flow (yield, price, recovery, approve → work order) is real.
//! Whichever source proposes the dish, the numbers — portions, ingredients,
//! recovery — are computed here, never taken from the model.

use std::collections::HashMap;

use crate::contract::Idr;

pub struct Recipe {
    pub name: &'static str,
    /// (product id, stock units per portion). The first line is the at-risk
    /// item the dish exists to use up.
    pub lines: &'static [(&'static str, f64)],
    pub price: Idr,
    pub method: &'static str,
}

pub const RECIPES: &[Recipe] = &[
    Recipe {
        name: "Croissant Bread Pudding",
        lines: &[("p2", 0.25), ("p6", 0.1), ("p7", 0.02)],
        price: 18_000,
        method: "Tear the croissants, soak in milk and palm sugar custard, bake 25 minutes.",
    },
    Recipe {
        name: "Milk Bun French Toast",
        lines: &[("p9", 1.0), ("p6", 0.05), ("p7", 0.01)],
        price: 16_000,
        method: "Dip halved buns in milk and egg, pan-fry, finish with palm sugar syrup.",
    },
    Recipe {
        name: "Palm Sugar Toast",
        lines: &[("p8", 0.125), ("p7", 0.015)],
        price: 15_000,
        method: "Thick wholemeal toast brushed with butter and palm sugar caramel.",
    },
];

/// No more portions than a café sells of one special in a day.
pub const MAX_PORTIONS: i64 = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct Special {
    pub name: String,
    pub portions: i64,
    pub price: Idr,
    pub expected_recovered: Idr,
    /// (product id, whole stock units used).
    pub uses: Vec<(String, i64)>,
    pub method: String,
    /// (product id, stock units per portion), main at-risk item first.
    pub lines: Vec<(String, f64)>,
}

/// Size a dish against stock: as many portions as use up the at-risk main
/// ingredient, limited by every ingredient on hand and by what a day sells.
/// The same arithmetic whether the dish came from the library or a model.
pub fn evaluate(
    name: &str,
    lines: &[(String, f64)],
    price: Idr,
    method: &str,
    available: &HashMap<String, i64>,
    at_risk: &HashMap<String, i64>,
) -> Option<Special> {
    let (main, per_main) = lines.first()?;
    let risk = *at_risk.get(main)?;
    if risk <= 0 || lines.iter().any(|(_, per)| !per.is_finite() || *per <= 0.0) {
        return None;
    }
    let wanted = (risk as f64 / per_main).floor() as i64;
    let possible = lines
        .iter()
        .map(|(pid, per)| {
            let have = *available.get(pid).unwrap_or(&0) as f64;
            (have / per).floor() as i64
        })
        .min()?;
    let portions = wanted.min(possible).min(MAX_PORTIONS);
    if portions <= 0 {
        return None;
    }
    Some(Special {
        name: name.into(),
        portions,
        price,
        expected_recovered: portions * price,
        uses: lines
            .iter()
            .map(|(pid, per)| (pid.clone(), (per * portions as f64).ceil() as i64))
            .collect(),
        method: method.into(),
        lines: lines.to_vec(),
    })
}

/// `available`: usable stock per product. `at_risk`: units expected to go
/// unsold. Picks the recipe that rescues the most at-risk units by value.
pub fn propose(
    available: &HashMap<String, i64>,
    at_risk: &HashMap<String, i64>,
    cost: &HashMap<String, Idr>,
) -> Option<Special> {
    RECIPES
        .iter()
        .filter_map(|r| {
            let lines: Vec<(String, f64)> = r
                .lines
                .iter()
                .map(|(p, per)| ((*p).to_owned(), *per))
                .collect();
            let s = evaluate(r.name, &lines, r.price, r.method, available, at_risk)?;
            let rescued_value = s.uses[0].1 * cost.get(&s.uses[0].0).copied().unwrap_or(0);
            Some((rescued_value, s))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, s)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, i64)]) -> HashMap<String, i64> {
        pairs.iter().map(|(k, v)| ((*k).into(), *v)).collect()
    }

    #[test]
    fn uses_at_risk_croissants_within_stock() {
        let available = map(&[("p2", 14), ("p6", 6), ("p7", 9)]);
        let at_risk = map(&[("p2", 2)]);
        let cost = map(&[("p2", 36_000)]);
        let s = propose(&available, &at_risk, &cost).unwrap();
        assert_eq!(s.name, "Croissant Bread Pudding");
        assert_eq!(s.portions, 8, "2 boxes × 4 croissants");
        assert_eq!(s.expected_recovered, 8 * 18_000);
        assert_eq!(s.uses[0], ("p2".into(), 2));
    }

    #[test]
    fn missing_ingredient_means_no_special() {
        let available = map(&[("p2", 14), ("p7", 9)]); // no milk
        let at_risk = map(&[("p2", 8)]);
        assert!(propose(&available, &at_risk, &map(&[])).is_none());
    }

    #[test]
    fn never_more_than_a_day_can_sell() {
        let available = map(&[("p2", 50), ("p6", 50), ("p7", 50)]);
        let at_risk = map(&[("p2", 40)]);
        let s = propose(&available, &at_risk, &map(&[("p2", 36_000)])).unwrap();
        assert_eq!(s.portions, MAX_PORTIONS);
    }

    #[test]
    fn nothing_at_risk_nothing_proposed() {
        let available = map(&[("p2", 14), ("p6", 6), ("p7", 9)]);
        assert!(propose(&available, &map(&[]), &map(&[])).is_none());
    }
}
