//! User center adapter for the original user-token API and its stored statistics.
use crate::modules::user_token_db;
use antigravity_portal::{
    db::User,
    native::{NativeToken, TokenBackend},
    Error, Result,
};
use rusqlite::params;
use serde_json::json;

pub struct NativeUserTokens;

impl TokenBackend for NativeUserTokens {
    fn create(&self, user: &User, previous: Option<&str>) -> Result<NativeToken> {
        let previous = previous
            .map(user_token_db::get_token_by_id)
            .transpose()
            .map_err(Error::Internal)?
            .flatten();
        // Rotation keeps the administrator's native expiry, IP and curfew settings.
        let expires_at = previous.as_ref().and_then(|token| token.expires_at);
        let token = user_token_db::create_token(
            user.email.clone(),
            if expires_at.is_some() {
                "custom"
            } else {
                "never"
            }
            .into(),
            Some(format!("用户中心账户 {}", user.id)),
            previous.as_ref().map_or(0, |token| token.max_ips),
            previous
                .as_ref()
                .and_then(|token| token.curfew_start.clone()),
            previous.as_ref().and_then(|token| token.curfew_end.clone()),
            expires_at,
        )
        .map_err(Error::Internal)?;
        if !user.enabled || previous.as_ref().is_some_and(|token| !token.enabled) {
            self.set_enabled(&token.id, false)?;
        }
        self.get(&token.id)?
            .ok_or_else(|| Error::Internal("原版 Token 创建后不可用".into()))
    }

    fn get(&self, id: &str) -> Result<Option<NativeToken>> {
        let Some(token) = user_token_db::get_token_by_id(id).map_err(Error::Internal)? else {
            return Ok(None);
        };
        let conn = user_token_db::connect_db().map_err(Error::Internal)?;
        let mut stmt = conn.prepare(
            "SELECT id,model,input_tokens,output_tokens,request_time,status
             FROM token_usage_logs WHERE token_id=?1 ORDER BY request_time DESC,rowid DESC LIMIT 30"
        )?;
        let usage = stmt.query_map(params![id], |row| Ok(json!({
            "id":row.get::<_,String>(0)?,"model":row.get::<_,Option<String>>(1)?,
            "input_tokens":row.get::<_,Option<i32>>(2)?,"output_tokens":row.get::<_,Option<i32>>(3)?,
            "request_time":row.get::<_,i64>(4)?,"status":row.get::<_,Option<u16>>(5)?
        })))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(NativeToken {
            suffix: token
                .token
                .chars()
                .rev()
                .take(8)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
            id: token.id,
            token: token.token,
            enabled: token.enabled,
            expires_at: token.expires_at,
            created_at: token.created_at,
            total_requests: token.total_requests,
            total_tokens_used: token.total_tokens_used,
            last_used_at: token.last_used_at,
            usage,
        }))
    }

    fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        user_token_db::update_token(id, None, None, Some(enabled), None, None, None)
            .map_err(Error::Internal)
    }
}
