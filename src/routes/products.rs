//! Kelola produk — sumber datanya katalog produk worksh.io.

use axum::Router;
use axum::routing::get;

use crate::{AppError, AppState};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list))
}

// TODO: query produk ke works-server lewat `state.works`.
async fn list() -> Result<(), AppError> {
    Err(AppError::NotImplemented)
}
