//! User identities linked to the application's original user-token implementation.
pub mod account_http;
pub mod account_services;
pub mod billing;
pub mod db;
pub mod http;
pub mod mail;
pub mod native;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Invalid(String),
    Unauthorized,
    Forbidden,
    InsufficientBalance,
    Conflict,
    Unavailable(String),
    Internal(String),
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal(e.to_string())
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Invalid(s) => (StatusCode::BAD_REQUEST, "invalid_request", s),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "请重新登录或检查密钥".into(),
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "无权执行此操作".into()),
            Self::InsufficientBalance => (
                StatusCode::PAYMENT_REQUIRED,
                "insufficient_balance",
                "余额不足，请联系管理员充值".into(),
            ),
            Self::Conflict => (
                StatusCode::CONFLICT,
                "conflict",
                "账号已存在或操作发生冲突".into(),
            ),
            Self::Unavailable(s) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", s),
            Self::Internal(s) => {
                tracing::error!(error = %s, "Portal operation failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "操作失败，请稍后重试".into(),
                )
            }
        };
        (
            status,
            Json(json!({"error":{"code":code,"type":code,"message":message}})),
        )
            .into_response()
    }
}

pub struct Portal {
    pub store: db::Store,
    pub config: http::Config,
    pub tokens: Option<Arc<dyn native::TokenBackend>>,
    token_lock: Mutex<()>,
}
impl Portal {
    pub fn new(
        store: db::Store,
        config: http::Config,
        tokens: Option<Arc<dyn native::TokenBackend>>,
    ) -> Self {
        Self {
            store,
            config,
            tokens,
            token_lock: Mutex::new(()),
        }
    }
}
static PORTAL: OnceLock<Arc<Portal>> = OnceLock::new();

pub fn initialize(data_dir: &Path, tokens: Arc<dyn native::TokenBackend>) -> Result<()> {
    let config = http::Config::from_env()?;
    let store = db::Store::open(data_dir.join("portal.db"))?;
    store.recover_billing()?;
    let portal = Portal::new(store, config, Some(tokens));
    for user in portal.store.users()? {
        portal.ensure_native_token(&user)?;
    }
    PORTAL
        .set(Arc::new(portal))
        .map_err(|_| Error::Internal("Portal already initialized".into()))
}

pub fn instance() -> Result<Arc<Portal>> {
    PORTAL
        .get()
        .cloned()
        .ok_or_else(|| Error::Unavailable("用户服务尚未初始化".into()))
}

pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
}
