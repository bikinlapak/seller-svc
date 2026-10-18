//! Demo kit (Phase 9): reset the tenant to its seeded morning, or re-run the
//! agent over the current state. Not reachable from the browser — seller-app
//! never proxies these, and on Cloud Run the service only admits IAM callers.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::{AppError, AppState, agent, demo_state};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/reset", post(reset))
        .route("/admin/run", post(run))
}

async fn reset(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let fresh = demo_state(&state.action_keys)?;
    state.store.replace(fresh)?;
    Ok(Json(json!({ "reset": true })))
}

async fn run(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    {
        let mut s = state.store.lock()?;
        agent::morning_run(&mut s, &state.action_keys)?;
    }
    state.store.touched("*");
    Ok(Json(json!({ "ran": true })))
}
