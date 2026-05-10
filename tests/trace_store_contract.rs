use serde_json::json;
use sparrow_agent::{
    trace::{TaskStatus, TraceEventType},
    trace_store::TraceStore,
};

#[test]
fn trace_store_replays_events_after_sequence() {
    let store = TraceStore::new();
    let task = store.create_task("conv_1".into(), "msg_1".into());

    store
        .append_event(
            &task.task_id,
            TraceEventType::TaskStarted,
            json!({ "message": "hi" }),
        )
        .unwrap();
    store
        .append_event(
            &task.task_id,
            TraceEventType::ModelCallStarted,
            json!({ "node_id": "model_1" }),
        )
        .unwrap();

    let (replay, _rx) = store.subscribe(&task.task_id, 1).unwrap();

    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].seq, 2);
    assert_eq!(replay[0].event_type, TraceEventType::ModelCallStarted);
}

#[test]
fn trace_store_marks_succeeded_from_terminal_event() {
    let store = TraceStore::new();
    let task = store.create_task("conv_1".into(), "msg_1".into());

    store
        .append_event(
            &task.task_id,
            TraceEventType::TaskCompleted,
            json!({ "duration_ms": 7, "final_answer": "done" }),
        )
        .unwrap();

    let snapshot = store.snapshot(&task.task_id).unwrap();
    assert_eq!(snapshot.status, TaskStatus::Succeeded);
    assert_eq!(snapshot.events[0].seq, 1);
}

#[test]
fn trace_store_fails_task_when_event_limit_is_exceeded() {
    let store = TraceStore::with_max_events(1);
    let task = store.create_task("conv_1".into(), "msg_1".into());

    store
        .append_event(&task.task_id, TraceEventType::TaskStarted, json!({}))
        .unwrap();
    let failed = store
        .append_event(&task.task_id, TraceEventType::ModelCallStarted, json!({}))
        .unwrap();

    assert_eq!(failed.event_type, TraceEventType::TaskFailed);
    assert_eq!(
        store.snapshot(&task.task_id).unwrap().status,
        TaskStatus::Failed
    );
}
