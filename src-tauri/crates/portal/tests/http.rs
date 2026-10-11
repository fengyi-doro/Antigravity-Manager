mod common;
use antigravity_portal::http::router_with;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
pub use common::modules;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(
    app: &Router,
    path: &str,
    method: &str,
    cookie: &str,
    csrf: &str,
    body: Value,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Origin", "https://portal.example.com")
                .header("Cookie", cookie)
                .header("x-portal-csrf", csrf)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let data = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&data).unwrap_or(Value::Null),
    )
}
#[tokio::test]
async fn sessions_csrf_native_keys_and_statistics_remain_isolated() {
    let (_dir, portal) = common::portal();
    let app = router_with(portal);
    let (status, headers, user) = call(
        &app,
        "/portal/api/auth/register",
        "POST",
        "",
        "",
        json!({"email":"alice@example.com","password":"test-password-long"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cookie = headers["set-cookie"].to_str().unwrap().to_owned();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("Secure")
            && cookie.contains("SameSite=Strict")
    );
    let cookie = cookie.split(';').next().unwrap();
    let csrf = user["csrf"].as_str().unwrap();
    let key = user["api_key"].as_str().unwrap();
    let token = common::original_tokens::get_token_by_value(key)
        .unwrap()
        .unwrap();
    common::original_tokens::record_token_usage_and_ip(
        &token.id,
        "127.0.0.1",
        "fixture",
        120,
        30,
        200,
        None,
    )
    .unwrap();
    let (status, headers, snapshot) =
        call(&app, "/portal/api/me", "GET", cookie, "", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(snapshot["data"]["key"]["total_tokens_used"], 150);
    assert!(snapshot["data"]["key"].get("token").is_none());
    assert_eq!(snapshot["data"]["legacy"]["available"], 0);
    assert_eq!(
        call(&app, "/portal/api/key", "GET", "", "", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "/portal/api/key", "DELETE", cookie, "", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (_, _, shown) = call(&app, "/portal/api/key", "GET", cookie, "", Value::Null).await;
    assert_eq!(shown["api_key"], key);
    let (_, other_headers, _) = call(
        &app,
        "/portal/api/auth/register",
        "POST",
        "",
        "",
        json!({"email":"bob@example.com","password":"test-password-long"}),
    )
    .await;
    let other_cookie = other_headers["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let (_, _, other) = call(&app, "/portal/api/me", "GET", other_cookie, "", Value::Null).await;
    assert_ne!(other["data"]["key"]["id"], token.id);
    assert_eq!(other["data"]["key"]["total_tokens_used"], 0);
    let (_, _, replacement) = call(&app, "/portal/api/key", "POST", cookie, csrf, json!({})).await;
    assert_ne!(replacement["api_key"], key);
    assert!(
        !common::original_tokens::validate_token(key, "127.0.0.1")
            .unwrap()
            .0
    );
    assert_eq!(
        call(&app, "/portal/api/key", "DELETE", cookie, csrf, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        !common::original_tokens::validate_token(
            replacement["api_key"].as_str().unwrap(),
            "127.0.0.1"
        )
        .unwrap()
        .0
    );
}
#[tokio::test]
async fn billing_prices_are_public_but_online_payment_is_disabled() {
    let (_dir, portal) = common::portal();
    let app = router_with(portal);
    let (_, _, config) = call(&app, "/portal/api/config", "GET", "", "", Value::Null).await;
    assert_eq!(config["payment_enabled"], false);
    assert_eq!(config["billing_enabled"], true);
    assert_eq!(config["pricing"]["cached_per_million"], 0.06);
    for path in [
        "/portal/api/orders",
        "/portal/api/orders/old/reconcile",
        "/portal/api/payments/notify",
    ] {
        assert_eq!(
            call(&app, path, "POST", "", "", json!({})).await.0,
            StatusCode::GONE
        );
    }
}
