use anyhow::Result;

use crate::{
    api::ChoiceMessage,
    context::{
        budget::{ContextPolicy, TokenEstimator},
        compiler::{CompileInput, CompiledContext, ContextCompiler},
        memory::{DeltaOutcome, MemoryDelta, WorkingMemory},
        summary::{ContextSummarizer, fallback_turn_summary},
        transcript::{ConversationTranscript, MessageId},
    },
    tool_result_processor::ToolResultMetadata,
};

pub struct ContextManager {
    transcript: ConversationTranscript,
    working_memory: WorkingMemory,
    policy: ContextPolicy,
    estimator: TokenEstimator,
    model: String,
}

impl ContextManager {
    pub fn new(system_prompt: String, model: String, policy: ContextPolicy) -> Self {
        Self {
            transcript: ConversationTranscript::new(system_prompt),
            working_memory: WorkingMemory::default(),
            policy,
            estimator: TokenEstimator,
            model,
        }
    }

    pub fn record_user(&mut self, content: String) -> MessageId {
        self.transcript.record_user(content)
    }

    pub fn record_assistant(&mut self, message: ChoiceMessage) -> MessageId {
        self.transcript.record_assistant(message)
    }

    pub fn record_tool_result(
        &mut self,
        tool_call_id: String,
        content: String,
        metadata: ToolResultMetadata,
    ) -> MessageId {
        self.transcript
            .record_tool_result(tool_call_id, content, metadata)
    }

    pub fn apply_memory_delta(&mut self, delta: MemoryDelta) -> DeltaOutcome {
        self.working_memory.apply_delta(&delta)
    }

    pub fn compile_request(&mut self, input: CompileInput) -> Result<CompiledContext> {
        ContextCompiler::new(self.policy.clone(), self.estimator).compile(
            &mut self.transcript,
            &self.working_memory,
            &self.model,
            input,
        )
    }

    pub async fn maybe_compact(
        &mut self,
        summarizer: &dyn ContextSummarizer,
    ) -> Result<CompactionReport> {
        let mut report = CompactionReport::default();
        let recent_turns = self.policy.recent_turns.max(1);
        let compact_until = self.transcript.turns().len().saturating_sub(recent_turns);

        for turn in &mut self.transcript.turns_mut()[..compact_until] {
            if turn.summary.is_some() {
                continue;
            }

            match summarizer.summarize_turn(turn).await {
                Ok(summary) => {
                    turn.summary = Some(summary);
                    report.compacted_turns += 1;
                }
                Err(error) => {
                    turn.summary = Some(fallback_turn_summary(
                        turn,
                        self.policy.max_tool_exchange_tokens,
                        &self.estimator,
                    ));
                    report.compacted_turns += 1;
                    report.fallback_summaries += 1;
                    report
                        .warnings
                        .push(format!("summarizer failed for turn {}: {error}", turn.id));
                }
            }
        }

        Ok(report)
    }

    pub fn audit_messages(&self) -> Vec<crate::api::ChatMessage> {
        self.transcript.to_legacy_messages()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactionReport {
    pub compacted_turns: usize,
    pub fallback_summaries: usize,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::{
        api::ChoiceMessage,
        context::{
            ContextManager, ContextSummarizer, DeltaOutcome, MemoryDelta,
            budget::ContextPolicy,
            compiler::CompileInput,
            transcript::{ConversationTurn, TurnSummary},
        },
    };

    struct FailingSummarizer;

    #[async_trait::async_trait]
    impl ContextSummarizer for FailingSummarizer {
        async fn summarize_turn(&self, _turn: &ConversationTurn) -> Result<TurnSummary> {
            anyhow::bail!("boom")
        }
    }

    #[tokio::test]
    async fn context_manager_falls_back_when_summarizer_fails() {
        let policy = ContextPolicy {
            recent_turns: 1,
            ..ContextPolicy::default()
        };
        let mut manager = ContextManager::new("system".into(), "deepseek-v4-pro".into(), policy);
        manager.record_user("first".into());
        manager.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: Some("answer".into()),
            reasoning_content: None,
            tool_calls: None,
        });
        manager.record_user("second".into());

        let report = manager.maybe_compact(&FailingSummarizer).await.unwrap();
        let compiled = manager.compile_request(CompileInput).unwrap();

        assert_eq!(report.compacted_turns, 1);
        assert_eq!(report.fallback_summaries, 1);
        assert!(compiled.report.summarized_turns >= 1);
    }

    #[test]
    fn apply_memory_delta_then_compile_injects_it() {
        let mut manager = ContextManager::new(
            "system".into(),
            "deepseek-v4-pro".into(),
            ContextPolicy::default(),
        );

        let outcome = manager.apply_memory_delta(MemoryDelta::AddFact {
            content: "fact one".into(),
        });
        assert_eq!(outcome, DeltaOutcome::Applied);

        let compiled = manager.compile_request(CompileInput).unwrap();
        let memory_content = compiled.messages.iter().find_map(|message| {
            message
                .content
                .as_deref()
                .filter(|content| content.starts_with("Working memory for the ongoing task"))
        });
        let memory_content = memory_content.expect("working memory message should be injected");
        assert!(memory_content.contains("fact one"));
    }
}
