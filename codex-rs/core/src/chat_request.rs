//! Chat Completions request body for `wire_api = "chat"` (fork-only, WI-1286).
//!
//! This is the request half of the Chat Completions path. The HTTP client and
//! SSE parser live in `codex-api`; the conversion lives here because `Prompt`
//! and `ResponseItem` do.
//!
//! Chat Completions only knows JSON-schema function tools, so namespaced tools
//! are flattened to `namespace__name` and freeform tools (apply_patch) become a
//! function taking one string argument, `input`. [`ChatToolMap`] records that
//! flattening so tool calls coming back can be restored to the items the rest
//! of Codex expects.

use crate::client_common::Prompt;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_tools::FreeformTool;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;

/// Argument name carrying a freeform tool's raw input over Chat Completions.
const FREEFORM_INPUT_ARG: &str = "input";

/// Flatten a namespaced tool to a single Chat Completions function name.
/// Chat Completions has no namespace, so the namespace is part of the name.
pub(crate) fn chat_tool_name(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(namespace) if !namespace.is_empty() => format!("{namespace}__{name}"),
        _ => name.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ChatTool {
    name: String,
    namespace: Option<String>,
    freeform: bool,
}

/// Maps the flattened Chat Completions function names of one request back to
/// the Codex tools they stand for.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ChatToolMap {
    tools: HashMap<String, ChatTool>,
}

impl ChatToolMap {
    fn insert(&mut self, namespace: Option<&str>, name: &str, freeform: bool) -> String {
        let chat_name = chat_tool_name(namespace, name);
        self.tools.insert(
            chat_name.clone(),
            ChatTool {
                name: name.to_string(),
                namespace: namespace.map(str::to_string),
                freeform,
            },
        );
        chat_name
    }

    /// Restore a function call parsed from a chat stream to the item Codex
    /// would have received over the Responses API: the original name and
    /// namespace, and a `CustomToolCall` for freeform tools.
    pub(crate) fn restore(&self, item: ResponseItem) -> ResponseItem {
        let ResponseItem::FunctionCall {
            id,
            name,
            arguments,
            call_id,
            ..
        } = &item
        else {
            return item;
        };
        let Some(tool) = self.tools.get(name) else {
            return item;
        };
        if tool.freeform {
            return ResponseItem::CustomToolCall {
                id: id.clone(),
                status: None,
                call_id: call_id.clone(),
                name: tool.name.clone(),
                namespace: tool.namespace.clone(),
                input: freeform_input(arguments),
                internal_chat_message_metadata_passthrough: None,
            };
        }
        ResponseItem::FunctionCall {
            id: id.clone(),
            name: tool.name.clone(),
            namespace: tool.namespace.clone(),
            arguments: arguments.clone(),
            encrypted_function_args: None,
            call_id: call_id.clone(),
            internal_chat_message_metadata_passthrough: None,
        }
    }
}

/// A model may answer a single-string function with the bare string rather
/// than `{"input": ...}`; accept both.
fn freeform_input(arguments: &str) -> String {
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(map)) => match map.get(FREEFORM_INPUT_ARG) {
            Some(Value::String(input)) => input.clone(),
            _ => arguments.to_string(),
        },
        Ok(Value::String(input)) => input,
        _ => arguments.to_string(),
    }
}

/// Build the JSON body posted to `/chat/completions`, and the map that turns
/// the tool calls in its response back into Codex tool calls.
///
/// Responses-only items (`Reasoning`, `WebSearch`, tool search, …) are
/// dropped rather than failing the turn: a chat provider cannot represent them.
pub(crate) fn build_chat_request(model: &str, prompt: &Prompt) -> (Value, ChatToolMap) {
    let mut messages = Vec::new();
    if !prompt.base_instructions.text.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": prompt.base_instructions.text,
        }));
    }
    for item in &prompt.input {
        push_item(&mut messages, item);
    }

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    let mut tool_map = ChatToolMap::default();
    let tools = chat_tools(prompt.tools.as_ref(), &mut tool_map);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["parallel_tool_calls"] = Value::Bool(prompt.parallel_tool_calls);
    }
    (body, tool_map)
}

fn push_item(messages: &mut Vec<Value>, item: &ResponseItem) {
    match item {
        ResponseItem::Message { role, content, .. } => {
            let content = message_content(content);
            if content.is_empty() {
                return;
            }
            // Chat Completions has no developer role on most providers.
            let role = if role == "developer" { "system" } else { role };
            messages.push(json!({ "role": role, "content": content }));
        }
        ResponseItem::FunctionCall {
            name,
            namespace,
            arguments,
            call_id,
            ..
        } => push_tool_call(
            messages,
            call_id,
            chat_tool_name(namespace.as_deref(), name),
            arguments.clone(),
        ),
        ResponseItem::CustomToolCall {
            name,
            namespace,
            input,
            call_id,
            ..
        } => push_tool_call(
            messages,
            call_id,
            chat_tool_name(namespace.as_deref(), name),
            json!({ FREEFORM_INPUT_ARG: input }).to_string(),
        ),
        ResponseItem::FunctionCallOutput {
            call_id, output, ..
        } => {
            if let Some(call_id) = call_id.as_deref() {
                push_tool_output(messages, call_id, output);
            }
        }
        ResponseItem::CustomToolCallOutput {
            call_id, output, ..
        } => push_tool_output(messages, call_id, output),
        ResponseItem::AdditionalTools { .. }
        | ResponseItem::AgentMessage { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ConfigurationUpdate { .. }
        | ResponseItem::CompactionTrigger { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => {}
    }
}

/// Consecutive tool calls become one assistant message with several
/// `tool_calls`, which is how Chat Completions represents a parallel call.
/// A call straight after assistant text joins that message too.
fn push_tool_call(messages: &mut Vec<Value>, call_id: &str, name: String, arguments: String) {
    let call = json!({
        "id": call_id,
        "type": "function",
        "function": { "name": name, "arguments": arguments },
    });
    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("assistant")
    {
        match last.get_mut("tool_calls").and_then(Value::as_array_mut) {
            Some(calls) => calls.push(call),
            None => last["tool_calls"] = Value::Array(vec![call]),
        }
        return;
    }
    messages.push(json!({
        "role": "assistant",
        "content": Value::Null,
        "tool_calls": [call],
    }));
}

fn push_tool_output(messages: &mut Vec<Value>, call_id: &str, output: &FunctionCallOutputPayload) {
    if call_id.is_empty() {
        return;
    }
    messages.push(json!({
        "role": "tool",
        "tool_call_id": call_id,
        "content": tool_output_text(output),
    }));
}

fn message_content(content: &[ContentItem]) -> String {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                Some(text.as_str())
            }
            ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_output_text(output: &FunctionCallOutputPayload) -> String {
    output
        .text_content()
        .map(str::to_string)
        .or_else(|| output.body.to_text())
        .unwrap_or_default()
}

fn chat_tools(tools: &[ToolSpec], map: &mut ChatToolMap) -> Vec<Value> {
    let mut out = Vec::new();
    for tool in tools {
        match tool {
            ToolSpec::Function(function) => out.push(function_tool(None, function, map)),
            ToolSpec::Freeform(freeform) => out.push(freeform_tool(None, freeform, map)),
            ToolSpec::Namespace(namespace) => {
                let ns = Some(namespace.name.as_str());
                for tool in &namespace.tools {
                    out.push(match tool {
                        ResponsesApiNamespaceTool::Function(function) => {
                            function_tool(ns, function, map)
                        }
                        ResponsesApiNamespaceTool::Custom(freeform) => {
                            freeform_tool(ns, freeform, map)
                        }
                    });
                }
            }
            ToolSpec::ToolSearch { .. } | ToolSpec::WebSearch { .. } => {}
        }
    }
    out
}

fn function_tool(
    namespace: Option<&str>,
    function: &ResponsesApiTool,
    map: &mut ChatToolMap,
) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": map.insert(namespace, &function.name, /*freeform*/ false),
            "description": function.description,
            "parameters": function.parameters,
        },
    })
}

fn freeform_tool(namespace: Option<&str>, tool: &FreeformTool, map: &mut ChatToolMap) -> Value {
    let input_description = format!(
        "The raw tool input, written in this {} grammar:\n{}",
        tool.format.syntax, tool.format.definition
    );
    json!({
        "type": "function",
        "function": {
            "name": map.insert(namespace, &tool.name, /*freeform*/ true),
            "description": tool.description,
            "parameters": {
                "type": "object",
                "properties": {
                    FREEFORM_INPUT_ARG: { "type": "string", "description": input_description },
                },
                "required": [FREEFORM_INPUT_ARG],
                "additionalProperties": false,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::build_chat_request;
    use crate::client_common::Prompt;
    use codex_protocol::models::BaseInstructions;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::ResponseItem;
    use codex_tools::FreeformTool;
    use codex_tools::FreeformToolFormat;
    use codex_tools::JsonSchema;
    use codex_tools::ResponsesApiNamespace;
    use codex_tools::ResponsesApiNamespaceTool;
    use codex_tools::ResponsesApiTool;
    use codex_tools::ToolSpec;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;

    fn function(name: &str) -> ResponsesApiTool {
        ResponsesApiTool {
            name: name.to_string(),
            description: format!("{name} description"),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::default(),
            output_schema: None,
        }
    }

    #[test]
    fn chat_request_converts_messages_tools_and_parallel_calls() {
        let prompt = Prompt {
            base_instructions: BaseInstructions {
                text: "be brief".to_string(),
                provenance: None,
            },
            input: vec![
                ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "list files".to_string(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "shell".to_string(),
                    namespace: None,
                    arguments: "{\"command\":\"ls\"}".to_string(),
                    encrypted_function_args: None,
                    call_id: "call-1".to_string(),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCall {
                    id: None,
                    name: "work_get".to_string(),
                    namespace: Some("vogt".to_string()),
                    arguments: "{\"ref\":\"WI-1\"}".to_string(),
                    encrypted_function_args: None,
                    call_id: "call-2".to_string(),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::FunctionCallOutput {
                    id: None,
                    call_id: Some("call-1".to_string()),
                    name: Some("shell".to_string()),
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text("a.rs".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::Reasoning {
                    id: None,
                    summary: Vec::new(),
                    content: None,
                    encrypted_content: None,
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            tools: Arc::from(vec![
                ToolSpec::Function(function("shell")),
                ToolSpec::Namespace(ResponsesApiNamespace {
                    name: "vogt".to_string(),
                    description: "vogt".to_string(),
                    tools: vec![ResponsesApiNamespaceTool::Function(function("work_get"))],
                }),
            ]),
            parallel_tool_calls: true,
            output_schema: None,
            output_schema_strict: true,
            cyber_access_program: None,
        };

        assert_eq!(
            build_chat_request("grok", &prompt).0,
            json!({
                "model": "grok",
                "stream": true,
                "stream_options": { "include_usage": true },
                "parallel_tool_calls": true,
                "messages": [
                    { "role": "system", "content": "be brief" },
                    { "role": "user", "content": "list files" },
                    {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call-1",
                                "type": "function",
                                "function": { "name": "shell", "arguments": "{\"command\":\"ls\"}" }
                            },
                            {
                                "id": "call-2",
                                "type": "function",
                                "function": { "name": "vogt__work_get", "arguments": "{\"ref\":\"WI-1\"}" }
                            }
                        ]
                    },
                    { "role": "tool", "tool_call_id": "call-1", "content": "a.rs" }
                ],
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": "shell",
                            "description": "shell description",
                            "parameters": {}
                        }
                    },
                    {
                        "type": "function",
                        "function": {
                            "name": "vogt__work_get",
                            "description": "work_get description",
                            "parameters": {}
                        }
                    }
                ]
            })
        );
    }

    fn prompt_with(input: Vec<ResponseItem>, tools: Vec<ToolSpec>) -> Prompt {
        Prompt {
            base_instructions: BaseInstructions {
                text: String::new(),
                provenance: None,
            },
            input,
            tools: Arc::from(tools),
            parallel_tool_calls: false,
            output_schema: None,
            output_schema_strict: true,
            cyber_access_program: None,
        }
    }

    fn apply_patch_tool() -> ToolSpec {
        ToolSpec::Freeform(FreeformTool {
            name: "apply_patch".to_string(),
            description: "edit files".to_string(),
            defer_loading: None,
            format: FreeformToolFormat {
                r#type: "grammar".to_string(),
                syntax: "lark".to_string(),
                definition: "start: patch".to_string(),
            },
        })
    }

    #[test]
    fn freeform_tool_round_trips_as_single_string_function() {
        let prompt = prompt_with(
            vec![
                ResponseItem::CustomToolCall {
                    id: None,
                    status: None,
                    call_id: "call-p".to_string(),
                    name: "apply_patch".to_string(),
                    namespace: None,
                    input: "*** Begin Patch".to_string(),
                    internal_chat_message_metadata_passthrough: None,
                },
                ResponseItem::CustomToolCallOutput {
                    id: None,
                    call_id: "call-p".to_string(),
                    name: None,
                    output: FunctionCallOutputPayload::from_text("Done".to_string()),
                    internal_chat_message_metadata_passthrough: None,
                },
            ],
            vec![apply_patch_tool()],
        );
        let (body, map) = build_chat_request("grok", &prompt);
        assert_eq!(
            body["messages"],
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-p",
                        "type": "function",
                        "function": { "name": "apply_patch", "arguments": "{\"input\":\"*** Begin Patch\"}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "call-p", "content": "Done" }
            ])
        );
        assert_eq!(
            body["tools"][0]["function"]["parameters"]["required"],
            json!(["input"])
        );

        let call = |arguments: &str| ResponseItem::FunctionCall {
            id: None,
            name: "apply_patch".to_string(),
            namespace: None,
            arguments: arguments.to_string(),
            encrypted_function_args: None,
            call_id: "call-q".to_string(),
            internal_chat_message_metadata_passthrough: None,
        };
        let expected = ResponseItem::CustomToolCall {
            id: None,
            status: None,
            call_id: "call-q".to_string(),
            name: "apply_patch".to_string(),
            namespace: None,
            input: "*** Begin Patch".to_string(),
            internal_chat_message_metadata_passthrough: None,
        };
        assert_eq!(
            map.restore(call("{\"input\":\"*** Begin Patch\"}")),
            expected
        );
        assert_eq!(map.restore(call("\"*** Begin Patch\"")), expected);
    }

    #[test]
    fn namespaced_mcp_tool_call_is_restored() {
        let prompt = prompt_with(
            Vec::new(),
            vec![ToolSpec::Namespace(ResponsesApiNamespace {
                name: "mcp__vogt".to_string(),
                description: "vogt".to_string(),
                tools: vec![ResponsesApiNamespaceTool::Function(function("work_get"))],
            })],
        );
        let (body, map) = build_chat_request("grok", &prompt);
        assert_eq!(body["tools"][0]["function"]["name"], "mcp__vogt__work_get");
        let restored = map.restore(ResponseItem::FunctionCall {
            id: None,
            name: "mcp__vogt__work_get".to_string(),
            namespace: None,
            arguments: "{}".to_string(),
            encrypted_function_args: None,
            call_id: "call-m".to_string(),
            internal_chat_message_metadata_passthrough: None,
        });
        assert_eq!(
            restored,
            ResponseItem::FunctionCall {
                id: None,
                name: "work_get".to_string(),
                namespace: Some("mcp__vogt".to_string()),
                arguments: "{}".to_string(),
                encrypted_function_args: None,
                call_id: "call-m".to_string(),
                internal_chat_message_metadata_passthrough: None,
            }
        );
    }
}
