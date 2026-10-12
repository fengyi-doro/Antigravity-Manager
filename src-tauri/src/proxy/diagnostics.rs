//! Bounded, credential-free request diagnostics shared by all protocol adapters.
use serde::{Deserialize, Serialize};

pub fn response_failed(value: &serde_json::Value) -> bool {
    matches!(
        value.get("type").and_then(|v| v.as_str()),
        Some("error" | "response.failed")
    ) || value.get("error").is_some_and(|v| !v.is_null())
        || value
            .get("response")
            .and_then(|v| v.get("error"))
            .is_some_and(|v| !v.is_null())
}

pub fn error_class(status: u16, message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if message.contains("validation_required") || message.contains("verify your account") {
        "account_validation"
    } else if message.contains("invalid_grant") || message.contains("token refresh") {
        "token_refresh"
    } else {
        match status {
            200..=299 => "success",
            401 => "authentication",
            402 => "insufficient_balance",
            403 => "permission_or_region",
            404 => "model_or_endpoint",
            408 | 504 => "timeout",
            429 => "rate_limit",
            499 => "client_cancelled",
            500..=599 => "upstream_or_gateway",
            0 => "network",
            _ => "request_error",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamAttempt {
    pub sequence: u32,
    pub model: String,
    pub account_id: Option<String>,
    pub endpoint: String,
    pub status: Option<u16>,
    pub elapsed_ms: u64,
    pub error_class: String,
}

impl UpstreamAttempt {
    pub fn new(
        model: &str,
        account: Option<&str>,
        endpoint: &str,
        status: Option<u16>,
        elapsed_ms: u64,
        class: &str,
    ) -> Self {
        // Never accept a URL query, credentials, raw error body, or request payload.
        let endpoint = url::Url::parse(endpoint)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into());
        Self {
            sequence: 0,
            model: model.chars().take(128).collect(),
            account_id: account.map(|s| s.chars().take(128).collect()),
            endpoint,
            status,
            elapsed_ms,
            error_class: class.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_failures_are_distinct_from_nullable_success_fields() {
        assert!(response_failed(
            &serde_json::json!({"error":{"message":"failed"}})
        ));
        assert!(response_failed(
            &serde_json::json!({"type":"response.failed","response":{"error":{"code":"failed"}}})
        ));
        assert!(response_failed(&serde_json::json!({"type":"error"})));
        assert!(!response_failed(
            &serde_json::json!({"error":null,"response":{"error":null}})
        ));
    }
    #[test]
    fn retries_keep_causes_distinct_and_endpoint_secrets_are_excluded() {
        assert_eq!(error_class(404, ""), "model_or_endpoint");
        assert_eq!(error_class(429, ""), "rate_limit");
        assert_eq!(
            error_class(403, "VALIDATION_REQUIRED"),
            "account_validation"
        );
        assert_eq!(error_class(503, "Token refresh timed out"), "token_refresh");
        assert_eq!(error_class(499, ""), "client_cancelled");
        let a = UpstreamAttempt::new(
            "model",
            Some("id"),
            "https://user:secret@example.com/path?token=secret",
            Some(404),
            5,
            "model_or_endpoint",
        );
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(a.endpoint, "example.com");
        assert!(!json.contains("secret"));
    }
}
