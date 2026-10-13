//! Evidence from the raw upstream stream, before adapters can synthesize an ending.
#[derive(Default)]
pub struct StreamCompletion {
    pending: Vec<u8>,
    terminal: bool,
    failed: bool,
}

impl StreamCompletion {
    pub fn observe(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            self.line(&line);
        }
    }

    fn line(&mut self, bytes: &[u8]) {
        let Ok(line) = std::str::from_utf8(bytes) else {
            self.failed = true;
            return;
        };
        let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else {
            return;
        };
        if data == "[DONE]" {
            self.terminal = true;
            return;
        }
        let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
            self.failed = true;
            return;
        };
        let inner = json.get("response").unwrap_or(&json);
        if json.get("error").is_some_and(|e| !e.is_null())
            || inner.get("error").is_some_and(|e| !e.is_null())
        {
            self.failed = true;
        }
        if inner
            .get("candidates")
            .and_then(|v| v.as_array())
            .is_some_and(|candidates| {
                candidates.iter().any(|candidate| {
                    candidate
                        .get("finishReason")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| !s.is_empty())
                })
            })
        {
            self.terminal = true;
        }
    }

    pub fn finish(&mut self) -> bool {
        let tail = std::mem::take(&mut self.pending);
        if !tail.is_empty() {
            self.line(&tail);
        }
        self.terminal && !self.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_endings_must_be_real_and_error_free() {
        for data in ["data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"partial\"}]}}]}\n\n", "data:{\"error\":{\"message\":\"failure\"}}\n\ndata:[DONE]\n\n", "data:{\"response\":{\"error\":\"failure\"}}\n\ndata:[DONE]\n\n", "data:{\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\ndata:{\"error\":\"late failure\"}\n\n"] {
            let mut evidence = StreamCompletion::default();
            for chunk in data.as_bytes().chunks(3) { evidence.observe(chunk); }
            assert!(!evidence.finish());
        }
        for data in [
            "data:{\"response\":{\"candidates\":[{\"finishReason\":\"STOP\"}]}}\r\n\r\n",
            "data:{\"error\":null,\"candidates\":[{\"finishReason\":\"MAX_TOKENS\"}]}\n\n",
            "data: [DONE]\n\n",
        ] {
            let mut evidence = StreamCompletion::default();
            for chunk in data.as_bytes().chunks(1) {
                evidence.observe(chunk);
            }
            assert!(evidence.finish());
        }
    }
}
