//! Web builder — halaman toko online yang disimpan sebagai draft di worksh.io.

use axum::Router;
use axum::routing::get;

use crate::{AppError, AppState};

pub fn router() -> Router<AppState> {
    Router::new().route("/pages", get(list_pages))
}

// TODO: daftar halaman toko dari works-server lewat `state.works`.
async fn list_pages() -> Result<(), AppError> {
    Err(AppError::NotImplemented)
}
