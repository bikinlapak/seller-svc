//! seller-svc — backend for the bikinlapak seller app.
//!
//! This service owns no product or page data of its own: products and the web
//! builder live in worksh.io and are reached through the works-server GraphQL
//! API (see [`works`]).

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

/// Shared state for every handler. Cheap to clone — it is all `Arc`s.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub works: works::WorksClient,
    /// Signing keys for action tokens.
    pub action_keys: Arc<token::ActionKeys>,
    /// Record of spent tokens — single use is enforced here.
    pub tokens: Arc<token::TokenGuard>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, AppError> {
        let works = works::WorksClient::new(&config.works_graphql_url)?;

        // With no key from Secret Manager, generate a throwaway one. Tokens
        // then die on every restart — right for development, wrong for
        // production, so say so loudly.
        let action_keys = match &config.action_key_pkcs8 {
            Some(der) => {
                let pair = ring::signature::Ed25519KeyPair::from_pkcs8(der).map_err(|_| {
                    AppError::Config(
                        "SELLER_SVC_ACTION_KEY is not a valid Ed25519 PKCS#8 key".into(),
                    )
                })?;
                use ring::signature::KeyPair;
                token::ActionKeys::new(der, pair.public_key().as_ref())
            }
            None => {
                tracing::warn!(
                    "SELLER_SVC_ACTION_KEY is empty: using a throwaway key. \
                     Action tokens die on every restart — not for production."
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
