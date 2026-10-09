use crate::{blocking, db, instance, Error, Result};
use axum::{
    body::Body,
    extract::Request,
    http::{header, HeaderValue, Method},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{json, Value};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

pub fn allowed_models() -> Vec<String> {
    std::env::var("ABV_PORTAL_ALLOWED_MODELS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
pub struct Budget {
    input: usize,
    output: u64,
    sent: AtomicBool,
    rejected: AtomicBool,
}
tokio::task_local! {pub static ACTIVE_BUDGET: Arc<Budget>;}

/// Capture before spawning: Tokio task locals do not propagate automatically.
pub fn scope_current<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let budget = ACTIVE_BUDGET.try_with(Clone::clone).ok();
    async move {
        match budget {
            Some(b) => ACTIVE_BUDGET.scope(b, future).await,
            None => future.await,
        }
    }
}

/// Run immediately before each actual upstream network send, after all normalizers.
/// Portal requests use one bounded text generation; automatic retries/auxiliary
/// generations cannot silently spend an unreserved second request.
pub fn constrain_upstream(method: &str, body: &mut Value) -> std::result::Result<(), String> {
    let Ok(budget) = ACTIVE_BUDGET.try_with(Clone::clone) else {
        return Ok(());
    };
    if !matches!(method, "generateContent" | "streamGenerateContent") {
        return Err("Portal keys do not allow auxiliary upstream requests".into());
    }
    if body["model"].as_str().is_some_and(|model| {
        ["image", "audio", "video", "tts"]
            .iter()
            .any(|kind| model.to_lowercase().contains(kind))
    }) {
        return Err("This model is not enabled for token billing".into());
    }
    let payload = if body.get("request").is_some() {
        &mut body["request"]
    } else {
        body
    };
    if unsupported_content(payload) {
        return Err("Portal billing currently supports text requests only".into());
    }
    if payload.get("generationConfig").is_none() {
        payload["generationConfig"] = json!({});
    }
    let gc = &mut payload["generationConfig"];
    let output = gc["maxOutputTokens"]
        .as_u64()
        .unwrap_or(budget.output)
        .min(budget.output);
    gc["maxOutputTokens"] = json!(output);
    gc["candidateCount"] = json!(1);
    if let Some(thinking) = gc.get_mut("thinkingConfig") {
        let limit = output.saturating_sub(1);
        let requested = thinking["thinkingBudget"].as_u64().unwrap_or(limit / 2);
        thinking["thinkingBudget"] = json!(requested.min(limit));
    }
    let size = serde_json::to_vec(payload)
        .map_err(|_| "Cannot measure upstream payload")?
        .len();
    if size > budget.input {
        return Err("Expanded upstream context exceeds the reserved input budget".into());
    }
    if budget.sent.swap(true, Ordering::AcqRel) {
        return Err("Portal request budget already used; retry as a new request".into());
    }
    Ok(())
}
pub fn note_upstream_response(status: u16) {
    if matches!(status, 400 | 401 | 403 | 404 | 422 | 429) {
        let _ = ACTIVE_BUDGET.try_with(|budget| budget.rejected.store(true, Ordering::Release));
    }
}
pub fn api_key(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v))
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
        })
        .or_else(|| {
            request
                .headers()
                .get("x-goog-api-key")
                .and_then(|v| v.to_str().ok())
        })
}
pub async fn handle(request: Request, next: Next) -> Response {
    match handle_inner(request, next).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}
async fn handle_inner(request: Request, next: Next) -> Result<Response> {
    let p = instance()?;
    let key = api_key(&request).ok_or(Error::Unauthorized)?.to_owned();
    let store = p.store.clone();
    let identity = blocking(move || store.key_identity(&key)).await?;
    let path = request.uri().path().to_owned();
    let models = allowed_models();
    if request.method() == Method::GET && path == "/v1/models" {
        return Ok(Json(json!({"object":"list","data":models.iter().map(|m|json!({"id":m,"object":"model","owned_by":"portal"})).collect::<Vec<_>>()})).into_response());
    }
    if request.method() == Method::GET && path == "/v1beta/models" {
        return Ok(Json(json!({"models":models.iter().map(|m|json!({"name":format!("models/{m}"),"supportedGenerationMethods":["generateContent","streamGenerateContent"]})).collect::<Vec<_>>()})).into_response());
    }
    let gemini = path.starts_with("/v1beta/models/")
        && (path.ends_with(":generateContent") || path.ends_with(":streamGenerateContent"));
    if request.method() != Method::POST
        || !(gemini
            || matches!(
                path.as_str(),
                "/v1/chat/completions" | "/v1/completions" | "/v1/responses" | "/v1/messages"
            ))
    {
        return Err(Error::Forbidden);
    }
    let (mut parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1_048_576)
        .await
        .map_err(|_| Error::Invalid("请求体超过 1 MB".into()))?;
    let mut value: Value =
        serde_json::from_slice(&bytes).map_err(|_| Error::Invalid("请求必须为 JSON".into()))?;
    if !value.is_object()
        || unsupported_content(&value)
        || ["previous_response_id", "conversation", "cachedContent"]
            .iter()
            .any(|k| value.get(*k).is_some())
        || value["n"].as_u64().unwrap_or(1) != 1
        || value["generationConfig"]["candidateCount"]
            .as_u64()
            .unwrap_or(1)
            != 1
    {
        return Err(Error::Invalid(
            "当前用户 Key 支持单次文本请求，请携带完整对话；暂不支持图片、音视频和服务端历史引用"
                .into(),
        ));
    }
    let model = if gemini {
        path.trim_start_matches("/v1beta/models/")
            .split(':')
            .next()
            .unwrap_or("")
            .to_owned()
    } else {
        value["model"].as_str().unwrap_or("").to_owned()
    };
    if !models.contains(&model) {
        return Err(Error::Invalid(
            "该模型尚未开放，请使用用户中心列出的模型".into(),
        ));
    }
    let output_fields = ["max_tokens", "max_completion_tokens", "max_output_tokens"];
    let explicit = output_fields.iter().filter_map(|k| value.get(*k)).chain(
        value
            .get("generationConfig")
            .and_then(|v| v.get("maxOutputTokens")),
    );
    let mut output = None;
    for field in explicit {
        let v = field
            .as_u64()
            .filter(|v| *v > 0 && *v <= 65_536)
            .ok_or_else(|| Error::Invalid("输出上限应在 1–65536 token 之间".into()))?;
        output = Some(output.map_or(v, |previous: u64| previous.min(v)));
    }
    let output = output.unwrap_or(4096);
    // Ask OpenAI-compatible streaming adapters to include final usage.
    if path == "/v1/chat/completions" && value["stream"] == true {
        value["stream_options"] = json!({"include_usage":true});
    }
    // Conservative text budget: one token per serialized UTF-8 byte, plus
    // room for gateway-injected context. Final outbound payload is checked too.
    let input = bytes.len() + 65_536;
    let store = p.store.clone();
    let reserve_identity = identity.clone();
    let reserve_model = model.clone();
    let request_id = blocking(move || {
        store.rate_limit(
            &format!("api:{}", reserve_identity.user_id),
            120,
            60,
            db::now(),
        )?;
        store.reserve(
            &reserve_identity,
            &reserve_model,
            input as i64 + output as i64,
            db::now(),
        )
    })
    .await?;
    let mut guard = ReservationGuard {
        store: p.store.clone(),
        id: request_id.clone(),
        finished: false,
    };
    let budget = Arc::new(Budget {
        input,
        output,
        sent: AtomicBool::new(false),
        rejected: AtomicBool::new(false),
    });
    parts.headers.remove(header::CONTENT_LENGTH);
    let request = Request::from_parts(
        parts,
        Body::from(serde_json::to_vec(&value).map_err(|e| Error::Internal(e.to_string()))?),
    );
    let response = match tokio::time::timeout(
        Duration::from_secs(180),
        ACTIVE_BUDGET.scope(budget.clone(), next.run(request)),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return Err(Error::Unavailable("请求超时，额度正在对账".into())),
    };
    let mut response = response;
    for header in [
        "x-account-email",
        "x-account-id",
        "x-session-id",
        "x-antigravity-session-id",
    ] {
        response.headers_mut().remove(header);
    }
    let status = response.status();
    if !status.is_success() {
        // Only failures known to have happened before an upstream send are refundable.
        if !budget.sent.load(Ordering::Acquire) || budget.rejected.load(Ordering::Acquire) {
            guard.finish(0, 0).await?;
        }
        let mut response=(status,Json(json!({"error":{
            "type":"upstream_error","code":"upstream_error","request_id":request_id,
            "message":if guard.finished {"上游未接受请求，预留额度已释放"} else {"上游请求未完成，额度等待对账"}
        }}))).into_response();
        response.headers_mut().insert(
            "x-portal-request-id",
            HeaderValue::from_str(&request_id)
                .map_err(|_| Error::Internal("Request ID invalid".into()))?,
        );
        return Ok(response);
    }
    let is_sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("text/event-stream");
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        "x-portal-request-id",
        HeaderValue::from_str(&request_id)
            .map_err(|_| Error::Internal("Request ID invalid".into()))?,
    );
    let stream = async_stream::stream! {
        // Ownership must move into the body so disconnects retain the reservation.
        let mut guard=guard;
        let mut source=body.into_data_stream();
        let mut usage=Usage::default();
        let mut buffer=Vec::new();
        let mut clean=true;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(900);
        loop {
            let item=match tokio::time::timeout_at(deadline,ACTIVE_BUDGET.scope(budget.clone(),source.next())).await {
                Ok(item)=>item,
                Err(_)=>{clean=false;break;}
            };
            match item {
                Some(Ok(chunk))=>{
                    buffer.extend_from_slice(&chunk);
                    if is_sse {
                        while let Some(pos)=buffer.iter().position(|v|*v==b'\n') {
                            let line:Vec<_>=buffer.drain(..=pos).collect();
                            if let Ok(text)=std::str::from_utf8(&line) {
                                if let Some(data)=text.trim().strip_prefix("data:") {
                                    if data.trim()=="[DONE]" {usage.terminal=true;}
                                    if let Ok(v)=serde_json::from_str::<Value>(data.trim()) {usage.observe(&v);}
                                }
                            }
                        }
                    }
                    if buffer.len()>4_194_304 {clean=false;break;}
                    yield Ok::<Bytes,std::io::Error>(chunk);
                }
                Some(Err(_))=>{clean=false;break;}
                None=>break,
            }
        }
        if !is_sse && clean {
            if let Ok(v)=serde_json::from_slice::<Value>(&buffer) {usage.observe(&v);}
        }
        if clean && !usage.failed && usage.complete() && (!is_sse || usage.terminal) {
            if let Err(e)=guard.finish(usage.input.unwrap_or(0),usage.output.unwrap_or(0)).await {
                tracing::error!(?e,"Portal settlement retained for review");
            }
        }
    };
    Ok(Response::from_parts(parts, Body::from_stream(stream)))
}

struct ReservationGuard {
    store: db::Store,
    id: String,
    finished: bool,
}
impl ReservationGuard {
    async fn finish(&mut self, input: i64, output: i64) -> Result<()> {
        let store = self.store.clone();
        let id = self.id.clone();
        blocking(move || store.settle(&id, input, output, db::now())).await?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for ReservationGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let store = self.store.clone();
        let id = self.id.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || {
                if let Err(e) = store.mark_review(&id) {
                    tracing::error!(?e, "Could not mark reservation for review");
                }
            });
        }
    }
}
fn unsupported_content(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            if [
                "inlineData",
                "fileData",
                "cachedContent",
                "googleSearch",
                "google_search",
                "urlContext",
            ]
            .iter()
            .any(|key| map.contains_key(*key))
                || map
                    .get("responseModalities")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.iter().any(|v| v.as_str() != Some("TEXT")))
            {
                return true;
            }
            if let Some(kind) = map.get("type").and_then(Value::as_str) {
                if [
                    "image",
                    "image_url",
                    "input_image",
                    "audio",
                    "input_audio",
                    "file",
                    "input_file",
                    "video",
                    "web_search",
                    "web_search_preview",
                    "computer_use_preview",
                ]
                .contains(&kind)
                {
                    return true;
                }
            }
            map.values().any(unsupported_content)
        }
        Value::Array(items) => items.iter().any(unsupported_content),
        _ => false,
    }
}
#[derive(Default, Debug)]
pub struct Usage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub terminal: bool,
    pub failed: bool,
}
impl Usage {
    pub fn complete(&self) -> bool {
        self.input.is_some() && self.output.is_some()
    }
    pub fn observe(&mut self, v: &Value) {
        if matches!(
            v["type"].as_str(),
            Some("response.completed" | "message_stop")
        ) {
            self.terminal = true;
        }
        if v.get("error").is_some()
            || matches!(
                v["type"].as_str(),
                Some("error" | "response.failed" | "response.incomplete")
            )
        {
            self.failed = true;
        }
        for key in ["candidates", "choices"] {
            if v[key].as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("finishReason")
                        .or_else(|| item.get("finish_reason"))
                        .is_some_and(|v| !v.is_null())
                })
            }) {
                self.terminal = true;
            }
        }
        if let Some(u) = v.get("usage").or_else(|| v.get("usageMetadata")) {
            let number = |names: &[&str]| {
                names
                    .iter()
                    .find_map(|k| u.get(*k).and_then(Value::as_i64))
                    .filter(|v| *v >= 0)
            };
            let input = number(&[
                "prompt_tokens",
                "input_tokens",
                "promptTokenCount",
                "total_input_tokens",
            ]);
            let cache = number(&["cache_read_input_tokens"]).unwrap_or(0)
                + number(&["cache_creation_input_tokens"]).unwrap_or(0);
            if let Some(input) = input {
                self.input = Some(self.input.unwrap_or(0).max(input.saturating_add(cache)));
            }
            let output = number(&[
                "completion_tokens",
                "output_tokens",
                "candidatesTokenCount",
                "total_output_tokens",
            ]);
            if let Some(mut output) = output {
                if u.get("candidatesTokenCount").is_some() {
                    output = output.saturating_add(number(&["thoughtsTokenCount"]).unwrap_or(0));
                }
                self.output = Some(self.output.unwrap_or(0).max(output));
            }
            if let (Some(total), Some(input)) =
                (number(&["total_tokens", "totalTokenCount"]), self.input)
            {
                self.output = Some(self.output.unwrap_or(0).max(total.saturating_sub(input)));
            }
        }
        for key in ["message", "response"] {
            if let Some(nested) = v.get(key) {
                self.observe(nested);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn final_upstream_budget_overrides_model_expansion_and_blocks_retries() {
        let budget = Arc::new(Budget {
            input: 5000,
            output: 2048,
            sent: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
        });
        ACTIVE_BUDGET.scope(budget,async {
            let mut body=json!({"request":{"contents":[{"parts":[{"text":"hello"}]}],
                "generationConfig":{"maxOutputTokens":65536,"thinkingConfig":{"thinkingBudget":32000}}}});
            constrain_upstream("streamGenerateContent",&mut body).unwrap();
            assert_eq!(body["request"]["generationConfig"]["maxOutputTokens"],2048);
            assert_eq!(body["request"]["generationConfig"]["thinkingConfig"]["thinkingBudget"],2047);
            assert!(constrain_upstream("streamGenerateContent",&mut body).is_err());
        }).await;
    }
    #[tokio::test]
    async fn spawned_streams_keep_budget_and_expanded_context_is_rejected() {
        let budget = Arc::new(Budget {
            input: 20,
            output: 1000,
            sent: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
        });
        ACTIVE_BUDGET.scope(budget,async {
            tokio::spawn(scope_current(async {
                let mut body=json!({"request":{"contents":[{"parts":[{"text":"a lot of injected context"}]}]}});
                assert!(constrain_upstream("generateContent",&mut body).is_err());
                assert!(constrain_upstream("loadCodeAssist",&mut body).is_err());
            })).await.unwrap();
        }).await;
    }
    #[test]
    fn usage_is_cumulative_and_cache_and_reasoning_are_not_counted_twice() {
        let mut usage = Usage::default();
        usage.observe(&json!({"type":"message_start","message":{"usage":{"input_tokens":100,"cache_read_input_tokens":50,"output_tokens":0}}}));
        usage.observe(&json!({"type":"message_delta","usage":{"output_tokens":20}}));
        usage.observe(&json!({"type":"message_delta","usage":{"output_tokens":30}}));
        usage.observe(&json!({"type":"message_stop"}));
        assert_eq!(usage.input, Some(150));
        assert_eq!(usage.output, Some(30));
        assert!(usage.terminal);
        let mut gemini = Usage::default();
        gemini.observe(&json!({"response":{"usageMetadata":{"promptTokenCount":200,"cachedContentTokenCount":50,
            "candidatesTokenCount":30,"thoughtsTokenCount":10,"totalTokenCount":240}}}));
        assert_eq!(gemini.input, Some(200));
        assert_eq!(gemini.output, Some(40));
        gemini.observe(&json!({"error":{"message":"cancelled"}}));
        assert!(gemini.failed);
    }
}
