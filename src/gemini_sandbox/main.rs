//! Sandbox: try Gemini models on Vertex AI (Agent Platform) with ADC — no
//! API key. Kept out of the service itself; this is a place to poke at
//! models before wiring them into `src/agent/`.
//!
//! Docs: "Get started with Gemini 3/4" on cloud.google.com — both use
//! `locations/global` and `generateContent`, with the caller's Google
//! identity as the credential (`roles/aiplatform.user` on the service
//! account / user is the only IAM needed).
//!
//! Bearer token resolution, in order:
//!   1. `GCP_ACCESS_TOKEN` env var, if set (handy for scripted runs)
//!   2. the metadata server, when running on Cloud Run (`K_SERVICE` set) —
//!      the production path: the runtime identity is the credential
//!   3. `gcloud auth print-access-token` on a developer machine
//!
//! Run:
//!   make gemini-sandbox                        # default model suite
//!   cargo run --bin gemini_sandbox -- gemini-4-argon "Say OK"
//!   VERTEX_MODEL=gemini-3.6-flash make gemini-sandbox
//!
//! Failures print per model and don't stop the suite: a 404 usually means
//! the model id or location is wrong for this project, a 403 means IAM.

use std::time::Instant;

use serde_json::{Value, json};

const DEFAULT_PROJECT: &str = "project-9567c06f-f9aa-4397-a2c";
const DEFAULT_LOCATION: &str = "global";
const DEFAULT_PROMPT: &str = "Explain how AI works in a few words";
/// Baseline (stable), the docs' current 3.x, the first 3.x preview, and 4.
const DEFAULT_SUITE: &[&str] = &[
    "gemini-2.5-flash",
    "gemini-3-flash-preview",
    "gemini-3.6-flash",
    "gemini-4-argon",
];

const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";
const API_ENDPOINT: &str = "https://aiplatform.googleapis.com";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project = env_or("VERTEX_PROJECT", DEFAULT_PROJECT);
    let location = env_or("VERTEX_LOCATION", DEFAULT_LOCATION);
    let prompt = env_or("VERTEX_PROMPT", DEFAULT_PROMPT);

    // Optional args override the default suite: `gemini_sandbox MODEL...`
    let args: Vec<String> = std::env::args().skip(1).collect();
    let models: Vec<String> = match std::env::var("VERTEX_MODEL") {
        Ok(m) => vec![m],
        Err(_) if !args.is_empty() => args,
        Err(_) => DEFAULT_SUITE.iter().map(|s| s.to_string()).collect(),
    };

    let token = token().await?;
    let client = reqwest::Client::new();

    println!("project:  {project} @ {location}");
    println!("prompt:   {prompt}\n");

    let mut failures = 0;
    for model in &models {
        match call(&client, &token, &project, &location, model, &prompt).await {
            Ok((elapsed, reply)) => {
                println!("{model:<24} {:>5.1}s  {reply}", elapsed.as_secs_f32());
            }
            Err(e) => {
                failures += 1;
                eprintln!("{model:<24} FAILED  {e}");
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}

async fn call(
    client: &reqwest::Client,
    token: &str,
    project: &str,
    location: &str,
    model: &str,
    prompt: &str,
) -> Result<(std::time::Duration, String), String> {
    let url = format!(
        "{API_ENDPOINT}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent"
    );
    // Request body straight from the docs: one user turn, one text part.
    let body = json!({ "contents": { "role": "user", "parts": { "text": prompt } } });

    let started = Instant::now();
    let res = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();

    let status = res.status();
    let payload: Value = res.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let msg = payload["error"]["message"].as_str().unwrap_or("?");
        return Err(format!("HTTP {status}: {msg}"));
    }

    let text = payload["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("(unexpected response shape)")
        .replace('\n', " ");
    Ok((elapsed, text))
}

async fn token() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(t) = std::env::var("GCP_ACCESS_TOKEN") {
        return Ok(t);
    }

    // On Cloud Run the runtime service account's token comes from the
    // metadata server — no key material anywhere.
    if std::env::var("K_SERVICE").is_ok() {
        let res = reqwest::Client::new()
            .get(METADATA_TOKEN_URL)
            .header("Metadata-Flavor", "Google")
            .send()
            .await?;
        let v: Value = res.json().await?;
        return Ok(v["access_token"]
            .as_str()
            .ok_or("metadata server returned no access_token")?
            .to_string());
    }

    // Developer machine: same token the curl example in the docs uses.
    let out = std::process::Command::new("gcloud")
        .args(["auth", "print-access-token"])
        .output()?;
    if !out.status.success() {
        return Err("gcloud auth print-access-token failed — run `gcloud auth login`".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}
