//! Assistant: home, threads, settings, and the action path.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::contract::{AiSettings, AssistantHome, ResultCard, Thread};
use crate::{AppError, AppState, seed};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/home", get(home))
        .route("/assistant/thread", get(thread))
        .route("/assistant/settings", get(settings))
        .route("/actions/{token}", post(action))
}

/// Home aggregate: one request per screen.
async fn home() -> Json<AssistantHome> {
    Json(seed::home())
}

#[derive(Deserialize)]
struct ThreadQuery {
    topic: String,
}

async fn thread(
    State(state): State<AppState>,
    Query(q): Query<ThreadQuery>,
) -> Result<Json<Thread>, AppError> {
    seed::thread(&q.topic, &state.action_keys)?
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn settings() -> Json<AiSettings> {
    Json(seed::ai_settings())
}

#[derive(Deserialize)]
struct ActionBody {
    quantity: i64,
}

/// Run an action from a suggestion card.
///
/// This is the only endpoint that changes state on the assistant's say-so, and
/// the order matters: verify the signature, then claim single use along with
/// the quantity cap, and only then mutate. There is no other path — the agent
/// holds no write credential at all.
async fn action(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<ActionBody>,
) -> Result<Json<ResultCard>, AppError> {
    let claims = state.action_keys.verify(&token)?;

    // TODO(Phase 6): the tenant comes from the merchant's session, not a
    // constant.
    let tenant = seed::TENANT;
    state.tokens.claim(&claims, tenant, body.quantity)?;

    tracing::info!(
        tenant = %tenant,
        jti = %claims.jti,
        sug = %claims.sug,
        act = %claims.act,
        quantity = body.quantity,
        cap = claims.max_qty,
        "action approved"
    );

    // TODO(Phase 6): createPurchaseOrder + submitPurchaseOrder through
    // state.works, then write the result card and decision.recorded to the
    // outbox.
    let quantity = body.quantity;
    let unit = &claims.unit;
    let message =
        format!("Hello Sumber Kopi, we'd like to order {quantity} {unit} of Arabica (PO-0771).");

    Ok(Json(ResultCard {
        title: "PO-0771 created".into(),
        detail: format!("{quantity} {unit} · Sumber Kopi · arrives Wednesday"),
        wa_url: format!("https://wa.me/?text={}", urlencode(&message)),
        po_href: "/stock/s1".into(),
    }))
}

/// Percent-encoding for a query string. Enough for the WhatsApp text; reach
/// for `form_urlencoded` if this ever needs to do more.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn encodes_spaces_and_non_ascii() {
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("PO-0771."), "PO-0771.");
        // Make sure multibyte characters survive intact.
        assert_eq!(urlencode("±"), "%C2%B1");
    }
}
