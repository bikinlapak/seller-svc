//! Stock: position per product with cover, expiry and open orders.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::contract::StockList;
use crate::{AppError, AppState, views};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list))
}

async fn list(State(state): State<AppState>) -> Result<Json<StockList>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(views::stock_list(&s)))
}
