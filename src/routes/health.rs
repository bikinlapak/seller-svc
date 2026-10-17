use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/healthz", get(healthz))
}

/// Liveness only: it does not touch works-server, so the orchestrator will not
/// restart this service just because an upstream is down.
async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
