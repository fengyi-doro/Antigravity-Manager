pub fn is_billable(method: &str, path: &str) -> bool {
    if method != "POST" {
        return false;
    }
    if matches!(
        path,
        "/v1/messages/count_tokens" | "/v1/models/detect" | "/v1/thinking/end"
    ) {
        return false;
    }
    if let Some(model_action) = path.strip_prefix("/v1beta/models/") {
        if let Some(model) = model_action
            .strip_suffix("/countTokens")
            .or_else(|| model_action.strip_suffix(":countTokens"))
        {
            if !model.is_empty() && !model.contains('/') {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn free_endpoints_are_exact_paths() {
        assert!(!is_billable("POST", "/v1/messages/count_tokens"));
        assert!(!is_billable(
            "POST",
            "/v1beta/models/gemini-flash/countTokens"
        ));
        assert!(!is_billable(
            "POST",
            "/v1beta/models/gemini-flash:countTokens"
        ));
        for path in [
            "/v1/chat/completions",
            "/v1beta/models/count_tokens/generateContent",
            "/v1/messages/count_tokens/extra",
        ] {
            assert!(is_billable("POST", path));
        }
    }
}
