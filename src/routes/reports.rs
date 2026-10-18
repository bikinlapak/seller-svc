//! Reports: waste by destination with the diversion score (FR-54), the
//! impact dashboard (FR-23, FR-24), and the activity log behind them.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::contract::{Dashboard, WasteReport};
use crate::{AppError, AppState, views};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reports/waste", get(waste))
        .route("/dashboard", get(dashboard))
        .route("/activity", get(activity))
}

async fn waste(State(state): State<AppState>) -> Result<Json<WasteReport>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(views::waste_report(&s)))
}

async fn dashboard(State(state): State<AppState>) -> Result<Json<Dashboard>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(views::dashboard(&s)))
}

/// Audit log and outbox, newest first: what was decided, on which inputs,
/// and which events left for Pub/Sub.
async fn activity(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let s = state.store.lock()?;
    let audit: Vec<Value> = s
        .audit
        .iter()
        .rev()
        .take(50)
        .map(|a| {
            json!({ "at": a.at, "actor": a.actor, "action": a.action, "suggestion": a.suggestion, "inputs": a.inputs, "outcome": a.outcome })
        })
        .collect();
    let events: Vec<Value> = s.outbox.iter().rev().take(100).cloned().collect();
    Ok(Json(
        json!({ "audit": audit, "events": events, "moderation": s.moderation.len() }),
    ))
}
