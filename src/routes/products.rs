//! Products: list and detail.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::contract::{ProductDetail, ProductList};
use crate::{AppError, AppState, views};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(detail))
}

async fn list(State(state): State<AppState>) -> Result<Json<ProductList>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(views::product_list(&s)))
}

async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProductDetail>, AppError> {
    let s = state.store.lock()?;
    views::product_detail(&s, &id)
        .map(Json)
        .ok_or(AppError::NotFound)
}
