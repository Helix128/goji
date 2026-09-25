use std::collections::BTreeMap;

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;

use crate::advisor::request_advisor_consultation;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;

const TOOL_NAME: &str = "consult_advisor";

#[derive(Deserialize)]
struct AdvisorArgs {
    summary: String,
}

pub(crate) struct AdvisorHandler;

impl ToolExecutor<ToolInvocation> for AdvisorHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec::Function(ResponsesApiTool {
            name: TOOL_NAME.to_string(),
            description: "Ask the configured read-only advisor model for brief strategic guidance. Use when a difficult decision, repeated failure, or significant change would benefit from another perspective. Supply a concise summary of the objective, current approach, changes, failures, and question. The advisor cannot use tools or modify files.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                BTreeMap::from([(
                    "summary".to_string(),
                    JsonSchema::string(Some(
                        "Concise worker state and the question for the advisor".to_string(),
                    )),
                )]),
                Some(vec!["summary".to_string()]),
                Some(false.into()),
            ),
            output_schema: None,
        })
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            let ToolPayload::Function { arguments } = &invocation.payload else {
                return Err(FunctionCallError::RespondToModel(
                    "consult_advisor requires function arguments".to_string(),
                ));
            };
            let args: AdvisorArgs = parse_arguments(arguments)?;
            let feedback = request_advisor_consultation(
                invocation.session.as_ref(),
                invocation.step_context.turn.as_ref(),
                &args.summary,
                &invocation.cancellation_token,
            )
            .await
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
            Ok(boxed_tool_output(FunctionToolOutput::from_text(
                format!("Advisor feedback:\n{feedback}"),
                Some(true),
            )))
        })
    }
}

impl CoreToolRuntime for AdvisorHandler {}
