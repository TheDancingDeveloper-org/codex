use super::*;
use codex_extension_api::ExtensionData;
use codex_extension_api::TurnItemContributor;
use codex_protocol::ResponseItemId;
use codex_protocol::items::AgentMessageContent;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tracing_subscriber::prelude::*;

struct RewriteAgentMessageContributor;

impl TurnItemContributor for RewriteAgentMessageContributor {
    fn contribute<'a>(
        &'a self,
        _thread_store: &'a ExtensionData,
        _turn_store: &'a ExtensionData,
        item: &'a mut TurnItem,
    ) -> codex_extension_api::ExtensionFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let TurnItem::AgentMessage(agent_message) = item {
                agent_message.content = vec![AgentMessageContent::Text {
                    text: "plan contributed assistant text".to_string(),
                }];
            }
            Ok(())
        })
    }
}

fn assistant_output_text(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: Some(ResponseItemId::with_suffix("msg", "1")),
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

#[test]
fn post_sampling_token_estimate_is_disabled_by_always_on_sinks() {
    let feedback = codex_feedback::CodexFeedback::new();
    let subscriber = tracing_subscriber::registry()
        .with(feedback.logger_layer())
        .with(tracing_subscriber::fmt::layer().with_filter(codex_state::log_db::default_filter()));

    static METADATA: tracing::Metadata<'static> = tracing::metadata! {
        name: "post sampling token estimate filter probe",
        target: POST_SAMPLING_TOKEN_ESTIMATE_TARGET,
        level: tracing::Level::TRACE,
        fields: &["turn_id", "estimated_token_count", "message"],
        callsite: &CALLSITE,
        kind: tracing::metadata::Kind::EVENT.hint(),
    };
    static CALLSITE: tracing::callsite::DefaultCallsite =
        tracing::callsite::DefaultCallsite::new(&METADATA);

    assert!(tracing::Subscriber::register_callsite(&subscriber, &METADATA).is_never());
}

#[tokio::test]
async fn plan_mode_uses_contributed_turn_item_for_last_agent_message() {
    let (mut session, turn_context) = crate::session::tests::make_session_and_context().await;
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.turn_item_contributor(Arc::new(RewriteAgentMessageContributor));
    session.services.extensions = Arc::new(builder.build());
    let turn_store = ExtensionData::new(turn_context.sub_id.clone());
    let mut state = PlanModeStreamState::new(&turn_context.sub_id);
    let mut last_agent_message = None;
    let item = assistant_output_text("original assistant text");

    let step_context = StepContext::for_test(Arc::new(turn_context));
    let handled = handle_assistant_item_done_in_plan_mode(
        &session,
        &step_context,
        &turn_store,
        &item,
        &mut state,
        /*previously_active_item*/ None,
        &mut last_agent_message,
    )
    .await;

    assert!(handled);
    assert_eq!(
        last_agent_message.as_deref(),
        Some("plan contributed assistant text")
    );
}

#[test]
fn realtime_user_verification_notice_excludes_request_payload() {
    let event = EventMsg::ElicitationRequest(codex_protocol::approvals::ElicitationRequestEvent {
        turn_id: None,
        server_name: "private-server-name".to_string(),
        id: codex_protocol::mcp::RequestId::String("private-request-id".to_string()),
        request: codex_protocol::approvals::ElicitationRequest::UserVerification {
            meta: None,
            title: "private-title".to_string(),
            description: "private-description".to_string(),
            challenge: "private-challenge".to_string(),
        },
    });
    assert_eq!(
        realtime_text_for_event(&event),
        Some(RealtimeEventText::Handoff(
            "<user_verification_notice>User verification is required. Please respond in the app.</user_verification_notice>".to_string(),
            None,
        )),
    );
}

fn token_status_for_prompt_check(
    scope_limit: Option<i64>,
    prefill: Option<i64>,
    context_window: Option<i64>,
) -> super::super::context_window::ContextWindowTokenStatus {
    super::super::context_window::ContextWindowTokenStatus {
        active_context_tokens: 0,
        auto_compact_scope_tokens: 0,
        auto_compact_scope_limit: scope_limit,
        full_context_window_limit: context_window,
        base_window_tokens_remaining: None,
        auto_compact_window_prefill_tokens: prefill,
        full_context_window_limit_reached: false,
        token_limit_reached: true,
        turn_end_compaction_threshold_reached: false,
    }
}

#[test]
fn prompt_over_limit_detects_a_prefix_that_compaction_cannot_shrink() {
    // WI-2832: a 130.5k-token prompt after compaction against a 128k limit.
    let status = token_status_for_prompt_check(Some(128_000), None, Some(258_400));
    assert!(prompt_over_limit(
        AutoCompactTokenLimitScope::Total,
        130_516,
        &status
    ));
    // A normal post-compaction prompt leaves room for the turn to make progress.
    assert!(!prompt_over_limit(
        AutoCompactTokenLimitScope::Total,
        43_000,
        &status
    ));
}

#[test]
fn prompt_over_limit_ignores_the_prefix_for_body_after_prefix_scope() {
    let status = token_status_for_prompt_check(Some(20_000), Some(110_000), Some(258_400));
    assert!(!prompt_over_limit(
        AutoCompactTokenLimitScope::BodyAfterPrefix,
        125_000,
        &status
    ));
    assert!(prompt_over_limit(
        AutoCompactTokenLimitScope::BodyAfterPrefix,
        130_000,
        &status
    ));
    // Without a recorded prefix the body cannot be measured; only the hard window applies.
    let status = token_status_for_prompt_check(Some(20_000), None, Some(100_000));
    assert!(!prompt_over_limit(
        AutoCompactTokenLimitScope::BodyAfterPrefix,
        90_000,
        &status
    ));
    assert!(prompt_over_limit(
        AutoCompactTokenLimitScope::BodyAfterPrefix,
        100_000,
        &status
    ));
}
