use super::*;
use codex_client::TransportError;
use futures::TryStreamExt;
use pretty_assertions::assert_eq;
use tokio_test::io::Builder as IoBuilder;
use tokio_util::io::ReaderStream;

async fn collect(sse: &str) -> Vec<Result<ResponseEvent, ApiError>> {
    let reader = IoBuilder::new().read(sse.as_bytes()).build();
    let stream = ReaderStream::new(reader).map_err(|err| TransportError::Network(err.to_string()));
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(process_chat_sse(
        Box::pin(stream),
        tx,
        Duration::from_secs(1),
        /*telemetry*/ None,
    ));
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

fn done_items(events: &[Result<ResponseEvent, ApiError>]) -> Vec<ResponseItem> {
    events
        .iter()
        .filter_map(|event| match event {
            Ok(ResponseEvent::OutputItemDone(item)) => Some(item.clone()),
            _ => None,
        })
        .collect()
}

fn call(call_id: &str, name: &str, arguments: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: name.to_string(),
        namespace: None,
        arguments: arguments.to_string(),
        encrypted_function_args: None,
        call_id: call_id.to_string(),
        internal_chat_message_metadata_passthrough: None,
    }
}

fn assert_completed(events: &[Result<ResponseEvent, ApiError>]) {
    assert!(matches!(
        events.last(),
        Some(Ok(ResponseEvent::Completed { .. }))
    ));
}

#[tokio::test]
async fn text_stream_emits_deltas_message_and_usage() {
    let events = collect(concat!(
        "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"id\":\"c1\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n",
        "data: [DONE]\n\n",
    ))
    .await;
    let deltas: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Ok(ResponseEvent::OutputTextDelta(delta)) => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, vec!["Hel", "lo"]);
    assert_eq!(
        done_items(&events),
        vec![assistant_message("Hello".to_string())]
    );
    match events.last() {
        Some(Ok(ResponseEvent::Completed {
            response_id,
            token_usage: Some(usage),
            ..
        })) => {
            assert_eq!(response_id, "c1");
            assert_eq!(
                (usage.input_tokens, usage.output_tokens, usage.total_tokens),
                (10, 2, 12)
            );
        }
        other => panic!("expected Completed with usage, got {other:?}"),
    }
}

#[tokio::test]
async fn single_tool_call_arguments_are_concatenated() {
    let events = collect(concat!(
        "data: {\"id\":\"c2\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"id\":\"c2\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"command\\\":\"}}]}}]}\n\n",
        "data: {\"id\":\"c2\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"[\\\"ls\\\"]}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    ))
    .await;
    assert_eq!(
        done_items(&events),
        vec![call("call_a", "shell", "{\"command\":[\"ls\"]}")]
    );
    assert_completed(&events);
}

#[tokio::test]
async fn parallel_tool_calls_are_split_by_index() {
    let events = collect(concat!(
        "data: {\"id\":\"c3\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":[\\\"pwd\\\"]}\"}},{\"index\":1,\"id\":\"call_2\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\"\"}}]}}]}\n\n",
        "data: {\"id\":\"c3\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\":[\\\"date\\\"]}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    ))
    .await;
    assert_eq!(
        done_items(&events),
        vec![
            call("call_1", "shell", "{\"command\":[\"pwd\"]}"),
            call("call_2", "shell", "{\"command\":[\"date\"]}"),
        ]
    );
}

#[tokio::test]
async fn apply_patch_and_mcp_calls_keep_flattened_names() {
    // The caller (core ChatToolMap) restores these to CustomToolCall and a
    // namespaced FunctionCall; the parser passes the chat names through.
    let events = collect(concat!(
        "data: {\"id\":\"c4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Editing.\",\"tool_calls\":[{\"index\":0,\"id\":\"call_p\",\"function\":{\"name\":\"apply_patch\",\"arguments\":\"{\\\"input\\\":\\\"*** Begin Patch\\\\n*** End Patch\\\"}\"}}]}}]}\n\n",
        "data: {\"id\":\"c4\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_m\",\"function\":{\"name\":\"mcp__vogt__work_get\",\"arguments\":\"{\\\"ref\\\":\\\"WI-1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
    ))
    .await;
    assert_eq!(
        done_items(&events),
        vec![
            assistant_message("Editing.".to_string()),
            call(
                "call_p",
                "apply_patch",
                "{\"input\":\"*** Begin Patch\\n*** End Patch\"}"
            ),
            call("call_m", "mcp__vogt__work_get", "{\"ref\":\"WI-1\"}"),
        ]
    );
    // No [DONE]: finish_reason alone completes the stream.
    assert_completed(&events);
}

#[tokio::test]
async fn error_chunk_and_truncated_stream_fail() {
    let events = collect("data: {\"error\":{\"message\":\"rate limited\"}}\n\n").await;
    assert!(matches!(events.last(), Some(Err(ApiError::Stream(m))) if m == "rate limited"));

    let events = collect(
        "data: {\"id\":\"c5\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"}}]}\n\n",
    )
    .await;
    assert!(matches!(events.last(), Some(Err(ApiError::Stream(_)))));
}
