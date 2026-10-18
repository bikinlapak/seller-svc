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

/// The merchant writes; the assistant answers in the same request:
///
/// 1. store the message;
/// 2. understand it — Gemini when configured and the mode is not Frugal,
///    keyword rules otherwise;
/// 3. compute the answer from the engine;
/// 4. reword it into the merchant's language if that is not English.
///
/// Model calls happen outside the state lock, and any failure falls back to
/// the deterministic path.
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
    let topic = body.topic.as_str();
    let (products, open, mode, language) = {
        let mut s = state.store.lock()?;
        if agent::topic_title(topic).is_none() {
            return Err(AppError::NotFound);
        }
        answer::push_staff(&mut s, topic, text);
        let products: Vec<String> = s.data.products.iter().map(|p| p.name.clone()).collect();
        let open = s.threads.get(topic).and_then(|m| {
            m.iter().rev().find_map(|x| match x {
                crate::contract::Message::SuggestionCard { card, .. }
                    if card.state == crate::contract::CardState::Open =>
                {
                    Some(card.title.clone())
                }
                _ => None,
            })
        });
        (
            products,
            open,
            s.settings.mode.clone(),
            s.settings.language.clone(),
        )
    };
    state.store.touched(topic);

    let llm = state.llm.as_ref().filter(|_| mode != "frugal");
    let intent = match llm {
        Some(g) => answer::llm_intent(g, text, &products, open.as_deref())
            .await
            .unwrap_or_else(|| answer::rules(text)),
        None => answer::rules(text),
    };
    tracing::debug!(?intent, "understood");

    let mut reply = {
        let mut s = state.store.lock()?;
        answer::answer_intent(&mut s, &state.action_keys, topic, &intent)?
    };

    if let (Some(g), crate::contract::Message::Text { text, actions, .. }) =
        (state.llm.as_ref(), &mut reply)
    {
        let labels: Vec<String> = actions.iter().flatten().map(|a| a.label.clone()).collect();
        if let Some((t, l)) = answer::localise(g, &language, text, &labels).await {
            *text = t;
            for (a, label) in actions.iter_mut().flatten().zip(l) {
                a.label = label;
            }
        }
    }

    let thread = {
        let mut s = state.store.lock()?;
        s.push(topic, reply);
        agent::thread(&s, topic, &state.vision.source()).ok_or(AppError::NotFound)?
    };
    state.store.touched(topic);
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

fn ai_info(state: &AppState) -> views::AiInfo {
    let (calls_today, daily_cap) = state.llm.as_ref().map_or((0, 0), |g| g.usage());
    views::AiInfo {
        model: state.llm.as_ref().map(|g| g.model.clone()),
        calls_today,
        daily_cap,
    }
}

async fn settings(State(state): State<AppState>) -> Result<Json<AiSettings>, AppError> {
    let s = state.store.lock()?;
    Ok(Json(views::settings(&s, &ai_info(&state))))
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
    Ok(Json(views::settings(&s, &ai_info(&state))))
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
