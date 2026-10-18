//! seller-svc — backend for the bikinlapak seller app, and (for the
//! hackathon build) the PasStok engine behind its Shop Assistant.
//!
//! Products, stock and the web builder belong to worksh.io and are reached
//! through the works-server GraphQL API (see [`works`]). Until that is wired,
//! [`shop::demo`] supplies the same shapes for one tenant. PasStok itself —
//! forecast, reorder and expiry rules, the agent, the executor — lives in
//! [`engine`], [`agent`] and [`executor`], with its state in [`store`].

pub mod agent;
pub mod clock;
pub mod config;
pub mod contract;
pub mod engine;
pub mod error;
pub mod executor;
pub mod fmt;
pub mod llm;
pub mod media;
pub mod routes;
pub mod shop;
pub mod store;
pub mod token;
pub mod views;
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
    /// PasStok state for the tenant.
    pub store: Arc<store::Store>,
    pub vision: Arc<agent::vision::VisionEngine>,
    /// Gemini, when configured. Words and perception only; `None` = the
    /// deterministic stand-ins everywhere.
    pub llm: Option<Arc<llm::Gemini>>,
}

/// The demo tenant at the start of its day, with the morning run done.
pub fn demo_state(keys: &token::ActionKeys) -> Result<store::State, AppError> {
    let today = clock::today();
    let data = shop::demo::build(today);
    let mut state = store::State::new(data, today);
    state.waste = shop::demo::past_waste(&state.data, today);
    agent::morning_run(&mut state, keys)?;
    Ok(state)
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

        let state = demo_state(&action_keys)?;
        let llm = llm::Gemini::from_config(&config).map(Arc::new);
        let vision = agent::vision::VisionEngine::new(llm.clone());
        tracing::info!(
            gemini = %llm.as_ref().map_or("off".into(), |g| g.label()),
            "Gemini"
        );
        tracing::info!(vision = %vision.source(), "PasStok ready");

        Ok(Self {
            config: Arc::new(config),
            works,
            action_keys: Arc::new(action_keys),
            tokens: Arc::new(token::TokenGuard::new()),
            store: Arc::new(store::Store::new(state)),
            vision: Arc::new(vision),
            llm,
        })
    }
}

/// Background model work after the morning run: today's special from
/// Gemini. Fire-and-forget — the library idea is already on the card, and a
/// failure just leaves it there.
pub fn spawn_ai_refresh(state: &AppState) {
    let Some(g) = state.llm.clone() else { return };
    let store = state.store.clone();
    let keys = state.action_keys.clone();
    tokio::spawn(async move {
        if let Err(e) = agent::idea::refresh_special(&store, &keys, &g).await {
            tracing::warn!(error = %e, "special refresh failed");
        }
    });
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
