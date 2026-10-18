//! Router tests through `tower::ServiceExt::oneshot` — no port is opened.
//!
//! These walk the demo's flows end to end against the real engine and
//! executor: suggestion → approve → PO, Q&A, markdown, special, waste photo →
//! write-off → donation with checklist → receipt.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use seller_svc::{AppState, Config, build_router};
use serde_json::{Value, json};
use tower::ServiceExt;

fn state() -> AppState {
    AppState::new(Config::from_lookup(|_| None).unwrap()).unwrap()
}

static KEY: AtomicU64 = AtomicU64::new(0);

/// One service instance per test.
///
/// It has to be shared rather than rebuilt per request: the signing keys, the
/// spent-token record and the store live in `AppState`, so two instances mean
/// two key pairs and a token from one is rejected by the other.
struct Svc {
    app: axum::Router,
}

impl Svc {
    fn new() -> Self {
        Self {
            app: build_router(state()),
        }
    }

    async fn raw(&self, req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        (
            status,
            res.into_body().collect().await.unwrap().to_bytes().to_vec(),
        )
    }

    async fn send(&self, req: Request<Body>) -> (StatusCode, Value) {
        let (status, body) = self.raw(req).await;
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    /// A POST with a fresh idempotency key: every call is a new tap.
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let key = format!("k{}", KEY.fetch_add(1, Ordering::Relaxed));
        self.post_with_key(path, body, &key).await
    }

    async fn post_with_key(&self, path: &str, body: Value, key: &str) -> (StatusCode, Value) {
        self.send(
            Request::post(path)
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    async fn thread(&self, topic: &str) -> Value {
        let (status, json) = self
            .get(&format!("/v1/assistant/thread?topic={topic}"))
            .await;
        assert_eq!(status, StatusCode::OK, "thread {topic}");
        json
    }

    /// The first open card in a thread whose title contains `needle`.
    async fn card(&self, topic: &str, needle: &str) -> Value {
        let t = self.thread(topic).await;
        t["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| {
                m["type"] == "suggestion_card"
                    && m["card"]["title"].as_str().unwrap().contains(needle)
            })
            .unwrap_or_else(|| panic!("no card with {needle:?} in {topic}: {t}"))
            .clone()
    }

    async fn arabica_token(&self) -> String {
        self.card("reorder", "Arabica").await["card"]["token"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

fn messages(thread: &Value) -> &Vec<Value> {
    thread["messages"].as_array().unwrap()
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (status, json) = Svc::new().get("/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn health_alias_returns_ok() {
    let svc = Svc::new();
    let (status, json) = svc.get("/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn home_is_computed_and_every_link_has_a_thread() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/home").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["shop"]["name"], "Jasa Coffee Shop");
    let attention = json["attention"].as_array().unwrap();
    assert!(
        attention.iter().any(|a| a["title"]
            .as_str()
            .unwrap()
            .starts_with("Arabica Beans 1 kg runs out")),
        "{attention:?}"
    );
    assert!(attention.iter().any(|a| a["icon"] == "expiry"));
    for item in attention.iter().chain(json["commands"].as_array().unwrap()) {
        let topic = item["href"]
            .as_str()
            .unwrap()
            .trim_start_matches("/assistant/");
        svc.thread(topic).await;
    }
}

#[tokio::test]
async fn an_unknown_thread_is_404_and_a_missing_topic_400() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/assistant/thread?topic=nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "not_found");
    let (status, _) = svc.get("/v1/assistant/thread").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reorder_cards_are_signed_and_grouped_by_supplier() {
    let svc = Svc::new();
    let arabica = svc.card("reorder", "Arabica").await;
    assert_eq!(arabica["author"], "agent");
    assert_eq!(arabica["card"]["state"], "open");
    assert_eq!(arabica["card"]["unit"], "kg");
    let q = arabica["card"]["quantity"].as_i64().unwrap();
    assert!((10..=15).contains(&q), "{q}");
    let token = arabica["card"]["token"].as_str().unwrap();
    assert_eq!(token.split('.').count(), 3, "not a JWT: {token}");

    let milk = svc.card("reorder", "Susu Nusantara").await;
    assert_eq!(milk["card"]["lines"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn approving_a_reduced_quantity_creates_a_po_and_updates_everything() {
    let svc = Svc::new();
    let token = svc.arabica_token().await;
    let (status, json) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": 9 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["title"].as_str().unwrap().starts_with("PO-"));
    assert!(json["detail"].as_str().unwrap().contains("9 kg"));
    assert!(json["waUrl"].as_str().unwrap().contains("wa.me/62"));
    assert!(json["waUrl"].as_str().unwrap().contains("9%20kg"));

    // The card is handled, the result is in the thread.
    let t = svc.thread("reorder").await;
    let ms = messages(&t);
    assert!(ms.iter().any(|m| m["type"] == "result_card"));
    let card = ms
        .iter()
        .find(|m| {
            m["card"]["title"]
                .as_str()
                .is_some_and(|x| x.contains("Arabica"))
        })
        .unwrap();
    assert_eq!(card["card"]["state"], "approved");

    // Stock knows it's on order.
    let (_, stock) = svc.get("/v1/stock").await;
    let row = stock["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "p3")
        .unwrap();
    assert!(row["tags"].as_array().unwrap().contains(&json!("on_order")));
    assert!(row["right"].as_str().unwrap().starts_with("PO-"));

    // Audit and event outbox carry the decision.
    let (_, activity) = svc.get("/v1/activity").await;
    assert_eq!(activity["audit"][0]["action"], "create_po");
    assert!(
        activity["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["topic"] == "passtok.decision.recorded" && e["decision"] == "edited")
    );
}

#[tokio::test]
async fn an_action_rejects_a_quantity_above_the_cap_without_burning_the_token() {
    let svc = Svc::new();
    let token = svc.arabica_token().await;
    let (status, json) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": 999 }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json["error"]["code"], "qty_above_cap");
    let (status, _) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": 5 }))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn an_action_rejects_a_forged_token() {
    let (status, json) = Svc::new()
        .post("/v1/actions/not.a.jwt", json!({ "quantity": 1 }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"]["code"], "token_invalid");
}

#[tokio::test]
async fn an_action_rejects_a_zero_quantity() {
    let svc = Svc::new();
    let token = svc.arabica_token().await;
    let (status, json) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": 0 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn an_action_token_works_only_once() {
    let svc = Svc::new();
    let token = svc.arabica_token().await;
    let path = format!("/v1/actions/{token}");
    let (first, _) = svc.post(&path, json!({ "quantity": 4 })).await;
    assert_eq!(first, StatusCode::OK);
    let (second, json) = svc.post(&path, json!({ "quantity": 4 })).await;
    assert_eq!(second, StatusCode::CONFLICT);
    assert_eq!(json["error"]["code"], "token_used");
}

#[tokio::test]
async fn a_retry_with_the_same_idempotency_key_gets_the_first_answer() {
    let svc = Svc::new();
    let token = svc.arabica_token().await;
    let path = format!("/v1/actions/{token}");
    let (a, first) = svc
        .post_with_key(&path, json!({ "quantity": 4 }), "same")
        .await;
    let (b, again) = svc
        .post_with_key(&path, json!({ "quantity": 4 }), "same")
        .await;
    assert_eq!((a, b), (StatusCode::OK, StatusCode::OK));
    assert_eq!(first, again);
    let (_, activity) = svc.get("/v1/activity").await;
    let pos = activity["audit"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["action"] == "create_po")
        .count();
    assert_eq!(pos, 1, "one PO, not two");
}

#[tokio::test]
async fn an_action_rejects_a_token_from_another_instance() {
    let other = Svc::new();
    let token = other.arabica_token().await;
    let (status, json) = Svc::new()
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": 1 }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"]["code"], "token_invalid");
}

#[tokio::test]
async fn a_multi_line_order_checks_each_line_against_its_cap() {
    let svc = Svc::new();
    let card = svc.card("reorder", "Susu Nusantara").await;
    let token = card["card"]["token"].as_str().unwrap();
    let lines = card["card"]["lines"].as_array().unwrap();
    let over: Vec<Value> = lines
        .iter()
        .map(|l| json!({ "productId": l["productId"], "quantity": l["quantity"].as_i64().unwrap() + 6 }))
        .collect();
    let (status, _) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "lines": over }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Drop one line to zero: a one-line PO.
    let edited = vec![
        json!({ "productId": lines[0]["productId"], "quantity": lines[0]["quantity"] }),
        json!({ "productId": lines[1]["productId"], "quantity": 0 }),
    ];
    let (status, json) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "lines": edited }))
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["detail"].as_str().unwrap().contains("Susu Nusantara"));
}

#[tokio::test]
async fn reject_spends_the_token_and_marks_the_card() {
    let svc = Svc::new();
    let token = svc.arabica_token().await;
    let (status, _) = svc
        .post(&format!("/v1/actions/{token}/reject"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let card = svc.card("reorder", "Arabica").await;
    assert_eq!(card["card"]["state"], "rejected");
    let (status, _) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": 3 }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn questions_are_answered_from_the_stored_inputs() {
    let svc = Svc::new();
    let ask = |text: &'static str| {
        let svc = &svc;
        async move {
            let (status, t) = svc
                .post(
                    "/v1/assistant/messages",
                    json!({ "topic": "reorder", "text": text }),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{t}");
            messages(&t).last().unwrap().clone()
        }
    };

    let why = ask("why so much?").await;
    assert_eq!(why["author"], "agent");
    assert!(
        why["text"].as_str().unwrap().contains("covers the next"),
        "{why}"
    );

    let what_if = ask("what if I order 5").await;
    let action = &what_if["actions"][0];
    assert_eq!(action["quantity"], 5);
    let token = action["token"].as_str().unwrap();
    let (status, json) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": 5 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");

    // Inline buttons are retired once the suggestion is handled.
    let t = svc.thread("reorder").await;
    assert!(messages(&t).iter().all(|m| m["actions"].is_null()));

    let off_topic = ask("write me a poem").await;
    assert!(
        off_topic["text"]
            .as_str()
            .unwrap()
            .contains("I can help with")
    );
}

#[tokio::test]
async fn a_closure_is_saved_and_the_order_recomputed() {
    let svc = Svc::new();
    let (_, t) = svc
        .post(
            "/v1/assistant/messages",
            json!({ "topic": "reorder", "text": "why 12? we're closed two days next week" }),
        )
        .await;
    let reply = messages(&t).last().unwrap();
    assert!(
        reply["text"].as_str().unwrap().contains("Closed")
            || reply["text"].as_str().unwrap().contains("Noted")
    );
    let (_, activity) = svc.get("/v1/activity").await;
    assert!(
        activity["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["topic"] == "passtok.calendar.override")
    );
}

#[tokio::test]
async fn stock_questions_work_from_any_thread() {
    let svc = Svc::new();
    let (_, t) = svc
        .post(
            "/v1/assistant/messages",
            json!({ "topic": "stock", "text": "how much oat milk is left?" }),
        )
        .await;
    let text = messages(&t).last().unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.starts_with("Oat Milk 1 L: 2 L usable"), "{text}");
    assert!(text.contains("past its date"), "{text}");
}

#[tokio::test]
async fn a_markdown_goes_live_on_the_product() {
    let svc = Svc::new();
    let card = svc.card("expiry", "Croissant").await;
    let n = card["card"]["quantity"].as_i64().unwrap();
    let token = card["card"]["token"].as_str().unwrap();
    let (status, json) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": n }))
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let (_, products) = svc.get("/v1/products").await;
    let p2 = products["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "p2")
        .unwrap();
    assert!(p2["status"].as_str().unwrap().contains("% off"), "{p2}");
}

#[tokio::test]
async fn todays_special_creates_a_listing_and_counts_as_repurposed() {
    let svc = Svc::new();
    let card = svc.card("special", "portion").await;
    let n = card["card"]["quantity"].as_i64().unwrap();
    let token = card["card"]["token"].as_str().unwrap();
    let (status, json) = svc
        .post(&format!("/v1/actions/{token}"), json!({ "quantity": n }))
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["title"].as_str().unwrap().starts_with("WO-"));
    let href = json["links"][0]["href"].as_str().unwrap();
    let (status, detail) = svc.get(&format!("/v1{href}")).await;
    assert_eq!(status, StatusCode::OK, "{href}");
    assert_eq!(detail["product"]["category"], "Specials");
    let (_, report) = svc.get("/v1/reports/waste").await;
    assert!(
        report["byDestination"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == "repurposed")
    );
}

/// A minimal JPEG with an APP1 segment carrying fake GPS.
fn jpeg_with_gps() -> Vec<u8> {
    vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46, 0xFF, 0xE1, 0x00, 0x06, b'G', b'P', b'S',
        b'!', 0xFF, 0xDA, 0x00, 0x02, 0x11, 0x22, 0xFF, 0xD9,
    ]
}

async fn upload(svc: &Svc) -> Value {
    let (status, json) = svc
        .send(
            Request::post("/v1/waste/photos")
                .header("content-type", "image/jpeg")
                .body(Body::from(jpeg_with_gps()))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    json
}

#[tokio::test]
async fn a_waste_photo_becomes_a_grounded_card_and_loses_its_gps() {
    let svc = Svc::new();
    let up = upload(&svc).await;
    let photo = up["photoId"].as_str().unwrap();

    let (status, bytes) = svc
        .raw(
            Request::get(format!("/v1/photos/{photo}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!bytes.windows(4).any(|w| w == b"GPS!"), "GPS survived");

    let t = svc.thread("waste").await;
    let ms = messages(&t);
    let photo_msg = ms.iter().find(|m| m["type"] == "photo").unwrap();
    assert_eq!(photo_msg["photoUrl"], format!("/photos/{photo}"));
    let card = &ms.iter().find(|m| m["type"] == "waste_card").unwrap()["card"];
    let items = card["items"].as_array().unwrap();
    assert!(!items.is_empty());
    for item in items {
        let ids: Vec<&str> = item["destinations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect();
        let selected: Vec<&Value> = item["destinations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["selected"] == true)
            .collect();
        assert_eq!(selected.len(), 1);
        assert_ne!(
            selected[0]["id"], "landfill",
            "landfill is never preselected"
        );
        if item["severity"] == "critical" {
            assert!(!ids.contains(&"donated") && !ids.contains(&"repurposed"));
            assert!(item["warning"].is_string());
        }
    }
    assert!(card["uncertain"]["candidates"].as_array().unwrap().len() <= 3);
}

#[tokio::test]
async fn non_images_are_refused() {
    let (status, _) = Svc::new()
        .send(
            Request::post("/v1/waste/photos")
                .header("content-type", "image/jpeg")
                .body(Body::from("not an image"))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn waste_to_donation_to_receipt() {
    let svc = Svc::new();
    upload(&svc).await;
    let t = svc.thread("waste").await;
    let card = messages(&t)
        .iter()
        .find(|m| m["type"] == "waste_card")
        .unwrap()["card"]
        .clone();
    let token = card["token"].as_str().unwrap();
    let items = card["items"].as_array().unwrap();
    let good = items.iter().find(|i| i["severity"] != "critical").unwrap();
    let past = items.iter().find(|i| i["severity"] == "critical").unwrap();

    // Donating past-date food is refused, and the token survives.
    let bad = json!({ "items": [
        { "itemId": past["id"], "quantity": past["quantity"], "destination": "donated" }
    ]});
    let (status, json) = svc.post(&format!("/v1/actions/{token}"), bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{json}");

    let uncertain = &card["uncertain"];
    let ok = json!({ "items": [
        { "itemId": good["id"], "quantity": good["quantity"], "destination": "donated" },
        { "itemId": past["id"], "quantity": past["quantity"], "destination": "bsf_or_compost" },
        { "itemId": uncertain["itemId"], "quantity": 1, "destination": "staff_meal", "productId": uncertain["candidates"][0]["id"] },
    ]});
    let (status, json) = svc.post(&format!("/v1/actions/{token}"), ok).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["links"][0]["href"], "/assistant/donation");

    let d = svc.thread("donation").await;
    let dcard = messages(&d)
        .iter()
        .find(|m| m["type"] == "donation_card")
        .unwrap()["card"]
        .clone();
    let dtoken = dcard["token"].as_str().unwrap();
    let recipient = dcard["recipients"][0]["id"].clone();
    let window = dcard["pickupWindows"][0].clone();

    // Checklist is a hard gate.
    let partial = json!({ "checklist": { "before_expiry": true, "storage_ok": true }, "recipientId": recipient, "pickupWindow": window });
    let (status, json) = svc.post(&format!("/v1/actions/{dtoken}"), partial).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("checklist")
    );

    let all: serde_json::Map<String, Value> = dcard["checklist"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap().to_owned(), json!(true)))
        .collect();
    let full = json!({ "checklist": all, "recipientId": recipient, "pickupWindow": window });
    let (status, json) = svc.post(&format!("/v1/actions/{dtoken}"), full).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let receipt = json["links"][0]["href"].as_str().unwrap();

    let (status, pdf) = svc
        .raw(
            Request::get(format!("/v1{receipt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(pdf.starts_with(b"%PDF"));

    let (_, report) = svc.get("/v1/reports/waste").await;
    assert!(report["diversionPercent"].is_number());
    assert!(
        report["recent"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["value"].as_str().unwrap().starts_with("Donate"))
    );
}

#[tokio::test]
async fn stock_tiles_count_the_same_rows_the_filters_show() {
    let (status, json) = Svc::new().get("/v1/stock").await;
    assert_eq!(status, StatusCode::OK);
    let rows = json["rows"].as_array().unwrap();
    let count = |tag: &str| {
        rows.iter()
            .filter(|r| r["tags"].as_array().unwrap().contains(&json!(tag)))
            .count() as u64
    };
    assert_eq!(json["summary"]["lowCount"], count("low"));
    assert_eq!(json["summary"]["expiringCount"], count("expiring"));
    assert!(count("low") > 0);
}

#[tokio::test]
async fn products_total_matches_the_list_and_detail_has_a_forecast() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/products").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["total"].as_u64().unwrap() as usize,
        json["products"].as_array().unwrap().len()
    );
    for p in json["products"].as_array().unwrap() {
        if p["price"].is_null() {
            assert!(p["subtitle"].is_string(), "{}", p["name"]);
        }
    }
    let (status, detail) = svc.get("/v1/products/p3").await;
    assert_eq!(status, StatusCode::OK);
    let series = detail["series"].as_array().unwrap();
    assert!(series.iter().any(|s| s["forecast"].is_number()));
    assert!(series.iter().any(|s| s["actual"].is_number()));
    assert!(detail["forecastNote"].as_str().unwrap().contains("TimesFM"));
    let (status, _) = svc.get("/v1/products/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn settings_persist() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/assistant/settings").await;
    assert_eq!(status, StatusCode::OK);
    let forecast = json["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "TimesFM")
        .unwrap();
    assert_eq!(forecast["system"], true);

    let (status, _) = svc
        .send(
            Request::put("/v1/assistant/settings")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "selectedMode": "thorough", "autonomy": { "markdown": true } })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, json) = svc.get("/v1/assistant/settings").await;
    assert_eq!(json["selectedMode"], "thorough");
    let md = json["autonomy"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "markdown")
        .unwrap();
    assert_eq!(md["enabled"], true);

    let (status, _) = svc
        .send(
            Request::put("/v1/assistant/settings")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "selectedMode": "reckless" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_answer_can_be_reported() {
    let svc = Svc::new();
    let t = svc.thread("reorder").await;
    let id = messages(&t)[0]["id"].clone();
    let (status, _) = svc
        .post(
            "/v1/assistant/report",
            json!({ "topic": "reorder", "messageId": id, "reason": "wrong number" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = svc
        .post(
            "/v1/assistant/report",
            json!({ "topic": "reorder", "messageId": "nope" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dashboard_has_impact_and_forecast_series() {
    let (status, json) = Svc::new().get("/v1/dashboard").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["impact"].as_array().unwrap().len(), 4);
    assert!(json["openSuggestions"].as_u64().unwrap() > 0);
    assert!(!json["atRisk"].as_array().unwrap().is_empty());
    assert!(!json["series"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_demo_kit_resets_the_morning() {
    let svc = Svc::new();
    let token = svc.arabica_token().await;
    svc.post(&format!("/v1/actions/{token}"), json!({ "quantity": 3 }))
        .await;
    let (status, _) = svc.post("/v1/admin/reset", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        svc.card("reorder", "Arabica").await["card"]["state"],
        "open"
    );
}

#[tokio::test]
async fn the_builder_is_still_501() {
    let (status, _) = Svc::new().get("/v1/builder/pages").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}
