//! Exercise the real quota middleware with a deterministic, local upstream fixture.
use antigravity_portal::{db, initialize, instance, proxy};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct Fixture {
    mode: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

async fn upstream(State(fixture): State<Fixture>) -> Response {
    let mut body = json!({"request":{"contents":[{"parts":[{"text":"fixture"}]}]}});
    proxy::constrain_upstream("generateContent", &mut body).unwrap();
    fixture.calls.fetch_add(1, Ordering::SeqCst);
    let mut response = match fixture.mode.load(Ordering::SeqCst) {
        0 => Json(json!({"usage":{"prompt_tokens":10,"completion_tokens":20}})).into_response(),
        1 => (
            [("content-type", "text/event-stream")],
            "data: {\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n",
        )
            .into_response(),
        2 => (
            [("content-type", "text/event-stream")],
            "data: {\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1}}\n\n",
        )
            .into_response(),
        3 => {
            proxy::note_upstream_response(429);
            (StatusCode::TOO_MANY_REQUESTS, "private provider details").into_response()
        }
        _ => (StatusCode::BAD_GATEWAY, "private provider details").into_response(),
    };
    response
        .headers_mut()
        .insert("x-account-email", "private@example.com".parse().unwrap());
    response
}

fn request(key: &str) -> Request<Body> {
    Request::builder().method("POST").uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"model":"fixture-text","max_tokens":32,"messages":[{"role":"user","content":"hello"}]}).to_string())).unwrap()
}

#[tokio::test]
async fn middleware_settles_complete_usage_holds_interrupted_usage_and_stops_at_zero() {
    let dir = tempfile::tempdir().unwrap();
    // This integration test has its own process and is the only test initializing the singleton.
    std::env::set_var("ABV_PORTAL_PUBLIC_URL", "http://127.0.0.1:18048");
    std::env::set_var("ABV_PORTAL_ALLOWED_MODELS", "fixture-text");
    for name in ["ABV_EPAY_URL", "ABV_EPAY_PID", "ABV_EPAY_KEY"] {
        std::env::remove_var(name);
    }
    initialize(dir.path()).unwrap();
    let store = &instance().unwrap().store;
    let user = store
        .register("meter@example.com", "test-password-long", db::now())
        .unwrap();
    let key = store.rotate_key(&user.id, db::now()).unwrap();
    let fixture = Fixture::default();
    let app = Router::new()
        .route("/v1/chat/completions", post(upstream))
        .layer(middleware::from_fn(proxy::handle))
        .with_state(fixture.clone());
    for (mode, charge) in [(0, 30), (1, 3), (3, 0)] {
        fixture.mode.store(mode, Ordering::SeqCst);
        let before = store.dashboard(&user.id, db::now()).unwrap()["available"]
            .as_i64()
            .unwrap();
        let response = app.clone().oneshot(request(&key)).await.unwrap();
        assert!(!response.headers().contains_key("x-account-email"));
        let body = to_bytes(response.into_body(), 100_000).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("private provider details"));
        let after = store.dashboard(&user.id, db::now()).unwrap();
        assert_eq!(after["available"], before - charge);
        assert_eq!(after["reserved"], 0);
    }
    for mode in [2, 4] {
        fixture.mode.store(mode, Ordering::SeqCst);
        let response = app.clone().oneshot(request(&key)).await.unwrap();
        to_bytes(response.into_body(), 100_000).await.unwrap();
    }
    // A client disconnect before even reading the successful body must also hold its budget.
    fixture.mode.store(0, Ordering::SeqCst);
    drop(app.clone().oneshot(request(&key)).await.unwrap());
    let mut reviewed = false;
    for _ in 0..100 {
        if store.admin_users().unwrap()["review"]
            .as_array()
            .unwrap()
            .len()
            == 3
        {
            reviewed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        reviewed,
        "interrupted and unread responses remain reviewable"
    );
    assert!(
        store.dashboard(&user.id, db::now()).unwrap()["reserved"]
            .as_i64()
            .unwrap()
            > 0
    );
    for item in store.admin_users().unwrap()["review"].as_array().unwrap() {
        store
            .manual_settle(
                item["id"].as_str().unwrap(),
                0,
                0,
                db::now(),
                "Fixture verified no spend",
            )
            .unwrap();
    }
    let available = store.dashboard(&user.id, db::now()).unwrap()["available"]
        .as_i64()
        .unwrap();
    let identity = store.key_identity(&key).unwrap();
    let last = store
        .reserve(&identity, "fixture-text", available, db::now())
        .unwrap();
    store.settle(&last, available, 0, db::now()).unwrap();
    let calls = fixture.calls.load(Ordering::SeqCst);
    let denied = app.clone().oneshot(request(&key)).await.unwrap();
    assert_eq!(denied.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        fixture.calls.load(Ordering::SeqCst),
        calls,
        "empty quota never reaches upstream"
    );
    store.revoke_keys(&user.id).unwrap();
    assert_eq!(
        app.oneshot(request(&key)).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}
