mod common;
use antigravity_portal::{http::router_with, mail::MailConfig, Portal};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use base64::Engine as _;
pub use common::modules;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::mpsc,
};
use tower::ServiceExt;

async fn smtp_fixture() -> (u16, mpsc::Receiver<String>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                write
                    .write_all(b"220 local-fixture ESMTP\r\n")
                    .await
                    .unwrap();
                let mut data = false;
                let mut message = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if data {
                        if line == "." {
                            data = false;
                            let _ = tx.send(message.clone()).await;
                            write.write_all(b"250 queued\r\n").await.unwrap();
                        } else {
                            message.push_str(&line);
                            message.push('\n');
                        }
                        continue;
                    }
                    let reply = if line.starts_with("EHLO") {
                        "250-local-fixture\r\n250 8BITMIME\r\n"
                    } else if line.starts_with("DATA") {
                        data = true;
                        "354 End with dot\r\n"
                    } else if line.starts_with("QUIT") {
                        write.write_all(b"221 Bye\r\n").await.unwrap();
                        break;
                    } else {
                        "250 OK\r\n"
                    };
                    write.write_all(reply.as_bytes()).await.unwrap();
                }
            });
        }
    });
    (port, rx, task)
}
async fn call(
    app: &Router,
    path: &str,
    cookie: &str,
    csrf: &str,
    body: Value,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
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
    let body = to_bytes(response.into_body(), 100_000).await.unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}
async fn receive(rx: &mut mpsc::Receiver<String>) -> String {
    let message = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let message = if message.contains("Content-Transfer-Encoding: base64") {
        let body = message.split_once("\n\n").unwrap().1.replace("\n", "");
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(body)
                .unwrap(),
        )
        .unwrap()
    } else {
        message
    };
    let start = message
        .find("mail_")
        .unwrap_or_else(|| panic!("No plaintext fixture token in message: {message}"));
    message[start..].chars().take(69).collect()
}
#[tokio::test]
async fn local_smtp_registration_verification_and_password_reset_round_trip() {
    let (_dir, base) = common::portal();
    let (port, mut mail, server) = smtp_fixture().await;
    let mut config = base.config.clone();
    config.require_email_verification = true;
    config.mail = Some(MailConfig {
        host: "127.0.0.1".into(),
        port,
        from: "fixture@example.invalid".into(),
        username: String::new(),
        password: String::new(),
        security: "local".into(),
    });
    let p = Arc::new(Portal::new(base.store.clone(), config, base.tokens.clone()));
    let app = router_with(p.clone());
    let (status, headers, result) = call(
        &app,
        "/portal/api/auth/register",
        "",
        "",
        json!({"email":"smtp@example.invalid","password":"old-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(result["api_key"].is_null());
    let user = result["user"]["id"].as_str().unwrap();
    assert!(p.store.native_token_id(user).unwrap().is_none());
    let cookie = headers["set-cookie"].to_str().unwrap();
    let csrf = result["csrf"].as_str().unwrap();
    let verify = receive(&mut mail).await;
    assert_eq!(
        call(
            &app,
            "/portal/api/auth/verify",
            cookie,
            "wrong",
            json!({"token":verify})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "/portal/api/auth/verify",
            cookie,
            csrf,
            json!({"token":verify})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(p.store.native_token_id(user).unwrap().is_some());
    assert_eq!(
        call(
            &app,
            "/portal/api/auth/verify",
            cookie,
            csrf,
            json!({"token":verify})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let known = call(
        &app,
        "/portal/api/auth/forgot",
        "",
        "",
        json!({"email":"smtp@example.invalid"}),
    )
    .await;
    let unknown = call(
        &app,
        "/portal/api/auth/forgot",
        "",
        "",
        json!({"email":"missing@example.invalid"}),
    )
    .await;
    assert_eq!(known.0, unknown.0);
    assert_eq!(known.2, unknown.2);
    let reset = receive(&mut mail).await;
    assert_eq!(
        call(
            &app,
            "/portal/api/auth/reset",
            "",
            "",
            json!({"token":reset,"password":"new-password"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(p
        .store
        .session(
            cookie.split(';').next().unwrap().split_once('=').unwrap().1,
            antigravity_portal::db::now()
        )
        .is_err());
    assert_eq!(
        call(
            &app,
            "/portal/api/auth/login",
            "",
            "",
            json!({"email":"smtp@example.invalid","password":"old-password"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "/portal/api/auth/login",
            "",
            "",
            json!({"email":"smtp@example.invalid","password":"new-password"})
        )
        .await
        .0,
        StatusCode::OK
    );
    server.abort();
}

#[tokio::test]
async fn missing_smtp_is_explicit_and_refund_requests_require_csrf() {
    let (_dir, p) = common::portal();
    let app = router_with(p.clone());
    assert_eq!(
        call(
            &app,
            "/portal/api/auth/forgot",
            "",
            "",
            json!({"email":"a@example.invalid"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let user = p
        .store
        .register(
            "refund-http@example.invalid",
            "password",
            antigravity_portal::db::now(),
        )
        .unwrap();
    let (session, csrf) = p
        .store
        .create_session(&user.id, antigravity_portal::db::now())
        .unwrap();
    let cookie = format!("abv_portal_session={session}");
    p.store.credit_wallet(&user.id, 500, "http-credit").unwrap();
    let input = json!({"credit_reference":"http-credit","amount_cents":100,"reason":"unused"});
    assert_eq!(
        call(&app, "/portal/api/refunds", &cookie, "", input.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "/portal/api/refunds", &cookie, &csrf, input)
            .await
            .0,
        StatusCode::OK
    );
    let admin = antigravity_portal::account_http::admin_router_with(p.clone());
    let id = p.store.wallet_history(&user.id).unwrap()["refunds"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        call(
            &admin,
            &format!("/portal/refunds/{id}"),
            "",
            "",
            json!({"action":"approve"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &admin,
            &format!("/portal/refunds/{id}"),
            "",
            "",
            json!({"action":"pay","payment_reference":"fixture-offline"})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
}
