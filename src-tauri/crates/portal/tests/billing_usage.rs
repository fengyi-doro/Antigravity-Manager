// Test the actual shared pipeline calculator without desktop runtime dependencies.
#[allow(dead_code)]
#[path = "../../../src/proxy/pipeline/usage.rs"]
mod usage;
use serde_json::json;
use usage::CanonicalUsage;

#[test]
fn reasoning_is_billed_once_in_each_upstream_format() {
    let standard = CanonicalUsage::from_gemini(&json!({"promptTokenCount":1000,
        "cachedContentTokenCount":800,"candidatesTokenCount":20,"thoughtsTokenCount":30,"totalTokenCount":1050}));
    assert_eq!(
        (
            standard.total_input_tokens,
            standard.cached_tokens,
            standard.output_tokens
        ),
        (1000, 800, 50)
    );
    let modern = CanonicalUsage::from_gemini(&json!({"total_input_tokens":1000,
        "total_cached_tokens":800,"total_output_tokens":20,"total_thought_tokens":30}));
    assert_eq!(modern.output_tokens, 50);
    let openai = CanonicalUsage::from_gemini(&json!({"prompt_tokens":1000,
        "prompt_tokens_details":{"cached_tokens":800},"completion_tokens":50,
        "completion_tokens_details":{"reasoning_tokens":30}}));
    assert_eq!(openai.output_tokens, 50);
    assert_eq!(openai.uncached_input_tokens, 200);
    let claude = CanonicalUsage::from_gemini(&json!({"input_tokens":200,
        "cache_read_input_tokens":800,"output_tokens":50}));
    assert_eq!(
        (
            claude.total_input_tokens,
            claude.cached_tokens,
            claude.output_tokens
        ),
        (1000, 800, 50)
    );
}
