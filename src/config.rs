use std::net::SocketAddr;

use crate::AppError;

/// Read once at start-up from the environment (prefix `SELLER_SVC_`).
/// The full list lives in `.env.example`.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub log_json: bool,
    /// works-server GraphQL endpoint, e.g. `http://127.0.0.1:8000/graphql`.
    pub works_graphql_url: String,
    pub cors_allowed_origins: Vec<String>,
    /// Ed25519 private key (PKCS#8 DER, base64) for action tokens. Empty =
    /// generate a throwaway key at start-up; development only.
    pub action_key_pkcs8: Option<Vec<u8>>,
    /// Gemini via Vertex AI in this project (no key: runtime identity or
    /// ADC). Empty, with no API key either = deterministic stand-ins only.
    pub vertex_project: Option<String>,
    /// Vertex location. Flash-lite models are served on `global`.
    pub vertex_location: String,
    /// Gemini API key, for setups without GCP. Takes precedence over Vertex.
    pub gemini_api_key: Option<String>,
    pub gemini_model: String,
    /// Model calls allowed per day before falling back to the stand-ins.
    pub gemini_daily_cap: u32,
}

impl Config {
    pub fn from_env() -> Result<Self, AppError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Kept separate from `from_env` so it can be tested without touching
    /// the process environment.
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

        let action_key_pkcs8 = match get("SELLER_SVC_ACTION_KEY") {
            Some(b64) if !b64.trim().is_empty() => Some(
                base64_decode(b64.trim())
                    .ok_or_else(|| AppError::Config("SELLER_SVC_ACTION_KEY bukan base64".into()))?,
            ),
            _ => None,
        };

        let nonempty = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let gemini_api_key = nonempty("SELLER_SVC_GEMINI_API_KEY");
        let vertex_project = nonempty("SELLER_SVC_VERTEX_PROJECT");
        let vertex_location =
            nonempty("SELLER_SVC_VERTEX_LOCATION").unwrap_or_else(|| "global".into());
        let gemini_model =
            nonempty("SELLER_SVC_GEMINI_MODEL").unwrap_or_else(|| "gemini-3.5-flash-lite".into());
        let gemini_daily_cap = match nonempty("SELLER_SVC_GEMINI_DAILY_CAP") {
            Some(v) => v
                .parse()
                .map_err(|e| AppError::Config(format!("SELLER_SVC_GEMINI_DAILY_CAP: {e}")))?,
            None => 2000,
        };

        Ok(Self {
            bind_addr,
            log_json,
            works_graphql_url,
            cors_allowed_origins,
            action_key_pkcs8,
            vertex_project,
            vertex_location,
            gemini_api_key,
            gemini_model,
            gemini_daily_cap,
        })
    }
}

/// Standard base64, padded or not. One small function is cheaper than
/// pulling in a crate to read a single environment variable.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut table = [255u8; 256];
    for (i, c) in ALPHABET.iter().enumerate() {
        table[*c as usize] = i as u8;
    }

    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in s.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()) {
        let v = table[c as usize];
        if v == 255 {
            return None;
        }
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
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
    fn no_action_key_means_an_ephemeral_one() {
        let cfg = Config::from_lookup(|_| None).unwrap();
        assert!(cfg.action_key_pkcs8.is_none());
    }

    #[test]
    fn reads_a_base64_action_key() {
        // "hello world" -> aGVsbG8gd29ybGQ=
        let cfg = Config::from_lookup(|k| {
            (k == "SELLER_SVC_ACTION_KEY").then(|| "aGVsbG8gd29ybGQ=".into())
        })
        .unwrap();
        assert_eq!(cfg.action_key_pkcs8.unwrap(), b"hello world");
    }

    #[test]
    fn rejects_an_action_key_that_is_not_base64() {
        let err =
            Config::from_lookup(|k| (k == "SELLER_SVC_ACTION_KEY").then(|| "not base64!".into()));
        assert!(matches!(err, Err(AppError::Config(_))));
    }

    #[test]
    fn gemini_is_off_unless_configured() {
        let cfg = Config::from_lookup(|_| None).unwrap();
        assert!(cfg.vertex_project.is_none() && cfg.gemini_api_key.is_none());
        assert_eq!(cfg.vertex_location, "global");
        assert_eq!(cfg.gemini_model, "gemini-3.5-flash-lite");
    }

    #[test]
    fn rejects_invalid_bind_addr() {
        let err = Config::from_lookup(|k| (k == "SELLER_SVC_BIND_ADDR").then(|| "nope".into()));
        assert!(matches!(err, Err(AppError::Config(_))));
    }
}
