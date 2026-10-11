use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::proxy::middleware::auth::UserTokenIdentity;
use crate::proxy::server::AppState;
use crate::proxy::thinking_store::{sanitize_session_id, SessionScope, ThinkingStore};

#[derive(Debug, Deserialize, Default)]
pub struct EndSessionBody {
    #[serde(default)]
    pub session_id: Option<String>,
}

fn json_with_session(
    status: StatusCode,
    client_id: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(client_id) {
        headers.insert("X-Session-Id", v);
    }
    (status, headers, Json(body)).into_response()
}

#[derive(Debug)]
struct ResolvedSession {
    client_id: String,
    store_key: String,
}

fn resolve_end_key(
    headers: &HeaderMap,
    body_sid: Option<&str>,
    tenant_id: Option<&str>,
) -> Result<ResolvedSession, (StatusCode, String)> {
    let header_sid = crate::proxy::thinking_store::explicit_session_id(headers, None);
    let supplied_id = body_sid
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .or(header_sid.as_deref());
    if let Some(client_id) = supplied_id {
        if let Some(store_key) =
            SessionScope::store_key_for_client_id(headers, client_id, tenant_id)
        {
            return Ok(ResolvedSession {
                client_id: client_id.to_string(),
                store_key,
            });
        }
    }

    let from_body = body_sid
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| sanitize_session_id(s));
    let fallback = from_body.unwrap_or_default();
    if fallback.is_empty() && header_sid.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "session_id is required (body.session_id or X-Session-Id header)".to_string(),
        ));
    }
    let scope = SessionScope::resolve(headers, None, None, fallback, tenant_id);
    Ok(ResolvedSession {
        client_id: scope.client_id,
        store_key: scope.store_key,
    })
}

/// POST /v1/thinking/end
/// Body: { "session_id": "..." }  (or X-Session-Id header)
pub async fn handle_end_session(
    State(_state): State<AppState>,
    headers: HeaderMap,
    user_identity: Option<Extension<UserTokenIdentity>>,
    Json(body): Json<EndSessionBody>,
) -> impl IntoResponse {
    match resolve_end_key(
        &headers,
        body.session_id.as_deref(),
        user_identity
            .as_ref()
            .map(|identity| identity.token_id.as_str()),
    ) {
        Ok(scope) => {
            let result = ThinkingStore::global().end_session(&scope.store_key);
            tracing::info!(
                "[ThinkingStore] Ended session {} (deleted {} turns, {} bytes)",
                scope.client_id,
                result.deleted_turns,
                result.deleted_bytes
            );
            json_with_session(
                StatusCode::OK,
                &scope.client_id,
                json!({
                    "ok": true,
                    "session_id": result.session_id,
                    "deleted_turns": result.deleted_turns,
                    "deleted_bytes": result.deleted_bytes
                }),
            )
        }
        Err((status, msg)) => (
            status,
            Json(json!({
                "ok": false,
                "error": { "message": msg }
            })),
        )
            .into_response(),
    }
}

/// DELETE /v1/thinking/sessions/:session_id
pub async fn handle_delete_session(
    State(_state): State<AppState>,
    headers: HeaderMap,
    user_identity: Option<Extension<UserTokenIdentity>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match resolve_end_key(
        &headers,
        Some(&session_id),
        user_identity
            .as_ref()
            .map(|identity| identity.token_id.as_str()),
    ) {
        Ok(scope) => {
            let result = ThinkingStore::global().end_session(&scope.store_key);
            json_with_session(
                StatusCode::OK,
                &scope.client_id,
                json!({
                    "ok": true,
                    "session_id": result.session_id,
                    "deleted_turns": result.deleted_turns,
                    "deleted_bytes": result.deleted_bytes
                }),
            )
        }
        Err((status, msg)) => (
            status,
            Json(json!({
                "ok": false,
                "error": { "message": msg }
            })),
        )
            .into_response(),
    }
}

/// GET /v1/thinking/sessions/:session_id
pub async fn handle_session_stats(
    State(_state): State<AppState>,
    headers: HeaderMap,
    user_identity: Option<Extension<UserTokenIdentity>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match resolve_end_key(
        &headers,
        Some(&session_id),
        user_identity
            .as_ref()
            .map(|identity| identity.token_id.as_str()),
    ) {
        Ok(scope) => {
            let (turns, bytes) = ThinkingStore::global()
                .session_stats(&scope.store_key)
                .unwrap_or((0, 0));
            json_with_session(
                StatusCode::OK,
                &scope.client_id,
                json!({
                    "ok": true,
                    "session_id": scope.client_id,
                    "turns": turns,
                    "bytes": bytes
                }),
            )
        }
        Err((status, msg)) => (
            status,
            Json(json!({
                "ok": false,
                "error": { "message": msg }
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_resolves_returned_id_in_the_authenticated_tenant() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-native-user".parse().unwrap());
        headers.insert("x-session-id", "client-conversation".parse().unwrap());
        let generated = SessionScope::resolve(
            &headers,
            None,
            None,
            "content-anchor",
            Some("native-user-id"),
        );

        let from_body =
            resolve_end_key(&headers, Some(&generated.client_id), Some("native-user-id")).unwrap();
        assert_eq!(from_body.client_id, generated.client_id);
        assert_eq!(from_body.store_key, generated.store_key);

        headers.insert("x-session-id", generated.client_id.parse().unwrap());
        let from_header = resolve_end_key(&headers, None, Some("native-user-id")).unwrap();
        assert_eq!(from_header.store_key, generated.store_key);
    }

    #[test]
    fn management_never_uses_another_users_namespace() {
        let headers = HeaderMap::new();
        let generated = SessionScope::resolve(&headers, None, None, "anchor", Some("user-a"));
        let other = resolve_end_key(&headers, Some(&generated.client_id), Some("user-b")).unwrap();
        assert_ne!(other.store_key, generated.store_key);
    }

    #[test]
    fn management_resolves_master_key_sessions_across_protocol_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-master".parse().unwrap());
        let generated = SessionScope::resolve(&headers, None, None, "anchor", None);
        headers.remove("authorization");
        headers.insert("x-goog-api-key", "sk-master".parse().unwrap());
        let resolved = resolve_end_key(&headers, Some(&generated.client_id), None).unwrap();
        assert_eq!(resolved.store_key, generated.store_key);
    }

    #[test]
    fn raw_session_ids_keep_the_existing_resolution() {
        let headers = HeaderMap::new();
        let expected = SessionScope::resolve(&headers, None, None, "raw-session", Some("user-a"));
        let resolved = resolve_end_key(&headers, Some("raw-session"), Some("user-a")).unwrap();
        assert_eq!(resolved.store_key, expected.store_key);
        assert!(resolve_end_key(&headers, None, Some("user-a")).is_err());
    }

    #[test]
    fn canonical_ids_cannot_supply_a_tenant_prefix_or_arbitrary_name() {
        let headers = HeaderMap::new();
        for id in [
            "other-tenant:sess-0123456789abcdef",
            "sess-0123456789abcde",
            "sess-0123456789abcdef0",
            "sess-0123456789abcdeg",
            "sess-0123456789abcdeF",
            "ordinary-conversation",
        ] {
            assert!(SessionScope::store_key_for_client_id(&headers, id, Some("user-a")).is_none());
        }
    }
}
