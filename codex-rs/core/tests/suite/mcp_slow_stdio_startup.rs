//! Covers a non-required stdio MCP server whose `initialize` answer arrives late
//! (TheDancingDeveloper-org/codex#26).

use std::time::Duration;

use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::stdio_server_bin;
use core_test_support::test_codex::test_codex;
use serde_json::json;

const SERVER_NAME: &str = "slow";
const TOOL_NAMESPACE: &str = "mcp__slow";
const FLAT_TOOL_NAME: &str = "mcp__slow__echo";
const CALL_ID: &str = "call-slow-echo";

async fn run_first_turn_with_slow_server(
    startup_grace: Duration,
    initialize_delay: Duration,
) -> anyhow::Result<(serde_json::Value, String)> {
    let responses_server = responses::start_mock_server().await;
    let command = stdio_server_bin()?;
    let mock = responses::mount_sse_sequence(
        &responses_server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("resp-1"),
                // The model names the tool in its flattened form, without a namespace.
                responses::ev_function_call(
                    CALL_ID,
                    FLAT_TOOL_NAME,
                    &json!({ "message": "ping" }).to_string(),
                ),
                responses::ev_completed("resp-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("resp-2"),
                responses::ev_assistant_message("msg-2", "done"),
                responses::ev_completed("resp-2"),
            ]),
        ],
    )
    .await;

    let server = serde_json::from_value(json!({
        "command": command,
        "env": { "MCP_TEST_INITIALIZE_DELAY_MS": initialize_delay.as_millis().to_string() },
        "enabled_tools": ["echo"],
        "startup_timeout_sec": 20,
    }))?;
    let fixture = test_codex()
        .with_model_info_override("gpt-5.4", |model| model.supports_search_tool = false)
        .with_config(move |config| {
            config.mcp_optional_startup_grace = startup_grace;
            let mut servers = config.mcp_servers.get().clone();
            servers.insert(SERVER_NAME.to_string(), server);
            config
                .mcp_servers
                .set(servers)
                .expect("slow MCP server should satisfy test configuration");
        })
        .build(&responses_server)
        .await?;

    tokio::time::timeout(
        Duration::from_secs(30),
        fixture.submit_turn("use the slow server"),
    )
    .await??;

    let requests = mock.requests();
    let first_request = requests[0].body_json();
    let output = requests[1]
        .function_call_output_text(CALL_ID)
        .expect("the tool call should produce an output");
    fixture.codex.shutdown_and_wait().await?;
    Ok((first_request, output))
}

/// `exec` disables the optional-startup grace, so the first request waits for the
/// handshake and carries the slow server's tools.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_grace_first_request_includes_slow_stdio_server_tools() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let (first_request, output) =
        run_first_turn_with_slow_server(Duration::ZERO, Duration::from_millis(1500)).await?;

    assert!(
        responses::namespace_child_tool(&first_request, TOOL_NAMESPACE, "echo").is_some(),
        "the first request must carry the slow server's tools: {first_request}"
    );
    assert!(
        output.contains("ping") && !output.contains("unsupported call"),
        "a flattened MCP tool name should reach the server: {output}"
    );
    Ok(())
}

/// With a short grace the server is still handshaking when the model calls it, and the
/// model is told to retry instead of being told the tool does not exist.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn call_to_starting_server_reports_still_starting() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let (first_request, output) =
        run_first_turn_with_slow_server(Duration::from_millis(50), Duration::from_secs(10)).await?;

    assert!(
        responses::namespace_child_tool(&first_request, TOOL_NAMESPACE, "echo").is_none(),
        "a server still handshaking after the grace must be absent: {first_request}"
    );
    assert!(
        output.contains("MCP server `slow` is still starting"),
        "unexpected output: {output}"
    );
    Ok(())
}
