use crate::{
    blocking,
    db::{self, User},
    instance,
    payment::Payment,
    Error, Portal, Result,
};
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use subtle::ConstantTimeEq;

pub struct Config {
    pub public_url: String,
    pub api_base_url: String,
    pub secure_cookie: bool,
    pub registration_open: bool,
    pub trust_proxy: bool,
    pub payment: Option<Payment>,
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
        let payment = match (
            std::env::var("ABV_EPAY_URL"),
            std::env::var("ABV_EPAY_PID"),
            std::env::var("ABV_EPAY_KEY"),
        ) {
            (Ok(url), Ok(pid), Ok(key))
                if !url.is_empty() && !pid.is_empty() && !key.is_empty() =>
            {
                Some(Payment::new(&url, pid, key, public_url.clone())?)
            }
            _ => None,
        };
        if payment.is_some() && url.scheme() != "https" {
            return Err(Error::Invalid("开启真实支付需要 HTTPS 用户中心域名".into()));
        }
        Ok(Self {
            api_base_url: std::env::var("ABV_PORTAL_API_BASE_URL")
                .unwrap_or_else(|_| format!("{public_url}/v1")),
            secure_cookie: url.scheme() == "https",
            public_url,
            registration_open: std::env::var("ABV_PORTAL_REGISTRATION_OPEN").as_deref()
                != Ok("false"),
            trust_proxy: std::env::var("ABV_PORTAL_TRUST_PROXY").as_deref() == Ok("true"),
            payment,
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
        .route("/portal/api/key", post(rotate_key).delete(revoke_key))
        .route("/portal/api/orders", post(create_order))
        .route("/portal/api/orders/:id/reconcile", post(reconcile))
        .route(
            "/portal/api/payments/notify",
            get(notify_get).post(notify_post),
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
async fn config(State(p): State<Arc<Portal>>) -> Json<Value> {
    Json(
        json!({"registration_open":p.config.registration_open,"payment_enabled":p.config.payment.is_some(),
        "api_base_url":p.config.api_base_url,"models":crate::proxy::allowed_models(),"signup_tokens":db::SIGNUP_TOKENS,"tokens_per_yuan":1_000_000,
        "packages":[{"cents":100,"tokens":1_000_000},{"cents":1000,"tokens":10_000_000},{"cents":5000,"tokens":50_000_000},{"cents":10000,"tokens":100_000_000}]}),
    )
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
    let store = p.store.clone();
    let (user, token, csrf, key) = blocking(move || {
        store.rate_limit(&bucket, 3, 86_400, db::now())?;
        let user = store.register(&body.email, &body.password, db::now())?;
        let (token, csrf) = store.create_session(&user.id, db::now())?;
        let key = store.rotate_key(&user.id, db::now())?;
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
    let data = blocking(move || p.store.dashboard(&id, db::now())).await?;
    Ok(Json(json!({"user":user,"csrf":csrf,"data":data})))
}
async fn rotate_key(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<Json<Value>> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    let key = blocking(move || p.store.rotate_key(&user.id, db::now())).await?;
    Ok(Json(json!({"api_key":key})))
}
async fn revoke_key(State(p): State<Arc<Portal>>, headers: HeaderMap) -> Result<StatusCode> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    blocking(move || p.store.revoke_keys(&user.id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct OrderInput {
    amount_cents: i64,
    channel: String,
    idempotency_key: String,
}
async fn create_order(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Json(body): Json<OrderInput>,
) -> Result<Json<Value>> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    let payment = p
        .config
        .payment
        .clone()
        .ok_or_else(|| Error::Unavailable("充值暂未开放".into()))?;
    let order = blocking(move || {
        p.store
            .rate_limit(&format!("orders:{}", user.id), 20, 3600, db::now())?;
        p.store.create_order(
            &user.id,
            body.amount_cents,
            &body.channel,
            &body.idempotency_key,
            db::now(),
        )
    })
    .await?;
    Ok(Json(
        json!({"checkout":payment.checkout(&order)?,"order":order}),
    ))
}
async fn reconcile(
    State(p): State<Arc<Portal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let (user, _) = authenticated(p.clone(), &headers, true).await?;
    let payment = p
        .config
        .payment
        .clone()
        .ok_or_else(|| Error::Unavailable("充值暂未开放".into()))?;
    let store = p.store.clone();
    let owner = user.id.clone();
    let oid = id.clone();
    let order = blocking(move || {
        store.rate_limit(&format!("query:{owner}"), 12, 60, db::now())?;
        store.order(&oid, &owner)
    })
    .await?;
    if let Some((trade, cents, channel)) = payment.query(&order).await? {
        let store = p.store.clone();
        let oid = id.clone();
        blocking(move || store.credit_order(&oid, &trade, cents, &channel, db::now())).await?;
    }
    Ok(Json(
        serde_json::to_value(blocking(move || p.store.order(&id, &user.id)).await?)
            .map_err(|e| Error::Internal(e.to_string()))?,
    ))
}
async fn notify_get(
    State(p): State<Arc<Portal>>,
    Query(fields): Query<BTreeMap<String, String>>,
) -> Result<&'static str> {
    process_notify(p, fields).await
}
async fn notify_post(
    State(p): State<Arc<Portal>>,
    axum::Form(fields): axum::Form<BTreeMap<String, String>>,
) -> Result<&'static str> {
    process_notify(p, fields).await
}
async fn process_notify(p: Arc<Portal>, fields: BTreeMap<String, String>) -> Result<&'static str> {
    let payment = p
        .config
        .payment
        .clone()
        .ok_or_else(|| Error::Unavailable("充值暂未开放".into()))?;
    blocking(move || payment.notify(&p.store, &fields, db::now())).await?;
    Ok("success")
}

/// Merge this into the existing administrator router BEFORE its strict auth layer.
pub fn admin_router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/portal/users", get(admin_users))
        .route("/portal/users/:id", get(admin_user).patch(admin_enable))
        .route("/portal/requests/:id/settle", post(admin_settle))
}
async fn admin_users() -> Result<Json<Value>> {
    let p = instance()?;
    Ok(Json(blocking(move || p.store.admin_users()).await?))
}
async fn admin_user(Path(id): Path<String>) -> Result<Json<Value>> {
    let p = instance()?;
    Ok(Json(
        blocking(move || p.store.dashboard(&id, db::now())).await?,
    ))
}
#[derive(Deserialize)]
struct EnableInput {
    enabled: bool,
}
async fn admin_enable(Path(id): Path<String>, Json(body): Json<EnableInput>) -> Result<StatusCode> {
    let p = instance()?;
    blocking(move || p.store.set_enabled(&id, body.enabled)).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct SettleInput {
    input_tokens: i64,
    output_tokens: i64,
    reason: String,
}
async fn admin_settle(Path(id): Path<String>, Json(body): Json<SettleInput>) -> Result<StatusCode> {
    if body.reason.trim().len() < 4 || body.reason.len() > 500 {
        return Err(Error::Invalid("请填写对账依据".into()));
    }
    let p = instance()?;
    blocking(move || {
        p.store.manual_settle(
            &id,
            body.input_tokens,
            body.output_tokens,
            db::now(),
            &body.reason,
        )
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
