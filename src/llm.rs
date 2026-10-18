//! Gemini client, for words and perception only (PRD §8: "numbers come from
//! SQL and Go, words come from Gemini").
//!
//! Two backends:
//! - **Vertex AI** (`SELLER_SVC_VERTEX_PROJECT`): no key anywhere. On Cloud
//!   Run the token comes from the metadata server as the runtime service
//!   account (needs `roles/aiplatform.user`); locally from Application
//!   Default Credentials (`gcloud auth application-default login`).
//! - **Gemini API** (`SELLER_SVC_GEMINI_API_KEY`), for setups without GCP.
//!
//! Every caller treats a failure as "use the deterministic path": a model
//! outage degrades wording and perception, never the numbers or the writes.
//! A per-day call cap keeps a runaway loop from becoming a bill.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::config::Config;

const TIMEOUT: Duration = Duration::from_secs(25);
const METADATA_TOKEN: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("daily Gemini call cap reached ({0})")]
    Capped(u32),
    #[error("auth: {0}")]
    Auth(String),
    #[error("http: {0}")]
    Http(String),
    #[error("response: {0}")]
    Response(String),
}

enum Creds {
    /// Cloud Run / GCE: the runtime service account.
    Metadata,
    /// `gcloud auth application-default login` on a laptop.
    AuthorizedUser {
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },
}

enum Backend {
    Vertex {
        project: String,
        location: String,
        creds: Creds,
    },
    ApiKey(String),
}

pub struct Gemini {
    http: reqwest::Client,
    backend: Backend,
    pub model: String,
    token: Mutex<Option<(String, Instant)>>,
    usage: Mutex<(NaiveDate, u32)>,
    pub daily_cap: u32,
}

#[derive(Deserialize)]
struct AdcFile {
    #[serde(rename = "type")]
    kind: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
}

fn adc_creds() -> Option<Creds> {
    let path = std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| format!("{h}/.config/gcloud/application_default_credentials.json"))
        })?;
    let raw = std::fs::read_to_string(path).ok()?;
    let f: AdcFile = serde_json::from_str(&raw).ok()?;
    if f.kind != "authorized_user" {
        // Service-account key files are deliberately not supported: the
        // deployed service uses its runtime identity, never a key.
        return None;
    }
    Some(Creds::AuthorizedUser {
        client_id: f.client_id?,
        client_secret: f.client_secret?,
        refresh_token: f.refresh_token?,
    })
}

impl Gemini {
    /// `None` when no backend is configured: callers use the stand-ins.
    pub fn from_config(cfg: &Config) -> Option<Self> {
        let backend = if let Some(key) = &cfg.gemini_api_key {
            Backend::ApiKey(key.clone())
        } else if let Some(project) = &cfg.vertex_project {
            Backend::Vertex {
                project: project.clone(),
                location: cfg.vertex_location.clone(),
                creds: adc_creds().unwrap_or(Creds::Metadata),
            }
        } else {
            return None;
        };
        let http = reqwest::Client::builder().timeout(TIMEOUT).build().ok()?;
        Some(Self {
            http,
            backend,
            model: cfg.gemini_model.clone(),
            token: Mutex::new(None),
            usage: Mutex::new((crate::clock::today(), 0)),
            daily_cap: cfg.gemini_daily_cap,
        })
    }

    pub fn label(&self) -> String {
        match &self.backend {
            Backend::Vertex { .. } => format!("{} · Vertex AI", self.model),
            Backend::ApiKey(_) => format!("{} · Gemini API", self.model),
        }
    }

    /// Calls made today and the cap, for the settings screen.
    pub fn usage(&self) -> (u32, u32) {
        let u = self
            .usage
            .lock()
            .map(|u| *u)
            .unwrap_or((crate::clock::today(), 0));
        let calls = if u.0 == crate::clock::today() { u.1 } else { 0 };
        (calls, self.daily_cap)
    }

    fn count_call(&self) -> Result<(), LlmError> {
        let mut u = self
            .usage
            .lock()
            .map_err(|_| LlmError::Http("usage lock poisoned".into()))?;
        let today = crate::clock::today();
        if u.0 != today {
            *u = (today, 0);
        }
        if u.1 >= self.daily_cap {
            return Err(LlmError::Capped(self.daily_cap));
        }
        u.1 += 1;
        Ok(())
    }

    async fn access_token(&self, creds: &Creds) -> Result<String, LlmError> {
        if let Ok(guard) = self.token.lock()
            && let Some((t, exp)) = guard.as_ref()
            && Instant::now() + Duration::from_secs(60) < *exp
        {
            return Ok(t.clone());
        }
        #[derive(Deserialize)]
        struct Token {
            access_token: String,
            expires_in: u64,
        }
        let res = match creds {
            Creds::Metadata => {
                self.http
                    .get(METADATA_TOKEN)
                    .header("Metadata-Flavor", "Google")
                    .send()
                    .await
            }
            Creds::AuthorizedUser {
                client_id,
                client_secret,
                refresh_token,
            } => {
                self.http
                    .post("https://oauth2.googleapis.com/token")
                    .form(&[
                        ("grant_type", "refresh_token"),
                        ("client_id", client_id),
                        ("client_secret", client_secret),
                        ("refresh_token", refresh_token),
                    ])
                    .send()
                    .await
            }
        }
        .map_err(|e| LlmError::Auth(e.to_string()))?
        .error_for_status()
        .map_err(|e| LlmError::Auth(e.to_string()))?;
        let t: Token = res
            .json()
            .await
            .map_err(|e| LlmError::Auth(e.to_string()))?;
        if let Ok(mut guard) = self.token.lock() {
            *guard = Some((
                t.access_token.clone(),
                Instant::now() + Duration::from_secs(t.expires_in),
            ));
        }
        Ok(t.access_token)
    }

    /// One structured-output call. `parts` are Gemini content parts
    /// (`{"text": …}` or `{"inlineData": …}`); `schema` is the response
    /// schema. Temperature is low: these are extraction and rewording jobs.
    pub async fn json<T: DeserializeOwned>(
        &self,
        system: &str,
        parts: Vec<Value>,
        schema: Value,
    ) -> Result<T, LlmError> {
        self.count_call()?;
        let body = json!({
            "systemInstruction": { "parts": [{ "text": system }] },
            "contents": [{ "role": "user", "parts": parts }],
            "generationConfig": {
                "temperature": 0.2,
                "responseMimeType": "application/json",
                "responseSchema": schema,
            }
        });
        let req = match &self.backend {
            Backend::Vertex {
                project,
                location,
                creds,
            } => {
                let host = if location == "global" {
                    "aiplatform.googleapis.com".to_owned()
                } else {
                    format!("{location}-aiplatform.googleapis.com")
                };
                let url = format!(
                    "https://{host}/v1/projects/{project}/locations/{location}/publishers/google/models/{}:generateContent",
                    self.model
                );
                let token = self.access_token(creds).await?;
                self.http.post(url).bearer_auth(token)
            }
            Backend::ApiKey(key) => self
                .http
                .post(format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                    self.model
                ))
                .header("x-goog-api-key", key),
        };
        let started = Instant::now();
        let res = req
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let status = res.status();
        let v: Value = res
            .json()
            .await
            .map_err(|e| LlmError::Response(e.to_string()))?;
        if !status.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("no message");
            return Err(LlmError::Http(format!("{status}: {msg}")));
        }
        tracing::info!(
            model = %self.model,
            ms = started.elapsed().as_millis() as u64,
            tokens_in = v["usageMetadata"]["promptTokenCount"].as_u64(),
            tokens_out = v["usageMetadata"]["candidatesTokenCount"].as_u64(),
            "gemini call"
        );
        let text = v["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| LlmError::Response("no text in response".into()))?;
        serde_json::from_str(text).map_err(|e| LlmError::Response(e.to_string()))
    }
}

/// The numbers in a text, normalised so "18,000", "18.000" and "18000" are
/// the same number and "1.8" / "1,8" match each other.
pub fn numbers_in(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut s = String::new();
            while i < chars.len()
                && (chars[i].is_ascii_digit()
                    || (matches!(chars[i], '.' | ',')
                        && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())))
            {
                if chars[i].is_ascii_digit() {
                    s.push(chars[i]);
                }
                i += 1;
            }
            let trimmed = s.trim_start_matches('0');
            out.insert(if trimmed.is_empty() {
                "0".into()
            } else {
                trimmed.into()
            });
        } else {
            i += 1;
        }
    }
    out
}

/// True when every number in `output` already appears in `source` — the
/// guard that keeps a rewording model from introducing a figure.
pub fn grounded(output: &str, source: &str) -> bool {
    let allowed = numbers_in(source);
    numbers_in(output).is_subset(&allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_normalise_separators() {
        let n = numbers_in("13 kg, Rp18,000 or Rp18.000, 1.8 a day, 07.00");
        assert!(n.contains("13"));
        assert!(n.contains("18000"));
        assert!(n.contains("18"));
        assert!(n.contains("700"));
    }

    #[test]
    fn a_translation_that_keeps_the_numbers_is_grounded() {
        let en = "13 kg covers the next 9 days at about 1.8 kg a day.";
        let id = "13 kg cukup untuk 9 hari ke depan, sekitar 1,8 kg per hari.";
        assert!(grounded(id, en));
    }

    #[test]
    fn an_invented_number_is_caught() {
        let en = "13 kg covers the next 9 days.";
        assert!(!grounded("15 kg cukup untuk 9 hari.", en));
    }
}
