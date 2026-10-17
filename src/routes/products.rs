//! Product catalogue — sourced from the worksh.io catalogue.

use axum::routing::get;
use axum::{Json, Router};

use crate::contract::ProductList;
use crate::{AppState, seed};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list))
}

// TODO(Phase 2): query works-server through `state.works`, with cursor
// pagination. Search and category filtering move there too.
async fn list() -> Json<ProductList> {
    Json(seed::products())
}
