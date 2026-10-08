use std::net::SocketAddr;

use crate::AppError;

/// Konfigurasi dibaca sekali saat start dari environment (prefix `SELLER_SVC_`).
/// Daftar lengkapnya ada di `.env.example`.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub log_json: bool,
    /// Endpoint GraphQL works-server, mis. `http://127.0.0.1:8000/graphql`.
    pub works_graphql_url: String,
    pub cors_allowed_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, AppError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Dipisah dari `from_env` supaya bisa dites tanpa mengubah env proses.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, AppError> {
        let bind_addr = get("SELLER_SVC_BIND_ADDR")
            .unwrap_or_else(|| "127.0.0.1:8080".into())
            .parse()
            .map_err(|e| AppError::Config(format!("SELLER_SVC_BIND_ADDR: {e}")))?;

        let log_json = matches!(get("SELLER_SVC_LOG_JSON").as_deref(), Some("1" | "true"));

        let works_graphql_url = get("SELLER_SVC_WORKS_GRAPHQL_URL")
            .unwrap_or_else(|| "http://127.0.0.1:8000/graphql".into());

        let cors_allowed_origins = get("SELLER_SVC_CORS_ORIGINS")
            .unwrap_or_else(|| "http://localhost:5173".into())
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Self {
            bind_addr,
            log_json,
            works_graphql_url,
            cors_allowed_origins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_env_is_empty() {
        let cfg = Config::from_lookup(|_| None).unwrap();
        assert_eq!(cfg.bind_addr.port(), 8080);
        assert_eq!(cfg.cors_allowed_origins, vec!["http://localhost:5173"]);
    }

    #[test]
    fn rejects_invalid_bind_addr() {
        let err = Config::from_lookup(|k| (k == "SELLER_SVC_BIND_ADDR").then(|| "nope".into()));
        assert!(matches!(err, Err(AppError::Config(_))));
    }
}
