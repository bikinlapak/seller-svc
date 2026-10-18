//! Screen-shaped views computed from the store and the engine. One function
//! per screen, so a number shown on two screens comes from one place — the
//! stock tiles and the stock filters count the same tagged rows.

use chrono::Duration;

use crate::agent::analysis::{Analysis, analyze, analyze_all};
use crate::agent::explain::run_out_label;
use crate::clock::relative_day;
use crate::contract::*;
use crate::engine::forecast::holdout_accuracy;
use crate::engine::waste::{Destination, diversion};
use crate::fmt::{idr, idr_short, kg, percent, qty, trim};
use crate::shop::{Category, Product as ShopProduct};
use crate::store::State;

pub const TAG_LOW: &str = "low";
pub const TAG_EXPIRING: &str = "expiring";
pub const TAG_ON_ORDER: &str = "on_order";

fn severity_of(a: &Analysis) -> Severity {
    let lead = a.input.map_or(1, |i| i.lead_time_days) as f64;
    if a.plan.is_some_and(|p| p.needed) || a.cover <= lead + 1.0 {
        Severity::Critical
    } else if a.at_risk_total() > 0 || a.cover < 7.0 {
        Severity::Warn
    } else {
        Severity::Ok
    }
}

fn open_po(state: &State, product_id: &str) -> Option<(String, chrono::NaiveDate)> {
    state
        .orders
        .iter()
        .filter(|o| o.status != "received" && o.lines.iter().any(|(p, _)| p == product_id))
        .map(|o| (o.number.clone(), o.expected))
        .next()
}

fn markdown_of<'a>(state: &'a State, product_id: &str) -> Option<&'a crate::store::Markdown> {
    state.markdowns.iter().find(|m| m.product_id == product_id)
}

fn product_row(state: &State, p: &ShopProduct, a: Option<&Analysis>) -> Product {
    let today = state.today;
    let (stock_label, stock_severity) = match a {
        None => (
            if p.category == Category::Specials {
                "Today only".to_string()
            } else {
                "Made to order".to_string()
            },
            Severity::Ok,
        ),
        Some(a) => {
            let total = state.data.total_on_hand(&p.id);
            let soonest = state
                .data
                .lots_of(&p.id)
                .into_iter()
                .filter_map(|l| l.expiry)
                .find(|e| (*e - today).num_days() <= 1);
            let label = match soonest {
                Some(e) if e < today => format!("{} · past date", qty(total, &p.unit)),
                Some(e) => format!("{} · exp. {}", qty(total, &p.unit), relative_day(e, today)),
                None => qty(total, &p.unit),
            };
            (label, severity_of(a))
        }
    };
    let (status, status_important) = if p.draft {
        ("Draft".to_string(), false)
    } else if p.category == Category::Specials {
        ("Listed today".to_string(), true)
    } else if let Some(m) = markdown_of(state, &p.id) {
        (format!("{}% off {}", m.pct, m.starts), true)
    } else if let Some((po, _)) = open_po(state, &p.id) {
        (format!("{po} sent"), false)
    } else if a.is_some_and(|a| a.plan.is_some_and(|pl| pl.needed)) {
        ("Reorder suggested".to_string(), true)
    } else {
        ("Active".to_string(), false)
    };
    Product {
        id: p.id.clone(),
        name: p.name.clone(),
        price: p.sell_price,
        price_label: p.sell_price.map(idr),
        subtitle: match (p.sell_price, p.category) {
            (Some(_), _) => None,
            (None, Category::Supplies) => Some("Supply · not for sale".into()),
            (None, _) => Some("Ingredient · not for sale".into()),
        },
        category: if p.draft {
            "Drafts".into()
        } else {
            p.category.label().into()
        },
        stock_label,
        stock_severity,
        status,
        status_important,
        photo_colour: p.colour.clone(),
        photo_url: p.photo_url.clone(),
    }
}

pub fn product_list(state: &State) -> ProductList {
    let analyses = analyze_all(state);
    let products: Vec<Product> = state
        .data
        .products
        .iter()
        .map(|p| {
            let a = analyses.iter().find(|a| a.product_id == p.id);
            product_row(state, p, a)
        })
        .collect();
    let mut categories = vec!["All".to_string()];
    for c in [
        Category::Specials,
        Category::Bread,
        Category::Pastries,
        Category::Drinks,
        Category::Ingredients,
        Category::Supplies,
    ] {
        if products.iter().any(|p| p.category == c.label()) {
            categories.push(c.label().into());
        }
    }
    if products.iter().any(|p| p.category == "Drafts") {
        categories.push("Drafts".into());
    }
    ProductList {
        total: products.len(),
        categories,
        products,
    }
}

pub fn series(state: &State, a: &Analysis, past: i64, ahead: usize) -> Vec<SeriesPoint> {
    let mut out: Vec<SeriesPoint> = state
        .data
        .history(&a.product_id)
        .iter()
        .rev()
        .take(past as usize)
        .rev()
        .map(|d| SeriesPoint {
            date: d.date.format("%Y-%m-%d").to_string(),
            actual: Some(d.qty),
            forecast: None,
            upper: None,
            censored: d.stockout || d.closed,
        })
        .collect();
    for i in 0..ahead {
        out.push(SeriesPoint {
            date: (state.today + Duration::days(i as i64))
                .format("%Y-%m-%d")
                .to_string(),
            actual: None,
            forecast: Some(crate::agent::analysis::round2(a.forecast.mean_at(i))),
            upper: Some(crate::agent::analysis::round2(a.forecast.upper_at(i))),
            censored: false,
        });
    }
    out
}

fn accuracy_label(state: &State, product_id: &str) -> String {
    match holdout_accuracy(state.data.history(product_id)) {
        Some(acc) => format!(
            "Last-week error {}% vs {}% for a 4-week average{}",
            percent(acc.model_wape),
            percent(acc.baseline_wape),
            if acc.beats_baseline() {
                ""
            } else {
                " — no better than the average yet"
            }
        ),
        None => "Not enough history to measure accuracy".into(),
    }
}

pub fn product_detail(state: &State, id: &str) -> Option<ProductDetail> {
    let p = state.data.product(id)?;
    let a = analyze(state, id);
    let row = product_row(state, p, a.as_ref());
    let today = state.today;
    let mut facts = Vec::new();
    let mut push = |label: &str, value: String| {
        facts.push(Fact {
            label: label.into(),
            value,
        })
    };
    if let Some(price) = p.sell_price {
        push("Price", idr(price));
    }
    if p.cost > 0 {
        push("Cost", format!("{} per {}", idr(p.cost), p.unit));
    }
    if let Some(s) = p
        .supplier_id
        .as_deref()
        .and_then(|s| state.data.supplier(s))
    {
        push(
            "Supplier",
            format!("{} · delivers in {} d", s.name, s.lead_time_days),
        );
        push(
            "Order in",
            format!(
                "packs of {} · minimum {}",
                p.pack_size,
                qty(p.min_order_qty, &p.unit)
            ),
        );
    }
    if let Some(d) = p.shelf_life_days {
        push("Shelf life", format!("{d} days"));
    }
    let mut lots = Vec::new();
    let mut forecast_note = String::new();
    let mut series_points = Vec::new();
    if let Some(a) = &a {
        push(
            "Selling",
            format!(
                "about {} {} a day",
                trim(a.forecast.daily_rate(), 1),
                crate::fmt::plural(&p.unit, 2)
            ),
        );
        push(
            "Cover",
            format!("{} days · {}", trim(a.cover, 1), run_out_label(a, state)),
        );
        if a.on_order > 0 {
            let po = open_po(state, id).map(|(n, e)| format!(" ({n}, {})", relative_day(e, today)));
            push(
                "On order",
                format!("{}{}", qty(a.on_order, &p.unit), po.unwrap_or_default()),
            );
        }
        if let Some(m) = markdown_of(state, id) {
            push(
                "Discount",
                format!(
                    "{}% off {}–{} on {}",
                    m.pct,
                    m.starts,
                    m.ends,
                    qty(m.qty, &p.unit)
                ),
            );
        }
        for l in state.data.lots_of(id) {
            let risk = a.risks.iter().find(|r| r.lot_id == l.id);
            let (label, sev) = match l.expiry {
                Some(e) if e < today => ("past date".to_string(), Severity::Critical),
                Some(e) => {
                    let days = (e - today).num_days();
                    (
                        format!("expires {}", relative_day(e, today)),
                        if days <= 1 {
                            Severity::Critical
                        } else if days <= 3 {
                            Severity::Warn
                        } else {
                            Severity::Ok
                        },
                    )
                }
                None => ("no expiry".to_string(), Severity::Ok),
            };
            lots.push(LotView {
                id: l.id.clone(),
                quantity: l.qty,
                unit: p.unit.clone(),
                expiry_label: label,
                severity: sev,
                at_risk: risk.map_or(0, |r| r.at_risk_qty),
            });
        }
        forecast_note = format!(
            "{} · {}. {}.",
            a.forecast.method,
            a.forecast.confidence.label(),
            accuracy_label(state, id)
        );
        series_points = series(state, a, 28, 14);
    }
    Some(ProductDetail {
        product: row,
        description: p.description.clone(),
        unit: p.unit.clone(),
        facts,
        lots,
        series: series_points,
        forecast_note,
    })
}

pub fn stock_list(state: &State) -> StockList {
    let today = state.today;
    let mut rows: Vec<(Severity, f64, StockRow)> = analyze_all(state)
        .iter()
        .filter_map(|a| {
            let p = state.data.product(&a.product_id)?;
            let sev = severity_of(a);
            let at_risk = a.at_risk_total();
            let expired: i64 = state
                .data
                .lots_of(&p.id)
                .iter()
                .filter(|l| l.expiry.is_some_and(|e| e < today))
                .map(|l| l.qty)
                .sum();
            let soonest_risk = a
                .risks
                .iter()
                .filter(|r| r.at_risk_qty > 0)
                .min_by_key(|r| r.expiry);
            let po = open_po(state, &p.id);

            let mut tags = Vec::new();
            if sev == Severity::Critical {
                tags.push(TAG_LOW.to_string());
            }
            if at_risk > 0 || expired > 0 {
                tags.push(TAG_EXPIRING.to_string());
            }
            if po.is_some() {
                tags.push(TAG_ON_ORDER.to_string());
            }

            let left = if expired > 0 {
                format!("{} past date", qty(expired, &p.unit))
            } else if let Some(r) = soonest_risk {
                format!(
                    "{} expire {} unsold",
                    qty(r.at_risk_qty, &p.unit),
                    relative_day(r.expiry, today)
                )
            } else if sev == Severity::Critical {
                format!(
                    "{} · {} days",
                    capitalise(&run_out_label(a, state)),
                    trim(a.cover, 1)
                )
            } else {
                format!("Fine · {} days", trim(a.cover.min(99.0), 0))
            };
            let (right, right_important) = match &po {
                Some((n, e)) => (format!("{n} arrives {}", relative_day(*e, today)), true),
                None => {
                    let n = state.data.lots_of(&p.id).len();
                    (
                        if n == 1 {
                            format!("lot {}", state.data.lots_of(&p.id)[0].id)
                        } else {
                            format!("{n} lots")
                        },
                        false,
                    )
                }
            };
            // Bar: days of cover against two weeks.
            let percent = ((a.cover / 14.0) * 100.0).clamp(4.0, 100.0).round() as i64;
            Some((
                sev,
                a.cover,
                StockRow {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    quantity_label: qty(state.data.total_on_hand(&p.id), &p.unit),
                    percent,
                    severity: sev,
                    left,
                    right,
                    right_important,
                    tags,
                },
            ))
        })
        .collect();
    let rank = |s: Severity| match s {
        Severity::Critical => 0,
        Severity::Warn => 1,
        Severity::Ok => 2,
    };
    rows.sort_by(|a, b| {
        rank(a.0)
            .cmp(&rank(b.0))
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let rows: Vec<StockRow> = rows.into_iter().map(|(_, _, r)| r).collect();
    let value: i64 = state
        .data
        .products
        .iter()
        .map(|p| state.data.usable_on_hand(&p.id, today) * p.cost)
        .sum();
    let count = |tag: &str| {
        rows.iter()
            .filter(|r| r.tags.iter().any(|t| t == tag))
            .count()
    };
    StockList {
        location: state.data.location.clone(),
        summary: StockSummary {
            value: idr_short(value),
            low_count: count(TAG_LOW),
            expiring_count: count(TAG_EXPIRING),
        },
        rows,
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn waste_report(state: &State) -> WasteReport {
    let today = state.today;
    let week: Vec<_> = state
        .waste
        .iter()
        .filter(|w| (today - w.date).num_days() < 7)
        .collect();
    let prev: Vec<_> = state
        .waste
        .iter()
        .filter(|w| (7..14).contains(&(today - w.date).num_days()))
        .collect();
    let d = diversion(week.iter().copied());
    let p = diversion(prev.iter().copied());
    let by_destination = d
        .by_destination
        .iter()
        .map(|t| DestinationShare {
            id: t.destination.id().into(),
            label: t.destination.label().into(),
            value: idr(t.value),
            weight: kg(t.grams),
            percent: if d.total_value > 0 {
                (t.value * 100 + d.total_value / 2) / d.total_value
            } else {
                0
            },
            landfill: t.destination == Destination::Landfill,
        })
        .collect();
    let mut recent: Vec<_> = state.waste.iter().collect();
    recent.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
    let recent = recent
        .into_iter()
        .take(8)
        .map(|w| Fact {
            label: format!(
                "{} · {} {}",
                relative_day(w.date, today),
                qty(w.qty, &w.unit),
                w.product_name
            ),
            value: format!("{} · {}", w.destination.label(), idr(w.value_at_cost)),
        })
        .collect();
    WasteReport {
        period: "Last 7 days".into(),
        diversion_percent: d.score.map(percent),
        previous_percent: p.score.map(percent),
        total_value: idr(d.total_value),
        total_weight: kg(d.total_grams),
        photos: d.photos,
        by_destination,
        recent,
        method: "Value at cost price from the stock ledger; weight from each product's unit weight. Diversion = 1 − landfill ÷ everything that did not sell at full price. Earlier weeks are seeded demo history.".into(),
    }
}

pub fn dashboard(state: &State) -> Dashboard {
    let all = analyze_all(state);
    let today = state.today;
    let mut at_risk: Vec<AtRiskLot> = Vec::new();
    for a in &all {
        let Some(p) = state.data.product(&a.product_id) else {
            continue;
        };
        for r in a.risks.iter().filter(|r| r.at_risk_qty > 0) {
            at_risk.push(AtRiskLot {
                product_id: p.id.clone(),
                name: p.name.clone(),
                lot_id: r.lot_id.clone(),
                at_risk: format!("{} of {}", r.at_risk_qty, qty(r.qty_remaining, &p.unit)),
                expiry_label: if r.expiry < today {
                    "past date".into()
                } else {
                    format!("expires {}", relative_day(r.expiry, today))
                },
                value: idr(r.at_risk_qty * p.cost),
            });
        }
    }
    let open = state
        .suggestions
        .values()
        .filter(|s| s.state == CardState::Open)
        .count();
    let decided = state
        .suggestions
        .values()
        .filter(|s| s.state != CardState::Open)
        .count();
    let approved = state
        .suggestions
        .values()
        .filter(|s| s.state == CardState::Approved)
        .count();

    // Impact on the simulated month: the oldest recorded week stands in for
    // "before PasStok", the latest for "with it".
    let week = |from: i64| {
        diversion(
            state
                .waste
                .iter()
                .filter(move |w| (from..from + 7).contains(&(today - w.date).num_days())),
        )
    };
    let before = week(21);
    let after = week(0);
    let landfill = |d: &crate::engine::waste::Diversion| {
        d.by_destination
            .iter()
            .find(|t| t.destination == Destination::Landfill)
            .map_or((0, 0), |t| (t.value, t.grams))
    };
    let (lb, gb) = landfill(&before);
    let (la, ga) = landfill(&after);
    let stockouts: usize = all
        .iter()
        .map(|a| {
            state
                .data
                .history(&a.product_id)
                .iter()
                .filter(|d| d.stockout)
                .count()
        })
        .sum();
    let impact = vec![
        Stat {
            label: "Landfill avoided / week".into(),
            value: idr_short((lb - la).max(0)),
            note: format!("{} less than 3 weeks ago", kg((gb - ga).max(0))),
        },
        Stat {
            label: "Kept out of landfill".into(),
            value: after
                .score
                .map_or("–".into(), |s| format!("{}%", percent(s))),
            note: before
                .score
                .map_or(String::new(), |s| format!("was {}%", percent(s))),
        },
        Stat {
            label: "Stockout days (90 d)".into(),
            value: stockouts.to_string(),
            note: "excluded from the forecast".into(),
        },
        Stat {
            label: "Suggestions accepted".into(),
            value: (approved * 100)
                .checked_div(decided)
                .map_or("–".into(), |p| format!("{p}%")),
            note: format!("{approved} of {decided} decided"),
        },
    ];

    let mut focus: Vec<&Analysis> = all.iter().collect();
    focus.sort_by(|a, b| {
        a.cover
            .partial_cmp(&b.cover)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let series = focus
        .into_iter()
        .take(4)
        .filter_map(|a| {
            let p = state.data.product(&a.product_id)?;
            Some(ProductSeries {
                product_id: p.id.clone(),
                name: p.name.clone(),
                unit: p.unit.clone(),
                accuracy: accuracy_label(state, &p.id),
                series: series(state, a, 21, 14),
            })
        })
        .collect();

    Dashboard {
        impact,
        open_suggestions: open,
        at_risk,
        series,
        method_note: "Forecast: weekday-seasonal level with an 80% upper bound, standing in for BigQuery AI.FORECAST (TimesFM). Stockout and closed days are not read as zero demand. Impact compares the latest week with three weeks earlier on seeded demo history.".into(),
    }
}

/// What the AI settings screen reports about the model in use.
pub struct AiInfo {
    pub model: Option<String>,
    pub calls_today: u32,
    pub daily_cap: u32,
}

pub fn settings(state: &State, ai: &AiInfo) -> AiSettings {
    let mode = |id: &str, label: &str, note: &str| AssistantMode {
        id: id.into(),
        label: label.into(),
        note: note.into(),
    };
    let model = |task: &str, note: Option<&str>, name: &str, system: bool| TaskModel {
        task: task.into(),
        note: note.map(Into::into),
        name: name.into(),
        system,
    };
    let enabled = |id: &str| state.settings.autonomy.get(id).copied().unwrap_or(false);
    AiSettings {
        modes: vec![
            mode("frugal", "Frugal", "short answers, small model"),
            mode("balanced", "Balanced", "default"),
            mode("thorough", "Thorough", "longer reasoning"),
        ],
        selected_mode: state.settings.mode.clone(),
        models: {
            let gem = ai.model.as_deref();
            let frugal = state.settings.mode == "frugal";
            vec![
                model(
                    "Understanding questions",
                    Some(if frugal || gem.is_none() {
                        "keyword rules"
                    } else {
                        "answers are computed by the system"
                    }),
                    match gem {
                        Some(m) if !frugal => m,
                        _ => "Rules",
                    },
                    gem.is_none() || frugal,
                ),
                model(
                    "Reading waste photos",
                    gem.is_none().then_some("stand-in, no model configured"),
                    gem.unwrap_or("Stand-in"),
                    gem.is_none(),
                ),
                model(
                    "Menu ideas",
                    Some("portions and prices checked by the system"),
                    gem.unwrap_or("Recipe library"),
                    gem.is_none(),
                ),
                // A system number: never invented by a chat model, and the
                // merchant deserves to know which figures those are.
                model(
                    "Stock forecasting",
                    Some("computed by the system, not by chat"),
                    "TimesFM",
                    true,
                ),
            ]
        },
        autonomy: vec![
            Autonomy {
                id: "small-po".into(),
                label: "Auto-approve small orders".into(),
                note: Some("≤Rp500k, after 3 approvals with no edits".into()),
                enabled: enabled("small-po"),
            },
            Autonomy {
                id: "markdown".into(),
                label: "Auto-discount near expiry".into(),
                note: None,
                enabled: enabled("markdown"),
            },
        ],
        language: LanguageSetting {
            options: ["English", "Indonesia · santai", "Indonesia · formal"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            selected: state.settings.language.clone(),
        },
        budget: match ai.model {
            Some(_) => Budget {
                cap_label: format!("{} calls/day", ai.daily_cap),
                used_percent: (i64::from(ai.calls_today) * 100)
                    .checked_div(i64::from(ai.daily_cap))
                    .unwrap_or(0),
                note: format!(
                    "{} model calls today. At the cap the assistant falls back to rules and templates until tomorrow.",
                    ai.calls_today
                ),
            },
            None => Budget {
                cap_label: "No model".into(),
                used_percent: 0,
                note: "No Gemini configured: everything runs on rules, templates and the stand-ins.".into(),
            },
        },
        privacy: "Your shop's data is not used to train models. Waste photos are kept for 90 days as evidence."
            .into(),
    }
}
