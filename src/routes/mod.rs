pub mod assistant;
pub mod builder;
pub mod health;
pub mod products;
pub mod stock;

use axum::Router;

use crate::AppState;

/// Every versioned endpoint seller-app uses.
pub fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(assistant::router())
        .nest("/products", products::router())
        .nest("/stock", stock::router())
        .nest("/builder", builder::router())
}
