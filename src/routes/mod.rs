pub mod asisten;
pub mod builder;
pub mod health;
pub mod products;
pub mod stock;

use axum::Router;

use crate::AppState;

/// Semua endpoint berversi untuk seller-app.
pub fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(asisten::router())
        .nest("/products", products::router())
        .nest("/stock", stock::router())
        .nest("/builder", builder::router())
}
