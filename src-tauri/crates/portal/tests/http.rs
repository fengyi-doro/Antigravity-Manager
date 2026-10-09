use antigravity_portal::{
    db::Store,
    http::{router_with, Config},
    payment::Payment,
    Portal,
};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn app(pay: bool) -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().unwrap();
    let portal = Arc::new(Portal {
        store: Store::open(dir.path().join("portal.db")).unwrap(),
        config: Config {
            public_url: "https://portal.example.com".into(),
            api_base_url: "https://api.example.com/v1".into(),
            secure_cookie: true,
            registration_open: true,
            trust_proxy: false,
            payment: pay.then(|| {
                Payment::new(
                    "https://pay.example.com/",
                    "42".into(),
                    "mock-merchant-key-only".into(),
                    "https://portal.example.com".into(),
                )
                .unwrap()
            }),
        },
    });
    (dir, router_with(portal))
}
async fn signup(app: &Router, email: &str) -> (String, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/portal/api/auth/register")
                .header("Origin", "https://portal.example.com")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({"email":email,"password":"test-password-long","role":"admin"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("Secure")
            && cookie.contains("SameSite=Strict")
    );
    let json: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    (cookie.split(';').next().unwrap().to_owned(), json)
}
#[tokio::test]
async fn sessions_csrf_and_customer_data_are_isolated() {
    let (_dir, app) = app(false);
    let bad = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/portal/api/auth/register")
                .header("Origin", "https://evil.example")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    r#"{"email":"a@example.com","password":"test-password-long"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::FORBIDDEN);
    let (cookie, user) = signup(&app, "alice@example.com").await;
    assert!(user["user"].get("role").is_none());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/portal/api/me")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let snapshot: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(snapshot["data"]["gift_available"], 5_000_000);
    assert!(snapshot["data"]["key"].get("digest").is_none());
    let no_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/portal/api/key")
                .header("Origin", "https://portal.example.com")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
    let api_key = user["api_key"].as_str().unwrap();
    let cannot_use_key_as_session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/portal/api/me")
                .header("Authorization", format!("Bearer {api_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cannot_use_key_as_session.status(), StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn payment_configuration_and_order_ownership() {
    let (_dir, app) = app(true);
    let (cookie, user) = signup(&app, "alice@example.com").await;
    let (other_cookie, other) = signup(&app, "bob@example.com").await;
    let create=app.clone().oneshot(Request::builder().method("POST").uri("/portal/api/orders")
        .header("Origin","https://portal.example.com").header("cookie",&cookie).header("x-portal-csrf",user["csrf"].as_str().unwrap())
        .header("Content-Type","application/json").body(Body::from(json!({"amount_cents":100,"channel":"wxpay","idempotency_key":"test-idempotency-123","tokens":900000000}).to_string())).unwrap()).await.unwrap();
    assert_eq!(create.status(), StatusCode::OK);
    let order: Value =
        serde_json::from_slice(&to_bytes(create.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(order["order"]["tokens"], 1_000_000);
    assert_eq!(order["checkout"]["fields"]["money"], "1.00");
    let attempt = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/portal/api/orders/{}/reconcile",
                    order["order"]["id"].as_str().unwrap()
                ))
                .header("Origin", "https://portal.example.com")
                .header("cookie", other_cookie)
                .header("x-portal-csrf", other["csrf"].as_str().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(attempt.status(), StatusCode::FORBIDDEN);
}
