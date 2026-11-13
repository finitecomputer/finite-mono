use serde_json::Value;

/// Raw marketplace token counts only; Finite subscription weights never apply.
pub(super) struct Usage {
    prompt: u64,
    completion: u64,
    total: u64,
}

impl Usage {
    pub(super) fn from_value(value: &Value, model: &str, output_limit: u64) -> Option<Self> {
        if value.get("model")?.as_str()? != model {
            return None;
        }
        let usage = value.get("usage")?;
        let prompt = usage.get("prompt_tokens")?.as_u64()?;
        let completion = usage.get("completion_tokens")?.as_u64()?;
        let total = usage.get("total_tokens")?.as_u64()?;
        // Match Numes's exact whole-number contract, including JS safe integers.
        if prompt.checked_add(completion)? != total
            || total > 9_007_199_254_740_991
            || completion > output_limit
        {
            return None;
        }
        Some(Self {
            prompt,
            completion,
            total,
        })
    }

    pub(super) fn format(&self, model: &str) -> String {
        format!(
            "prompt={},completion={},total={},model={model}",
            self.prompt, self.completion, self.total
        )
    }
}

pub(super) struct StreamUsage {
    pending: Vec<u8>,
    model: String,
    output_limit: u64,
    usage: Option<Usage>,
    done: bool,
    invalid: bool,
}

impl StreamUsage {
    pub(super) fn new(model: &str, output_limit: u64) -> Self {
        Self {
            pending: Vec::new(),
            model: model.into(),
            output_limit,
            usage: None,
            done: false,
            invalid: false,
        }
    }

    pub(super) fn push(&mut self, chunk: &[u8]) {
        // Byte buffering preserves UTF-8 across transport boundaries. Bound each
        // SSE line without ever retaining the complete transcript.
        for byte in chunk {
            if *byte == b'\n' {
                let line = std::mem::take(&mut self.pending);
                self.line(&line);
            } else if self.pending.len() < 1024 * 1024 {
                self.pending.push(*byte);
            } else {
                self.invalid = true;
            }
        }
    }

    fn line(&mut self, line: &[u8]) {
        let Ok(line) = std::str::from_utf8(line) else {
            self.invalid = true;
            return;
        };
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim();
        if self.done {
            self.invalid = true;
            return;
        }
        if data == "[DONE]" {
            self.done = true;
            return;
        }
        // include_usage requests one final usage event. Any subsequent data
        // other than [DONE] makes these counts nonfinal or ambiguous.
        if self.usage.is_some() {
            self.invalid = true;
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            self.invalid = true;
            return;
        };
        if value.get("model").and_then(Value::as_str) != Some(&self.model) {
            self.invalid = true;
        }
        if value.get("usage").is_some_and(|v| !v.is_null()) {
            match Usage::from_value(&value, &self.model, self.output_limit) {
                Some(usage) => self.usage = Some(usage),
                None => self.invalid = true,
            }
        }
    }

    pub(super) fn finish(self) -> Option<Usage> {
        if self.done && !self.invalid && self.pending.is_empty() {
            self.usage
        } else {
            None
        }
    }
}
