use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    api::ChatMessage,
    context::{
        budget::{ContextBudget, ContextPolicy, TokenEstimator},
        memory::WorkingMemory,
        summary::fallback_turn_summary,
        transcript::{ConversationTranscript, RetentionClass},
    },
};

#[derive(Debug, Clone, Default)]
pub struct CompileInput;

#[derive(Debug, Clone)]
pub struct CompiledContext {
    pub messages: Vec<ChatMessage>,
    pub report: ContextCompileReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCompileReport {
    pub estimated_prompt_tokens: usize,
    pub target_prompt_tokens: usize,
    pub reserved_completion_tokens: usize,
    pub included_recent_turns: usize,
    pub summarized_turns: usize,
    pub included_tool_exchanges: usize,
    pub summarized_tool_exchanges: usize,
    pub reasoning_policy: String,
    pub preserved_reasoning_messages: usize,
    pub summarized_reasoning_messages: usize,
    pub warnings: Vec<String>,
}

pub struct ContextCompiler {
    policy: ContextPolicy,
    estimator: TokenEstimator,
}

impl ContextCompiler {
    pub fn new(policy: ContextPolicy, estimator: TokenEstimator) -> Self {
        Self { policy, estimator }
    }

    pub fn compile(
        &self,
        transcript: &mut ConversationTranscript,
        memory: &WorkingMemory,
        model: &str,
        _input: CompileInput,
    ) -> Result<CompiledContext> {
        let budget = ContextBudget::for_model(model, &self.policy);

        if !self.policy.enabled {
            let messages = transcript.to_legacy_messages();
            validate_protocol_safe_messages(&messages)?;
            let report = ContextCompileReport {
                estimated_prompt_tokens: self.estimator.estimate_messages(&messages),
                target_prompt_tokens: budget.target_prompt_tokens,
                reserved_completion_tokens: budget.reserved_completion_tokens,
                included_recent_turns: transcript.turns().len(),
                summarized_turns: 0,
                included_tool_exchanges: transcript
                    .turns()
                    .iter()
                    .map(|turn| turn.tool_exchange_count())
                    .sum(),
                summarized_tool_exchanges: 0,
                reasoning_policy: "legacy".into(),
                preserved_reasoning_messages: transcript
                    .turns()
                    .iter()
                    .map(|turn| turn.reasoning_message_count())
                    .sum(),
                summarized_reasoning_messages: 0,
                warnings: Vec::new(),
            };
            return Ok(CompiledContext { messages, report });
        }

        let turn_count = transcript.turns().len();
        let min_full_start = turn_count.saturating_sub(1);
        let mut full_start = turn_count.saturating_sub(self.policy.recent_turns.max(1));
        let mut pack = self.build_pack(transcript, memory, &budget, full_start, true, true);

        while pack.report.estimated_prompt_tokens > budget.target_prompt_tokens
            && full_start < min_full_start
        {
            full_start += 1;
            pack = self.build_pack(transcript, memory, &budget, full_start, true, true);
        }

        if pack.report.estimated_prompt_tokens > budget.target_prompt_tokens && pack.has_summary {
            pack = self.build_pack(transcript, memory, &budget, full_start, true, false);
            pack.report.warnings.push(
                "history summaries were omitted because the current turn exceeded the target budget"
                    .into(),
            );
        }

        if pack.report.estimated_prompt_tokens > budget.target_prompt_tokens && pack.has_memory {
            pack = self.build_pack(transcript, memory, &budget, full_start, false, false);
            pack.report.warnings.push(
                "working memory was omitted because the current turn exceeded the target budget"
                    .into(),
            );
        }

        if pack.report.estimated_prompt_tokens > budget.target_prompt_tokens {
            pack.report.warnings.push(format!(
                "compiled context is over target budget: estimated={} target={}",
                pack.report.estimated_prompt_tokens, budget.target_prompt_tokens
            ));
        }

        validate_protocol_safe_messages(&pack.messages)?;

        Ok(CompiledContext {
            messages: pack.messages,
            report: pack.report,
        })
    }

    fn build_pack(
        &self,
        transcript: &mut ConversationTranscript,
        memory: &WorkingMemory,
        budget: &ContextBudget,
        full_start: usize,
        include_memory: bool,
        include_summary: bool,
    ) -> BuiltPack {
        let mut messages = vec![ChatMessage::system(transcript.system_prompt().to_string())];
        let mut has_memory = false;
        let mut has_summary = false;
        let turn_count = transcript.turns().len();
        let full_start = full_start.min(turn_count);

        if include_memory
            && let Some(memory_message) =
                memory.to_context_message(&self.estimator, self.policy.max_memory_tokens)
        {
            has_memory = true;
            messages.push(memory_message);
        }

        let summarized_turns = full_start;
        let summarized_tool_exchanges: usize = transcript.turns()[..full_start]
            .iter()
            .map(|turn| turn.tool_exchange_count())
            .sum();
        let summarized_reasoning_messages: usize = transcript.turns()[..full_start]
            .iter()
            .map(|turn| turn.reasoning_message_count())
            .sum();

        if include_summary && full_start > 0 {
            let summary = self.history_summary_message(transcript, full_start);
            if !summary.trim().is_empty() {
                has_summary = true;
                messages.push(ChatMessage::user(summary));
            }
        }

        let mut included_tool_exchanges = 0;
        let mut preserved_reasoning_messages = 0;
        let mut omitted_reasoning_messages = 0;

        for turn in &transcript.turns()[full_start..] {
            included_tool_exchanges += turn.tool_exchange_count();
            let reasoning_count = turn.reasoning_message_count();
            if budget.preserve_reasoning_content {
                preserved_reasoning_messages += reasoning_count;
            } else {
                omitted_reasoning_messages += reasoning_count;
            }
            messages.extend(turn.to_messages(budget.preserve_reasoning_content));
        }

        let estimated_prompt_tokens = self.estimator.estimate_messages(&messages);
        let included_recent_turns = turn_count.saturating_sub(full_start);
        let report = ContextCompileReport {
            estimated_prompt_tokens,
            target_prompt_tokens: budget.target_prompt_tokens,
            reserved_completion_tokens: budget.reserved_completion_tokens,
            included_recent_turns,
            summarized_turns,
            included_tool_exchanges,
            summarized_tool_exchanges,
            reasoning_policy: budget.reasoning_policy_label.clone(),
            preserved_reasoning_messages,
            summarized_reasoning_messages: summarized_reasoning_messages
                .saturating_add(omitted_reasoning_messages),
            warnings: Vec::new(),
        };

        BuiltPack {
            messages,
            report,
            has_memory,
            has_summary,
        }
    }

    fn history_summary_message(
        &self,
        transcript: &mut ConversationTranscript,
        full_start: usize,
    ) -> String {
        let mut summaries = Vec::new();
        for turn in &mut transcript.turns_mut()[..full_start] {
            turn.retention = RetentionClass::Summarized;
            if turn.summary.is_none() {
                turn.summary = Some(fallback_turn_summary(
                    turn,
                    self.policy.max_tool_exchange_tokens,
                    &self.estimator,
                ));
            }
            if let Some(summary) = &turn.summary {
                summaries.push(summary.content.clone());
            }
        }

        let content = format!(
            "Previous conversation summary. This message is compressed context, not a new request.\n\n{}",
            summaries.join("\n\n")
        );
        self.estimator
            .truncate_text_to_tokens(&content, self.policy.max_summary_tokens)
    }
}

struct BuiltPack {
    messages: Vec<ChatMessage>,
    report: ContextCompileReport,
    has_memory: bool,
    has_summary: bool,
}

pub fn validate_protocol_safe_messages(messages: &[ChatMessage]) -> Result<()> {
    let mut expected_tool_call_ids = Vec::<String>::new();
    let mut seen_tool_call_ids = HashSet::<String>::new();

    for (index, message) in messages.iter().enumerate() {
        if !expected_tool_call_ids.is_empty() && message.role != "tool" {
            bail!(
                "assistant tool_calls at message {} are missing adjacent tool results before message {}",
                index.saturating_sub(1),
                index
            );
        }

        match message.role.as_str() {
            "assistant" => {
                if let Some(tool_calls) = message.tool_calls.as_deref()
                    && !tool_calls.is_empty()
                {
                    expected_tool_call_ids = tool_calls
                        .iter()
                        .map(|tool_call| tool_call.id.clone())
                        .collect();
                    seen_tool_call_ids.clear();
                }
            }
            "tool" => {
                let tool_call_id = message
                    .tool_call_id
                    .as_deref()
                    .context("tool message missing tool_call_id")?;
                if !expected_tool_call_ids
                    .iter()
                    .any(|expected| expected == tool_call_id)
                {
                    bail!("orphan tool message for tool_call_id `{tool_call_id}`");
                }
                if !seen_tool_call_ids.insert(tool_call_id.to_string()) {
                    bail!("duplicate tool message for tool_call_id `{tool_call_id}`");
                }
                if seen_tool_call_ids.len() == expected_tool_call_ids.len() {
                    expected_tool_call_ids.clear();
                    seen_tool_call_ids.clear();
                }
            }
            _ => {}
        }
    }

    if !expected_tool_call_ids.is_empty() {
        bail!(
            "assistant tool_calls missing tool results for ids: {}",
            expected_tool_call_ids.join(", ")
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        api::{ChatMessage, ChoiceMessage, FunctionCall, ToolCall},
        context::{
            budget::{ContextPolicy, ReasoningRetentionPolicy, TokenEstimator},
            compiler::{CompileInput, ContextCompiler, validate_protocol_safe_messages},
            memory::WorkingMemory,
            transcript::ConversationTranscript,
        },
        tool_result_processor::ToolResultMetadata,
    };

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: "{}".into(),
            },
        }
    }

    fn metadata() -> ToolResultMetadata {
        ToolResultMetadata {
            original_chars: 2,
            injected_chars: 2,
            truncated: false,
            artifact_path: None,
        }
    }

    #[test]
    fn compile_request_preserves_recent_turns_under_budget() {
        let mut transcript = ConversationTranscript::new("system".into());
        transcript.record_user("hello".into());
        transcript.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: Some("hi".into()),
            reasoning_content: None,
            tool_calls: None,
        });
        let compiler = ContextCompiler::new(ContextPolicy::default(), TokenEstimator);

        let compiled = compiler
            .compile(
                &mut transcript,
                &WorkingMemory::default(),
                "deepseek-v4-pro",
                CompileInput,
            )
            .unwrap();

        assert_eq!(compiled.messages.len(), 3);
        assert_eq!(compiled.messages[1].content.as_deref(), Some("hello"));
        assert_eq!(compiled.messages[2].content.as_deref(), Some("hi"));
        assert_eq!(compiled.report.included_recent_turns, 1);
        assert_eq!(compiled.report.summarized_turns, 0);
    }

    #[test]
    fn compile_request_summarizes_old_turns_when_over_budget() {
        let mut transcript = ConversationTranscript::new("system".into());
        for index in 0..4 {
            transcript.record_user(format!("question {index} {}", "x".repeat(40_000)));
            transcript.record_assistant(ChoiceMessage {
                role: "assistant".into(),
                content: Some(format!("answer {index} {}", "y".repeat(40_000))),
                reasoning_content: Some("reasoning".into()),
                tool_calls: None,
            });
        }
        let policy = ContextPolicy {
            recent_turns: 4,
            target_ratio: 0.35,
            max_summary_tokens: 256,
            ..ContextPolicy::default()
        };
        let compiler = ContextCompiler::new(policy, TokenEstimator);

        let compiled = compiler
            .compile(
                &mut transcript,
                &WorkingMemory::default(),
                "other-model",
                CompileInput,
            )
            .unwrap();

        assert_eq!(compiled.report.included_recent_turns, 1);
        assert!(compiled.report.summarized_turns >= 3);
        assert!(compiled.messages.iter().any(|message| {
            message
                .content
                .as_deref()
                .is_some_and(|content| content.contains("Previous conversation summary"))
        }));
    }

    #[test]
    fn compile_request_never_emits_orphan_tool_messages() {
        let messages = vec![
            ChatMessage::system("system"),
            ChatMessage::tool("orphan", "call_1"),
        ];

        let error = validate_protocol_safe_messages(&messages).unwrap_err();

        assert!(error.to_string().contains("orphan tool message"));
    }

    #[test]
    fn compile_request_keeps_tool_exchange_complete() {
        let mut transcript = ConversationTranscript::new("system".into());
        transcript.record_user("read".into());
        transcript.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: None,
            reasoning_content: Some("Need file.".into()),
            tool_calls: Some(vec![tool_call("call_1", "read_file")]),
        });
        transcript.record_tool_result("call_1".into(), "ok".into(), metadata());
        let compiler = ContextCompiler::new(ContextPolicy::default(), TokenEstimator);

        let compiled = compiler
            .compile(
                &mut transcript,
                &WorkingMemory::default(),
                "deepseek-v4-pro",
                CompileInput,
            )
            .unwrap();

        let assistant_index = compiled
            .messages
            .iter()
            .position(|message| message.role == "assistant")
            .unwrap();
        assert_eq!(compiled.messages[assistant_index + 1].role, "tool");
        assert_eq!(
            compiled.messages[assistant_index + 1]
                .tool_call_id
                .as_deref(),
            Some("call_1")
        );
        assert_eq!(compiled.report.included_tool_exchanges, 1);
    }

    #[test]
    fn compile_request_preserves_deepseek_reasoning_content_for_retained_assistant_messages() {
        let mut transcript = ConversationTranscript::new("system".into());
        transcript.record_user("hello".into());
        transcript.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: Some("hi".into()),
            reasoning_content: Some("private reasoning".into()),
            tool_calls: None,
        });
        let policy = ContextPolicy {
            reasoning_policy: ReasoningRetentionPolicy::OmitWhenSupported,
            ..ContextPolicy::default()
        };
        let compiler = ContextCompiler::new(policy, TokenEstimator);

        let compiled = compiler
            .compile(
                &mut transcript,
                &WorkingMemory::default(),
                "deepseek-v4-pro",
                CompileInput,
            )
            .unwrap();

        let assistant = compiled
            .messages
            .iter()
            .find(|message| message.role == "assistant")
            .unwrap();
        assert_eq!(
            assistant.reasoning_content.as_deref(),
            Some("private reasoning")
        );
        assert_eq!(compiled.report.reasoning_policy, "preserve");
        assert_eq!(compiled.report.preserved_reasoning_messages, 1);
    }

    #[test]
    fn compile_request_can_omit_reasoning_only_when_policy_allows() {
        let mut transcript = ConversationTranscript::new("system".into());
        transcript.record_user("hello".into());
        transcript.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: Some("hi".into()),
            reasoning_content: Some("omit me".into()),
            tool_calls: None,
        });
        let policy = ContextPolicy {
            reasoning_policy: ReasoningRetentionPolicy::OmitWhenSupported,
            ..ContextPolicy::default()
        };
        let compiler = ContextCompiler::new(policy, TokenEstimator);

        let compiled = compiler
            .compile(
                &mut transcript,
                &WorkingMemory::default(),
                "other-model",
                CompileInput,
            )
            .unwrap();

        let assistant = compiled
            .messages
            .iter()
            .find(|message| message.role == "assistant")
            .unwrap();
        assert!(assistant.reasoning_content.is_none());
        assert_eq!(compiled.report.reasoning_policy, "omit_when_supported");
    }

    #[test]
    fn disabled_context_config_matches_legacy_message_order() {
        let mut transcript = ConversationTranscript::new("system".into());
        transcript.record_user("read".into());
        transcript.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: None,
            reasoning_content: Some("Need a tool.".into()),
            tool_calls: Some(vec![tool_call("call_1", "read_file")]),
        });
        transcript.record_tool_result("call_1".into(), "ok".into(), metadata());
        transcript.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: Some("done".into()),
            reasoning_content: Some("Final reasoning.".into()),
            tool_calls: None,
        });
        let legacy_messages = transcript.to_legacy_messages();
        let policy = ContextPolicy {
            enabled: false,
            reasoning_policy: ReasoningRetentionPolicy::OmitWhenSupported,
            ..ContextPolicy::default()
        };
        let compiler = ContextCompiler::new(policy, TokenEstimator);

        let compiled = compiler
            .compile(
                &mut transcript,
                &WorkingMemory::default(),
                "other-model",
                CompileInput,
            )
            .unwrap();

        assert_eq!(
            serde_json::to_value(&compiled.messages).unwrap(),
            serde_json::to_value(&legacy_messages).unwrap()
        );
        assert_eq!(compiled.report.reasoning_policy, "legacy");
    }
}
