//! The model client: Chat Completions request shape and streaming response
//! parsing. `ChatModel` is the seam the turn loop depends on so tests can
//! swap in a scripted fake instead of a real OpenAI-compatible endpoint.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    fn plain(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn system(c: impl Into<String>) -> Self { Self::plain("system", c) }
    pub fn user(c: impl Into<String>) -> Self { Self::plain("user", c) }
    pub fn assistant(c: impl Into<String>) -> Self { Self::plain("assistant", c) }
    pub fn assistant_tools(content: String, calls: Vec<ToolCall>) -> Self {
        Self { role: "assistant".into(), content: Some(content), tool_calls: Some(calls), tool_call_id: None }
    }
    pub fn tool(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self { role: "tool".into(), content: Some(content.into()), tool_calls: None, tool_call_id: Some(id.into()) }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelReply {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    /// Worth trying again later: rate limits, 5xx, network.
    Retryable(String),
    /// A bug or a rejected request; retrying will not help.
    Fatal(String),
}

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: Option<&Value>,
        on_text: &mut (dyn for<'r> FnMut(&'r str) + Send),
    ) -> Result<ModelReply, ModelError>;
}

/// Folds Chat Completions stream lines into one reply.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    reply: ModelReply,
}

impl StreamAccumulator {
    pub fn push_line(&mut self, line: &str, on_text: &mut (dyn for<'r> FnMut(&'r str) + Send)) -> Result<(), ModelError> {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            return Ok(()); // blank lines, comments, event: lines
        };
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|e| ModelError::Fatal(format!("unreadable stream chunk: {e}")))?;
        if let Some(u) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.reply.input_tokens = u["prompt_tokens"].as_u64().unwrap_or(0);
            self.reply.output_tokens = u["completion_tokens"].as_u64().unwrap_or(0);
        }
        let Some(delta) = chunk["choices"].get(0).map(|c| &c["delta"]) else {
            return Ok(());
        };
        if let Some(t) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            on_text(t);
            self.reply.content.push_str(t);
        }
        for tc in delta["tool_calls"].as_array().into_iter().flatten() {
            let i = tc["index"].as_u64().unwrap_or(0) as usize;
            while self.reply.tool_calls.len() <= i {
                self.reply.tool_calls.push(ToolCall {
                    id: String::new(),
                    kind: "function".into(),
                    function: FunctionCall { name: String::new(), arguments: String::new() },
                });
            }
            let call = &mut self.reply.tool_calls[i];
            if let Some(id) = tc["id"].as_str() {
                call.id = id.to_string();
            }
            if let Some(n) = tc["function"]["name"].as_str() {
                call.function.name.push_str(n);
            }
            if let Some(a) = tc["function"]["arguments"].as_str() {
                call.function.arguments.push_str(a);
            }
        }
        Ok(())
    }

    pub fn finish(self) -> ModelReply {
        self.reply
    }
}

pub fn request_body(model: &str, messages: &[ChatMessage], tools: Option<&Value>) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
        // Keep conversations out of OpenAI's stored state (spec: Retention).
        "store": false,
    });
    if let Some(t) = tools {
        body["tools"] = t.clone();
    }
    body
}

pub struct OpenAiModel {
    http: Arc<reqwest::Client>,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiModel {
    pub fn new(http: Arc<reqwest::Client>, base_url: String, api_key: String, model: String) -> Self {
        Self { http, base_url: base_url.trim_end_matches('/').to_string(), api_key, model }
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response, ModelError> {
        let res = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(body)
            .timeout(Duration::from_secs(55))
            .send()
            .await
            .map_err(|e| ModelError::Retryable(format!("OpenAI unreachable: {e}")))?;
        let status = res.status();
        if status.is_success() {
            return Ok(res);
        }
        // The body may echo our request; keep only the status in errors.
        if status.as_u16() == 429 || status.is_server_error() {
            Err(ModelError::Retryable(format!("OpenAI returned {status}")))
        } else {
            Err(ModelError::Fatal(format!("OpenAI rejected the request: {status}")))
        }
    }
}

#[async_trait]
impl ChatModel for OpenAiModel {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: Option<&Value>,
        on_text: &mut (dyn for<'r> FnMut(&'r str) + Send),
    ) -> Result<ModelReply, ModelError> {
        let body = request_body(&self.model, messages, tools);
        // One retry, and only before anything streamed to the user.
        let mut res = match self.send(&body).await {
            Err(ModelError::Retryable(_)) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                self.send(&body).await?
            }
            other => other?,
        };
        let mut acc = StreamAccumulator::default();
        // Split on '\n' bytes before decoding, so a multi-byte character that
        // straddles two network chunks is never cut in half.
        let mut pending: Vec<u8> = Vec::new();
        while let Some(bytes) = res
            .chunk()
            .await
            .map_err(|e| ModelError::Retryable(format!("stream interrupted: {e}")))?
        {
            pending.extend_from_slice(&bytes);
            while let Some(i) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=i).collect();
                let line = String::from_utf8_lossy(&line);
                acc.push_line(line.trim_end_matches(['\r', '\n']), on_text)?;
            }
        }
        if !pending.is_empty() {
            acc.push_line(String::from_utf8_lossy(&pending).trim_end(), on_text)?;
        }
        Ok(acc.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(lines: &[&str]) -> (ModelReply, String) {
        let mut acc = StreamAccumulator::default();
        let mut seen = String::new();
        for l in lines {
            acc.push_line(l, &mut |t: &str| seen.push_str(t)).unwrap();
        }
        (acc.finish(), seen)
    }

    #[test]
    fn text_deltas_stream_and_accumulate() {
        let (r, seen) = feed(&[
            r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
            "",
            r#"data: {"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3}}"#,
            "data: [DONE]",
        ]);
        assert_eq!(seen, "Hello");
        assert_eq!(r.content, "Hello");
        assert!(r.tool_calls.is_empty());
        assert_eq!((r.input_tokens, r.output_tokens), (12, 3));
    }

    #[test]
    fn tool_call_arguments_are_joined_by_index() {
        let (r, seen) = feed(&[
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"search_mail","arguments":""}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":"}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"contract\"}"}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_2","type":"function","function":{"name":"read_message","arguments":"{\"ref\":\"m1\"}"}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert_eq!(seen, "");
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].id, "call_1");
        assert_eq!(r.tool_calls[0].function.name, "search_mail");
        assert_eq!(r.tool_calls[0].function.arguments, r#"{"query":"contract"}"#);
        assert_eq!(r.tool_calls[1].function.name, "read_message");
    }

    #[test]
    fn non_data_lines_are_ignored_and_bad_json_is_fatal() {
        let mut acc = StreamAccumulator::default();
        acc.push_line(": keep-alive", &mut |_: &str| {}).unwrap();
        acc.push_line("event: ping", &mut |_: &str| {}).unwrap();
        assert!(matches!(acc.push_line("data: {not json", &mut |_: &str| {}), Err(ModelError::Fatal(_))));
    }

    #[test]
    fn request_never_stores_and_never_sets_temperature() {
        let body = request_body("gpt-5.6-terra", &[ChatMessage::user("hi")], None);
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert!(body.get("temperature").is_none());
        assert!(body.get("tools").is_none());
        let with_tools = request_body("m", &[], Some(&serde_json::json!([{"type": "function"}])));
        assert!(with_tools["tools"].is_array());
    }

    #[test]
    fn tool_messages_serialize_in_chat_completions_shape() {
        let call = ToolCall { id: "c1".into(), kind: "function".into(), function: FunctionCall { name: "n".into(), arguments: "{}".into() } };
        let v = serde_json::to_value(ChatMessage::assistant_tools(String::new(), vec![call])).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["tool_calls"][0]["type"], "function");
        let t = serde_json::to_value(ChatMessage::tool("c1", "{}")).unwrap();
        assert_eq!(t["tool_call_id"], "c1");
        assert!(serde_json::to_value(ChatMessage::user("x")).unwrap().get("tool_calls").is_none());
    }
}
