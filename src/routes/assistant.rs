//! Assistant: home, threads, messages, the live stream, settings, reports to
//! moderation, and the action path.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::agent::{self, answer};
use crate::contract::{AiSettings, AssistantHome, ResultCard, SettingsUpdate, Thread};
use crate::executor::{self, ActionBody};
use crate::store::ModerationReport;
use crate::{AppError, AppState, views};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/home", get(home))
        .route("/assistant/thread", get(thread))
        .route("/assistant/messages", post(message))
        .route("/assistant/stream", get(stream))
        .route("/assistant/settings", get(settings).put(update_settings))
        .route("/assistant/report", post(report))
        .route("/actions/{token}", post(action))
        .route("/actions/{token}/reject", post(reject))
}

/// Home aggregate: one request per screen.
async fn home(State(state): State<AppState>) -> Result<Json<AssistantHome>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(agent::home(&s)))
}

#[derive(Deserialize)]
struct ThreadQuery {
    topic: String,
}

async fn thread(
    State(state): State<AppState>,
    Query(q): Query<ThreadQuery>,
) -> Result<Json<Thread>, AppError> {
    let s = state.store.lock()?;
    agent::thread(&s, &q.topic, &state.vision.source())
        .map(Json)
        .ok_or(AppError::NotFound)
}

#[derive(Deserialize)]
struct MessageBody {
    topic: String,
    text: String,
}

/// The merchant writes; the assistant answers in the same request. Answers
/// are computed, not generated, so there is nothing slow to stream.
async fn message(
    State(state): State<AppState>,
    Json(body): Json<MessageBody>,
) -> Result<Json<Thread>, AppError> {
    let text = body.text.trim();
    if text.is_empty() || text.chars().count() > 500 {
        return Err(AppError::BadRequest(
            "a message is 1 to 500 characters".into(),
        ));
    }
    let thread = {
        let mut s = state.store.lock()?;
        if agent::topic_title(&body.topic).is_none() {
            return Err(AppError::NotFound);
        }
        answer::reply(&mut s, &state.action_keys, &body.topic, text)?;
        agent::thread(&s, &body.topic, &state.vision.source()).ok_or(AppError::NotFound)?
    };
    state.store.touched(&body.topic);
    Ok(Json(thread))
}

/// Server-sent events: `change` whenever the thread (or everything, `*`)
/// changes. The client refetches; messages are not duplicated on the wire.
async fn stream(
    State(state): State<AppState>,
    Query(q): Query<ThreadQuery>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let topic = q.topic;
    let rx = BroadcastStream::new(state.store.changes.subscribe());
    let events = rx.filter_map(move |msg| match msg {
        Ok(t) if t == topic || t == "*" => Some(Ok(Event::default().event("change").data(t))),
        // Lagged: the client may have missed changes — tell it to resync.
        Err(_) => Some(Ok(Event::default().event("resync").data("*"))),
        _ => None,
    });
    Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("ping"),
    )
}

async fn settings(State(state): State<AppState>) -> Result<Json<AiSettings>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(views::settings(&s)))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(body): Json<SettingsUpdate>,
) -> Result<Json<AiSettings>, AppError> {
    let mut s = state.store.lock()?;
    if let Some(m) = body.selected_mode {
        if !["frugal", "balanced", "thorough"].contains(&m.as_str()) {
            return Err(AppError::BadRequest(format!("unknown mode {m}")));
        }
        s.settings.mode = m;
    }
    for (k, v) in body.autonomy {
        if !["small-po", "markdown"].contains(&k.as_str()) {
            return Err(AppError::BadRequest(format!("unknown setting {k}")));
        }
        s.settings.autonomy.insert(k, v);
    }
    if let Some(l) = body.language {
        if !["English", "Indonesia · santai", "Indonesia · formal"].contains(&l.as_str()) {
            return Err(AppError::BadRequest(format!("unknown language {l}")));
        }
        s.settings.language = l;
    }
    Ok(Json(views::settings(&s)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportBody {
    topic: String,
    message_id: String,
    #[serde(default)]
    reason: String,
}

/// "Report answer" → moderation queue (FR-30, Permendag 19/2026 Art. 47).
async fn report(
    State(state): State<AppState>,
    Json(body): Json<ReportBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut s = state.store.lock()?;
    let exists = s
        .threads
        .get(&body.topic)
        .is_some_and(|m| m.iter().any(|x| x.id() == body.message_id));
    if !exists {
        return Err(AppError::NotFound);
    }
    s.moderation.push(ModerationReport {
        message_id: body.message_id.clone(),
        topic: body.topic.clone(),
        reason: body.reason.chars().take(300).collect(),
    });
    s.emit(
        "passtok.moderation.reported",
        serde_json::json!({ "message_id": body.message_id, "topic": body.topic }),
    );
    Ok(Json(serde_json::json!({ "queued": s.moderation.len() })))
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|k| !k.is_empty() && k.len() <= 128)
        .map(str::to_owned)
}

/// Run an action from a card.
///
/// The only endpoint that changes state on the assistant's say-so: verify
/// the signature, then the executor validates, claims single use with the
/// cap, and only then mutates. A retry with the same `Idempotency-Key` gets
/// the first answer back instead of a 409 (FR-19).
async fn action(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: Option<Json<ActionBody>>,
) -> Result<Json<ResultCard>, AppError> {
    let key = idempotency_key(&headers);
    let claims = state.action_keys.verify(&token)?;
    let body = body.map(|Json(b)| b).unwrap_or_default();

    let (card, topic) = {
        let mut s = state.store.lock()?;
        if let Some(prev) = key.as_ref().and_then(|k| s.idempotent.get(k)) {
            return Ok(Json(prev.clone()));
        }
        let card = executor::execute(&mut s, &state.action_keys, &state.tokens, &claims, &body)?;
        if let Some(k) = key {
            s.idempotent.insert(k, card.clone());
        }
        let topic = s
            .suggestions
            .get(&claims.sug)
            .map(|x| x.topic.clone())
            .unwrap_or_else(|| "*".into());
        (card, topic)
    };
    tracing::info!(act = %claims.act, sug = %claims.sug, "action applied");
    state.store.touched(&topic);
    if claims.act == "record_waste" {
        state.store.touched("donation");
    }
    Ok(Json(card))
}

async fn reject(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<ResultCard>, AppError> {
    let claims = state.action_keys.verify(&token)?;
    let card = {
        let mut s = state.store.lock()?;
        executor::reject(&mut s, &state.tokens, &claims)?
    };
    state.store.touched("*");
    Ok(Json(card))
}
