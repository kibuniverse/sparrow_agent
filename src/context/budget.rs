use serde::{Deserialize, Serialize};

use crate::api::ChatMessage;

pub const DEFAULT_CONTEXT_ENABLED: bool = true;
pub const DEFAULT_TARGET_RATIO: f32 = 0.60;
pub const DEFAULT_COMPACTION_TRIGGER_RATIO: f32 = 0.40;
pub const DEFAULT_RESERVED_COMPLETION_TOKENS: usize = 8_192;
pub const DEFAULT_RECENT_TURNS: usize = 6;
pub const DEFAULT_MAX_TOOL_EXCHANGE_TOKENS: usize = 4_096;
pub const DEFAULT_MAX_MEMORY_TOKENS: usize = 12_000;
pub const DEFAULT_MAX_SUMMARY_TOKENS: usize = 8_000;

const DEFAULT_UNKNOWN_CONTEXT_WINDOW_TOKENS: usize = 128_000;
const DEEPSEEK_V4_CONTEXT_TOKENS: usize = 1_000_000;
const MESSAGE_OVERHEAD_TOKENS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningRetentionPolicy {
    Auto,
    Preserve,
    OmitWhenSupported,
}

impl ReasoningRetentionPolicy {
    pub fn from_str(value: &str) -> Self {
        match value {
            "preserve" => Self::Preserve,
            "omit_when_supported" => Self::OmitWhenSupported,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Preserve => "preserve",
            Self::OmitWhenSupported => "omit_when_supported",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextPolicy {
    pub enabled: bool,
    pub target_ratio: f32,
    pub compaction_trigger_ratio: f32,
    pub reserved_completion_tokens: usize,
    pub recent_turns: usize,
    pub max_tool_exchange_tokens: usize,
    pub max_memory_tokens: usize,
    pub max_summary_tokens: usize,
    pub reasoning_policy: ReasoningRetentionPolicy,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_CONTEXT_ENABLED,
            target_ratio: DEFAULT_TARGET_RATIO,
            compaction_trigger_ratio: DEFAULT_COMPACTION_TRIGGER_RATIO,
            reserved_completion_tokens: DEFAULT_RESERVED_COMPLETION_TOKENS,
            recent_turns: DEFAULT_RECENT_TURNS,
            max_tool_exchange_tokens: DEFAULT_MAX_TOOL_EXCHANGE_TOKENS,
            max_memory_tokens: DEFAULT_MAX_MEMORY_TOKENS,
            max_summary_tokens: DEFAULT_MAX_SUMMARY_TOKENS,
            reasoning_policy: ReasoningRetentionPolicy::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextBudget {
    pub context_window_tokens: usize,
    pub target_prompt_tokens: usize,
    pub reserved_completion_tokens: usize,
    pub preserve_reasoning_content: bool,
    pub reasoning_policy_label: String,
}

impl ContextBudget {
    pub fn for_model(model: &str, policy: &ContextPolicy) -> Self {
        let context_window_tokens =
            model_context_window_tokens(model).unwrap_or(DEFAULT_UNKNOWN_CONTEXT_WINDOW_TOKENS);
        let ratio_target = ((context_window_tokens as f32) * policy.target_ratio) as usize;
        let target_prompt_tokens = ratio_target
            .saturating_sub(policy.reserved_completion_tokens)
            .max(1);
        let preserve_reasoning_content = preserves_reasoning_content(model, policy);
        let reasoning_policy_label = if preserve_reasoning_content {
            "preserve"
        } else {
            "omit_when_supported"
        }
        .to_string();

        Self {
            context_window_tokens,
            target_prompt_tokens,
            reserved_completion_tokens: policy.reserved_completion_tokens,
            preserve_reasoning_content,
            reasoning_policy_label,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenEstimator;

impl TokenEstimator {
    pub fn estimate_messages(&self, messages: &[ChatMessage]) -> usize {
        messages
            .iter()
            .map(|message| self.estimate_message(message))
            .sum()
    }

    pub fn estimate_message(&self, message: &ChatMessage) -> usize {
        let mut tokens = MESSAGE_OVERHEAD_TOKENS + self.estimate_text(&message.role);

        if let Some(content) = &message.content {
            tokens += self.estimate_text(content);
        }
        if let Some(tool_call_id) = &message.tool_call_id {
            tokens += self.estimate_text(tool_call_id);
        }
        if let Some(reasoning_content) = &message.reasoning_content {
            tokens += self.estimate_text(reasoning_content);
        }
        if let Some(tool_calls) = &message.tool_calls {
            let serialized = serde_json::to_string(tool_calls).unwrap_or_default();
            tokens += self.estimate_text(&serialized);
        }

        tokens
    }

    pub fn estimate_text(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }

        let chars = text.chars().count();
        let bytes = text.len();
        let cjk_chars = text.chars().filter(|ch| is_cjk(*ch)).count();
        let ascii_chars = chars.saturating_sub(cjk_chars);

        let chars_estimate = chars.div_ceil(3);
        let bytes_estimate = bytes.div_ceil(4);
        let mixed_estimate = cjk_chars
            .saturating_mul(2)
            .saturating_add(ascii_chars.div_ceil(4));

        chars_estimate.max(bytes_estimate).max(mixed_estimate)
    }

    pub fn truncate_text_to_tokens(&self, text: &str, max_tokens: usize) -> String {
        if self.estimate_text(text) <= max_tokens {
            return text.to_string();
        }

        let max_chars = max_tokens.saturating_mul(3).max(1);
        let mut truncated = text.chars().take(max_chars).collect::<String>();
        if truncated.len() < text.len() {
            truncated.push_str("\n[truncated]");
        }
        truncated
    }
}

pub fn model_context_window_tokens(model: &str) -> Option<usize> {
    match model {
        "deepseek-v4-flash" | "deepseek-v4-pro" => Some(DEEPSEEK_V4_CONTEXT_TOKENS),
        _ => None,
    }
}

pub fn preserves_reasoning_content(model: &str, policy: &ContextPolicy) -> bool {
    if model.to_ascii_lowercase().contains("deepseek") {
        return true;
    }

    matches!(policy.reasoning_policy, ReasoningRetentionPolicy::Preserve)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ContextPolicy, ReasoningRetentionPolicy, TokenEstimator, preserves_reasoning_content,
    };

    #[test]
    fn estimator_is_conservative_for_cjk_text() {
        let estimator = TokenEstimator;

        assert!(estimator.estimate_text("你好世界") >= 8);
        assert!(estimator.estimate_text("hello world") >= 3);
    }

    #[test]
    fn deepseek_models_always_preserve_reasoning() {
        let policy = ContextPolicy {
            reasoning_policy: ReasoningRetentionPolicy::OmitWhenSupported,
            ..ContextPolicy::default()
        };

        assert!(preserves_reasoning_content("deepseek-v4-pro", &policy));
        assert!(!preserves_reasoning_content("other-model", &policy));
    }
}
