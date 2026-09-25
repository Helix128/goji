use anyhow::Result;
use codex_config::config_toml::AdvisorConfigToml;
use codex_core::TurnInputRequest;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::EventMsg;
use codex_protocol::user_input::UserInput;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::MockServer;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn advisor_uses_separate_model_without_tools_and_returns_to_worker() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = MockServer::start().await;
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("worker-1"),
                responses::ev_function_call(
                    "advisor-call",
                    "consult_advisor",
                    &json!({ "summary": "Goal: fix parser; tests fail on empty input." })
                        .to_string(),
                ),
                responses::ev_completed("worker-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("advisor-1"),
                responses::ev_assistant_message("advisor-message", "Check the empty-input branch."),
                responses::ev_completed("advisor-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("worker-2"),
                responses::ev_assistant_message("worker-message", "I will check that branch."),
                responses::ev_completed("worker-2"),
            ]),
        ],
    )
    .await;
    let test = test_codex()
        .with_config(|config| {
            config.model = Some("gpt-5.5".to_string());
            config.advisor = AdvisorConfigToml {
                enabled: true,
                model: Some("gpt-6-sol".to_string()),
                reasoning_effort: Some(ReasoningEffort::High),
            };
        })
        .build_with_auto_env(&server)
        .await?;
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "Fix the parser".to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].body_json()["model"], "gpt-5.5");
    assert_eq!(requests[1].body_json()["model"], "gpt-6-sol");
    assert_eq!(requests[1].body_json()["reasoning"]["effort"], "high");
    assert!(requests[1].body_json()["tools"].is_null());
    assert!(
        requests[1].body_json()["input"]
            .to_string()
            .contains("fix parser")
    );
    assert_eq!(requests[2].body_json()["model"], "gpt-5.5");
    assert!(
        requests[2]
            .function_call_output("advisor-call")
            .to_string()
            .contains("Check the empty-input branch")
    );
    Ok(())
}
