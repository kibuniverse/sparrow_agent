use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::json;
use sparrow_agent::{
    config::{AppConfig, ConfirmationPolicy, FilesystemConfig, FilesystemMode, StreamingConfig},
    server::{ServerState, build_router},
    trace::TraceEventType,
    trace_store::{TaskSnapshot, TraceStore},
};
use tower::ServiceExt;

#[tokio::test]
async fn server_health_returns_ok() {
    let app = build_router(ServerState::new(test_config(), Arc::new(TraceStore::new())));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn server_snapshot_returns_stored_trace_events() {
    let traces = Arc::new(TraceStore::new());
    let task = traces.create_task("conv_1".into(), "msg_1".into());
    traces
        .append_event(&task.task_id, TraceEventType::TaskStarted, json!({}))
        .unwrap();
    let app = build_router(ServerState::new(test_config(), traces));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent/tasks/{}", task.task_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let snapshot: TaskSnapshot = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(snapshot.task_id, task.task_id);
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.events[0].event_type, TraceEventType::TaskStarted);
}

#[tokio::test]
async fn server_events_endpoint_replays_trace_sse_frames() {
    let traces = Arc::new(TraceStore::new());
    let task = traces.create_task("conv_1".into(), "msg_1".into());
    traces
        .append_event(&task.task_id, TraceEventType::TaskStarted, json!({}))
        .unwrap();
    traces
        .append_event(
            &task.task_id,
            TraceEventType::TaskCompleted,
            json!({ "duration_ms": 1, "final_answer": "done" }),
        )
        .unwrap();
    let app = build_router(ServerState::new(test_config(), traces));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/agent/tasks/{}/events?after_seq=1",
                    task.task_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("event: trace"));
    assert!(body.contains("id: 2"));
    assert!(body.contains(r#""type":"task.completed""#));
}

#[tokio::test]
async fn browser_router_serves_frontend_index() {
    let frontend = tempfile::tempdir().unwrap();
    std::fs::write(
        frontend.path().join("index.html"),
        "<!doctype html><title>Sparrow Inspector</title>",
    )
    .unwrap();

    let app = sparrow_agent::server::build_browser_router(
        ServerState::new(test_config(), Arc::new(TraceStore::new())),
        frontend.path().to_path_buf(),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("Sparrow Inspector"));
}

#[tokio::test]
async fn browser_router_falls_back_for_task_deep_links() {
    let frontend = tempfile::tempdir().unwrap();
    std::fs::write(
        frontend.path().join("index.html"),
        "<!doctype html><title>Sparrow Inspector</title>",
    )
    .unwrap();

    let app = sparrow_agent::server::build_browser_router(
        ServerState::new(test_config(), Arc::new(TraceStore::new())),
        frontend.path().to_path_buf(),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/tasks/task_123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("Sparrow Inspector"));
}

#[tokio::test]
async fn browser_router_opens_generated_trace_archive() {
    let frontend = tempfile::tempdir().unwrap();
    std::fs::write(frontend.path().join("index.html"), "<!doctype html>").unwrap();
    let trace_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        trace_dir.path().join("task_1.sparrow-trace.json"),
        r#"{"schema_version":1,"exported_at":"2026-05-10T00:00:00Z","source":"cli","task":{"task_id":"task_1","conversation_id":"conv_1","status":"succeeded","created_at":"2026-05-10T00:00:00Z","updated_at":"2026-05-10T00:00:01Z","events":[]}}"#,
    )
    .unwrap();

    let app = sparrow_agent::server::build_browser_router_with_trace_dir(
        ServerState::new(test_config(), Arc::new(TraceStore::new())),
        frontend.path().to_path_buf(),
        trace_dir.path().to_path_buf(),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agent/trace-files/task_1.sparrow-trace.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains(r#""task_id":"task_1""#));
}

#[tokio::test]
async fn browser_router_rejects_trace_archive_path_traversal() {
    let frontend = tempfile::tempdir().unwrap();
    std::fs::write(frontend.path().join("index.html"), "<!doctype html>").unwrap();
    let trace_dir = tempfile::tempdir().unwrap();
    let app = sparrow_agent::server::build_browser_router_with_trace_dir(
        ServerState::new(test_config(), Arc::new(TraceStore::new())),
        frontend.path().to_path_buf(),
        trace_dir.path().to_path_buf(),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agent/trace-files/..%2Fsecret.sparrow-trace.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

fn test_config() -> AppConfig {
    AppConfig {
        api_key: "test".into(),
        tavily_api_key: "test".into(),
        model: "deepseek-chat".into(),
        system_prompt: "You are a test agent.".into(),
        reasoning_effort: "high".into(),
        max_tool_rounds: 1,
        filesystem: FilesystemConfig {
            enabled: false,
            roots: Vec::new(),
            mode: FilesystemMode::ReadOnly,
            confirm: ConfirmationPolicy::Never,
            deny_patterns: Vec::new(),
            max_read_bytes: 1,
            max_write_bytes: 1,
        },
        mcp_servers: Vec::new(),
        streaming: StreamingConfig {
            enabled: true,
            show_reasoning: true,
            show_tool_call_deltas: false,
        },
    }
}
