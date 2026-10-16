//! seller-svc — backend dasbor penjual bikinlapak.
//!
//! Layanan ini tidak menyimpan data produk/halaman sendiri: produk dan web
//! builder dikelola di worksh.io, diakses lewat GraphQL works-server
//! (lihat [`works`]).

pub mod config;
pub mod contract;
pub mod error;
pub mod routes;
pub mod seed;
pub mod token;
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
    /// Kunci penandatangan token aksi.
    pub action_keys: Arc<token::ActionKeys>,
    /// Catatan token yang sudah terpakai — sekali pakai ditegakkan di sini.
    pub tokens: Arc<token::TokenGuard>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, AppError> {
        let works = works::WorksClient::new(&config.works_graphql_url)?;

        // Tanpa kunci dari Secret Manager, buat yang sementara. Token jadi
        // hangus tiap restart — benar untuk dev, salah untuk produksi, jadi
        // katakan dengan lantang.
        let action_keys = match &config.action_key_pkcs8 {
            Some(der) => {
                let pair = ring::signature::Ed25519KeyPair::from_pkcs8(der).map_err(|_| {
                    AppError::Config("SELLER_SVC_ACTION_KEY bukan PKCS#8 Ed25519 yang sah".into())
                })?;
                use ring::signature::KeyPair;
                token::ActionKeys::new(der, pair.public_key().as_ref())
            }
            None => {
                tracing::warn!(
                    "SELLER_SVC_ACTION_KEY kosong: memakai kunci sementara. \
                     Token aksi hangus setiap restart — jangan dipakai di produksi."
                );
                token::ActionKeys::ephemeral()?
            }
        };

        Ok(Self {
            config: Arc::new(config),
            works,
            action_keys: Arc::new(action_keys),
            tokens: Arc::new(token::TokenGuard::new()),
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
