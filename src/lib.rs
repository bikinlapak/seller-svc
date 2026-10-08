//! seller-svc — backend dasbor penjual bikinlapak.
//!
//! Layanan ini tidak menyimpan data produk/halaman sendiri: produk dan web
//! builder dikelola di worksh.io, diakses lewat GraphQL works-server
//! (lihat [`works`]).

pub mod config;
pub mod error;
pub mod routes;
pub mod works;

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderValue, Method, header};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub use config::Config;
pub use error::AppError;

/// State bersama untuk semua handler. Murah di-clone (isinya `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub works: works::WorksClient,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, AppError> {
        let works = works::WorksClient::new(&config.works_graphql_url)?;
        Ok(Self {
            config: Arc::new(config),
            works,
        })
    }
}

pub fn build_router(state: AppState) -> Router {
    let cors = cors_layer(&state.config.cors_allowed_origins);
    Router::new()
        .merge(routes::health::router())
        .nest("/v1", routes::v1_router())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

fn cors_layer(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .allow_credentials(true)
}
