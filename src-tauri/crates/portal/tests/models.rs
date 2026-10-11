//! Verify that administrator edits change discovery and enforcement together.
use antigravity_portal::{db, http, initialize, instance, proxy};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

static UPSTREAM_CALLS: AtomicUsize = AtomicUsize::new(0);

async fn upstream() -> Json<Value> {
    let mut body = json!({"request":{"contents":[{"parts":[{"text":"fixture"}]}]}});
    proxy::constrain_upstream("generateContent", &mut body).unwrap();
    UPSTREAM_CALLS.fetch_add(1, Ordering::SeqCst);
    Json(json!({"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}))
}

async fn call(app: &Router, path: &str, method: &str, key: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .method(method)
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 100_000).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_else(|_| json!({"text": String::from_utf8_lossy(&body)})))
}

#[tokio::test]
async fn saved_models_apply_immediately_to_config_lists_and_requests_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    // Each integration test binary has an independent singleton and environment.
    std::env::set_var("ABV_PORTAL_PUBLIC_URL", "http://127.0.0.1:18048");
    std::env::set_var("ABV_PORTAL_ALLOWED_MODELS", "fixture-original");
    for name in ["ABV_EPAY_URL", "ABV_EPAY_PID", "ABV_EPAY_KEY"] {
        std::env::remove_var(name);
    }
    initialize(dir.path()).unwrap();
    let portal = instance().unwrap();
    let user = portal.store.register("models@example.com", "test-password-long", db::now()).unwrap();
    let key = portal.store.rotate_key(&user.id, db::now()).unwrap();
    let api = Router::new()
        .route("/v1/models", get(|| async { Json(json!({})) }))
        .route("/v1beta/models", get(|| async { Json(json!({})) }))
        .route("/v1/chat/completions", post(upstream))
        .layer(middleware::from_fn(proxy::handle));
    // Production mounts these handlers inside the existing strict admin auth layer.
    let app = api
        .merge(http::router_with(portal.clone()))
        .nest("/api", http::admin_router());
    let (status, settings) = call(&app, "/api/portal/settings", "GET", "", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["allowed_models"], json!(["fixture-original"]));

    let (status, settings) = call(&app, "/api/portal/settings", "PUT", "",
        json!({"allowed_models":[" fixture-new ","fixture-new","fixture-second"]})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["allowed_models"], json!(["fixture-new","fixture-second"]));
    for (path, field) in [("/api/portal/settings", "allowed_models"), ("/portal/api/config", "models")] {
        let (_, value) = call(&app, path, "GET", "", Value::Null).await;
        assert_eq!(value[field], settings["allowed_models"]);
    }
    let (_, models) = call(&app, "/v1/models", "GET", &key, Value::Null).await;
    assert_eq!(models["data"][0]["id"], "fixture-new");
    assert_eq!(models["data"].as_array().unwrap().len(), 2);
    let (_, models) = call(&app, "/v1beta/models", "GET", &key, Value::Null).await;
    assert_eq!(models["models"][0]["name"], "models/fixture-new");

    for (model, expected) in [("fixture-original", StatusCode::BAD_REQUEST), ("fixture-new", StatusCode::OK)] {
        let (status, _) = call(&app, "/v1/chat/completions", "POST", &key,
            json!({"model":model,"max_tokens":32,"messages":[{"role":"user","content":"hello"}]})).await;
        assert_eq!(status, expected);
    }
    assert_eq!(UPSTREAM_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(portal.store.dashboard(&user.id, db::now()).unwrap()["reserved"], 0);

    for invalid in [json!({"allowed_models":["*"]}), json!({"allowed_models":["a b"]}),
        json!({"allowed_models":["a".repeat(201)]}), json!({"allowed_models":vec!["a";201]}),
        json!({"allowed_models":["fixture-new"],"unknown":true})] {
        let (status, _) = call(&app, "/api/portal/settings", "PUT", "", invalid).await;
        assert!(status.is_client_error());
        assert_eq!(portal.store.allowed_models().unwrap(), vec!["fixture-new","fixture-second"]);
    }
    let (status, _) = call(&app, "/api/portal/settings", "PUT", "", json!({"allowed_models":[]})).await;
    assert_eq!(status, StatusCode::OK);
    let (_, models) = call(&app, "/v1/models", "GET", &key, Value::Null).await;
    assert_eq!(models["data"], json!([]));
    let (status, _) = call(&app, "/v1/chat/completions", "POST", &key,
        json!({"model":"fixture-new","messages":[{"role":"user","content":"hello"}]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(UPSTREAM_CALLS.load(Ordering::SeqCst), 1);
    let reopened = db::Store::open(dir.path().join("portal.db")).unwrap();
    assert!(reopened.allowed_models().unwrap().is_empty(), "empty saved list must not revert to the environment default");
}

#[test]
fn version_one_database_migrates_without_changing_users() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("portal.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(include_str!("../src/schema.sql")).unwrap();
    conn.execute("INSERT INTO users(id,email,password_hash,created_at) VALUES('existing','existing@example.com','fixture',123)", []).unwrap();
    drop(conn);
    let store = db::Store::open(path.clone()).unwrap();
    store.set_allowed_models(vec!["fixture-new".into()]).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    assert_eq!(conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(conn.query_row("SELECT created_at FROM users WHERE id='existing'", [], |r| r.get::<_, i64>(0)).unwrap(), 123);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM audit WHERE action='set_allowed_models'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}
