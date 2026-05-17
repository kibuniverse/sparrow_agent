use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    trace::{DEFAULT_SNAPSHOT_MAX_BYTES, JsonSnapshot, TaskStatus, TraceEvent, TraceEventType},
    trace_file::TraceArchive,
    trace_store::TaskSnapshot,
};

pub const TRACE_ARCHIVE_V2_SCHEMA_VERSION: u32 = 2;

const REQUEST_DIFF_ENCODING: &str = "snapshot-diff/v1";
const REQUEST_KEYFRAME_ENCODING: &str = "snapshot-keyframe/v1";
const REQUEST_KEYFRAME_INTERVAL: usize = 8;
const REQUEST_DIFF_MAX_FULL_RATIO_NUMERATOR: usize = 75;
const REQUEST_DIFF_MAX_FULL_RATIO_DENOMINATOR: usize = 100;
const REQUEST_DIFF_MIN_SAVINGS_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceArchiveV2 {
    pub schema_version: u32,
    pub exported_at: DateTime<Utc>,
    pub source: String,
    pub compression: TraceCompressionMeta,
    pub task: CompactTaskSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceCompressionMeta {
    pub original_event_count: usize,
    pub compact_event_count: usize,
    pub event_runs: usize,
    pub snapshot_keyframes: usize,
    pub snapshot_delta_frames: usize,
    pub minified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactTaskSnapshot {
    pub task_id: String,
    pub conversation_id: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone)]
pub struct CompactEventsResult {
    pub events: Vec<TraceEvent>,
    pub event_runs: usize,
}

#[derive(Debug, Default)]
struct SnapshotCompactionStats {
    keyframes: usize,
    delta_frames: usize,
}

#[derive(Debug, Clone)]
struct RequestBase {
    node_id: String,
    value: Value,
    hash: String,
}

pub fn compact_archive(archive: TraceArchive) -> Result<TraceArchiveV2> {
    let TraceArchive {
        exported_at,
        source,
        task,
        ..
    } = archive;
    let TaskSnapshot {
        task_id,
        conversation_id,
        status,
        created_at,
        updated_at,
        events,
    } = task;

    let original_event_count = events.len();
    let compacted_events = compact_events(&events);
    let mut task = CompactTaskSnapshot {
        task_id,
        conversation_id,
        status,
        created_at,
        updated_at,
        events: compacted_events.events,
    };
    let snapshot_stats = compact_request_snapshots(&mut task.events)?;

    Ok(TraceArchiveV2 {
        schema_version: TRACE_ARCHIVE_V2_SCHEMA_VERSION,
        exported_at,
        source,
        compression: TraceCompressionMeta {
            original_event_count,
            compact_event_count: task.events.len(),
            event_runs: compacted_events.event_runs,
            snapshot_keyframes: snapshot_stats.keyframes,
            snapshot_delta_frames: snapshot_stats.delta_frames,
            minified: true,
        },
        task,
    })
}

pub fn expand_archive_v2(archive: TraceArchiveV2) -> Result<TraceArchive> {
    if archive.schema_version != TRACE_ARCHIVE_V2_SCHEMA_VERSION {
        bail!(
            "unsupported compact trace archive schema version {}",
            archive.schema_version
        );
    }

    let mut request_bases = HashMap::new();
    let mut events = Vec::with_capacity(archive.task.events.len());

    for mut event in archive.task.events {
        if event.event_type == TraceEventType::ModelCallStarted {
            expand_model_request_snapshot(&mut event, &mut request_bases)?;
        }
        events.push(event);
    }

    Ok(TraceArchive {
        schema_version: archive.schema_version,
        exported_at: archive.exported_at,
        source: archive.source,
        task: TaskSnapshot {
            task_id: archive.task.task_id,
            conversation_id: archive.task.conversation_id,
            status: archive.task.status,
            created_at: archive.task.created_at,
            updated_at: archive.task.updated_at,
            events,
        },
    })
}

pub fn compact_events(events: &[TraceEvent]) -> CompactEventsResult {
    let mut compacted = Vec::with_capacity(events.len());
    let mut run: Option<EventRun> = None;
    let mut event_runs = 0;

    for event in events {
        if let Some(candidate) = MergeCandidate::from_event(event) {
            match run.as_mut() {
                Some(active) if active.can_absorb(&candidate) => active.absorb(event, candidate),
                Some(_) => {
                    flush_event_run(&mut compacted, &mut run, &mut event_runs);
                    run = Some(EventRun::new(event, candidate));
                }
                None => run = Some(EventRun::new(event, candidate)),
            }
        } else {
            flush_event_run(&mut compacted, &mut run, &mut event_runs);
            compacted.push(event.clone());
        }
    }

    flush_event_run(&mut compacted, &mut run, &mut event_runs);

    CompactEventsResult {
        events: compacted,
        event_runs,
    }
}

fn compact_request_snapshots(events: &mut [TraceEvent]) -> Result<SnapshotCompactionStats> {
    let mut stats = SnapshotCompactionStats::default();
    let mut previous = None;
    let mut frames_since_keyframe = 0usize;

    for event in events {
        if event.event_type != TraceEventType::ModelCallStarted {
            continue;
        }

        let Some(node_id) = event
            .payload
            .get("node_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(request_value) = event.payload.get("request").cloned() else {
            continue;
        };
        let snapshot: JsonSnapshot = serde_json::from_value(request_value)
            .context("failed to parse model request snapshot")?;

        let mut encoded = None;
        if frames_since_keyframe < REQUEST_KEYFRAME_INTERVAL
            && let Some(base) = previous.as_ref()
        {
            encoded = build_request_delta_frame(base, &snapshot)?;
        }

        if let Some(frame) = encoded {
            set_payload_field(&mut event.payload, "request", frame)?;
            stats.delta_frames += 1;
            frames_since_keyframe += 1;
        } else {
            set_payload_field(&mut event.payload, "request", request_keyframe(&snapshot)?)?;
            stats.keyframes += 1;
            frames_since_keyframe = 0;
        }

        previous = Some(RequestBase {
            node_id,
            value: snapshot.value.clone(),
            hash: snapshot_hash(&snapshot.value)?,
        });
    }

    Ok(stats)
}

fn expand_model_request_snapshot(
    event: &mut TraceEvent,
    request_bases: &mut HashMap<String, RequestBase>,
) -> Result<()> {
    let Some(node_id) = event
        .payload
        .get("node_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Ok(());
    };
    let Some(request_value) = event.payload.get("request").cloned() else {
        return Ok(());
    };

    let snapshot = decode_request_snapshot(&request_value, request_bases)?;
    let encoded_snapshot =
        serde_json::to_value(&snapshot).context("failed to encode expanded request snapshot")?;
    set_payload_field(&mut event.payload, "request", encoded_snapshot)?;
    request_bases.insert(
        node_id.clone(),
        RequestBase {
            node_id,
            hash: snapshot_hash(&snapshot.value)?,
            value: snapshot.value,
        },
    );

    Ok(())
}

fn decode_request_snapshot(
    value: &Value,
    request_bases: &HashMap<String, RequestBase>,
) -> Result<JsonSnapshot> {
    let Some(encoding) = value.get("encoding").and_then(Value::as_str) else {
        return serde_json::from_value(value.clone())
            .context("failed to parse canonical request snapshot");
    };

    match encoding {
        REQUEST_KEYFRAME_ENCODING => {
            let snapshot: JsonSnapshot = serde_json::from_value(
                value
                    .get("snapshot")
                    .cloned()
                    .context("request keyframe missing snapshot")?,
            )
            .context("failed to parse request keyframe snapshot")?;
            verify_target_hash(value, &snapshot.value)?;
            Ok(snapshot)
        }
        REQUEST_DIFF_ENCODING => decode_request_delta(value, request_bases),
        other => bail!("unsupported request snapshot encoding {other}"),
    }
}

fn decode_request_delta(
    frame: &Value,
    request_bases: &HashMap<String, RequestBase>,
) -> Result<JsonSnapshot> {
    let base_node_id = frame
        .get("base_node_id")
        .and_then(Value::as_str)
        .context("request delta missing base_node_id")?;
    let base = request_bases
        .get(base_node_id)
        .with_context(|| format!("request delta base snapshot not found: {base_node_id}"))?;
    let expected_base_hash = frame
        .get("base_hash")
        .and_then(Value::as_str)
        .context("request delta missing base_hash")?;
    if base.hash != expected_base_hash {
        bail!("request delta base hash mismatch for {base_node_id}");
    }

    let base_messages = request_messages(&base.value).context("base request missing messages")?;
    let ops = frame
        .get("ops")
        .and_then(Value::as_array)
        .context("request delta missing ops")?;
    let mut messages = Vec::new();
    let mut consumed_base = 0usize;

    for op in ops {
        let op_name = op
            .get("op")
            .and_then(Value::as_str)
            .context("request delta op missing name")?;
        match op_name {
            "retain_prefix" => {
                let count = value_as_usize(op.get("count"), "retain_prefix.count")?;
                if count > base_messages.len() {
                    bail!("request delta retain_prefix exceeds base messages");
                }
                messages = base_messages[..count].to_vec();
                consumed_base = count;
            }
            "append" => {
                let appended = op
                    .get("messages")
                    .and_then(Value::as_array)
                    .context("request delta append missing messages")?;
                messages.extend(appended.iter().cloned());
            }
            "replace_range" => {
                let start = value_as_usize(op.get("start"), "replace_range.start")?;
                let delete = value_as_usize(op.get("delete"), "replace_range.delete")?;
                if start > base_messages.len() || start + delete > base_messages.len() {
                    bail!("request delta replace_range exceeds base messages");
                }
                messages = base_messages[..start].to_vec();
                consumed_base = start + delete;
                let replacement = op
                    .get("messages")
                    .and_then(Value::as_array)
                    .context("request delta replace_range missing messages")?;
                messages.extend(replacement.iter().cloned());
            }
            other => bail!("unsupported request delta op {other}"),
        }
    }

    if consumed_base < base_messages.len() && messages.len() < base_messages.len() {
        messages.extend(base_messages[consumed_base..].iter().cloned());
    }

    let mut value = base.value.clone();
    let Value::Object(map) = &mut value else {
        bail!("base request snapshot is not an object");
    };
    map.insert("message_count".into(), json!(messages.len()));
    map.insert("messages".into(), Value::Array(messages));

    let expected_count = value_as_usize(frame.get("message_count"), "message_count")?;
    let actual_count = request_messages(&value)
        .map(|messages| messages.len())
        .unwrap_or_default();
    if actual_count != expected_count {
        bail!("request delta message count mismatch");
    }
    verify_target_hash(frame, &value)?;

    Ok(JsonSnapshot::from_value(value, DEFAULT_SNAPSHOT_MAX_BYTES))
}

fn build_request_delta_frame(base: &RequestBase, target: &JsonSnapshot) -> Result<Option<Value>> {
    if request_shell(&base.value) != request_shell(&target.value) {
        return Ok(None);
    }

    let Some(base_messages) = request_messages(&base.value) else {
        return Ok(None);
    };
    let Some(target_messages) = request_messages(&target.value) else {
        return Ok(None);
    };

    let common_prefix = base_messages
        .iter()
        .zip(target_messages)
        .take_while(|(base, target)| base == target)
        .count();
    let mut ops = Vec::new();
    if common_prefix > 0 {
        ops.push(json!({ "op": "retain_prefix", "count": common_prefix }));
    }
    if common_prefix == base_messages.len() {
        if target_messages.len() > common_prefix {
            ops.push(json!({
                "op": "append",
                "messages": target_messages[common_prefix..],
            }));
        }
    } else {
        ops.push(json!({
            "op": "replace_range",
            "start": common_prefix,
            "delete": base_messages.len() - common_prefix,
            "messages": target_messages[common_prefix..],
        }));
    }

    let full_bytes = serde_json::to_vec(target)
        .context("failed to measure request snapshot")?
        .len();
    let mut frame = json!({
        "encoding": REQUEST_DIFF_ENCODING,
        "base_node_id": base.node_id,
        "target_hash": snapshot_hash(&target.value)?,
        "base_hash": base.hash,
        "message_count": target_messages.len(),
        "ops": ops,
        "stats": {
            "full_bytes": full_bytes,
            "patch_bytes": 0,
        },
    });
    let patch_bytes = serde_json::to_vec(&frame)
        .context("failed to measure request delta frame")?
        .len();
    frame["stats"]["patch_bytes"] = json!(patch_bytes);
    let patch_bytes = serde_json::to_vec(&frame)
        .context("failed to measure request delta frame")?
        .len();
    frame["stats"]["patch_bytes"] = json!(patch_bytes);

    let too_large = patch_bytes * REQUEST_DIFF_MAX_FULL_RATIO_DENOMINATOR
        > full_bytes * REQUEST_DIFF_MAX_FULL_RATIO_NUMERATOR;
    let too_little_savings =
        full_bytes.saturating_sub(patch_bytes) < REQUEST_DIFF_MIN_SAVINGS_BYTES;
    if too_large || too_little_savings {
        return Ok(None);
    }

    Ok(Some(frame))
}

fn request_keyframe(snapshot: &JsonSnapshot) -> Result<Value> {
    Ok(json!({
        "encoding": REQUEST_KEYFRAME_ENCODING,
        "target_hash": snapshot_hash(&snapshot.value)?,
        "message_count": request_messages(&snapshot.value)
            .map(|messages| messages.len())
            .unwrap_or_default(),
        "snapshot": snapshot,
    }))
}

fn verify_target_hash(frame: &Value, value: &Value) -> Result<()> {
    let expected = frame
        .get("target_hash")
        .and_then(Value::as_str)
        .context("request snapshot frame missing target_hash")?;
    let actual = snapshot_hash(value)?;
    if actual != expected {
        bail!("request snapshot target hash mismatch");
    }
    Ok(())
}

fn request_shell(value: &Value) -> Option<Value> {
    let mut shell = value.clone();
    let Value::Object(map) = &mut shell else {
        return None;
    };
    map.remove("message_count");
    map.remove("messages");
    Some(shell)
}

fn request_messages(value: &Value) -> Option<&Vec<Value>> {
    value.get("messages").and_then(Value::as_array)
}

fn snapshot_hash(value: &Value) -> Result<String> {
    let bytes =
        serde_json::to_vec(value).context("failed to serialize request snapshot for hash")?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("sha256:{}", hex_bytes(&hasher.finalize())))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn value_as_usize(value: Option<&Value>, field: &str) -> Result<usize> {
    let value = value.with_context(|| format!("request delta missing {field}"))?;
    let number = value
        .as_u64()
        .with_context(|| format!("request delta {field} is not an unsigned integer"))?;
    usize::try_from(number).with_context(|| format!("request delta {field} is too large"))
}

fn set_payload_field(payload: &mut Value, field: &str, value: Value) -> Result<()> {
    let Value::Object(map) = payload else {
        bail!("trace event payload is not an object");
    };
    map.insert(field.into(), value);
    Ok(())
}

fn flush_event_run(
    compacted: &mut Vec<TraceEvent>,
    run: &mut Option<EventRun>,
    event_runs: &mut usize,
) {
    if let Some(run) = run.take() {
        if run.count > 1 {
            *event_runs += 1;
        }
        compacted.push(run.into_event());
    }
}

#[derive(Debug, Clone)]
struct EventRun {
    first_event: TraceEvent,
    last_event: TraceEvent,
    candidate: MergeCandidate,
    seq_start: u64,
    timestamp_start: DateTime<Utc>,
    count: usize,
    delta: String,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
}

impl EventRun {
    fn new(event: &TraceEvent, candidate: MergeCandidate) -> Self {
        Self {
            first_event: event.clone(),
            last_event: event.clone(),
            seq_start: event.seq,
            timestamp_start: event.timestamp,
            count: 1,
            delta: candidate.delta.clone(),
            tool_call_id: candidate.tool_call_id.clone(),
            tool_name: candidate.tool_name.clone(),
            candidate,
        }
    }

    fn can_absorb(&self, candidate: &MergeCandidate) -> bool {
        self.candidate.same_stream(candidate)
            && optional_values_compatible(&self.tool_call_id, &candidate.tool_call_id)
            && optional_values_compatible(&self.tool_name, &candidate.tool_name)
    }

    fn absorb(&mut self, event: &TraceEvent, candidate: MergeCandidate) {
        self.last_event = event.clone();
        self.count += 1;
        self.delta.push_str(&candidate.delta);
        if self.tool_call_id.is_none() {
            self.tool_call_id = candidate.tool_call_id.clone();
        }
        if self.tool_name.is_none() {
            self.tool_name = candidate.tool_name.clone();
        }
    }

    fn into_event(self) -> TraceEvent {
        if self.count == 1 {
            return self.first_event;
        }

        let mut event = self.last_event;
        match &self.candidate.kind {
            MergeKind::Reasoning => {
                event.payload["delta"] = Value::String(self.delta);
            }
            MergeKind::FinalAnswer => {
                event.payload["content_delta"] = Value::String(self.delta);
            }
            MergeKind::ToolCall { .. } => {
                event.payload["tool_call"]["arguments_delta"] = Value::String(self.delta);
                event.payload["tool_call"]["tool_call_id"] =
                    self.tool_call_id.map(Value::String).unwrap_or(Value::Null);
                event.payload["tool_call"]["name"] =
                    self.tool_name.map(Value::String).unwrap_or(Value::Null);
            }
        }

        event.payload["_compact"] = json!({
            "kind": "event_run",
            "count": self.count,
            "seq_start": self.seq_start,
            "seq_end": event.seq,
            "timestamp_start": self.timestamp_start,
            "timestamp_end": event.timestamp,
        });
        event
    }
}

#[derive(Debug, Clone)]
struct MergeCandidate {
    kind: MergeKind,
    node_id: String,
    delta: String,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
}

impl MergeCandidate {
    fn from_event(event: &TraceEvent) -> Option<Self> {
        match event.event_type {
            TraceEventType::ModelCallReasoningDelta => {
                let node_id = event.payload.get("node_id")?.as_str()?.to_owned();
                let delta = event.payload.get("delta")?.as_str()?.to_owned();
                if delta.is_empty() {
                    return None;
                }
                Some(Self {
                    kind: MergeKind::Reasoning,
                    node_id,
                    delta,
                    tool_call_id: None,
                    tool_name: None,
                })
            }
            TraceEventType::ModelOutputDelta => {
                let node_id = event.payload.get("node_id")?.as_str()?.to_owned();
                match event.payload.get("kind")?.as_str()? {
                    "final_answer" => {
                        let delta = event.payload.get("content_delta")?.as_str()?.to_owned();
                        if delta.is_empty() {
                            return None;
                        }
                        Some(Self {
                            kind: MergeKind::FinalAnswer,
                            node_id,
                            delta,
                            tool_call_id: None,
                            tool_name: None,
                        })
                    }
                    "tool_calls" => {
                        let tool_call = event.payload.get("tool_call")?;
                        let index = tool_call.get("index")?.as_u64()?;
                        let delta = tool_call.get("arguments_delta")?.as_str()?.to_owned();
                        if delta.is_empty() {
                            return None;
                        }
                        Some(Self {
                            kind: MergeKind::ToolCall { index },
                            node_id,
                            delta,
                            tool_call_id: optional_string(tool_call.get("tool_call_id")),
                            tool_name: optional_string(tool_call.get("name")),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn same_stream(&self, other: &Self) -> bool {
        self.kind == other.kind && self.node_id == other.node_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MergeKind {
    Reasoning,
    FinalAnswer,
    ToolCall { index: u64 },
}

fn optional_values_compatible(left: &Option<String>, right: &Option<String>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
