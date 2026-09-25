use codex_async_utils::OrCancelExt;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_rollout_trace::InferenceTraceContext;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::responses_metadata::CodexResponsesRequestKind;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::turn_metadata::ExecutionMetadata;

const ADVISOR_INSTRUCTIONS: &str = "You are a read-only senior software engineer advising an active coding agent. Review the supplied snapshot. Find incorrect assumptions, root causes, architectural risks, likely regressions, missing validation, and simpler approaches. Give short actionable advice under Assessment, Main risk, Recommended next step, and Avoid. Do not claim to have inspected files beyond the snapshot. Do not take ownership of the task.";
const MAX_SNAPSHOT_CHARS: usize = 8_000;
const MAX_FEEDBACK_CHARS: usize = 4_000;

/// Consults a separate model without offering it any tools or changing worker settings.
pub(crate) async fn request_advisor_consultation(
    session: &Session,
    turn: &TurnContext,
    worker_summary: &str,
    cancellation: &CancellationToken,
) -> CodexResult<String> {
    let advisor = &turn.config.advisor;
    if !advisor.enabled {
        return Err(CodexErr::InvalidRequest(
            "advisor is disabled; set advisor.enabled = true in config".to_string(),
        ));
    }
    let model = advisor
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| CodexErr::InvalidRequest("advisor.model must be configured".to_string()))?;
    let model_info = session
        .services
        .models_manager
        .get_model_info(model, &turn.config.to_models_manager_config())
        .await;
    if model_info.used_fallback_model_metadata {
        return Err(CodexErr::InvalidRequest(format!(
            "advisor model {model} is not available in the model catalog"
        )));
    }
    let effort = advisor
        .reasoning_effort
        .clone()
        .or_else(|| model_info.default_reasoning_level.clone());
    if let Some(effort) = &advisor.reasoning_effort
        && !model_info.supported_reasoning_levels.is_empty()
        && !model_info
            .supported_reasoning_levels
            .iter()
            .any(|level| &level.effort == effort)
    {
        return Err(CodexErr::InvalidRequest(format!(
            "reasoning effort {effort} is unavailable for advisor model {model}"
        )));
    }

    let snapshot = build_snapshot(session, worker_summary).await;
    let prompt = Prompt {
        input: vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText { text: snapshot }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        base_instructions: BaseInstructions {
            text: ADVISOR_INSTRUCTIONS.to_string(),
            provenance: None,
        },
        ..Default::default()
    };
    let mut metadata = session
        .responses_metadata_for_turn_context(turn, CodexResponsesRequestKind::Turn)
        .await;
    metadata.tool_namespaces_info = None;
    ExecutionMetadata {
        model: &model_info.slug,
        reasoning_effort: effort.clone(),
        node_repl_disabled: model_info.node_repl_disabled,
        auto_review_enabled: false,
        node_repl_auto_review_required: false,
    }
    .apply_to(&mut metadata);
    let telemetry = turn
        .session_telemetry
        .clone()
        .with_model(model, &model_info.slug)
        .with_inference_request(/*service_tier*/ None, effort.as_ref());
    let mut client_session = session.services.model_client.new_session();
    let mut stream = client_session
        .stream(
            &prompt,
            &model_info,
            &telemetry,
            effort,
            model_info.default_reasoning_summary,
            /*service_tier*/ None,
            &metadata,
            &InferenceTraceContext::disabled(),
        )
        .or_cancel(cancellation)
        .await??;
    let mut feedback = String::new();
    loop {
        let event = stream
            .next()
            .or_cancel(cancellation)
            .await?
            .ok_or_else(|| {
                CodexErr::Stream("advisor response ended before completion".to_string())
            })??;
        match event {
            ResponseEvent::OutputItemDone(ResponseItem::Message { role, content, .. })
                if role == "assistant" =>
            {
                for item in content {
                    if let ContentItem::OutputText { text } = item {
                        let remaining = MAX_FEEDBACK_CHARS.saturating_sub(feedback.chars().count());
                        feedback.extend(text.chars().take(remaining));
                    }
                }
            }
            ResponseEvent::Completed { .. } => break,
            _ => {}
        }
    }
    let feedback = feedback.trim();
    if feedback.is_empty() {
        return Err(CodexErr::Stream("advisor returned no feedback".to_string()));
    }
    Ok(feedback.to_string())
}

async fn build_snapshot(session: &Session, worker_summary: &str) -> String {
    let history = session.clone_history().await;
    let mut snapshot = String::from("Worker summary and current question:\n");
    snapshot.extend(worker_summary.chars().take(3_000));
    snapshot.push_str("\n\nRecent conversation and results:\n");
    let recent = history
        .raw_items()
        .rev()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } => {
                let mut text = String::new();
                for item in content {
                    let part = match item {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => text,
                        _ => continue,
                    };
                    let remaining = 700usize.saturating_sub(text.chars().count());
                    if remaining == 0 {
                        break;
                    }
                    text.extend(part.chars().take(remaining));
                }
                (!text.is_empty()).then(|| format!("{role}: {text}"))
            }
            ResponseItem::FunctionCallOutput { output, .. } => output.text_content().map(|text| {
                format!(
                    "tool result: {}",
                    text.chars().take(700).collect::<String>()
                )
            }),
            _ => None,
        })
        .take(8)
        .collect::<Vec<_>>();
    for item in recent.into_iter().rev() {
        snapshot.push_str(&item);
        snapshot.push('\n');
    }
    snapshot.chars().take(MAX_SNAPSHOT_CHARS).collect()
}
