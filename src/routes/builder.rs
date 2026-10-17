//! Web builder — online-shop pages stored as drafts in worksh.io.

use axum::Router;
use axum::routing::get;

use crate::{AppError, AppState};

pub fn router() -> Router<AppState> {
    Router::new().route("/pages", get(list_pages))
}

// TODO: list shop pages from works-server through `state.works`.
async fn list_pages() -> Result<(), AppError> {
    Err(AppError::NotImplemented)
}
