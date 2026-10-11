use crate::{
    blocking,
    db::{self, User},
    instance, Error, Portal, Result,
};
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct Config {
    pub public_url: String,
    pub api_base_url: String,
    pub secure_cookie: bool,
    pub registration_open: bool,
    pub trust_proxy: bool,
}
impl Config {
    pub fn from_env() -> Result<Self> {
        let public_url = std::env::var("ABV_PORTAL_PUBLIC_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8045".into())
            .trim_end_matches('/')
            .to_owned();
        let url =
            url::Url::parse(&public_url).map_err(|_| Error::Invalid("用户中心域名无效".into()))?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (url.scheme() != "https" && !(local && url.scheme() == "http"))
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Invalid(
                "用户中心应配置 HTTPS 域名，HTTP 仅允许本机测试".into(),
            ));
        }
        Ok(Self {
            api_base_url: std::env::var("ABV_PORTAL_API_BASE_URL")
                .unwrap_or_else(|_| format!("{public_url}/v1")),
            secure_cookie: url.scheme() == "https",
            public_url,
            registration_open: std::env::var("ABV_PORTAL_REGISTRATION_OPEN").as_deref()
                != Ok("false"),
            trust_proxy: std::env::var("ABV_PORTAL_TRUST_PROXY").as_deref() == Ok("true"),
        })
    }
}
pub fn router() -> Router {
    match instance() {
        Ok(portal) => router_with(portal),
        Err(_) => Router::new().route("/portal/api/*path", get(unavailable).post(unavailable)),
    }
}
async fn unavailable() -> Error {
    Error::Unavailable("用户服务尚未初始化".into())
}
pub fn router_with(portal: Arc<Portal>) -> Router {
    Router::new()
        .route("/portal/api/config", get(config))
        .route("/portal/api/auth/register", post(register))
        .route("/portal/api/auth/login", post(login))
        .route("/portal/api/auth/logout", post(logout))
        .route("/portal/api/me", get(me))
        .route(
            "/portal/api/key",
            get(show_key).post(rotate_key).delete(revoke_key),
        )
        .route("/portal/api/orders", post(billing_disabled))
        .route("/portal/api/orders/:id/reconcile", post(billing_disabled))
        .route(
            "/portal/api/payments/notify",
            get(billing_disabled).post(billing_disabled),
        )
        .layer(DefaultBodyLimit::max(16_384))
        .layer(middleware::from_fn(cache_headers))
        .with_state(portal)
}
async fn cache_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}
fn origin(p: &Portal, headers: &HeaderMap) -> Result<()> {
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        != Some(p.config.public_url.as_str())
    {
        return Err(Error::Forbidden);
    }
    Ok(())
}
fn cookie(headers: &HeaderMap) -> Result<String> {
    let raw = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Unauthorized)?;
    raw.split(';')
        .find_map(|v| v.trim().strip_prefix("abv_portal_session="))
        .filter(|v| v.len() < 256)
        .map(str::to_owned)
        .ok_or(Error::Unauthorized)
}
async fn authenticated(
    p: Arc<Portal>,
    headers: &HeaderMap,
    mutation: bool,
) -> Result<(User, String)> {
    if mutation {
        origin(&p, headers)?;
    }
    let token = cookie(headers)?;
    let (user, csrf) = blocking(move || p.store.session(&token, db::now())).await?;
    if mutation
        && !bool::from(
            csrf.as_bytes().ct_eq(
                headers
                    .get("x-portal-csrf")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .as_bytes(),
            ),
        )
    {
        return Err(Error::Forbidden);
    }
    Ok((user, csrf))
}
fn cookie_header(config: &Config, token: &str, age: i64) -> Result<HeaderValue> {
    HeaderValue::from_str(&format!(
        "abv_portal_session={token}; Path=/portal/api; HttpOnly; SameSite=Strict; Max-Age={age}{}",
        if config.secure_cookie { "; Secure" } else { "" }
    ))
    .map_err(|_| Error::Internal("Invalid session cookie".into()))
}
async fn config(State(p): State<Arc<Portal>>) -> Result<Json<Value>> {
    Ok(Json(json!({"registration_open":p.config.registration_open,
        "billing_enabled":true,"pricing":crate::billing::pricing(),"payment_enabled":false,"api_base_url":p.config.api_base_url})))
}

#[derive(Deserialize)]
struct Credentials {
    email: String,
    password: String,
}
fn client_bucket(p: &Portal, headers: &HeaderMap, peer: Option<ConnectInfo<SocketAddr>>) -> String {
    // Forwarded addresses are accepted only after the operator explicitly trusts the reverse proxy.
    let forwarded = if p.config.trust_proxy {
        headers
            .get("cf-connecting-ip")
            .or_else(|| headers.get("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<std::net::IpAddr>().ok())
    } else {
        None
    };
    forwarded
        .or_else(|| peer.map(|v| v.0.ip()))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "unknown".into())
}
async fn register(
    State(p): State<Arc<Portal>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Result<Response> {
    origin(&p, &headers)?;
    if !p.config.registration_open {
        return Err(Error::Unavailable("注册暂未开放".into()));
    }
    let bucket = format!("signup:{}", client_bucket(&p, &headers, peer));
    let portal = p.clone();
    let store = p.store.clone();
    let (user, token, csrf, key) = blocking(move || {
        store.rate_limit(&bucket, 3, 86_400, db::now())?;
        let user = store.register(&body.email, &body.password, db::now())?;
        let (token, csrf) = store.create_session(&user.id, db::now())?;
        let key = portal
            .ensure_native_token(&user)?
            .ok_or_else(|| Error::Internal("原版 Token 不存在".into()))?
            .token;
        Ok((user, token, csrf, key))
    })
    .await?;
    let mut response = (
        StatusCode::CREATED,
        Json(json!({"user":user,"csrf":csrf,"api_key":key})),
    )
        .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&p.config, &token, 7 * 86_400)?,
    );
    Ok(response)
}
async fn login(
    State(p): State<Arc<Portal>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Result<Response> {
    origin(&p, &headers)?;
    let bucket = format!("login:{}", client_bucket(&p, &headers, peer));
    let portal = p.clone();
    let store = p.store.clone();
    let (user, token, csrf) = blocking(move || {
        store.rate_limit(&bucket, 30, 900, db::now())?;
        store.rate_limit(
            &format!(
                "login-account:{}",
                db::digest(&body.email.trim().to_lowercase())
            ),
            20,
            900,
            db::now(),
        )?;
        let user = store.login(&body.email, &body.password)?;
        portal.ensure_native_token(&user)?;
        let (token, csrf) = store.create_session(&user.id, db::now())?;
        Ok((user, token, csrf))
    })
    .await?;
    let mut response = Json(json!({"user":user,"csrf":csrf})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie_header(&p.config, &token, 7 * 86_400)?,
    );
    Ok(response)
}
async fn logout(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<Response> {
    authenticated(p.clone(), &headers, true).await?;
    let token = cookie(&headers)?;
    let store = p.store.clone();
    blocking(move || store.logout(&token)).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie_header(&p.config, "", 0)?);
    Ok(response)
}
async fn me(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<Json<Value>> {
    let (user, csrf) = authenticated(p.clone(), &headers, false).await?;
    let id = user.id.clone();
    let data = blocking(move || p.native_dashboard(&id)).await?;
    Ok(Json(json!({"user":user,"csrf":csrf,"data":data})))
}
async fn rotate_key(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<Json<Value>> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    let key = blocking(move || p.rotate_native_token(&user)).await?;
    Ok(Json(json!({"api_key":key})))
}
async fn revoke_key(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<StatusCode> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    blocking(move || p.set_native_enabled(&user.id, false)).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn show_key(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<Json<Value>> {
    let (user, _) = authenticated(p.clone(), &headers, false).await?;
    let key = blocking(move || {
        p.ensure_native_token(&user)?
            .map(|token| token.token)
            .ok_or(Error::Forbidden)
    })
    .await?;
    Ok(Json(json!({"api_key":key})))
}
async fn billing_disabled() -> Response {
    (
        StatusCode::GONE,
        Json(json!({"error":{"code":"billing_disabled",
        "message":"在线支付尚未开启，请联系管理员充值"}})),
    )
        .into_response()
}

/// Merge this into the existing administrator router BEFORE its strict auth layer.
pub fn admin_router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/portal/users", get(admin_users))
        .route("/portal/users/:id", get(admin_user).patch(admin_enable))
        .route("/portal/users/:id/credits", post(admin_credit))
        .route("/portal/requests/:id/settle", post(admin_settle))
}
#[derive(Deserialize)]
struct CreditInput {
    amount_cents: i64,
    reference: String,
}
#[derive(Deserialize)]
struct SettlementInput {
    status: u16,
    input_tokens: u32,
    cached_tokens: u32,
    output_tokens: u32,
}
async fn admin_settle(
    Path(id): Path<String>,
    Json(body): Json<SettlementInput>,
) -> Result<StatusCode> {
    let p = instance()?;
    if body.cached_tokens > body.input_tokens || !(200..=599).contains(&body.status) {
        return Err(Error::Invalid("无效的用量或状态".into()));
    }
    blocking(move || {
        p.store.audit("billing_reconcile", &id, &json!({"status":body.status,"input_tokens":body.input_tokens,"cached_tokens":body.cached_tokens,"output_tokens":body.output_tokens}).to_string())?;
        p.store.settle_billing(&id,body.status,Some((body.input_tokens,body.cached_tokens,body.output_tokens)))
    }).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn admin_credit(
    Path(id): Path<String>,
    Json(body): Json<CreditInput>,
) -> Result<Json<Value>> {
    let p = instance()?;
    Ok(Json(
        blocking(move || {
            p.store
                .credit_wallet(&id, body.amount_cents, &body.reference)?;
            p.store.wallet_dashboard(&id)
        })
        .await?,
    ))
}
async fn admin_users() -> Result<Json<Value>> {
    let p = instance()?;
    Ok(Json(blocking(move || p.store.admin_users()).await?))
}
async fn admin_user(Path(id): Path<String>) -> Result<Json<Value>> {
    let p = instance()?;
    Ok(Json(blocking(move || p.native_dashboard(&id)).await?))
}
#[derive(Deserialize)]
struct EnableInput {
    enabled: bool,
}
async fn admin_enable(Path(id): Path<String>, Json(body): Json<EnableInput>) -> Result<StatusCode> {
    let p = instance()?;
    blocking(move || {
        if body.enabled {
            p.store.set_enabled(&id, true)?;
            if let Err(error) = p.set_native_enabled(&id, true) {
                p.store.set_enabled(&id, false)?;
                return Err(error);
            }
        } else {
            p.set_native_enabled(&id, false)?;
            p.store.set_enabled(&id, false)?;
        }
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
