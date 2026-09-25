use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use codex_app_server_protocol::AdvisorStartParams;
use codex_app_server_protocol::AdvisorStartResponse;
use codex_app_server_protocol::ThreadHistoryMode;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;

const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn advisor_start_consults_separate_model_then_resumes_worker() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let responses = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("worker-1"),
                responses::ev_assistant_message("worker-message-1", "I checked the parser."),
                responses::ev_completed("worker-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("advisor-1"),
                responses::ev_assistant_message(
                    "advisor-message",
                    "Check the empty-input branch before changing the parser.",
                ),
                responses::ev_completed("advisor-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("worker-2"),
                responses::ev_assistant_message("worker-message-2", "I will check that branch."),
                responses::ev_completed("worker-2"),
            ]),
        ],
    )
    .await;

    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    MockResponsesConfig::new(&server.uri())
        .with_model("gpt-5.5")
        .with_provider_config("supports_websockets = false")
        .with_root_config(
            "advisor = { enabled = true, model = \"gpt-6-sol\", reasoning_effort = \"high\" }",
        )
        .write(codex_home.path())?;
    let mut app = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .build_initialized_with_timeout(READ_TIMEOUT)
        .await?;

    let thread_request = app
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.5".to_string()),
            cwd: Some(cwd.path().to_string_lossy().into_owned()),
            history_mode: Some(ThreadHistoryMode::Paginated),
            ..Default::default()
        })
        .await?;
    let ThreadStartResponse { thread, .. } =
        timeout(READ_TIMEOUT, app.read_response(thread_request)).await??;
    let thread_id = thread.id;

    let initial = app
        .start_turn_and_wait_for_completion(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "Fix the parser's empty-input handling.".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;

    let advisor_request = app
        .send_request(
            "advisor/start",
            Some(serde_json::to_value(AdvisorStartParams { thread_id })?),
        )
        .await?;
    let AdvisorStartResponse { turn } =
        timeout(READ_TIMEOUT, app.read_response(advisor_request)).await??;
    wait_for_turn_completed(&mut app, &turn.id).await?;

    let requests = responses.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].body_json()["model"], "gpt-5.5");
    assert_eq!(requests[1].body_json()["model"], "gpt-6-sol");
    assert_eq!(requests[1].body_json()["reasoning"]["effort"], "high");
    assert!(requests[1].body_json()["tools"].is_null());
    assert!(
        requests[1].body_json()["input"]
            .to_string()
            .contains("empty-input")
    );
    assert_eq!(requests[2].body_json()["model"], "gpt-5.5");
    assert!(
        requests[2].body_json()["input"]
            .to_string()
            .contains("Check the empty-input branch before changing the parser")
    );
    assert_ne!(initial.turn.id, turn.id);
    Ok(())
}

async fn wait_for_turn_completed(app: &mut TestAppServer, turn_id: &str) -> Result<()> {
    loop {
        let completed: TurnCompletedNotification =
            timeout(READ_TIMEOUT, app.read_notification("turn/completed")).await??;
        if completed.turn.id == turn_id {
            return Ok(());
        }
    }
}
