use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Instant};

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
                Err(write_error) => {
                    eprintln!("Warning: failed to write trace archive: {write_error}")
                }
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
