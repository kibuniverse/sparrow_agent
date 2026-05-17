use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sparrow_agent::{
    trace::{JsonSnapshot, TaskStatus, TraceEvent, TraceEventType},
    trace_compaction::{compact_archive, compact_events, expand_archive_v2},
    trace_file::TraceArchive,
    trace_store::TaskSnapshot,
};

#[test]
fn compact_events_merges_consecutive_reasoning_delta_for_same_model_call() {
    let events = vec![
        event(
            1,
            TraceEventType::ModelCallReasoningDelta,
            json!({ "node_id": "model_1", "delta": "hel" }),
        ),
        event(
            2,
            TraceEventType::ModelCallReasoningDelta,
            json!({ "node_id": "model_1", "delta": "lo" }),
        ),
    ];

    let compacted = compact_events(&events);

    assert_eq!(compacted.event_runs, 1);
    assert_eq!(compacted.events.len(), 1);
    assert_eq!(compacted.events[0].seq, 2);
    assert_eq!(compacted.events[0].payload["delta"], "hello");
    assert_eq!(compacted.events[0].payload["_compact"]["kind"], "event_run");
    assert_eq!(compacted.events[0].payload["_compact"]["count"], 2);
    assert_eq!(compacted.events[0].payload["_compact"]["seq_start"], 1);
    assert_eq!(compacted.events[0].payload["_compact"]["seq_end"], 2);
}

#[test]
fn compact_events_does_not_merge_across_node_or_lifecycle_boundary() {
    let events = vec![
        event(
            1,
            TraceEventType::ModelCallReasoningDelta,
            json!({ "node_id": "model_1", "delta": "a" }),
        ),
        event(
            2,
            TraceEventType::ModelCallReasoningDelta,
            json!({ "node_id": "model_2", "delta": "b" }),
        ),
        event(
            3,
            TraceEventType::ModelCallCompleted,
            json!({
                "node_id": "model_1",
                "duration_ms": 1,
                "finish_reason": "stop",
                "usage": null,
                "response": JsonSnapshot::from_value(json!({ "message": null }), 64 * 1024),
            }),
        ),
        event(
            4,
            TraceEventType::ModelCallReasoningDelta,
            json!({ "node_id": "model_1", "delta": "c" }),
        ),
    ];

    let compacted = compact_events(&events);

    assert_eq!(compacted.event_runs, 0);
    assert_eq!(compacted.events.len(), 4);
    assert!(
        compacted
            .events
            .iter()
            .all(|event| event.payload.get("_compact").is_none())
    );
}

#[test]
fn compact_events_merges_final_answer_output_delta() {
    let events = vec![
        event(
            1,
            TraceEventType::ModelOutputDelta,
            json!({ "node_id": "output_1", "kind": "final_answer", "content_delta": "Final " }),
        ),
        event(
            2,
            TraceEventType::ModelOutputDelta,
            json!({ "node_id": "output_1", "kind": "final_answer", "content_delta": "answer" }),
        ),
    ];

    let compacted = compact_events(&events);

    assert_eq!(compacted.event_runs, 1);
    assert_eq!(compacted.events.len(), 1);
    assert_eq!(compacted.events[0].payload["content_delta"], "Final answer");
    assert_eq!(compacted.events[0].payload["_compact"]["count"], 2);
}

#[test]
fn compact_events_keeps_tool_call_argument_streams_separate_by_index() {
    let events = vec![
        event(
            1,
            TraceEventType::ModelOutputDelta,
            json!({
                "node_id": "output_1",
                "kind": "tool_calls",
                "tool_call": {
                    "index": 0,
                    "tool_call_id": "call_1",
                    "name": "read_file",
                    "arguments_delta": "{\"path\""
                }
            }),
        ),
        event(
            2,
            TraceEventType::ModelOutputDelta,
            json!({
                "node_id": "output_1",
                "kind": "tool_calls",
                "tool_call": {
                    "index": 0,
                    "tool_call_id": "call_1",
                    "name": "read_file",
                    "arguments_delta": ":\"Cargo.toml\"}"
                }
            }),
        ),
        event(
            3,
            TraceEventType::ModelOutputDelta,
            json!({
                "node_id": "output_1",
                "kind": "tool_calls",
                "tool_call": {
                    "index": 1,
                    "tool_call_id": "call_2",
                    "name": "list_files",
                    "arguments_delta": "{}"
                }
            }),
        ),
    ];

    let compacted = compact_events(&events);

    assert_eq!(compacted.event_runs, 1);
    assert_eq!(compacted.events.len(), 2);
    assert_eq!(
        compacted.events[0].payload["tool_call"]["arguments_delta"],
        "{\"path\":\"Cargo.toml\"}"
    );
    assert_eq!(compacted.events[0].payload["_compact"]["count"], 2);
    assert_eq!(compacted.events[1].payload["tool_call"]["index"], 1);
}

#[test]
fn request_snapshot_delta_reconstructs_append_only_messages() {
    let first_request = request_snapshot(vec![json!({
        "role": "user",
        "content": "x".repeat(3000),
    })]);
    let second_request = request_snapshot(vec![
        json!({
            "role": "user",
            "content": "x".repeat(3000),
        }),
        json!({
            "role": "assistant",
            "content": "hello",
        }),
    ]);
    let archive = archive_with_events(vec![
        model_call_started(1, "model_1", first_request.clone()),
        model_call_started(2, "model_2", second_request.clone()),
    ]);

    let compact = compact_archive(archive.clone()).unwrap();

    assert_eq!(compact.compression.snapshot_keyframes, 1);
    assert_eq!(compact.compression.snapshot_delta_frames, 1);
    assert_eq!(
        compact.task.events[1].payload["request"]["encoding"],
        "snapshot-diff/v1"
    );

    let expanded = expand_archive_v2(compact).unwrap();
    assert_eq!(
        expanded.task.events[1].payload["request"]["value"],
        archive.task.events[1].payload["request"]["value"]
    );
}

#[test]
fn request_snapshot_delta_falls_back_to_keyframe_when_patch_is_not_smaller() {
    let first_request = request_snapshot(vec![json!({
        "role": "user",
        "content": "x",
    })]);
    let second_request = request_snapshot(vec![json!({
        "role": "user",
        "content": "y",
    })]);
    let archive = archive_with_events(vec![
        model_call_started(1, "model_1", first_request),
        model_call_started(2, "model_2", second_request),
    ]);

    let compact = compact_archive(archive).unwrap();

    assert_eq!(compact.compression.snapshot_keyframes, 2);
    assert_eq!(compact.compression.snapshot_delta_frames, 0);
    assert_eq!(
        compact.task.events[1].payload["request"]["encoding"],
        "snapshot-keyframe/v1"
    );
}

fn archive_with_events(events: Vec<TraceEvent>) -> TraceArchive {
    TraceArchive {
        schema_version: 1,
        exported_at: timestamp(0),
        source: "cli".into(),
        task: TaskSnapshot {
            task_id: "task_1".into(),
            conversation_id: "conv_1".into(),
            status: TaskStatus::Running,
            created_at: timestamp(0),
            updated_at: timestamp(events.len() as u64),
            events,
        },
    }
}

fn model_call_started(seq: u64, node_id: &str, request: JsonSnapshot) -> TraceEvent {
    event(
        seq,
        TraceEventType::ModelCallStarted,
        json!({
            "node_id": node_id,
            "round": seq,
            "model": "deepseek-chat",
            "request": request,
        }),
    )
}

fn request_snapshot(messages: Vec<Value>) -> JsonSnapshot {
    JsonSnapshot::from_value(
        json!({
            "model": "deepseek-chat",
            "message_count": messages.len(),
            "messages": messages,
            "tool_count": 0,
            "thinking": null,
            "reasoning_effort": null,
        }),
        64 * 1024,
    )
}

fn event(seq: u64, event_type: TraceEventType, payload: Value) -> TraceEvent {
    TraceEvent {
        seq,
        task_id: "task_1".into(),
        conversation_id: "conv_1".into(),
        timestamp: timestamp(seq),
        event_type,
        payload,
    }
}

fn timestamp(offset_seconds: u64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("2026-05-16T10:00:{offset_seconds:02}Z"))
        .unwrap()
        .with_timezone(&Utc)
}
