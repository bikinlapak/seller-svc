//! Katalog produk — sumber datanya katalog produk worksh.io.

use axum::routing::get;
use axum::{Json, Router};

use crate::contract::DaftarProduk;
use crate::{AppState, seed};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list))
}

// TODO(Fase 2): query produk ke works-server lewat `state.works`,
// dengan paginasi kursor. Pencarian dan filter kategori ikut ke sana.
async fn list() -> Json<DaftarProduk> {
    Json(seed::produk())
}
