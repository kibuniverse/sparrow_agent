# CLI Browser Loop Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 支持用户用 CLI 调用 agent 时，在浏览器中实时查询 loop 过程，并在 agent 完成后生成可由前端打开、预览和回放的 trace 文件。

**Architecture:** 复用现有 `TraceStore`、`TraceStoreSink`、`Agent::handle_user_input_with_trace()`、SSE API 和 React 任务详情页。新增一个 CLI 观察模式：CLI 仍然读写终端，同进程启动 HTTP 服务并共享同一个 `TraceStore`；每次用户输入会创建一个 trace task，CLI 打印实时 `/tasks/:task_id` URL。任务进入终态后，Rust 将 `TaskSnapshot` 写成 `.sparrow-trace.json` archive 文件，HTTP 服务提供只读文件打开接口，前端新增 `/trace-files/:fileName` 预览页和 `/replay/:fileName` 回放页。

**Tech Stack:** Rust 2024, tokio, axum, tower-http, serde_json, React/Vite, EventSource SSE.

---

## File Structure

- Modify: `Cargo.toml`
  - Enable `tower-http` `fs` feature so Rust server can serve `frontend/dist`.
- Modify: `src/lib.rs`
  - Export the new CLI observer module.
- Create: `src/cli_observer.rs`
  - Owns CLI browser-observation mode: shared `TraceStore`, background HTTP server, per-input task creation, live/replay URL printing, traced agent execution, trace archive writing.
- Create: `src/trace_file.rs`
  - Defines the durable trace archive schema, default trace directory, safe file names, JSON write/read helpers, and HTTP-safe file lookup.
- Modify: `src/server.rs`
  - Add a `build_browser_router(state, frontend_dist)` helper that composes existing API routes with static frontend serving and SPA fallback.
  - Add `GET /api/agent/trace-files/:file_name` for opening generated trace archives from the configured trace directory.
  - Keep `build_router()` behavior stable for existing tests and API-only usage.
- Modify: `src/main.rs`
  - Parse `--inspect` / `--browser-trace` and route into `cli_observer::run_cli_with_browser_trace`.
  - Preserve existing `cargo run` CLI and `cargo run -- --server` behavior.
- Modify: `frontend/src/types/trace.ts`
  - Add `TraceArchive`, `TraceArchiveSource`, and replay mode types.
- Modify: `frontend/src/api/agentTrace.ts`
  - Add API helpers for loading generated trace archive files.
- Modify: `frontend/src/router.ts`
  - Add `/trace-files/:fileName` and `/replay/:fileName` routes.
- Modify: `frontend/src/App.tsx`
  - Route live tasks, archive preview, archive replay, and local-file import into the same reducer-backed visualization.
- Create: `frontend/src/pages/TraceArchivePage.tsx`
  - Opens generated trace files by URL or imported files and previews the completed timeline.
- Create: `frontend/src/hooks/useTraceReplay.ts`
  - Drives deterministic replay by feeding archived events to `applyTraceEvent` according to speed and playback state.
- Create: `frontend/src/components/TraceReplayControls.tsx`
  - Provides play/pause, restart, step, speed, and progress controls.
- Modify: `README.md`
  - Document CLI browser trace mode, trace archive location, archive preview URLs, replay controls, and environment variables.
- Test: `tests/server_contract.rs`
  - Add coverage that static frontend routes are served, unknown SPA paths fall back to `index.html`, and generated trace archive files can be opened safely.
- Test: `tests/cli_observer_contract.rs`
  - Add low-level contract tests for task/replay URL construction and CLI task creation metadata without calling the model.
- Test: `tests/trace_file_contract.rs`
  - Add durable archive schema, path sanitization, and round-trip JSON tests.
- Test: `frontend/src/hooks/useTraceReplay.test.tsx`
  - Add deterministic replay behavior tests with fake timers.

## Task 1: Enable Static Frontend Serving

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Update dependency feature**

Change the existing `tower-http` dependency:

```toml
tower-http = { version = "0.6", features = ["cors", "fs"] }
```

- [ ] **Step 2: Check that dependency resolution still works**

Run:

```bash
cargo check
```

Expected: PASS. No source changes are required by this step beyond enabling the feature.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "chore: enable static file serving"
```

## Task 2: Add Browser Router for API Plus Frontend

**Files:**
- Modify: `src/server.rs`
- Test: `tests/server_contract.rs`

- [ ] **Step 1: Write failing server route tests**

Append these tests to `tests/server_contract.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test server_contract browser_router
```

Expected: FAIL because `build_browser_router` does not exist.

- [ ] **Step 3: Implement browser router**

In `src/server.rs`, add imports:

```rust
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};
```

Then add this public helper below `build_router`:

```rust
pub fn build_browser_router(state: ServerState, frontend_dist: PathBuf) -> Router {
    let index = frontend_dist.join("index.html");
    build_router(state).fallback_service(
        ServeDir::new(frontend_dist).fallback(ServeFile::new(index)),
    )
}
```

This keeps API routes from `build_router()` authoritative and lets React handle `/tasks/:task_id` deep links.

- [ ] **Step 4: Run targeted tests**

Run:

```bash
cargo test --test server_contract browser_router
```

Expected: PASS.

- [ ] **Step 5: Run all server contract tests**

Run:

```bash
cargo test --test server_contract
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server.rs tests/server_contract.rs
git commit -m "feat: serve inspector frontend from agent server"
```

## Task 3: Add Durable Trace Archive Files

**Files:**
- Create: `src/trace_file.rs`
- Modify: `src/lib.rs`
- Test: `tests/trace_file_contract.rs`

- [ ] **Step 1: Write failing archive contract tests**

Create `tests/trace_file_contract.rs`:

```rust
use chrono::Utc;
use serde_json::json;
use sparrow_agent::{
    trace::{TaskStatus, TraceEventType},
    trace_file::{
        TraceArchive, archive_file_name, default_trace_dir_from_cwd, read_trace_archive,
        safe_archive_file_path, write_trace_archive,
    },
    trace_store::TraceStore,
};

#[test]
fn archive_file_name_is_stable_and_safe() {
    assert_eq!(
        archive_file_name("task_01ABC"),
        "task_01ABC.sparrow-trace.json"
    );
    assert_eq!(
        archive_file_name("../task_01ABC"),
        "task_01ABC.sparrow-trace.json"
    );
}

#[test]
fn default_trace_dir_uses_cwd_sparrow_agent_traces() {
    assert_eq!(
        default_trace_dir_from_cwd("/Users/example/project")
            .display()
            .to_string(),
        "/Users/example/project/.sparrow_agent/traces"
    );
}

#[test]
fn safe_archive_file_path_rejects_path_traversal() {
    let dir = tempfile::tempdir().unwrap();

    assert!(safe_archive_file_path(dir.path(), "../secret.json").is_none());
    assert!(safe_archive_file_path(dir.path(), "nested/file.sparrow-trace.json").is_none());
    assert!(safe_archive_file_path(dir.path(), "task_1.json").is_none());
    assert!(
        safe_archive_file_path(dir.path(), "task_1.sparrow-trace.json")
            .unwrap()
            .ends_with("task_1.sparrow-trace.json")
    );
}

#[test]
fn write_and_read_trace_archive_round_trip_snapshot() {
    let store = TraceStore::new();
    let task = store.create_task("conv_1".into(), "msg_1".into());
    store
        .append_event(
            &task.task_id,
            TraceEventType::TaskStarted,
            json!({ "message": { "role": "user", "content": "hi" } }),
        )
        .unwrap();
    store
        .append_event(
            &task.task_id,
            TraceEventType::TaskCompleted,
            json!({ "duration_ms": 12, "final_answer": "done" }),
        )
        .unwrap();
    let dir = tempfile::tempdir().unwrap();

    let written = write_trace_archive(&store, &task.task_id, dir.path()).unwrap();
    let archive = read_trace_archive(&written).unwrap();

    assert_eq!(archive.schema_version, 1);
    assert_eq!(archive.source, "cli");
    assert_eq!(archive.task.task_id, task.task_id);
    assert_eq!(archive.task.status, TaskStatus::Succeeded);
    assert_eq!(archive.task.events.len(), 2);
}

#[test]
fn trace_archive_serializes_with_snapshot_key() {
    let store = TraceStore::new();
    let task = store.create_task("conv_1".into(), "msg_1".into());
    let snapshot = store.snapshot(&task.task_id).unwrap();
    let archive = TraceArchive {
        schema_version: 1,
        exported_at: Utc::now(),
        source: "cli".into(),
        task: snapshot,
    };

    let text = serde_json::to_string(&archive).unwrap();
    assert!(text.contains(r#""schema_version":1"#));
    assert!(text.contains(r#""source":"cli""#));
    assert!(text.contains(r#""task":"#));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test trace_file_contract
```

Expected: FAIL because `trace_file` is not exported.

- [ ] **Step 3: Export the module**

Add to `src/lib.rs`:

```rust
pub mod trace_file;
```

- [ ] **Step 4: Implement archive helpers**

Create `src/trace_file.rs`:

```rust
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::trace_store::{TaskSnapshot, TraceStore};

pub const TRACE_ARCHIVE_SCHEMA_VERSION: u32 = 1;
pub const TRACE_ARCHIVE_EXTENSION: &str = ".sparrow-trace.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceArchive {
    pub schema_version: u32,
    pub exported_at: DateTime<Utc>,
    pub source: String,
    pub task: TaskSnapshot,
}

pub fn default_trace_dir() -> Result<PathBuf> {
    if let Ok(path) = env::var("SPARROW_TRACE_DIR")
        && !path.trim().is_empty()
    {
        return Ok(PathBuf::from(path));
    }

    let cwd = env::current_dir().context("failed to read current working directory")?;
    Ok(default_trace_dir_from_cwd(cwd))
}

pub fn default_trace_dir_from_cwd(cwd: impl AsRef<Path>) -> PathBuf {
    cwd.as_ref().join(".sparrow_agent").join("traces")
}

pub fn archive_file_name(task_id: &str) -> String {
    let sanitized = task_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .collect::<String>();
    let task_id = if sanitized.is_empty() { "trace" } else { sanitized.as_str() };
    format!("{task_id}{TRACE_ARCHIVE_EXTENSION}")
}

pub fn safe_archive_file_path(trace_dir: &Path, file_name: &str) -> Option<PathBuf> {
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return None;
    }
    if !file_name.ends_with(TRACE_ARCHIVE_EXTENSION) {
        return None;
    }
    Some(trace_dir.join(file_name))
}

pub fn write_trace_archive(
    store: &TraceStore,
    task_id: &str,
    trace_dir: impl AsRef<Path>,
) -> Result<PathBuf> {
    let snapshot = store.snapshot(task_id)?;
    let archive = TraceArchive {
        schema_version: TRACE_ARCHIVE_SCHEMA_VERSION,
        exported_at: Utc::now(),
        source: "cli".into(),
        task: snapshot,
    };
    let trace_dir = trace_dir.as_ref();
    fs::create_dir_all(trace_dir)
        .with_context(|| format!("failed to create trace directory {}", trace_dir.display()))?;
    let path = trace_dir.join(archive_file_name(task_id));
    let contents = serde_json::to_string_pretty(&archive).context("failed to serialize trace")?;
    fs::write(&path, contents)
        .with_context(|| format!("failed to write trace archive {}", path.display()))?;
    Ok(path)
}

pub fn read_trace_archive(path: impl AsRef<Path>) -> Result<TraceArchive> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read trace archive {}", path.display()))?;
    let archive: TraceArchive =
        serde_json::from_str(&contents).context("failed to parse trace archive")?;
    if archive.schema_version != TRACE_ARCHIVE_SCHEMA_VERSION {
        bail!(
            "unsupported trace archive schema version {}",
            archive.schema_version
        );
    }
    Ok(archive)
}
```

- [ ] **Step 5: Run archive tests**

Run:

```bash
cargo test --test trace_file_contract
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/trace_file.rs tests/trace_file_contract.rs
git commit -m "feat: persist agent traces as archives"
```

## Task 4: Add Trace Archive HTTP Open Endpoint

**Files:**
- Modify: `src/server.rs`
- Test: `tests/server_contract.rs`

- [ ] **Step 1: Write failing archive endpoint tests**

Append these tests to `tests/server_contract.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test server_contract trace_archive
```

Expected: FAIL because `build_browser_router_with_trace_dir` and the route do not exist.

- [ ] **Step 3: Add server route and builder**

In `src/server.rs`, add imports:

```rust
use std::{path::PathBuf, sync::Arc};
use crate::trace_file::{read_trace_archive, safe_archive_file_path};
```

Extend `ServerState` with an optional trace archive directory:

```rust
#[derive(Clone)]
pub struct ServerState {
    pub config: AppConfig,
    pub conversations: Arc<ConversationStore>,
    pub traces: Arc<TraceStore>,
    pub trace_dir: Option<Arc<PathBuf>>,
}
```

Update `ServerState::new`:

```rust
impl ServerState {
    pub fn new(config: AppConfig, traces: Arc<TraceStore>) -> Self {
        Self {
            config,
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
```

Update `build_browser_router` and add `build_browser_router_with_trace_dir`:

```rust
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
    build_router(state.with_trace_dir(trace_dir))
        .route(
            "/api/agent/trace-files/{file_name}",
            get(open_trace_file),
        )
        .fallback_service(ServeDir::new(frontend_dist).fallback(ServeFile::new(index)))
}
```

Add the handler:

```rust
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
    let Some(path) = safe_archive_file_path(trace_dir, &file_name) else {
        return Err(ApiError::bad_request(
            "invalid_trace_file",
            "Trace file name is invalid.",
        ));
    };

    read_trace_archive(path)
        .map(Json)
        .map_err(|_| ApiError::not_found("trace_file_not_found", "Trace file was not found."))
}
```

- [ ] **Step 4: Run archive endpoint tests**

Run:

```bash
cargo test --test server_contract trace_archive
```

Expected: PASS.

- [ ] **Step 5: Run all server tests**

Run:

```bash
cargo test --test server_contract
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server.rs tests/server_contract.rs
git commit -m "feat: open generated trace archives"
```

## Task 5: Add CLI Observer Module

**Files:**
- Create: `src/cli_observer.rs`
- Modify: `src/lib.rs`
- Test: `tests/cli_observer_contract.rs`

- [ ] **Step 1: Write failing helper tests**

Create `tests/cli_observer_contract.rs`:

```rust
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use sparrow_agent::cli_observer::{
    browser_task_url, default_frontend_dist, inspect_addr_from_env_value, replay_trace_url,
};

#[test]
fn browser_task_url_points_to_task_route() {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787);

    assert_eq!(
        browser_task_url(addr, "task_abc"),
        "http://127.0.0.1:8787/tasks/task_abc"
    );
}

#[test]
fn replay_trace_url_points_to_replay_route() {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787);

    assert_eq!(
        replay_trace_url(addr, "task_abc.sparrow-trace.json"),
        "http://127.0.0.1:8787/replay/task_abc.sparrow-trace.json"
    );
}

#[test]
fn inspect_addr_from_env_value_uses_default_when_empty() {
    assert_eq!(
        inspect_addr_from_env_value(None).unwrap().to_string(),
        "127.0.0.1:8787"
    );
    assert_eq!(
        inspect_addr_from_env_value(Some("")).unwrap().to_string(),
        "127.0.0.1:8787"
    );
}

#[test]
fn inspect_addr_from_env_value_parses_override() {
    assert_eq!(
        inspect_addr_from_env_value(Some("127.0.0.1:9797"))
            .unwrap()
            .to_string(),
        "127.0.0.1:9797"
    );
}

#[test]
fn default_frontend_dist_points_at_frontend_dist() {
    assert!(default_frontend_dist().ends_with("frontend/dist"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test cli_observer_contract
```

Expected: FAIL because `cli_observer` is not exported.

- [ ] **Step 3: Export the module**

Add to `src/lib.rs`:

```rust
pub mod cli_observer;
```

- [ ] **Step 4: Implement CLI observer helpers and runner**

Create `src/cli_observer.rs`:

```rust
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result};

use crate::{
    agent::Agent,
    config::AppConfig,
    console::{is_exit_command, read_user_input},
    server::{ServerState, build_browser_router},
    trace::trace_id,
    trace_file::{archive_file_name, default_trace_dir, write_trace_archive},
    trace_store::{TraceStore, TraceStoreSink},
};

const DEFAULT_INSPECT_ADDR: &str = "127.0.0.1:8787";

pub fn inspect_addr_from_env_value(value: Option<&str>) -> Result<SocketAddr> {
    value
        .filter(|raw| !raw.trim().is_empty())
        .unwrap_or(DEFAULT_INSPECT_ADDR)
        .parse()
        .context("failed to parse SPARROW_INSPECT_ADDR")
}

pub fn default_frontend_dist() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("frontend/dist")
}

pub fn browser_task_url(addr: SocketAddr, task_id: &str) -> String {
    format!("http://{addr}/tasks/{task_id}")
}

pub fn replay_trace_url(addr: SocketAddr, file_name: &str) -> String {
    format!("http://{addr}/replay/{file_name}")
}

pub async fn run_cli_with_browser_trace(config: AppConfig, addr: SocketAddr) -> Result<()> {
    let traces = Arc::new(TraceStore::new());
    let state = ServerState::new(config.clone(), Arc::clone(&traces));
    let frontend_dist = default_frontend_dist();
    let trace_dir = default_trace_dir()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind inspector server to {addr}"))?;
    let local_addr = listener.local_addr()?;
    let router = build_browser_router(state, frontend_dist.clone());

    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("Inspector server failed: {error}");
        }
    });

    if !frontend_dist.join("index.html").exists() {
        eprintln!(
            "Warning: {} does not exist. Run `cd frontend && pnpm build` before opening browser task URLs.",
            frontend_dist.display()
        );
    }

    println!("Sparrow Agent ready. Type 'exit' or 'quit' to stop.");
    println!("Browser inspector listening on http://{local_addr}");

    let conversation_id = trace_id("conv");
    let mut agent = Agent::new(config).await?;

    loop {
        let context_usage_line = agent.context_usage_line();
        let Some(input) = read_user_input(">>> ", Some(&context_usage_line))? else {
            break;
        };

        if is_exit_command(&input) {
            break;
        }

        let task = traces.create_task(conversation_id.clone(), trace_id("msg"));
        println!("inspect> {}", browser_task_url(local_addr, &task.task_id));

        let sink = TraceStoreSink::new(Arc::clone(&traces), task.task_id.clone());
        let started = Instant::now();
        if let Err(error) = agent.handle_user_input_with_trace(input, &sink).await {
            if traces
                .snapshot(&task.task_id)
                .map(|snapshot| snapshot.status == crate::trace::TaskStatus::Running)
                .unwrap_or(false)
            {
                traces.mark_failed(
                    &task.task_id,
                    started.elapsed().as_millis() as u64,
                    error.to_string(),
                );
            }
            match write_trace_archive(&traces, &task.task_id, &trace_dir) {
                Ok(path) => {
                    let file_name = archive_file_name(&task.task_id);
                    println!("trace> {}", path.display());
                    println!("replay> {}", replay_trace_url(local_addr, &file_name));
                }
                Err(write_error) => eprintln!("Warning: failed to write trace archive: {write_error}"),
            }
            return Err(error);
        }

        let path = write_trace_archive(&traces, &task.task_id, &trace_dir)?;
        let file_name = archive_file_name(&task.task_id);
        println!("trace> {}", path.display());
        println!("replay> {}", replay_trace_url(local_addr, &file_name));
    }

    Ok(())
}
```

- [ ] **Step 5: Run helper tests**

Run:

```bash
cargo test --test cli_observer_contract
```

Expected: PASS.

- [ ] **Step 6: Run compiler check**

Run:

```bash
cargo check
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/cli_observer.rs tests/cli_observer_contract.rs
git commit -m "feat: add CLI browser observer"
```

## Task 6: Wire CLI Flags in Main

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace direct `std::env::args()` checks**

In `src/main.rs`, add `cli_observer::run_cli_with_browser_trace` to the import list:

```rust
use sparrow_agent::{
    agent::Agent,
    cli_observer::{inspect_addr_from_env_value, run_cli_with_browser_trace},
    config::AppConfig,
    console::{is_exit_command, read_user_input},
    server::run_server,
};
```

Then replace the current `--server` branch with:

```rust
    let args = std::env::args().collect::<Vec<_>>();

    if args.iter().any(|arg| arg == "--server") {
        let addr = std::env::var("SPARROW_SERVER_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8787".into())
            .parse()?;
        return run_server(config, addr).await;
    }

    if args
        .iter()
        .any(|arg| arg == "--inspect" || arg == "--browser-trace")
    {
        let addr = inspect_addr_from_env_value(
            std::env::var("SPARROW_INSPECT_ADDR").ok().as_deref(),
        )?;
        return run_cli_with_browser_trace(config, addr).await;
    }
```

- [ ] **Step 2: Verify existing CLI still compiles**

Run:

```bash
cargo check
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire CLI inspector flag"
```

## Task 7: Fix Frontend Task Detail Streaming for Direct Links

**Files:**
- Modify: `frontend/src/App.tsx`
- Test: `frontend/src/App.test.tsx`

- [ ] **Step 1: Add failing direct-link behavior test**

In `frontend/src/App.test.tsx`, add a helper to `FakeEventSource`:

```ts
  static lastUrl(): string | null {
    return FakeEventSource.instances.at(-1)?.url ?? null
  }
```

Add this fixture below the existing `events` array:

```ts
const cliEvents: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    timestamp: '2026-05-10T02:00:00.000Z',
    type: 'task.started',
    payload: { message: { role: 'user', content: 'hello from cli' } },
  },
  {
    seq: 2,
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    timestamp: '2026-05-10T02:00:01.000Z',
    type: 'model_call.started',
    payload: {
      node_id: 'model_cli_1',
      round: 1,
      model: 'deepseek-chat',
      request: snapshot({ messages: 2 }),
    },
  },
]
```

Add this test inside `describe('App trace visualization', () => { ... })`:

```ts
  it('streams a CLI-created task from a direct browser link', async () => {
    window.history.replaceState(null, '', '/tasks/task_cli_1')

    render(<App />)

    expect(await screen.findByRole('heading', { name: '任务详情' })).toBeInTheDocument()
    await waitFor(() => expect(FakeEventSource.lastUrl()).toBe(
      '/api/agent/tasks/task_cli_1/events?after_seq=2',
    ))
  })
```

Add this branch to `mockFetch` before the final 404 response:

```ts
  if (url === '/api/agent/tasks/task_cli_1') {
    const response: TaskSnapshot = {
      task_id: 'task_cli_1',
      conversation_id: 'conv_cli_1',
      status: 'running',
      created_at: '2026-05-10T02:00:00.000Z',
      updated_at: '2026-05-10T02:00:01.000Z',
      events: cliEvents,
    }
    return jsonResponse(response, 200)
  }
```

- [ ] **Step 2: Run the frontend test**

Run:

```bash
cd frontend && pnpm test -- App.test.tsx
```

Expected: FAIL if the current direct-link stream lifecycle does not start after snapshot application.

- [ ] **Step 3: Ensure route task id drives stream after snapshot**

In `frontend/src/App.tsx`, derive the active stream task id from the current route when present:

```tsx
  const activeTaskId = route.name === 'task' ? route.taskId : traceState.taskId
```

Then change `useTaskStream` to:

```tsx
  useTaskStream({
    taskId: activeTaskId,
    enabled: traceState.status === 'running' && Boolean(activeTaskId),
    lastSeq: traceState.lastSeq,
    onEvent: handleTraceEvent,
  })
```

This makes CLI-printed deep links reliable even when the user opens the browser after a task has already started.

- [ ] **Step 4: Run frontend tests**

Run:

```bash
cd frontend && pnpm test
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/App.tsx frontend/src/App.test.tsx
git commit -m "fix: stream direct task links"
```

## Task 8: Add Frontend Trace Archive Preview

**Files:**
- Modify: `frontend/src/types/trace.ts`
- Modify: `frontend/src/api/agentTrace.ts`
- Modify: `frontend/src/router.ts`
- Create: `frontend/src/pages/TraceArchivePage.tsx`
- Modify: `frontend/src/App.tsx`
- Test: `frontend/src/App.test.tsx`

- [ ] **Step 1: Add failing archive preview test**

In `frontend/src/App.test.tsx`, add this fixture below `cliEvents`:

```ts
const archive = {
  schema_version: 1,
  exported_at: '2026-05-10T02:00:02.000Z',
  source: 'cli',
  task: {
    task_id: 'task_cli_1',
    conversation_id: 'conv_cli_1',
    status: 'succeeded',
    created_at: '2026-05-10T02:00:00.000Z',
    updated_at: '2026-05-10T02:00:02.000Z',
    events: [
      ...cliEvents,
      {
        seq: 3,
        task_id: 'task_cli_1',
        conversation_id: 'conv_cli_1',
        timestamp: '2026-05-10T02:00:02.000Z',
        type: 'task.completed',
        payload: { duration_ms: 2000, final_answer: 'cli done' },
      },
    ],
  },
}
```

Add this test:

```ts
  it('opens a generated trace archive in preview mode', async () => {
    window.history.replaceState(null, '', '/trace-files/task_cli_1.sparrow-trace.json')

    render(<App />)

    expect(await screen.findByRole('heading', { name: 'Trace 预览' })).toBeInTheDocument()
    expect(await screen.findByText('cli done')).toBeInTheDocument()
  })
```

Add this branch to `mockFetch` before the final 404 response:

```ts
  if (url === '/api/agent/trace-files/task_cli_1.sparrow-trace.json') {
    return jsonResponse(archive, 200)
  }
```

- [ ] **Step 2: Run frontend test to verify it fails**

Run:

```bash
cd frontend && pnpm test -- App.test.tsx
```

Expected: FAIL because `/trace-files/:fileName` is not routed.

- [ ] **Step 3: Add archive types**

In `frontend/src/types/trace.ts`, add:

```ts
export type TraceArchiveSource = 'cli' | 'server' | 'imported' | string

export interface TraceArchive {
  schema_version: 1
  exported_at: string
  source: TraceArchiveSource
  task: TaskSnapshot
}
```

- [ ] **Step 4: Add archive API helpers**

In `frontend/src/api/agentTrace.ts`, update imports to include `TraceArchive`, then add:

```ts
export async function getTraceArchive(fileName: string): Promise<TraceArchive> {
  const response = await fetch(`/api/agent/trace-files/${encodeURIComponent(fileName)}`)
  return readJsonResponse<TraceArchive>(response)
}
```

- [ ] **Step 5: Add routes**

In `frontend/src/router.ts`, extend `AppRoute`:

```ts
  | { name: 'trace-file'; fileName: string }
  | { name: 'replay'; fileName: string }
```

Add matches before the task route fallback:

```ts
  const traceFileMatch = window.location.pathname.match(/^\/trace-files\/([^/]+)$/)
  if (traceFileMatch?.[1]) {
    return { name: 'trace-file', fileName: decodeURIComponent(traceFileMatch[1]) }
  }

  const replayMatch = window.location.pathname.match(/^\/replay\/([^/]+)$/)
  if (replayMatch?.[1]) {
    return { name: 'replay', fileName: decodeURIComponent(replayMatch[1]) }
  }
```

- [ ] **Step 6: Create preview page**

Create `frontend/src/pages/TraceArchivePage.tsx`:

```tsx
import { useEffect, useMemo, useState } from 'react'
import { AgentTraceApiError, getTraceArchive } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceTimeline } from '../components/TraceTimeline'
import type { TraceArchive } from '../types/trace'
import type { TraceState } from '../state/traceReducer'

interface TraceArchivePageProps {
  fileName: string
  state: TraceState
  onApplyArchive: (archive: TraceArchive) => void
  onBack: () => void
  onReplay: (fileName: string) => void
  onSelectNode: (nodeId: string) => void
}

export function TraceArchivePage({
  fileName,
  state,
  onApplyArchive,
  onBack,
  onReplay,
  onSelectNode,
}: TraceArchivePageProps) {
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    getTraceArchive(fileName)
      .then((archive) => {
        if (!cancelled) {
          onApplyArchive(archive)
        }
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(readArchiveError(caught))
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [fileName, onApplyArchive])

  useEffect(() => {
    if (state.selectedNodeId || state.rootNodeIds.length === 0) {
      return
    }
    const fallback = state.rootNodeIds.at(-1)
    if (fallback) {
      onSelectNode(fallback)
    }
  }, [onSelectNode, state.rootNodeIds, state.selectedNodeId])

  const selectedNode = useMemo(
    () => (state.selectedNodeId ? state.nodesById[state.selectedNodeId] ?? null : null),
    [state.nodesById, state.selectedNodeId],
  )

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b border-slate-300 pb-4">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">Trace 预览</h1>
            <p className="mt-1 text-sm text-slate-600">{fileName}</p>
          </div>
          <div className="flex gap-2">
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={() => onReplay(fileName)} type="button">
              回放
            </button>
            <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={onBack} type="button">
              返回聊天
            </button>
          </div>
        </div>
        {isLoading ? <LoadingInline label="正在加载 trace 文件" /> : null}
        {error ? <div className="rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">{error}</div> : null}
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={onSelectNode} state={state} />
          <TraceDetailPanel node={selectedNode} />
        </div>
      </div>
    </main>
  )
}

function readArchiveError(error: unknown): string {
  if (error instanceof AgentTraceApiError) {
    return error.message
  }
  return '无法加载 trace 文件。'
}
```

- [ ] **Step 7: Wire archive preview in App**

In `frontend/src/App.tsx`, import `TraceArchivePage`, `TraceArchive`, and add:

```tsx
  const applyArchive = useCallback((archive: TraceArchive) => {
    setTraceState(applyTraceSnapshot(createInitialTraceState(), archive.task))
    setConversationId(archive.task.conversation_id)
  }, [])
```

Add route handling before task route:

```tsx
  if (route.name === 'trace-file') {
    return (
      <TraceArchivePage
        fileName={route.fileName}
        onApplyArchive={applyArchive}
        onBack={() => navigateTo('/')}
        onReplay={(fileName) => navigateTo(`/replay/${encodeURIComponent(fileName)}`)}
        onSelectNode={selectNode}
        state={traceState}
      />
    )
  }
```

- [ ] **Step 8: Run preview test**

Run:

```bash
cd frontend && pnpm test -- App.test.tsx
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/types/trace.ts frontend/src/api/agentTrace.ts frontend/src/router.ts frontend/src/pages/TraceArchivePage.tsx frontend/src/App.tsx frontend/src/App.test.tsx
git commit -m "feat: preview trace archives"
```

## Task 9: Add Frontend Trace Replay

**Files:**
- Create: `frontend/src/hooks/useTraceReplay.ts`
- Test: `frontend/src/hooks/useTraceReplay.test.tsx`
- Create: `frontend/src/components/TraceReplayControls.tsx`
- Create: `frontend/src/pages/TraceReplayPage.tsx`
- Modify: `frontend/src/App.tsx`
- Test: `frontend/src/App.test.tsx`

- [ ] **Step 1: Write failing replay hook tests**

Create `frontend/src/hooks/useTraceReplay.test.tsx`:

```tsx
import { renderHook, act } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useTraceReplay } from './useTraceReplay'
import type { TraceEvent } from '../types/trace'

const events: TraceEvent[] = [
  {
    seq: 1,
    task_id: 'task_1',
    conversation_id: 'conv_1',
    timestamp: '2026-05-10T00:00:00.000Z',
    type: 'task.started',
    payload: { message: { role: 'user', content: 'hi' } },
  },
  {
    seq: 2,
    task_id: 'task_1',
    conversation_id: 'conv_1',
    timestamp: '2026-05-10T00:00:01.000Z',
    type: 'task.completed',
    payload: { duration_ms: 1000, final_answer: 'done' },
  },
]

describe('useTraceReplay', () => {
  it('steps through events manually', () => {
    const received: TraceEvent[] = []
    const { result } = renderHook(() => useTraceReplay({ events, onEvent: (event) => received.push(event) }))

    act(() => result.current.step())
    act(() => result.current.step())
    act(() => result.current.step())

    expect(received.map((event) => event.seq)).toEqual([1, 2])
    expect(result.current.currentIndex).toBe(2)
    expect(result.current.isComplete).toBe(true)
  })

  it('plays events with fake timers', () => {
    vi.useFakeTimers()
    const received: TraceEvent[] = []
    const { result } = renderHook(() => useTraceReplay({ events, onEvent: (event) => received.push(event), intervalMs: 100 }))

    act(() => result.current.play())
    act(() => vi.advanceTimersByTime(250))

    expect(received.map((event) => event.seq)).toEqual([1, 2])
    expect(result.current.isPlaying).toBe(false)
    vi.useRealTimers()
  })
})
```

- [ ] **Step 2: Run replay hook tests to verify they fail**

Run:

```bash
cd frontend && pnpm test -- useTraceReplay.test.tsx
```

Expected: FAIL because the hook does not exist.

- [ ] **Step 3: Implement replay hook**

Create `frontend/src/hooks/useTraceReplay.ts`:

```ts
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { TraceEvent } from '../types/trace'

interface UseTraceReplayOptions {
  events: TraceEvent[]
  onEvent: (event: TraceEvent) => void
  intervalMs?: number
}

export function useTraceReplay({ events, onEvent, intervalMs = 600 }: UseTraceReplayOptions) {
  const sortedEvents = useMemo(
    () => events.slice().sort((left, right) => left.seq - right.seq),
    [events],
  )
  const [currentIndex, setCurrentIndex] = useState(0)
  const [isPlaying, setIsPlaying] = useState(false)
  const onEventRef = useRef(onEvent)

  useEffect(() => {
    onEventRef.current = onEvent
  }, [onEvent])

  const step = useCallback(() => {
    setCurrentIndex((index) => {
      const event = sortedEvents[index]
      if (!event) {
        return index
      }
      onEventRef.current(event)
      return index + 1
    })
  }, [sortedEvents])

  const restart = useCallback(() => {
    setCurrentIndex(0)
    setIsPlaying(false)
  }, [])

  useEffect(() => {
    if (!isPlaying) {
      return
    }
    if (currentIndex >= sortedEvents.length) {
      setIsPlaying(false)
      return
    }
    const timer = window.setTimeout(step, intervalMs)
    return () => window.clearTimeout(timer)
  }, [currentIndex, intervalMs, isPlaying, sortedEvents.length, step])

  return {
    currentIndex,
    total: sortedEvents.length,
    isComplete: currentIndex >= sortedEvents.length,
    isPlaying,
    play: () => setIsPlaying(true),
    pause: () => setIsPlaying(false),
    restart,
    step,
  }
}
```

- [ ] **Step 4: Implement controls**

Create `frontend/src/components/TraceReplayControls.tsx`:

```tsx
interface TraceReplayControlsProps {
  currentIndex: number
  total: number
  isComplete: boolean
  isPlaying: boolean
  onPause: () => void
  onPlay: () => void
  onRestart: () => void
  onStep: () => void
}

export function TraceReplayControls({
  currentIndex,
  total,
  isComplete,
  isPlaying,
  onPause,
  onPlay,
  onRestart,
  onStep,
}: TraceReplayControlsProps) {
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-slate-300 pb-4">
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={isPlaying ? onPause : onPlay} type="button">
        {isPlaying ? '暂停' : '播放'}
      </button>
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" disabled={isComplete} onClick={onStep} type="button">
        单步
      </button>
      <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={onRestart} type="button">
        重来
      </button>
      <div className="min-w-40 text-sm text-slate-600">
        {currentIndex} / {total}
      </div>
    </div>
  )
}
```

- [ ] **Step 5: Implement replay page**

Create `frontend/src/pages/TraceReplayPage.tsx`:

```tsx
import { useCallback, useEffect, useMemo, useState } from 'react'
import { getTraceArchive } from '../api/agentTrace'
import { LoadingInline } from '../components/LoadingInline'
import { TraceDetailPanel } from '../components/TraceDetailPanel'
import { TraceReplayControls } from '../components/TraceReplayControls'
import { TraceTimeline } from '../components/TraceTimeline'
import { useTraceReplay } from '../hooks/useTraceReplay'
import { applyTraceEvent, createInitialTraceState, type TraceState } from '../state/traceReducer'
import type { TraceArchive, TraceEvent } from '../types/trace'

interface TraceReplayPageProps {
  fileName: string
  onBack: () => void
}

export function TraceReplayPage({ fileName, onBack }: TraceReplayPageProps) {
  const [archive, setArchive] = useState<TraceArchive | null>(null)
  const [state, setState] = useState<TraceState>(() => createInitialTraceState())
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    getTraceArchive(fileName)
      .then((loaded) => {
        if (!cancelled) {
          setArchive(loaded)
          setState(createInitialTraceState())
        }
      })
      .catch(() => {
        if (!cancelled) {
          setError('无法加载 trace 文件。')
        }
      })
    return () => {
      cancelled = true
    }
  }, [fileName])

  const handleReplayEvent = useCallback((event: TraceEvent) => {
    setState((current) => applyTraceEvent(current, event))
  }, [])

  const replay = useTraceReplay({
    events: archive?.task.events ?? [],
    onEvent: handleReplayEvent,
  })

  const restart = useCallback(() => {
    setState(createInitialTraceState())
    replay.restart()
  }, [replay])

  const selectedNode = useMemo(
    () => (state.selectedNodeId ? state.nodesById[state.selectedNodeId] ?? null : null),
    [state.nodesById, state.selectedNodeId],
  )

  return (
    <main className="min-h-dvh bg-slate-50">
      <div className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="text-2xl font-semibold text-slate-950">Trace 回放</h1>
            <p className="mt-1 text-sm text-slate-600">{fileName}</p>
          </div>
          <button className="h-9 rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700" onClick={onBack} type="button">
            返回聊天
          </button>
        </div>
        <TraceReplayControls
          currentIndex={replay.currentIndex}
          isComplete={replay.isComplete}
          isPlaying={replay.isPlaying}
          onPause={replay.pause}
          onPlay={replay.play}
          onRestart={restart}
          onStep={replay.step}
          total={replay.total}
        />
        {!archive && !error ? <LoadingInline label="正在加载 trace 文件" /> : null}
        {error ? <div className="mt-4 rounded-md border border-red-300 bg-red-50 p-4 text-sm text-red-700">{error}</div> : null}
        <div className="mt-5 grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]">
          <TraceTimeline onSelectNode={(nodeId) => setState((current) => ({ ...current, selectedNodeId: nodeId }))} state={state} />
          <TraceDetailPanel node={selectedNode} />
        </div>
      </div>
    </main>
  )
}
```

- [ ] **Step 6: Wire replay route in App**

In `frontend/src/App.tsx`, import `TraceReplayPage`, then add:

```tsx
  if (route.name === 'replay') {
    return <TraceReplayPage fileName={route.fileName} onBack={() => navigateTo('/')} />
  }
```

- [ ] **Step 7: Add replay route test**

In `frontend/src/App.test.tsx`, add:

```ts
  it('opens a generated trace archive in replay mode', async () => {
    window.history.replaceState(null, '', '/replay/task_cli_1.sparrow-trace.json')

    render(<App />)

    expect(await screen.findByRole('heading', { name: 'Trace 回放' })).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: '播放' })).toBeInTheDocument()
  })
```

- [ ] **Step 8: Run frontend tests**

Run:

```bash
cd frontend && pnpm test
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/hooks/useTraceReplay.ts frontend/src/hooks/useTraceReplay.test.tsx frontend/src/components/TraceReplayControls.tsx frontend/src/pages/TraceReplayPage.tsx frontend/src/App.tsx frontend/src/App.test.tsx
git commit -m "feat: replay trace archives"
```

## Task 10: Document CLI Browser Trace Mode

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a usage section**

Add this under `## CLI 模式` after the existing streaming explanation:

```markdown
### CLI + 浏览器观察模式

如果希望仍然从 CLI 输入任务，但在浏览器里查看每一轮 agent loop，可先构建前端，然后使用观察模式启动：

```bash
cd frontend
pnpm install
pnpm build
cd ..
cargo run -- --inspect
```

也可以使用等价参数：

```bash
cargo run -- --browser-trace
```

观察服务默认监听 `127.0.0.1:8787`，可通过 `SPARROW_INSPECT_ADDR` 覆盖：

```bash
SPARROW_INSPECT_ADDR=127.0.0.1:9797 cargo run -- --inspect
```

每次在 CLI 输入一条消息后，终端会先打印本轮实时任务地址：

```text
inspect> http://127.0.0.1:8787/tasks/task_01...
```

打开该地址即可查看本轮 loop 的模型调用、reasoning 增量、工具调用、工具输出和最终回答。任务完成或失败后，CLI 会将完整 trace 写入 `SPARROW_TRACE_DIR` 指定目录；如果未设置该变量，默认写入运行目录下的 `.sparrow_agent/traces`：

```text
trace> /Users/me/project/.sparrow_agent/traces/task_01....sparrow-trace.json
replay> http://127.0.0.1:8787/replay/task_01....sparrow-trace.json
```

打开 `replay>` 地址可在前端按事件顺序回放完整 trace；将路径中的 `/replay/` 改成 `/trace-files/` 可直接预览最终状态。
```

- [ ] **Step 2: Add environment variable row**

In the environment variable table, add:

```markdown
| `SPARROW_INSPECT_ADDR` | CLI 浏览器观察模式监听地址 | `127.0.0.1:8787` |
| `SPARROW_TRACE_DIR` | CLI 观察模式完成后写入 trace 文件的目录 | `<运行目录>/.sparrow_agent/traces` |
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: describe CLI browser trace mode"
```

## Task 11: End-to-End Verification

**Files:**
- No source files modified.

- [ ] **Step 1: Run Rust tests**

Run:

```bash
cargo test
```

Expected: PASS.

- [ ] **Step 2: Run frontend tests**

Run:

```bash
cd frontend && pnpm test
```

Expected: PASS.

- [ ] **Step 3: Build frontend assets**

Run:

```bash
cd frontend && pnpm build
```

Expected: PASS and `frontend/dist/index.html` exists.

- [ ] **Step 4: Smoke test CLI observer**

Run:

```bash
SPARROW_FILESYSTEM_ENABLED=false cargo run -- --inspect
```

At the CLI prompt, enter:

```text
hello
```

Expected terminal behavior:

```text
Browser inspector listening on http://127.0.0.1:8787
inspect> http://127.0.0.1:8787/tasks/task_...
trace> /Users/.../sparrow_agent/.sparrow_agent/traces/task_....sparrow-trace.json
replay> http://127.0.0.1:8787/replay/task_....sparrow-trace.json
```

Open the printed URL in a browser.

Expected browser behavior:

- The task detail page loads without a 404.
- The timeline shows `任务开始`, at least one `模型调用`, one `模型输出`, and `任务完成`.
- The detail panel shows the final answer for a completed task.
- The printed trace file exists on disk and contains `"schema_version": 1`.
- The printed replay URL opens the `Trace 回放` page.
- Clicking `播放` or `单步` advances the timeline event by event.

- [ ] **Step 5: Verify API snapshot manually**

Run with the printed task id:

```bash
curl -s http://127.0.0.1:8787/api/agent/tasks/<printed-task-id>
```

Expected: JSON contains:

```json
{
  "task_id": "<printed-task-id>",
  "status": "succeeded",
  "events": []
}
```

The real `events` array should not be empty; it should include `task.started`, `model_call.started`, `model_call.completed`, and `task.completed`.

- [ ] **Step 6: Verify generated trace archive manually**

Run with the printed trace file name:

```bash
curl -s http://127.0.0.1:8787/api/agent/trace-files/<printed-file-name>
```

Expected: JSON contains:

```json
{
  "schema_version": 1,
  "source": "cli",
  "task": {
    "status": "succeeded",
    "events": []
  }
}
```

The real `task.events` array should not be empty.

- [ ] **Step 7: Final commit if verification caused doc/test fixes**

```bash
git status --short
git add Cargo.toml src/lib.rs src/server.rs src/trace_file.rs src/cli_observer.rs src/main.rs tests/server_contract.rs tests/trace_file_contract.rs tests/cli_observer_contract.rs frontend/src/types/trace.ts frontend/src/api/agentTrace.ts frontend/src/router.ts frontend/src/pages/TraceArchivePage.tsx frontend/src/pages/TraceReplayPage.tsx frontend/src/hooks/useTraceReplay.ts frontend/src/hooks/useTraceReplay.test.tsx frontend/src/components/TraceReplayControls.tsx frontend/src/App.tsx frontend/src/App.test.tsx README.md
git commit -m "feat: inspect and replay CLI agent traces"
```

Skip this commit if every earlier task was already committed and `git status --short` is clean.

## Self-Review

Spec coverage:

- CLI 调用 agent: covered by `--inspect` / `--browser-trace` preserving terminal REPL.
- 浏览器查询 loop 过程: covered by printed `/tasks/:task_id` URL, shared `TraceStore`, snapshot API, SSE API, and frontend static serving.
- Agent 完成后生成 trace 文件: covered by `TraceArchive`, `write_trace_archive`, `SPARROW_TRACE_DIR`, and CLI `trace>` output.
- Frontend 预览打开 trace 文件: covered by `GET /api/agent/trace-files/:file_name` and `/trace-files/:fileName`.
- Frontend 回放 trace: covered by `/replay/:fileName`, `useTraceReplay`, and `TraceReplayControls`.
- Loop 过程 includes model rounds, reasoning deltas, model outputs, tool calls, tool results, terminal status: covered by existing traced agent path and existing frontend reducer.
- Non-invasive compatibility: existing `cargo run` and `cargo run -- --server` remain unchanged.

Placeholder scan:

- No `TBD`, `TODO`, or "implement later" items are left.
- Every code change task includes concrete files, snippets, commands, and expected outcomes.

Type consistency:

- `TraceStore`, `TraceStoreSink`, `TraceArchive`, `ServerState`, `build_browser_router`, and `build_browser_router_with_trace_dir` names match existing modules or are introduced before use.
- `SPARROW_INSPECT_ADDR` is parsed through `inspect_addr_from_env_value`.
- `SPARROW_TRACE_DIR` is read by `trace_file::default_trace_dir`.
- Browser live route uses existing React `/tasks/:task_id` route and existing `/api/agent/tasks/:task_id/events` endpoint.
- Browser archive routes use `/trace-files/:fileName`, `/replay/:fileName`, and `/api/agent/trace-files/:file_name`.
