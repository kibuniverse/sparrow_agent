use std::collections::BTreeMap;

use crate::api::{
    ChatCompletionStreamChunk, ChoiceDelta, ChoiceMessage, FunctionCall, StreamChoice, ToolCall,
    ToolCallDelta, Usage,
};

// ── Agent stream events ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum AgentStreamEvent {
    ResponseStarted {
        round: usize,
    },
    ReasoningStarted,
    ReasoningDelta(String),
    AnswerStarted,
    AnswerDelta(String),
    ToolCallDelta {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: Option<String>,
    },
    ResponseFinished {
        finish_reason: Option<String>,
    },
    Usage(Usage),
}

/// Receives streaming events for display or forwarding.
pub trait AgentEventSink {
    fn on_event(&mut self, event: &AgentStreamEvent) -> anyhow::Result<()>;
}

// ── Completed stream result ───────────────────────────────────────────

pub struct CompletedStreamResponse {
    pub message: ChoiceMessage,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

// ── Tool call accumulator ─────────────────────────────────────────────

struct ToolCallBuilder {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ToolCallBuilder {
    fn new() -> Self {
        Self {
            id: None,
            kind: None,
            name: None,
            arguments: String::new(),
        }
    }

    fn build(self) -> anyhow::Result<ToolCall> {
        let id = self
            .id
            .ok_or_else(|| anyhow::anyhow!("streamed tool call missing id"))?;
        let name = self
            .name
            .ok_or_else(|| anyhow::anyhow!("streamed tool call missing name"))?;
        Ok(ToolCall {
            id,
            kind: self.kind.unwrap_or_else(|| "function".into()),
            function: FunctionCall {
                name,
                arguments: self.arguments,
            },
        })
    }
}

// ── Stream accumulator ────────────────────────────────────────────────

enum Phase {
    NotStarted,
    Reasoning,
    Answer,
    Done,
}

pub struct StreamAccumulator {
    role: Option<String>,
    content: String,
    reasoning_content: String,
    tool_calls: BTreeMap<u32, ToolCallBuilder>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    phase: Phase,
}

impl StreamAccumulator {
    pub fn new() -> Self {
        Self {
            role: None,
            content: String::new(),
            reasoning_content: String::new(),
            tool_calls: BTreeMap::new(),
            finish_reason: None,
            usage: None,
            phase: Phase::NotStarted,
        }
    }

    pub fn push(
        &mut self,
        chunk: ChatCompletionStreamChunk,
        sink: &mut dyn AgentEventSink,
        round: usize,
    ) -> anyhow::Result<()> {
        if let Some(usage) = &chunk.usage {
            self.usage = Some(usage.clone());
            sink.on_event(&AgentStreamEvent::Usage(usage.clone()))?;
        }

        for StreamChoice {
            index: _,
            delta,
            finish_reason,
        } in chunk.choices
        {
            if matches!(self.phase, Phase::NotStarted) {
                sink.on_event(&AgentStreamEvent::ResponseStarted { round })?;
                self.phase = Phase::Reasoning;
            }

            if let Some(role) = &delta.role {
                self.role = Some(role.clone());
            }

            self.process_delta(delta, sink)?;

            if let Some(fr) = finish_reason {
                self.finish_reason = Some(fr);
            }
        }

        Ok(())
    }

    fn process_delta(
        &mut self,
        delta: ChoiceDelta,
        sink: &mut dyn AgentEventSink,
    ) -> anyhow::Result<()> {
        if let Some(reasoning) = &delta.reasoning_content {
            if !matches!(self.phase, Phase::Reasoning) {
                sink.on_event(&AgentStreamEvent::ReasoningStarted)?;
                self.phase = Phase::Reasoning;
            }
            sink.on_event(&AgentStreamEvent::ReasoningDelta(reasoning.clone()))?;
            self.reasoning_content.push_str(reasoning);
        }

        if let Some(content) = &delta.content {
            if !matches!(self.phase, Phase::Answer) {
                sink.on_event(&AgentStreamEvent::AnswerStarted)?;
                self.phase = Phase::Answer;
            }
            sink.on_event(&AgentStreamEvent::AnswerDelta(content.clone()))?;
            self.content.push_str(content);
        }

        if let Some(tool_call_deltas) = delta.tool_calls {
            for ToolCallDelta {
                index,
                id,
                kind,
                function,
            } in tool_call_deltas
            {
                let builder = self
                    .tool_calls
                    .entry(index)
                    .or_insert_with(ToolCallBuilder::new);

                let event_id = id.clone();
                let event_args = function.as_ref().and_then(|f| f.arguments.clone());

                if let Some(ref id) = id {
                    builder.id = Some(id.clone());
                }
                if let Some(kind) = kind {
                    builder.kind = Some(kind);
                }
                if let Some(ref func) = function {
                    if let Some(ref name) = func.name {
                        builder.name = Some(name.clone());
                    }
                    if let Some(ref args) = func.arguments {
                        builder.arguments.push_str(args);
                    }
                }

                sink.on_event(&AgentStreamEvent::ToolCallDelta {
                    index,
                    id: event_id,
                    name: None,
                    arguments_delta: event_args,
                })?;
            }
        }

        Ok(())
    }

    pub fn finish(
        mut self,
        sink: &mut dyn AgentEventSink,
    ) -> anyhow::Result<CompletedStreamResponse> {
        self.phase = Phase::Done;
        sink.on_event(&AgentStreamEvent::ResponseFinished {
            finish_reason: self.finish_reason.clone(),
        })?;

        let tool_calls = if self.tool_calls.is_empty() {
            None
        } else {
            let mut calls = Vec::new();
            for (_, builder) in self.tool_calls {
                calls.push(builder.build()?);
            }
            Some(calls)
        };

        let reasoning_content = if self.reasoning_content.is_empty() {
            None
        } else {
            Some(self.reasoning_content)
        };

        let content = if self.content.is_empty() && tool_calls.is_none() {
            None
        } else {
            Some(self.content)
        };

        let message = ChoiceMessage {
            role: self.role.unwrap_or_else(|| "assistant".into()),
            content,
            reasoning_content,
            tool_calls,
        };

        Ok(CompletedStreamResponse {
            message,
            finish_reason: self.finish_reason,
            usage: self.usage,
        })
    }
}
