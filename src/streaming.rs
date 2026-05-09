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
    Started,
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
                self.phase = Phase::Started;
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

        if let Some(content) = &delta.content
            && !content.is_empty()
        {
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
                    id: builder.id.clone(),
                    name: builder.name.clone(),
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

#[cfg(test)]
mod tests {
    use super::{AgentEventSink, AgentStreamEvent, StreamAccumulator};
    use crate::api::{
        ChatCompletionStreamChunk, ChoiceDelta, FunctionCallDelta, StreamChoice, ToolCallDelta,
    };

    #[derive(Default)]
    struct RecordingSink {
        events: Vec<String>,
    }

    impl AgentEventSink for RecordingSink {
        fn on_event(&mut self, event: &AgentStreamEvent) -> anyhow::Result<()> {
            match event {
                AgentStreamEvent::ResponseStarted { round } => {
                    self.events.push(format!("response_started:{round}"));
                }
                AgentStreamEvent::ReasoningStarted => {
                    self.events.push("reasoning_started".into());
                }
                AgentStreamEvent::ReasoningDelta(text) => {
                    self.events.push(format!("reasoning:{text}"));
                }
                AgentStreamEvent::AnswerStarted => {
                    self.events.push("answer_started".into());
                }
                AgentStreamEvent::AnswerDelta(text) => {
                    self.events.push(format!("answer:{text}"));
                }
                AgentStreamEvent::ToolCallDelta {
                    index,
                    name,
                    arguments_delta,
                    ..
                } => {
                    self.events.push(format!(
                        "tool:{index}:{}:{}",
                        name.as_deref().unwrap_or("<none>"),
                        arguments_delta.as_deref().unwrap_or("")
                    ));
                }
                AgentStreamEvent::ResponseFinished { finish_reason } => {
                    self.events.push(format!(
                        "finished:{}",
                        finish_reason.as_deref().unwrap_or("<none>")
                    ));
                }
                AgentStreamEvent::Usage(_) => {
                    self.events.push("usage".into());
                }
            }

            Ok(())
        }
    }

    #[test]
    fn emits_reasoning_started_after_role_only_chunk() {
        let mut accumulator = StreamAccumulator::new();
        let mut sink = RecordingSink::default();

        accumulator
            .push(chunk(delta_with_role("assistant"), None), &mut sink, 0)
            .unwrap();
        accumulator
            .push(
                chunk(delta_with_reasoning("Let me think."), None),
                &mut sink,
                0,
            )
            .unwrap();

        assert_eq!(
            sink.events,
            vec![
                "response_started:0",
                "reasoning_started",
                "reasoning:Let me think.",
            ]
        );
    }

    #[test]
    fn includes_accumulated_tool_name_in_argument_delta_events() {
        let mut accumulator = StreamAccumulator::new();
        let mut sink = RecordingSink::default();

        accumulator
            .push(
                chunk(delta_with_tool_name("call_1", "list_directory"), None),
                &mut sink,
                0,
            )
            .unwrap();
        accumulator
            .push(
                chunk(delta_with_tool_arguments("{\"path\":\"/tmp\"}"), None),
                &mut sink,
                0,
            )
            .unwrap();

        assert_eq!(
            sink.events,
            vec![
                "response_started:0",
                "tool:0:list_directory:",
                "tool:0:list_directory:{\"path\":\"/tmp\"}",
            ]
        );
    }

    #[test]
    fn ignores_empty_content_deltas() {
        let mut accumulator = StreamAccumulator::new();
        let mut sink = RecordingSink::default();

        accumulator
            .push(chunk(delta_with_content(""), None), &mut sink, 0)
            .unwrap();
        accumulator
            .push(chunk(delta_with_content("done"), None), &mut sink, 0)
            .unwrap();

        assert_eq!(
            sink.events,
            vec!["response_started:0", "answer_started", "answer:done"]
        );
    }

    fn chunk(delta: ChoiceDelta, finish_reason: Option<&str>) -> ChatCompletionStreamChunk {
        ChatCompletionStreamChunk {
            id: None,
            object: None,
            created: None,
            model: None,
            choices: vec![StreamChoice {
                index: 0,
                delta,
                finish_reason: finish_reason.map(str::to_string),
            }],
            usage: None,
        }
    }

    fn empty_delta() -> ChoiceDelta {
        ChoiceDelta {
            role: None,
            content: None,
            reasoning_content: None,
            tool_calls: None,
        }
    }

    fn delta_with_role(role: &str) -> ChoiceDelta {
        ChoiceDelta {
            role: Some(role.into()),
            ..empty_delta()
        }
    }

    fn delta_with_reasoning(reasoning: &str) -> ChoiceDelta {
        ChoiceDelta {
            reasoning_content: Some(reasoning.into()),
            ..empty_delta()
        }
    }

    fn delta_with_content(content: &str) -> ChoiceDelta {
        ChoiceDelta {
            content: Some(content.into()),
            ..empty_delta()
        }
    }

    fn delta_with_tool_name(id: &str, name: &str) -> ChoiceDelta {
        ChoiceDelta {
            tool_calls: Some(vec![ToolCallDelta {
                index: 0,
                id: Some(id.into()),
                kind: Some("function".into()),
                function: Some(FunctionCallDelta {
                    name: Some(name.into()),
                    arguments: None,
                }),
            }]),
            ..empty_delta()
        }
    }

    fn delta_with_tool_arguments(arguments: &str) -> ChoiceDelta {
        ChoiceDelta {
            tool_calls: Some(vec![ToolCallDelta {
                index: 0,
                id: None,
                kind: None,
                function: Some(FunctionCallDelta {
                    name: None,
                    arguments: Some(arguments.into()),
                }),
            }]),
            ..empty_delta()
        }
    }
}
