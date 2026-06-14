use std::sync::Arc;

use sparrow_agent::{
    api::{FunctionCall, ToolCall},
    context::{MemoryDelta, MemoryScope},
    memory_tool::{MemoryToolProvider, new_memory_buffer},
    tool_provider::ToolProvider,
};

fn update_call(arguments: &str) -> ToolCall {
    ToolCall {
        id: "call_1".into(),
        kind: "function".into(),
        function: FunctionCall {
            name: "updateMemory".into(),
            arguments: arguments.into(),
        },
    }
}

#[tokio::test]
async fn queues_valid_add_fact_delta() {
    let buffer = new_memory_buffer();
    let provider = MemoryToolProvider::new(Arc::clone(&buffer));

    let result = provider
        .execute(&update_call(r#"{"operation":"add_fact","content":"the sky is blue"}"#))
        .await
        .unwrap()
        .unwrap();

    assert!(
        result.starts_with("Memory update queued: add_fact"),
        "unexpected ack: {result}"
    );

    let queued = buffer.lock().unwrap();
    assert_eq!(queued.len(), 1);
    match &queued[0] {
        MemoryDelta::AddFact { content } => assert_eq!(content, "the sky is blue"),
        other => panic!("expected AddFact, got {other:?}"),
    }
}

#[tokio::test]
async fn clear_defaults_to_all_scope() {
    let buffer = new_memory_buffer();
    let provider = MemoryToolProvider::new(Arc::clone(&buffer));

    provider
        .execute(&update_call(r#"{"operation":"clear"}"#))
        .await
        .unwrap();

    match &buffer.lock().unwrap()[0] {
        MemoryDelta::Clear { scope } => assert_eq!(*scope, MemoryScope::All),
        other => panic!("expected Clear, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_unknown_operation_without_queueing() {
    let buffer = new_memory_buffer();
    let provider = MemoryToolProvider::new(Arc::clone(&buffer));

    let result = provider
        .execute(&update_call(r#"{"operation":"teleport"}"#))
        .await;

    assert!(result.is_err(), "unknown operation must error");
    assert!(
        buffer.lock().unwrap().is_empty(),
        "nothing should be queued on error"
    );
}

#[tokio::test]
async fn exposes_single_update_memory_definition() {
    let provider = MemoryToolProvider::new(new_memory_buffer());
    let definitions = provider.definitions();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].function.name, "updateMemory");
}
