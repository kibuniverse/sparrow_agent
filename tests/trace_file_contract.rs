use std::fs;

use chrono::Utc;
use serde_json::{Value, json};
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

    let raw = fs::read_to_string(&written).unwrap();
    let raw_json: Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(raw_json["schema_version"], 2);
    assert_eq!(raw_json["compression"]["original_event_count"], 2);
    assert_eq!(raw_json["compression"]["compact_event_count"], 2);
    assert_eq!(raw.lines().count(), 1);
    assert_eq!(archive.schema_version, 2);
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

#[test]
fn read_trace_archive_supports_legacy_v1_archive() {
    let store = TraceStore::new();
    let task = store.create_task("conv_1".into(), "msg_1".into());
    let archive = TraceArchive {
        schema_version: 1,
        exported_at: Utc::now(),
        source: "cli".into(),
        task: store.snapshot(&task.task_id).unwrap(),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.sparrow-trace.json");
    fs::write(&path, serde_json::to_string(&archive).unwrap()).unwrap();

    let read = read_trace_archive(&path).unwrap();

    assert_eq!(read.schema_version, 1);
    assert_eq!(read.task.task_id, task.task_id);
}
