use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::trace::{TaskStatus, TraceEvent, TraceEventType, TraceSink, trace_id};

const DEFAULT_MAX_EVENTS_PER_TASK: usize = 10_000;
const BROADCAST_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTaskHandle {
    pub task_id: String,
    pub conversation_id: String,
    pub client_message_id: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task_id: String,
    pub conversation_id: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub events: Vec<TraceEvent>,
}

pub struct TraceStore {
    tasks: Mutex<HashMap<String, StoredTask>>,
    max_events_per_task: usize,
}

struct StoredTask {
    task_id: String,
    conversation_id: String,
    client_message_id: String,
    status: TaskStatus,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    events: Vec<TraceEvent>,
    next_seq: u64,
    tx: broadcast::Sender<TraceEvent>,
}

impl TraceStore {
    pub fn new() -> Self {
        Self::with_max_events(DEFAULT_MAX_EVENTS_PER_TASK)
    }

    pub fn with_max_events(max_events_per_task: usize) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            max_events_per_task,
        }
    }

    pub fn create_task(
        &self,
        conversation_id: String,
        client_message_id: String,
    ) -> StoredTaskHandle {
        let mut tasks = self.tasks.lock().expect("trace store lock poisoned");

        if let Some(existing) = tasks
            .values()
            .find(|task| task.client_message_id == client_message_id)
        {
            return existing.handle();
        }

        let now = Utc::now();
        let task_id = trace_id("task");
        let (tx, _rx) = broadcast::channel(BROADCAST_CHANNEL_CAPACITY);
        let task = StoredTask {
            task_id: task_id.clone(),
            conversation_id,
            client_message_id,
            status: TaskStatus::Running,
            created_at: now,
            updated_at: now,
            events: Vec::new(),
            next_seq: 1,
            tx,
        };
        let handle = task.handle();
        tasks.insert(task_id, task);
        handle
    }

    pub fn find_by_client_message_id(&self, client_message_id: &str) -> Option<StoredTaskHandle> {
        let tasks = self.tasks.lock().expect("trace store lock poisoned");
        tasks
            .values()
            .find(|task| task.client_message_id == client_message_id)
            .map(StoredTask::handle)
    }

    pub fn append_event(
        &self,
        task_id: &str,
        event_type: TraceEventType,
        payload: Value,
    ) -> Result<TraceEvent> {
        let mut tasks = self.tasks.lock().expect("trace store lock poisoned");
        let Some(task) = tasks.get_mut(task_id) else {
            bail!("trace task not found: {task_id}");
        };

        let (event_type, payload) = if task.events.len() >= self.max_events_per_task
            && event_type != TraceEventType::TaskFailed
        {
            (
                TraceEventType::TaskFailed,
                json!({
                    "duration_ms": 0,
                    "error": "trace event limit exceeded",
                }),
            )
        } else {
            (event_type, payload)
        };

        let now = Utc::now();
        let event = TraceEvent {
            seq: task.next_seq,
            task_id: task.task_id.clone(),
            conversation_id: task.conversation_id.clone(),
            timestamp: now,
            event_type,
            payload,
        };

        task.next_seq += 1;
        task.updated_at = now;
        task.status = next_status(task.status, event.event_type);
        task.events.push(event.clone());
        let _ = task.tx.send(event.clone());

        Ok(event)
    }

    pub fn snapshot(&self, task_id: &str) -> Result<TaskSnapshot> {
        let tasks = self.tasks.lock().expect("trace store lock poisoned");
        let Some(task) = tasks.get(task_id) else {
            bail!("trace task not found: {task_id}");
        };

        Ok(task.snapshot())
    }

    pub fn subscribe(
        &self,
        task_id: &str,
        after_seq: u64,
    ) -> Result<(Vec<TraceEvent>, broadcast::Receiver<TraceEvent>)> {
        let tasks = self.tasks.lock().expect("trace store lock poisoned");
        let Some(task) = tasks.get(task_id) else {
            bail!("trace task not found: {task_id}");
        };

        let rx = task.tx.subscribe();
        let replay = task
            .events
            .iter()
            .filter(|event| event.seq > after_seq)
            .cloned()
            .collect();

        Ok((replay, rx))
    }

    pub fn mark_failed(&self, task_id: &str, duration_ms: u64, error: impl Into<String>) {
        let _ = self.append_event(
            task_id,
            TraceEventType::TaskFailed,
            json!({
                "duration_ms": duration_ms,
                "error": error.into(),
            }),
        );
    }

    pub fn mark_succeeded(&self, task_id: &str, duration_ms: u64, final_answer: impl Into<String>) {
        let _ = self.append_event(
            task_id,
            TraceEventType::TaskCompleted,
            json!({
                "duration_ms": duration_ms,
                "final_answer": final_answer.into(),
            }),
        );
    }
}

impl Default for TraceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl StoredTask {
    fn handle(&self) -> StoredTaskHandle {
        StoredTaskHandle {
            task_id: self.task_id.clone(),
            conversation_id: self.conversation_id.clone(),
            client_message_id: self.client_message_id.clone(),
            status: self.status,
        }
    }

    fn snapshot(&self) -> TaskSnapshot {
        TaskSnapshot {
            task_id: self.task_id.clone(),
            conversation_id: self.conversation_id.clone(),
            status: self.status,
            created_at: self.created_at,
            updated_at: self.updated_at,
            events: self.events.clone(),
        }
    }
}

#[derive(Clone)]
pub struct TraceStoreSink {
    store: Arc<TraceStore>,
    task_id: String,
}

impl TraceStoreSink {
    pub fn new(store: Arc<TraceStore>, task_id: impl Into<String>) -> Self {
        Self {
            store,
            task_id: task_id.into(),
        }
    }
}

impl TraceSink for TraceStoreSink {
    fn emit(&self, event_type: TraceEventType, payload: Value) {
        let _ = self.store.append_event(&self.task_id, event_type, payload);
    }
}

fn next_status(current: TaskStatus, event_type: TraceEventType) -> TaskStatus {
    match event_type {
        TraceEventType::TaskCompleted => TaskStatus::Succeeded,
        TraceEventType::TaskFailed => TaskStatus::Failed,
        _ => current,
    }
}
