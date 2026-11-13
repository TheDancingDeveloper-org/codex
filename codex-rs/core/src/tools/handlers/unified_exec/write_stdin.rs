use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolEnvironments;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::hook_names::HookToolName;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::LiveEnvironmentResolver;
use crate::tools::registry::PostToolUsePayload;
use crate::tools::registry::PreToolUsePayload;
use crate::tools::registry::ToolExecutor;
use crate::tools::sandboxing::ToolError;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecError;
use crate::unified_exec::WriteStdinInteractionEvent;
use crate::unified_exec::WriteStdinRequest;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;

use super::super::shell_spec::create_write_stdin_tool;
use super::post_unified_exec_tool_use_payload;

#[derive(Debug, Deserialize)]
struct WriteStdinArgs {
    #[serde(default)]
    chars: String,
    #[serde(default = "super::default_write_stdin_yield_time_ms")]
    yield_time_ms: u64,
    #[serde(default)]
    max_output_tokens: Option<usize>,
}

impl WriteStdinArgs {
    fn parse(payload: &ToolPayload) -> Result<Self, FunctionCallError> {
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(
                "write_stdin handler received unsupported payload".to_string(),
            ));
        };
        parse_arguments(arguments)
    }
}

pub struct WriteStdinHandler;

impl ToolExecutor<ToolInvocation> for WriteStdinHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("write_stdin")
    }

    fn spec(&self) -> ToolSpec {
        create_write_stdin_tool()
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(self.handle_call(invocation))
    }
}

impl WriteStdinHandler {
    async fn handle_call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            step_context,
            cancellation_token,
            call_id,
            payload,
            environments,
            ..
        } = invocation;

        let ToolEnvironments::Terminal(terminal) = environments else {
            return Err(FunctionCallError::Fatal(
                "write_stdin requires a terminal session".to_string(),
            ));
        };
        let args = WriteStdinArgs::parse(&payload)?;
        let context =
            UnifiedExecContext::new(session.clone(), step_context, cancellation_token, call_id);
        let response = session
            .services
            .unified_exec_manager
            .write_stdin(
                &context,
                WriteStdinRequest {
                    terminal: &terminal,
                    input: &args.chars,
                    yield_time_ms: args.yield_time_ms,
                    max_output_tokens: args.max_output_tokens,
                    truncation_policy: context
                        .step_context
                        .settings
                        .model_info
                        .truncation_policy
                        .into(),
                    interaction_event: Some(WriteStdinInteractionEvent {
                        session: &session,
                        turn: &turn,
                    }),
                },
            )
            .await
            .map_err(|err| {
                let message = match err {
                    UnifiedExecError::StdinApproval(ToolError::Rejected(reason)) => {
                        format!("write_stdin rejected: {reason}")
                    }
                    UnifiedExecError::StdinApproval(ToolError::Codex(err)) => {
                        format!("write_stdin approval failed: {err}")
                    }
                    err => format!("write_stdin failed: {err}"),
                };
                FunctionCallError::RespondToModel(message)
            })?;

        Ok(boxed_tool_output(response))
    }
}

impl CoreToolRuntime for WriteStdinHandler {
    fn live_environment_resolver(&self) -> Option<LiveEnvironmentResolver> {
        Some(super::terminal_session_for_input)
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    fn pre_tool_use_payload(&self, invocation: &ToolInvocation) -> Option<PreToolUsePayload> {
        // Empty writes are background polls and carry nothing to review. A
        // non-empty write is text the model is typing into the open shell, so
        // review it as a Bash command before it reaches the process.
        let args = WriteStdinArgs::parse(&invocation.payload).ok()?;
        if args.chars.is_empty() {
            return None;
        }
        // The directory the shell was started in, not its current directory: a
        // `cd` typed into the shell is not tracked. Null when the process is gone.
        let workdir = match &invocation.environments {
            ToolEnvironments::Terminal(terminal) => invocation
                .session
                .services
                .unified_exec_manager
                .start_cwd_for_process(terminal.process_id)
                .map(|cwd| serde_json::Value::String(cwd.to_string())),
            _ => None,
        }
        .unwrap_or(serde_json::Value::Null);
        Some(PreToolUsePayload {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::json!({ "command": args.chars, "workdir": workdir }),
        })
    }

    fn post_tool_use_payload(
        &self,
        invocation: &ToolInvocation,
        result: &dyn crate::tools::context::ToolOutput,
    ) -> Option<PostToolUsePayload> {
        // A `write_stdin` poll can observe final completion for the original
        // `exec_command`; emit that command's matching Bash PostToolUse.
        post_unified_exec_tool_use_payload(invocation, result)
    }
}
