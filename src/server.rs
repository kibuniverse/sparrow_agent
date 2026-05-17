use std::{convert::Infallible, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{net::TcpListener, time::sleep};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    config::AppConfig,
    conversation_store::ConversationStore,
    trace::{TaskStatus, TraceEvent, TraceEventType, trace_id},
    trace_store::{TaskSnapshot, TraceStore, TraceStoreSink},
};

#[derive(Clone)]
pub struct ServerState {
    pub config: AppConfig,
    pub conversations: Arc<ConversationStore>,
    pub traces: Arc<TraceStore>,
    pub trace_dir: Option<Arc<PathBuf>>,
}

impl ServerState {
    pub fn new(config: AppConfig, traces: Arc<TraceStore>) -> Self {
        Self {
            config: config.without_interactive_tools(),
            conversations: Arc::new(ConversationStore::new()),
            traces,
            trace_dir: None,
        }
    }

    pub fn with_trace_dir(mut self, trace_dir: PathBuf) -> Self {
        self.trace_dir = Some(Arc::new(trace_dir));
        self
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentTaskRequest {
    pub conversation_id: Option<String>,
    pub client_message_id: String,
    pub message: String,
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct CreateAgentTaskResponse {
    pub task_id: String,
    pub conversation_id: String,
    pub events_url: String,
    pub snapshot_url: String,
}

#[derive(Debug, Deserialize)]
pub struct EventStreamQuery {
    #[serde(default)]
    pub after_seq: u64,
}

fn api_routes() -> Router<ServerState> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/agent/tasks", post(create_task))
        .route("/api/agent/tasks/{task_id}", get(get_task))
        .route("/api/agent/tasks/{task_id}/events", get(stream_task_events))
}

pub fn build_router(state: ServerState) -> Router {
    api_routes()
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state)
}

pub fn build_browser_router(state: ServerState, frontend_dist: PathBuf) -> Router {
    let trace_dir = crate::trace_file::default_trace_dir()
        .unwrap_or_else(|_| PathBuf::from(".sparrow_agent/traces"));
    build_browser_router_with_trace_dir(state, frontend_dist, trace_dir)
}

pub fn build_browser_router_with_trace_dir(
    state: ServerState,
    frontend_dist: PathBuf,
    trace_dir: PathBuf,
) -> Router {
    let index = frontend_dist.join("index.html");
    api_routes()
        .route("/api/agent/trace-files/{file_name}", get(open_trace_file))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state.with_trace_dir(trace_dir))
        .fallback_service(ServeDir::new(frontend_dist).fallback(ServeFile::new(index)))
}

pub async fn run_server(config: AppConfig, addr: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind server to {addr}"))?;
    println!("Sparrow Agent server listening on http://{addr}");
    axum::serve(
        listener,
        build_router(ServerState::new(config, Arc::new(TraceStore::new()))),
    )
    .await
    .context("server failed")
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn create_task(
    State(state): State<ServerState>,
    Json(request): Json<CreateAgentTaskRequest>,
) -> std::result::Result<(StatusCode, Json<CreateAgentTaskResponse>), ApiError> {
    if !request.stream {
        return Err(ApiError::bad_request(
            "unsupported_mode",
            "Only stream=true is supported.",
        ));
    }

    if request.client_message_id.trim().is_empty() {
        return Err(ApiError::bad_request(
            "invalid_request",
            "client_message_id is required.",
        ));
    }

    if request.message.trim().is_empty() {
        return Err(ApiError::bad_request(
            "invalid_request",
            "message is required.",
        ));
    }

    if let Some(existing) = state
        .traces
        .find_by_client_message_id(&request.client_message_id)
    {
        return Ok((
            StatusCode::ACCEPTED,
            Json(task_response(&existing.task_id, &existing.conversation_id)),
        ));
    }

    let conversation_id = request
        .conversation_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| trace_id("conv"));

    if state.conversations.is_busy(&conversation_id).await {
        return Err(ApiError::conversation_busy());
    }

    let task = state
        .traces
        .create_task(conversation_id.clone(), request.client_message_id);

    if state
        .conversations
        .try_start_task(&conversation_id, &task.task_id)
        .await
        .is_err()
    {
        state
            .traces
            .mark_failed(&task.task_id, 0, "Conversation already has a running task.");
        return Err(ApiError::conversation_busy());
    }

    let task_id = task.task_id.clone();
    let message = request.message;
    let task_state = state.clone();
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let sink = TraceStoreSink::new(Arc::clone(&task_state.traces), task_id.clone());
        let result = async {
            let agent = task_state
                .conversations
                .agent_for_conversation(&conversation_id, &task_state.config)
                .await?;
            let mut agent = agent.lock().await;
            agent.handle_user_input_with_trace(message, &sink).await
        }
        .await;

        if let Err(error) = result
            && task_state
                .traces
                .snapshot(&task_id)
                .map(|snapshot| snapshot.status == TaskStatus::Running)
                .unwrap_or(false)
        {
            task_state.traces.mark_failed(
                &task_id,
                started.elapsed().as_millis() as u64,
                error.to_string(),
            );
        }

        task_state
            .conversations
            .finish_task(&conversation_id, &task_id)
            .await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(task_response(&task.task_id, &task.conversation_id)),
    ))
}

async fn get_task(
    State(state): State<ServerState>,
    Path(task_id): Path<String>,
) -> std::result::Result<Json<TaskSnapshot>, ApiError> {
    state
        .traces
        .snapshot(&task_id)
        .map(Json)
        .map_err(|_| ApiError::not_found("task_not_found", "Task was not found."))
}

async fn open_trace_file(
    State(state): State<ServerState>,
    Path(file_name): Path<String>,
) -> std::result::Result<Json<crate::trace_file::TraceArchive>, ApiError> {
    let Some(trace_dir) = state.trace_dir.as_deref() else {
        return Err(ApiError::not_found(
            "trace_file_not_found",
            "Trace file was not found.",
        ));
    };
    let Some(path) = crate::trace_file::safe_archive_file_path(trace_dir, &file_name) else {
        return Err(ApiError::bad_request(
            "invalid_trace_file",
            "Trace file name is invalid.",
        ));
    };

    crate::trace_file::read_trace_archive(path)
        .map(Json)
        .map_err(|_| ApiError::not_found("trace_file_not_found", "Trace file was not found."))
}

async fn stream_task_events(
    State(state): State<ServerState>,
    Path(task_id): Path<String>,
    Query(query): Query<EventStreamQuery>,
) -> std::result::Result<
    Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>,
    ApiError,
> {
    let snapshot = state
        .traces
        .snapshot(&task_id)
        .map_err(|_| ApiError::not_found("task_not_found", "Task was not found."))?;
    let (replay, mut rx) = state
        .traces
        .subscribe(&task_id, query.after_seq)
        .map_err(|_| ApiError::not_found("task_not_found", "Task was not found."))?;

    let stream = async_stream::stream! {
        for event in replay {
            let terminal = is_terminal_event(event.event_type);
            yield Ok(trace_sse_event(&event));
            if terminal {
                sleep(Duration::from_secs(2)).await;
                return;
            }
        }

        if is_terminal_status(snapshot.status) {
            sleep(Duration::from_secs(2)).await;
            return;
        }

        loop {
            match rx.recv().await {
                Ok(event) => {
                    let terminal = is_terminal_event(event.event_type);
                    yield Ok(trace_sse_event(&event));
                    if terminal {
                        sleep(Duration::from_secs(2)).await;
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

fn task_response(task_id: &str, conversation_id: &str) -> CreateAgentTaskResponse {
    CreateAgentTaskResponse {
        task_id: task_id.into(),
        conversation_id: conversation_id.into(),
        events_url: format!("/api/agent/tasks/{task_id}/events"),
        snapshot_url: format!("/api/agent/tasks/{task_id}"),
    }
}

fn trace_sse_event(event: &TraceEvent) -> Event {
    Event::default()
        .event("trace")
        .id(event.seq.to_string())
        .data(serde_json::to_string(event).unwrap_or_else(|_| "{}".into()))
}

fn is_terminal_event(event_type: TraceEventType) -> bool {
    matches!(
        event_type,
        TraceEventType::TaskCompleted | TraceEventType::TaskFailed
    )
}

fn is_terminal_status(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
    )
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    retryable: bool,
}

impl ApiError {
    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
            retryable: false,
        }
    }

    fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
            message: message.into(),
            retryable: false,
        }
    }

    fn conversation_busy() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conversation_busy",
            message: "Conversation already has a running task.".into(),
            retryable: true,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                    "retryable": self.retryable,
                },
            })),
        )
            .into_response()
    }
}
