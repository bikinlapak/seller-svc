use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use seller_svc::{AppState, Config, build_router};
use tower::ServiceExt;

fn app() -> axum::Router {
    let config = Config::from_lookup(|_| None).unwrap();
    build_router(AppState::new(config).unwrap())
}

#[tokio::test]
async fn healthz_returns_ok() {
    let res = app()
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn unimplemented_routes_return_501() {
    for path in ["/v1/products", "/v1/builder/pages"] {
        let res = app()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED, "{path}");
    }
}
