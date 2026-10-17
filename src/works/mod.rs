//! GraphQL client for works-server (the worksh.io API).
//!
//! Every call carries the logged-in merchant's access token, so works-server
//! enforces permissions and the tenant — this service does not duplicate
//! authorization rules.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::AppError;

#[derive(Clone)]
pub struct WorksClient {
    http: reqwest::Client,
    url: String,
}

#[derive(Serialize)]
struct GraphQlRequest<'a, V: Serialize> {
    query: &'a str,
    variables: V,
}

#[derive(serde::Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(serde::Deserialize)]
struct GraphQlError {
    message: String,
}

impl WorksClient {
    pub fn new(url: &str) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("seller-svc/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::Config(format!("http client: {e}")))?;
        Ok(Self {
            http,
            url: url.to_owned(),
        })
    }

    /// Run one GraphQL operation on behalf of `access_token`.
    pub async fn execute<V, T>(
        &self,
        access_token: &str,
        query: &str,
        variables: V,
    ) -> Result<T, AppError>
    where
        V: Serialize,
        T: DeserializeOwned,
    {
        let res = self
            .http
            .post(&self.url)
            .bearer_auth(access_token)
            .json(&GraphQlRequest { query, variables })
            .send()
            .await
            .map_err(|e| AppError::Works(e.to_string()))?;

        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::Unauthorized);
        }
        let body: GraphQlResponse<T> = res
            .error_for_status()
            .map_err(|e| AppError::Works(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::Works(e.to_string()))?;

        if let Some(first) = body.errors.into_iter().next() {
            return Err(AppError::Works(first.message));
        }
        body.data
            .ok_or_else(|| AppError::Works("response has no data".into()))
    }
}
