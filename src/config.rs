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
    /// Kunci privat Ed25519 (PKCS#8 DER, base64) untuk token aksi. Kosong =
    /// buat kunci sementara saat start; hanya untuk dev.
    pub action_key_pkcs8: Option<Vec<u8>>,
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

        let action_key_pkcs8 = match get("SELLER_SVC_ACTION_KEY") {
            Some(b64) if !b64.trim().is_empty() => Some(
                base64_decode(b64.trim())
                    .ok_or_else(|| AppError::Config("SELLER_SVC_ACTION_KEY bukan base64".into()))?,
            ),
            _ => None,
        };

        Ok(Self {
            bind_addr,
            log_json,
            works_graphql_url,
            cors_allowed_origins,
            action_key_pkcs8,
        })
    }
}

/// base64 standar (dengan atau tanpa padding). Satu fungsi kecil lebih murah
/// daripada menarik crate hanya untuk membaca satu env var.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const TABEL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut nilai = [255u8; 256];
    for (i, c) in TABEL.iter().enumerate() {
        nilai[*c as usize] = i as u8;
    }

    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bit = 0u32;
    for c in s.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()) {
        let v = nilai[c as usize];
        if v == 255 {
            return None;
        }
        buf = (buf << 6) | u32::from(v);
        bit += 6;
        if bit >= 8 {
            bit -= 8;
            out.push((buf >> bit) as u8);
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
    fn action_key_kosong_berarti_kunci_sementara() {
        let cfg = Config::from_lookup(|_| None).unwrap();
        assert!(cfg.action_key_pkcs8.is_none());
    }

    #[test]
    fn action_key_base64_dibaca() {
        // "hello world" -> aGVsbG8gd29ybGQ=
        let cfg = Config::from_lookup(|k| {
            (k == "SELLER_SVC_ACTION_KEY").then(|| "aGVsbG8gd29ybGQ=".into())
        })
        .unwrap();
        assert_eq!(cfg.action_key_pkcs8.unwrap(), b"hello world");
    }

    #[test]
    fn action_key_bukan_base64_ditolak() {
        let err =
            Config::from_lookup(|k| (k == "SELLER_SVC_ACTION_KEY").then(|| "bukan base64!".into()));
        assert!(matches!(err, Err(AppError::Config(_))));
    }

    #[test]
    fn rejects_invalid_bind_addr() {
        let err = Config::from_lookup(|k| (k == "SELLER_SVC_BIND_ADDR").then(|| "nope".into()));
        assert!(matches!(err, Err(AppError::Config(_))));
    }
}
