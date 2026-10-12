//! Chat Completions SSE parser for `wire_api = "chat"` (fork-only, WI-1286).
//!
//! Turns `chat.completion.chunk` events into the same [`ResponseEvent`]s the
//! Responses parser emits: text deltas, one assistant message, one
//! `FunctionCall` item per tool call (parallel calls arrive interleaved by
//! `index`), and a final `Completed` carrying usage. Tool names come back as
//! the flattened chat names; the caller restores namespaces and freeform
//! tools.

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

pub fn spawn_chat_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let upstream_request_id = stream_response
        .headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(process_chat_sse(
        stream_response.bytes,
        tx_event,
        idle_timeout,
        telemetry,
    ));
    ResponseStream {
        rx_event,
        upstream_request_id,
        interrupt: None,
    }
}

#[derive(Default)]
struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct ChatAccumulator {
    response_id: Option<String>,
    created_sent: bool,
    text: String,
    message_started: bool,
    tool_calls: BTreeMap<u64, ToolCallState>,
    usage: Option<TokenUsage>,
    finished: bool,
}

impl ChatAccumulator {
    /// Apply one chunk, returning the events it produces right away.
    fn apply(&mut self, chunk: &Value) -> Result<Vec<ResponseEvent>, ApiError> {
        if let Some(error) = chunk.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            return Err(ApiError::Stream(message));
        }
        let mut events = Vec::new();
        if self.response_id.is_none()
            && let Some(id) = chunk.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            self.response_id = Some(id.to_string());
        }
        if !self.created_sent {
            self.created_sent = true;
            events.push(ResponseEvent::Created {
                response_id: self.response_id.clone(),
            });
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(token_usage(usage));
        }
        let choices = chunk.get("choices").and_then(Value::as_array);
        for choice in choices.into_iter().flatten() {
            let delta = choice.get("delta");
            if let Some(text) = delta
                .and_then(|delta| delta.get("content"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                if !self.message_started {
                    self.message_started = true;
                    events.push(ResponseEvent::OutputItemAdded(assistant_message(
                        String::new(),
                    )));
                }
                self.text.push_str(text);
                events.push(ResponseEvent::OutputTextDelta(text.to_string()));
            }
            let tool_calls = delta
                .and_then(|delta| delta.get("tool_calls"))
                .and_then(Value::as_array);
            for (position, call) in tool_calls.into_iter().flatten().enumerate() {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .unwrap_or(position as u64);
                let state = self.tool_calls.entry(index).or_default();
                if let Some(id) = call.get("id").and_then(Value::as_str)
                    && !id.is_empty()
                {
                    state.id = id.to_string();
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str) {
                        state.name.push_str(name);
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        state.arguments.push_str(arguments);
                    }
                }
            }
            if choice
                .get("finish_reason")
                .is_some_and(|reason| !reason.is_null())
            {
                self.finished = true;
            }
        }
        Ok(events)
    }

    /// Emit the completed items and the terminal `Completed` event.
    fn finish(&mut self) -> Vec<ResponseEvent> {
        let mut events = Vec::new();
        if !self.created_sent {
            events.push(ResponseEvent::Created { response_id: None });
        }
        if self.message_started {
            events.push(ResponseEvent::OutputItemDone(assistant_message(
                std::mem::take(&mut self.text),
            )));
        }
        let response_id = self.response_id.clone().unwrap_or_default();
        for (index, call) in std::mem::take(&mut self.tool_calls) {
            if call.name.is_empty() {
                continue;
            }
            let call_id = if call.id.is_empty() {
                format!("call_{response_id}_{index}")
            } else {
                call.id
            };
            let arguments = if call.arguments.is_empty() {
                "{}".to_string()
            } else {
                call.arguments
            };
            events.push(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                id: None,
                name: call.name,
                namespace: None,
                arguments,
                encrypted_function_args: None,
                call_id,
                internal_chat_message_metadata_passthrough: None,
            }));
        }
        events.push(ResponseEvent::Completed {
            response_id,
            token_usage: self.usage.take(),
            usage_metadata: None,
            end_turn: None,
        });
        events
    }
}

fn assistant_message(text: String) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText { text }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn token_usage(usage: &Value) -> TokenUsage {
    let int = |value: Option<&Value>| value.and_then(Value::as_i64).unwrap_or_default();
    let input_tokens = int(usage.get("prompt_tokens"));
    let output_tokens = int(usage.get("completion_tokens"));
    TokenUsage {
        input_tokens,
        cached_input_tokens: int(usage.pointer("/prompt_tokens_details/cached_tokens")),
        output_tokens,
        reasoning_output_tokens: int(usage.pointer("/completion_tokens_details/reasoning_tokens")),
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(input_tokens + output_tokens),
        ..TokenUsage::default()
    }
}

pub async fn process_chat_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut state = ChatAccumulator::default();
    loop {
        let start = Instant::now();
        let response = tokio::select! {
            biased;
            _ = tx_event.closed() => return,
            response = timeout(idle_timeout, stream.next()) => response,
        };
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("chat SSE error: {e:#}");
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            // Some providers close the stream after the last chunk without
            // sending `[DONE]`; a finish_reason is enough to call it complete.
            Ok(None) if state.finished => break,
            Ok(None) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "chat stream closed before finish_reason".into(),
                    )))
                    .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };
        trace!("chat SSE event: {}", &sse.data);
        let data = sse.data.trim();
        if data == "[DONE]" {
            break;
        }
        if data.is_empty() {
            continue;
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(chunk) => chunk,
            Err(e) => {
                debug!("failed to parse chat SSE chunk: {e}");
                continue;
            }
        };
        match state.apply(&chunk) {
            Ok(events) => {
                for event in events {
                    if tx_event.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = tx_event.send(Err(error)).await;
                return;
            }
        }
    }
    for event in state.finish() {
        if tx_event.send(Ok(event)).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
