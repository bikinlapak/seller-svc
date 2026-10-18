//! Stock: position per product with cover, expiry and open orders.

use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::contract::StockList;
use crate::{AppError, AppState, views};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list)).route(
        "/photos",
        post(super::waste::count).layer(DefaultBodyLimit::max(super::waste::MAX_PHOTO_BYTES)),
    )
}

async fn list(State(state): State<AppState>) -> Result<Json<StockList>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(views::stock_list(&s)))
}
