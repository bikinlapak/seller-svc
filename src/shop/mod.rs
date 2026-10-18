//! The shop's catalog, suppliers, lots and demand history.
//!
//! In production every one of these comes from worksh.io (products,
//! `stockLots`, `productMovements`, suppliers) plus the PasStok stand-ins for
//! fields works-core does not have yet (`shelfLifeDays`, `leadTimeDays`,
//! `packSize`, `minOrderQty` — PRD §10). Until that is wired, [`demo`] builds a
//! deterministic dataset in exactly these shapes, so the engine above it runs
//! on realistic numbers and the demo repeats.

pub mod demo;

use chrono::NaiveDate;

use crate::contract::Idr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Bread,
    Pastries,
    Drinks,
    Ingredients,
    Supplies,
    Specials,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Bread => "Bread",
            Category::Pastries => "Pastries",
            Category::Drinks => "Drinks",
            Category::Ingredients => "Ingredients",
            Category::Supplies => "Supplies",
            Category::Specials => "Specials",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Product {
    pub id: String,
    pub name: String,
    pub category: Category,
    /// Stock unit: "kg", "L", "box", "pcs".
    pub unit: String,
    /// `None` = not sold on its own (an ingredient or a supply).
    pub sell_price: Option<Idr>,
    /// Cost per stock unit; the basis for every waste figure.
    pub cost: Idr,
    /// `false` for made-to-order drinks: no stock, no forecast of stock.
    pub track_stock: bool,
    pub perishable: bool,
    /// Shelf life of a fresh delivery. Lots carry their actual expiry.
    pub shelf_life_days: Option<u32>,
    /// `None` = made in house; no purchase order is ever suggested.
    pub supplier_id: Option<String>,
    pub pack_size: i64,
    pub min_order_qty: i64,
    /// Weight of one stock unit, for waste in kg (worksh.io `weightGram`).
    pub weight_gram: i64,
    pub draft: bool,
    /// Placeholder tile colour until real product photos exist.
    pub colour: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct Supplier {
    pub id: String,
    pub name: String,
    /// E.164 without the plus, for `wa.me` links.
    pub whatsapp: String,
    pub lead_time_days: u32,
}

#[derive(Debug, Clone)]
pub struct Lot {
    pub id: String,
    pub product_id: String,
    pub qty: i64,
    pub expiry: Option<NaiveDate>,
    pub received: NaiveDate,
}

/// One day of demand for one product.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayDemand {
    pub date: NaiveDate,
    pub qty: f64,
    /// On-hand was zero: a low number here is censored demand, not a quiet
    /// day (FR-3), so the forecast must not learn from it.
    pub stockout: bool,
    /// The shop was closed.
    pub closed: bool,
}

#[derive(Debug, Clone)]
pub struct ShopData {
    pub tenant: String,
    pub name: String,
    pub owner: String,
    pub location: String,
    pub products: Vec<Product>,
    pub suppliers: Vec<Supplier>,
    pub lots: Vec<Lot>,
    /// Oldest first, ending yesterday. Keyed by product id.
    pub history: Vec<(String, Vec<DayDemand>)>,
    /// Days the owner said the shop will be closed (FR-31).
    pub closures: Vec<NaiveDate>,
}

impl ShopData {
    pub fn product(&self, id: &str) -> Option<&Product> {
        self.products.iter().find(|p| p.id == id)
    }

    pub fn supplier(&self, id: &str) -> Option<&Supplier> {
        self.suppliers.iter().find(|s| s.id == id)
    }

    pub fn history(&self, product_id: &str) -> &[DayDemand] {
        self.history
            .iter()
            .find(|(id, _)| id == product_id)
            .map(|(_, h)| h.as_slice())
            .unwrap_or(&[])
    }

    /// Lots for one product, FEFO order (no-expiry lots last).
    pub fn lots_of(&self, product_id: &str) -> Vec<&Lot> {
        let mut lots: Vec<&Lot> = self
            .lots
            .iter()
            .filter(|l| l.product_id == product_id && l.qty > 0)
            .collect();
        lots.sort_by_key(|l| (l.expiry.is_none(), l.expiry, l.received));
        lots
    }

    /// Stock that can still be used or sold today: expired lots excluded.
    pub fn usable_on_hand(&self, product_id: &str, today: NaiveDate) -> i64 {
        self.lots_of(product_id)
            .iter()
            .filter(|l| l.expiry.is_none_or(|e| e >= today))
            .map(|l| l.qty)
            .sum()
    }

    pub fn total_on_hand(&self, product_id: &str) -> i64 {
        self.lots_of(product_id).iter().map(|l| l.qty).sum()
    }

    /// Take `qty` out of a product's lots, FEFO, or out of one lot. Returns
    /// what was actually taken (never more than is there).
    pub fn take(&mut self, product_id: &str, lot_id: Option<&str>, qty: i64) -> i64 {
        let mut ids: Vec<String> = match lot_id {
            Some(l) => vec![l.to_owned()],
            None => self
                .lots_of(product_id)
                .iter()
                .map(|l| l.id.clone())
                .collect(),
        };
        ids.dedup();
        let mut left = qty;
        for id in ids {
            if left == 0 {
                break;
            }
            if let Some(lot) = self
                .lots
                .iter_mut()
                .find(|l| l.id == id && l.product_id == product_id)
            {
                let n = lot.qty.min(left);
                lot.qty -= n;
                left -= n;
            }
        }
        qty - left
    }
}
