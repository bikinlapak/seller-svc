//! Router tests through `tower::ServiceExt::oneshot` — no port is opened.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use seller_svc::{AppState, Config, build_router};
use serde_json::Value;
use tower::ServiceExt;

fn state() -> AppState {
    AppState::new(Config::from_lookup(|_| None).unwrap()).unwrap()
}

/// One service instance per test.
///
/// It has to be shared rather than rebuilt per request: the signing keys and
/// the spent-token record live in `AppState`, so two instances mean two key
/// pairs and a token from one is rejected by the other.
struct Svc {
    app: axum::Router,
}

impl Svc {
    fn new() -> Self {
        Self {
            app: build_router(state()),
        }
    }

    async fn send(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            Request::post(path)
                .header("content-type", "application/json")
                .header("idempotency-key", "test")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    /// Pull a suggestion token from the thread, the way seller-app does.
    async fn suggestion_token(&self) -> String {
        let (_, json) = self.get("/v1/assistant/thread?topic=reorder").await;
        json["messages"][0]["card"]["token"]
            .as_str()
            .expect("the suggestion card must carry a token")
            .to_string()
    }
}

#[tokio::test]
async fn healthz_returns_ok() {
    let svc = Svc::new();
    let (status, json) = svc.get("/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn home_has_the_shape_seller_app_expects() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/home").await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(json["shop"]["name"], "Jasa Coffee Shop");
    assert!(json["shop"]["greeting"].is_string());

    let attention = json["attention"].as_array().unwrap();
    assert_eq!(attention.len(), 3);
    for a in attention {
        // Every attention row must point at a thread that actually exists.
        let topic = a["href"]
            .as_str()
            .unwrap()
            .trim_start_matches("/assistant/");
        let (s, _) = svc
            .get(&format!("/v1/assistant/thread?topic={topic}"))
            .await;
        assert_eq!(s, StatusCode::OK, "href {topic} has no thread");
    }

    for c in json["commands"].as_array().unwrap() {
        let topic = c["href"]
            .as_str()
            .unwrap()
            .trim_start_matches("/assistant/");
        let (s, _) = svc
            .get(&format!("/v1/assistant/thread?topic={topic}"))
            .await;
        assert_eq!(s, StatusCode::OK, "command {topic} has no thread");
    }
}

#[tokio::test]
async fn an_unknown_thread_is_404() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/assistant/thread?topic=nonsense").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn a_thread_without_a_topic_is_400() {
    let svc = Svc::new();
    let (status, _) = svc.get("/v1/assistant/thread").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_reorder_thread_carries_a_signed_suggestion() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/assistant/thread?topic=reorder").await;
    assert_eq!(status, StatusCode::OK);

    let messages = json["messages"].as_array().unwrap();
    let card = &messages[0];
    assert_eq!(card["type"], "suggestion_card");
    assert_eq!(card["author"], "agent");
    assert_eq!(card["card"]["quantity"], 12);
    assert_eq!(card["card"]["unit"], "kg");

    // A real JWT, not a placeholder.
    let token = card["card"]["token"].as_str().unwrap();
    assert_eq!(token.split('.').count(), 3, "not a JWT: {token}");
}

#[tokio::test]
async fn the_waste_thread_uses_camel_case() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/assistant/thread?topic=waste").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["source"], "Gemini vision");

    let messages = json["messages"].as_array().unwrap();
    // seller-app reads `photoLabel`, not `photo_label`.
    assert!(messages[0]["photoLabel"].is_string());
    assert_eq!(messages[0]["boxes"].as_array().unwrap().len(), 3);

    let card = &messages[1]["card"];
    let items = card["items"].as_array().unwrap();
    assert!(items[0]["severityLabel"].is_string());

    // Destination rules: anything past its date offers neither donation nor
    // sale, and Discard is never preselected.
    for item in items {
        let destinations = item["destinations"].as_array().unwrap();
        assert_eq!(
            destinations
                .iter()
                .filter(|d| d["selected"] == true)
                .count(),
            1,
            "exactly one destination must be preselected"
        );
        for d in destinations {
            if d["id"] == "discard" {
                assert_eq!(d["selected"], false, "Discard must never be preselected");
            }
        }
        if item["severity"] == "critical" {
            let ids: Vec<&str> = destinations
                .iter()
                .map(|d| d["id"].as_str().unwrap())
                .collect();
            assert!(
                !ids.contains(&"donation"),
                "past date must not offer donation"
            );
            assert!(!ids.contains(&"special"), "past date must not offer sale");
            assert!(item["warning"].is_string(), "a warning is required");
        }
    }
}

#[tokio::test]
async fn products_and_stock_have_the_right_shape() {
    let svc = Svc::new();

    let (status, json) = svc.get("/v1/products").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["total"].is_number());
    for p in json["products"].as_array().unwrap() {
        assert!(p["stockLabel"].is_string());
        assert!(["ok", "warn", "critical"].contains(&p["stockSeverity"].as_str().unwrap()));
        // A product with no price must explain itself in a subtitle.
        if p["priceLabel"].is_null() {
            assert!(p["subtitle"].is_string(), "{}", p["name"]);
        }
    }

    let (status, json) = svc.get("/v1/stock").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["summary"]["lowCount"].is_number());
    for r in json["rows"].as_array().unwrap() {
        let percent = r["percent"].as_i64().unwrap();
        assert!(
            (0..=100).contains(&percent),
            "percent out of range: {percent}"
        );
        assert!(r["quantityLabel"].is_string());
    }
}

#[tokio::test]
async fn ai_settings_marks_forecasting_as_a_system_number() {
    let svc = Svc::new();
    let (status, json) = svc.get("/v1/assistant/settings").await;
    assert_eq!(status, StatusCode::OK);

    let modes: Vec<&str> = json["modes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert!(modes.contains(&json["selectedMode"].as_str().unwrap()));

    let forecast = json["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["task"] == "Stock forecasting")
        .expect("a stock forecasting model must be listed");
    assert_eq!(forecast["system"], true);

    // English is the product's default language.
    assert_eq!(json["language"]["selected"], "English");
    assert!(
        json["language"]["options"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o == "English")
    );
}

/* ---------- The action path: the only way to write ---------- */

#[tokio::test]
async fn an_action_approves_a_reduced_quantity() {
    let svc = Svc::new();
    let token = svc.suggestion_token().await;
    let (status, json) = svc
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "quantity": 9 }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["title"], "PO-0771 created");
    assert!(json["detail"].as_str().unwrap().contains("9 kg"));
    // The supplier message carries the final quantity, not the proposed one.
    let wa = json["waUrl"].as_str().unwrap();
    assert!(
        wa.contains("9%20kg"),
        "waUrl lacks the final quantity: {wa}"
    );
    assert!(json["poHref"].is_string());
}

#[tokio::test]
async fn an_action_rejects_a_quantity_above_the_cap() {
    let svc = Svc::new();
    let token = svc.suggestion_token().await;
    let (status, json) = svc
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "quantity": 13 }),
        )
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json["error"]["code"], "qty_above_cap");
}

#[tokio::test]
async fn an_action_rejects_a_forged_token() {
    let svc = Svc::new();
    let (status, json) = svc
        .post(
            "/v1/actions/not.a.token",
            serde_json::json!({ "quantity": 9 }),
        )
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"]["code"], "token_invalid");
}

#[tokio::test]
async fn an_action_rejects_a_zero_quantity() {
    let svc = Svc::new();
    let token = svc.suggestion_token().await;
    let (status, json) = svc
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "quantity": 0 }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"]["code"], "bad_request");
}

/// Single use: a second tap on the same token must be refused.
#[tokio::test]
async fn an_action_token_works_only_once() {
    let svc = Svc::new();
    let token = svc.suggestion_token().await;
    let path = format!("/v1/actions/{token}");
    let body = serde_json::json!({ "quantity": 9 });

    let (first, _) = svc.post(&path, body.clone()).await;
    assert_eq!(first, StatusCode::OK);

    let (second, json) = svc.post(&path, body).await;
    assert_eq!(second, StatusCode::CONFLICT);
    assert_eq!(json["error"]["code"], "token_used");
}

/// A token signed by another instance is refused — the per-service key really
/// is used, not ignored.
#[tokio::test]
async fn an_action_rejects_a_token_from_another_instance() {
    let a = Svc::new();
    let b = Svc::new();
    let token = a.suggestion_token().await;

    let (status, json) = b
        .post(
            &format!("/v1/actions/{token}"),
            serde_json::json!({ "quantity": 9 }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"]["code"], "token_invalid");
}

#[tokio::test]
async fn the_builder_is_still_501() {
    let svc = Svc::new();
    let (status, _) = svc.get("/v1/builder/pages").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}
