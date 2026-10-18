//! Waste photos, photo evidence, and donation receipts.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::agent::{self, vision};
use crate::fmt::{idr, qty};
use crate::store::Photo;
use crate::{AppError, AppState, media};

pub const MAX_PHOTO_BYTES: usize = 8 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/waste/photos",
            post(upload).layer(DefaultBodyLimit::max(MAX_PHOTO_BYTES)),
        )
        .route("/photos/{id}", get(photo))
        .route("/donations/{id}/receipt.pdf", get(receipt))
}

/// Raw image bytes in, a waste session out (FR-32, FR-33).
async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    ingest(state, headers, body, vision::Purpose::Waste).await
}

/// Raw image bytes in, a stock-count session out. Mounted by the stock
/// router at `/v1/stock/photos`.
pub async fn count(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    ingest(state, headers, body, vision::Purpose::Stock).await
}

/// Shared photo intake. Metadata is stripped before the photo is stored or
/// sent anywhere; vision proposes, [`vision::ground`] keeps only the shop's
/// own candidates, and the card waits for the merchant.
async fn ingest(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
    purpose: vision::Purpose,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.is_empty() {
        return Err(AppError::BadRequest("no photo".into()));
    }
    let content_type =
        media::sniff(&body).ok_or_else(|| AppError::BadRequest("JPEG, PNG or WebP only".into()))?;
    if let Some(declared) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        && !declared.starts_with("image/")
        && declared != "application/octet-stream"
    {
        return Err(AppError::BadRequest("expected an image".into()));
    }
    let clean = media::strip_metadata(&body);

    let (photo_id, candidates, today) = {
        let mut s = state.store.lock()?;
        let id = s.next_id("ph");
        s.photos.insert(
            id.clone(),
            Photo {
                bytes: clean.clone(),
                content_type: content_type.into(),
            },
        );
        (id, vision::candidates(&s, purpose), s.today)
    };

    // The model call happens outside the lock.
    let detections = state
        .vision
        .analyze(&clean, content_type, &candidates, today, purpose)
        .await;

    let (session, topic) = {
        let mut s = state.store.lock()?;
        match purpose {
            vision::Purpose::Waste => (
                agent::post_waste_session(&mut s, &state.action_keys, &photo_id, &detections)?,
                "waste",
            ),
            vision::Purpose::Stock => (
                agent::stockcount::post_count_session(
                    &mut s,
                    &state.action_keys,
                    &photo_id,
                    &detections,
                )?,
                "stock",
            ),
        }
    };
    state.store.touched(topic);
    Ok(Json(
        json!({ "sessionId": session, "photoId": photo_id, "detected": detections.len() }),
    ))
}

async fn photo(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let s = state.store.lock()?;
    let p = s.photos.get(&id).ok_or(AppError::NotFound)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, p.content_type.clone()),
            (header::CACHE_CONTROL, "private, max-age=3600".into()),
        ],
        p.bytes.clone(),
    )
        .into_response())
}

/// Donation receipt (FR-49): number, date, donor, recipient, items with
/// expiry and value at cost, signature lines.
async fn receipt(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let id = id.trim_end_matches(".pdf").to_owned();
    let s = state.store.lock()?;
    let d = s.donations.get(&id).ok_or(AppError::NotFound)?;
    let Some(done) = d.completed_on else {
        return Err(AppError::NotFound);
    };
    let recipient = d
        .recipient_id
        .as_deref()
        .and_then(|r| s.recipients.iter().find(|x| x.id == r))
        .map_or("—".to_string(), |r| r.name.clone());
    let mut lines = vec![
        format!("Number: {}", d.id),
        format!("Date: {}", done.format("%-d %B %Y")),
        format!("Donor: {} ({})", s.data.name, s.data.location),
        format!("Recipient: {recipient}"),
        format!("Pickup: {}", d.pickup_window.clone().unwrap_or_default()),
        String::new(),
        "Items".into(),
    ];
    let mut total = 0;
    for i in &d.items {
        if let Some(p) = s.data.product(&i.product_id) {
            let v = i.qty * p.cost;
            total += v;
            lines.push(format!(
                "  {} {} - best before {} - {} at cost",
                qty(i.qty, &p.unit),
                p.name,
                i.expiry
                    .map_or("n/a".into(), |e| e.format("%-d %b %Y").to_string()),
                idr(v)
            ));
        }
    }
    lines.extend([
        format!("Total value at cost: {}", idr(total)),
        String::new(),
        "Food-safety checklist: before expiry date, storage temperature, sealed, allergens labelled - all confirmed by the donor.".into(),
        String::new(),
        String::new(),
        "Donor: ______________________        Recipient: ______________________".into(),
    ]);
    let pdf = media::text_pdf("Food donation receipt", &lines);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{}.pdf\"", d.id),
            ),
        ],
        pdf,
    )
        .into_response())
}
