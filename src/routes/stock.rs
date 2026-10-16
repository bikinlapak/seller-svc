//! Stok per lokasi, dengan ringkasan untuk kepala layar.

use axum::routing::get;
use axum::{Json, Router};

use crate::contract::DaftarStok;
use crate::{AppState, seed};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list))
}

// TODO(Fase 2): `stockByLocation` + `stockLots` dari works-server.
// `persen` dan label hari dihitung di sini, bukan di seller-app.
async fn list() -> Json<DaftarStok> {
    Json(seed::stok())
}
