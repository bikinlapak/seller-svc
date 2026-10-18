pub mod admin;
pub mod assistant;
pub mod builder;
pub mod health;
pub mod products;
pub mod reports;
pub mod stock;
pub mod waste;

use axum::Router;

use crate::AppState;

/// Every versioned endpoint seller-app uses.
pub fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(assistant::router())
        .merge(waste::router())
        .merge(reports::router())
        .merge(admin::router())
        .nest("/products", products::router())
        .nest("/stock", stock::router())
        .nest("/builder", builder::router())
}
