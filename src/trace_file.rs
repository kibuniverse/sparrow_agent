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
