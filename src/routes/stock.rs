//! Stock by location, with the summary the screen header needs.

use axum::routing::get;
use axum::{Json, Router};

use crate::contract::StockList;
use crate::{AppState, seed};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list))
}

// TODO(Phase 2): `stockByLocation` + `stockLots` from works-server. `percent`
// and the day labels are computed here, not in seller-app.
async fn list() -> Json<StockList> {
    Json(seed::stock())
}
