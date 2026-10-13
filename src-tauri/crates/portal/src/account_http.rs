use crate::{
    blocking, db,
    http::{authenticated, client_bucket, origin},
    Error, Portal, Result,
};
use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc};

pub fn public_routes() -> Router<Arc<Portal>> {
    Router::new()
        .route("/portal/api/auth/forgot", post(forgot))
        .route("/portal/api/auth/reset", post(reset))
        .route("/portal/api/auth/verification", post(resend))
        .route("/portal/api/auth/verify", post(verify))
        .route("/portal/api/refunds", post(refund_request))
}
pub fn admin_router_with<S: Clone + Send + Sync + 'static>(p: Arc<Portal>) -> Router<S> {
    Router::new()
        .route("/portal/users/:id/history", get(history))
        .route("/portal/refunds/:id", post(refund_action))
        .with_state(p)
}
/// The response never discloses whether a reset address exists or SMTP delivery succeeds.
pub async fn queue_mail(p: Arc<Portal>, email: String, purpose: &'static str) -> Result<()> {
    let mail = p
        .config
        .mail
        .clone()
        .ok_or_else(|| Error::Unavailable("邮箱服务尚未配置".into()))?;
    let store = p.store.clone();
    let challenge = blocking(move || store.issue_challenge(&email, purpose, db::now())).await?;
    if let Some((email, token)) = challenge {
        tokio::spawn(async move {
            if mail.send(&email, purpose, &token).await.is_err() {
                let store = p.store.clone();
                let _ = blocking(move || store.invalidate_challenge(&token)).await;
                tracing::warn!(
                    purpose,
                    "Account email delivery failed; challenge invalidated"
                );
            }
        });
    }
    Ok(())
}
#[derive(Deserialize)]
struct EmailInput {
    email: String,
}
async fn forgot(
    State(p): State<Arc<Portal>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<EmailInput>,
) -> Result<Json<Value>> {
    origin(&p, &headers)?;
    if p.config.mail.is_none() {
        return Err(Error::Unavailable("邮箱服务尚未配置，请联系管理员".into()));
    }
    if body.email.len() > 254 {
        return Err(Error::Invalid("邮箱地址过长".into()));
    }
    let bucket = format!("forgot:{}", client_bucket(&p, &headers, peer));
    let store = p.store.clone();
    let email = body.email.clone();
    blocking(move || {
        store.rate_limit(&bucket, 10, 3600, db::now())?;
        store.rate_limit(
            &format!("mail:{}", db::digest(&email.trim().to_lowercase())),
            3,
            900,
            db::now(),
        )
    })
    .await?;
    queue_mail(p, body.email, "reset").await?;
    Ok(Json(
        json!({"message":"如果该邮箱对应可用账号，将收到重置邮件。未收到时可稍后重试或联系管理员。"}),
    ))
}
#[derive(Deserialize)]
struct ResetInput {
    token: String,
    password: String,
}
async fn reset(
    State(p): State<Arc<Portal>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<ResetInput>,
) -> Result<StatusCode> {
    origin(&p, &headers)?;
    let bucket = format!("reset:{}", client_bucket(&p, &headers, peer));
    blocking(move || {
        p.store.rate_limit(&bucket, 20, 900, db::now())?;
        p.store
            .consume_challenge(&body.token, "reset", None, Some(&body.password), db::now())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn resend(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<Json<Value>> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    let store = p.store.clone();
    let id = user.id.clone();
    blocking(move || store.rate_limit(&format!("verify:{id}"), 3, 900, db::now())).await?;
    queue_mail(p, user.email, "verify").await?;
    Ok(Json(
        json!({"message":"已请求发送验证邮件，验证码 20 分钟有效；未收到时请检查垃圾邮件或联系管理员。"}),
    ))
}
#[derive(Deserialize)]
struct TokenInput {
    token: String,
}
async fn verify(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(body): Json<TokenInput>,
) -> Result<StatusCode> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    blocking(move || {
        p.store
            .rate_limit(&format!("verify-submit:{}", user.id), 20, 900, db::now())?;
        p.store
            .consume_challenge(&body.token, "verify", Some(&user.id), None, db::now())?;
        p.ensure_native_token(&user)?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct RefundInput {
    credit_reference: String,
    amount_cents: i64,
    reason: String,
}
async fn refund_request(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(body): Json<RefundInput>,
) -> Result<Json<Value>> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    let id = blocking(move || {
        p.store
            .rate_limit(&format!("refund:{}", user.id), 10, 3600, db::now())?;
        p.store.request_refund(
            &user.id,
            &body.credit_reference,
            body.amount_cents,
            &body.reason,
            db::now(),
        )
    })
    .await?;
    Ok(Json(json!({"id":id})))
}
async fn history(State(p): State<Arc<Portal>>, Path(id): Path<String>) -> Result<Json<Value>> {
    Ok(Json(blocking(move || p.store.wallet_history(&id)).await?))
}
#[derive(Deserialize)]
struct RefundAction {
    action: String,
    payment_reference: Option<String>,
}
async fn refund_action(
    State(p): State<Arc<Portal>>,
    Path(id): Path<String>,
    Json(body): Json<RefundAction>,
) -> Result<StatusCode> {
    blocking(move || {
        p.store.transition_refund(
            &id,
            &body.action,
            body.payment_reference.as_deref(),
            db::now(),
        )
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
