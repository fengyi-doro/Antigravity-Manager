//! Account-to-native-token links. No AI request handling or usage calculation lives here.
use crate::{db::User, Error, Portal, Result};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize)]
pub struct NativeToken {
    pub id: String,
    #[serde(skip_serializing)]
    pub token: String,
    pub suffix: String,
    pub enabled: bool,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub total_requests: i64,
    pub total_tokens_used: i64,
    pub last_used_at: Option<i64>,
    pub usage: Vec<Value>,
}

/// Implemented by the application using its original user_token_db module.
pub trait TokenBackend: Send + Sync {
    fn create(&self, user: &User, previous: Option<&str>) -> Result<NativeToken>;
    fn get(&self, id: &str) -> Result<Option<NativeToken>>;
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<()>;
}

impl Portal {
    fn backend(&self) -> Result<&dyn TokenBackend> {
        self.tokens
            .as_deref()
            .ok_or_else(|| Error::Unavailable("原版用户 Token 服务尚未初始化".into()))
    }

    /// Provision only accounts that have never been linked. Administrator deletion
    /// or suspension of a native token must not be undone by visiting the dashboard.
    pub fn ensure_native_token(&self, user: &User) -> Result<Option<NativeToken>> {
        let _lock = self
            .token_lock
            .lock()
            .map_err(|e| Error::Internal(e.to_string()))?;
        if let Some(id) = self.store.native_token_id(&user.id)? {
            return self.backend()?.get(&id);
        }
        self.create_native_token(user).map(Some)
    }

    fn create_native_token(&self, user: &User) -> Result<NativeToken> {
        let token = self.backend()?.create(user, None)?;
        if !user.enabled {
            self.backend()?.set_enabled(&token.id, false)?;
        }
        if let Err(error) = self.store.link_native_token(&user.id, &token.id) {
            // A token that failed to link must never remain usable.
            self.backend()?.set_enabled(&token.id, false)?;
            return Err(error);
        }
        Ok(token)
    }

    pub fn rotate_native_token(&self, user: &User) -> Result<String> {
        let _lock = self
            .token_lock
            .lock()
            .map_err(|e| Error::Internal(e.to_string()))?;
        let backend = self.backend()?;
        let previous = match self.store.native_token_id(&user.id)? {
            Some(id) => backend.get(&id)?,
            None => None,
        };
        // Create first so a failed creation cannot invalidate the working key.
        let replacement = backend.create(user, previous.as_ref().map(|token| token.id.as_str()))?;
        if let Some(previous) = &previous {
            if let Err(error) = backend.set_enabled(&previous.id, false) {
                backend.set_enabled(&replacement.id, false)?;
                return Err(error);
            }
        }
        if let Err(error) = self.store.link_native_token(&user.id, &replacement.id) {
            backend.set_enabled(&replacement.id, false)?;
            if let Some(previous) = previous {
                backend.set_enabled(&previous.id, previous.enabled)?;
            }
            return Err(error);
        }
        Ok(replacement.token)
    }

    pub fn set_native_enabled(&self, user: &str, enabled: bool) -> Result<()> {
        let _lock = self
            .token_lock
            .lock()
            .map_err(|e| Error::Internal(e.to_string()))?;
        if let Some(id) = self.store.native_token_id(user)? {
            if self.backend()?.get(&id)?.is_some() {
                self.backend()?.set_enabled(&id, enabled)?;
            }
        }
        Ok(())
    }

    pub fn native_dashboard(&self, user: &str) -> Result<Value> {
        let token = match self.store.native_token_id(user)? {
            Some(id) => self.backend()?.get(&id)?,
            None => None,
        };
        // Values come directly from the native token counters and usage rows.
        Ok(json!({"billing_enabled":false,"key":token,
            "legacy":self.store.legacy_dashboard(user)?}))
    }
}
