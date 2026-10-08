pub mod builder;
pub mod health;
pub mod products;

use axum::Router;

use crate::AppState;

/// Semua endpoint berversi untuk seller-app.
pub fn v1_router() -> Router<AppState> {
    Router::new()
        .nest("/products", products::router())
        .nest("/builder", builder::router())
}
