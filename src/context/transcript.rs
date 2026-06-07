use crate::{
    api::{ChatMessage, ChoiceMessage, ToolCall},
    tool_result_processor::ToolResultMetadata,
};

pub type MessageId = String;
pub type TurnId = String;

#[derive(Debug, Clone)]
pub struct ConversationTranscript {
    system_prompt: String,
    turns: Vec<ConversationTurn>,
}

impl ConversationTranscript {
    pub fn new(system_prompt: String) -> Self {
        Self {
            system_prompt,
            turns: Vec::new(),
        }
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn turns(&self) -> &[ConversationTurn] {
        &self.turns
    }

    pub fn turns_mut(&mut self) -> &mut [ConversationTurn] {
        &mut self.turns
    }

    pub fn record_user(&mut self, content: String) -> MessageId {
        let message = TranscriptMessage::new(ChatMessage::user(content));
        let id = message.id.clone();
        self.turns.push(ConversationTurn {
            id: new_id("turn"),
            user: message,
            assistant: None,
            tool_exchanges: Vec::new(),
            summary: None,
            retention: RetentionClass::Recent,
        });
        id
    }

    pub fn record_assistant(&mut self, message: ChoiceMessage) -> MessageId {
        let transcript_message = TranscriptMessage::new(choice_message_to_chat_message(message));
        let id = transcript_message.id.clone();
        let has_tool_calls = transcript_message
            .message
            .tool_calls
            .as_ref()
            .is_some_and(|tool_calls| !tool_calls.is_empty());

        let turn = self.ensure_current_turn();
        turn.summary = None;
        if has_tool_calls {
            turn.tool_exchanges.push(ToolExchange {
                assistant_message: transcript_message,
                tool_results: Vec::new(),
                summary: None,
                status: ToolExchangeStatus::Pending,
            });
        } else {
            turn.assistant = Some(transcript_message);
        }

        id
    }

    pub fn record_tool_result(
        &mut self,
        tool_call_id: String,
        content: String,
        metadata: ToolResultMetadata,
    ) -> MessageId {
        let result = TranscriptToolResult {
            id: new_id("msg"),
            tool_call_id,
            content,
            metadata,
        };
        let id = result.id.clone();
        let turn = self.ensure_current_turn();
        turn.summary = None;

        if let Some(exchange) = turn
            .tool_exchanges
            .iter_mut()
            .rev()
            .find(|exchange| exchange.accepts_tool_result(&result.tool_call_id))
        {
            exchange.summary = None;
            exchange.tool_results.push(result);
            if exchange.is_complete() {
                exchange.status = ToolExchangeStatus::Completed;
            }
        }

        id
    }

    pub fn to_legacy_messages(&self) -> Vec<ChatMessage> {
        let mut messages = vec![ChatMessage::system(self.system_prompt.clone())];
        for turn in &self.turns {
            messages.extend(turn.to_messages(true));
        }
        messages
    }

    fn ensure_current_turn(&mut self) -> &mut ConversationTurn {
        if self.turns.is_empty() {
            self.record_user(String::new());
        }

        self.turns
            .last_mut()
            .expect("record_user should have created a turn")
    }
}

#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub id: TurnId,
    pub user: TranscriptMessage,
    pub assistant: Option<TranscriptMessage>,
    pub tool_exchanges: Vec<ToolExchange>,
    pub summary: Option<TurnSummary>,
    pub retention: RetentionClass,
}

impl ConversationTurn {
    pub fn to_messages(&self, preserve_reasoning_content: bool) -> Vec<ChatMessage> {
        let mut messages = vec![self.user.message.clone()];

        for exchange in &self.tool_exchanges {
            messages.push(
                exchange
                    .assistant_message
                    .for_request(preserve_reasoning_content),
            );
            messages.extend(
                exchange
                    .tool_results
                    .iter()
                    .map(TranscriptToolResult::to_message),
            );
        }

        if let Some(assistant) = &self.assistant {
            messages.push(assistant.for_request(preserve_reasoning_content));
        }

        messages
    }

    pub fn tool_exchange_count(&self) -> usize {
        self.tool_exchanges.len()
    }

    pub fn reasoning_message_count(&self) -> usize {
        let tool_reasoning = self
            .tool_exchanges
            .iter()
            .filter(|exchange| exchange.assistant_message.has_reasoning_content())
            .count();
        let assistant_reasoning = usize::from(
            self.assistant
                .as_ref()
                .is_some_and(TranscriptMessage::has_reasoning_content),
        );

        tool_reasoning + assistant_reasoning
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptMessage {
    pub id: MessageId,
    pub message: ChatMessage,
}

impl TranscriptMessage {
    fn new(message: ChatMessage) -> Self {
        Self {
            id: new_id("msg"),
            message,
        }
    }

    pub fn for_request(&self, preserve_reasoning_content: bool) -> ChatMessage {
        let mut message = self.message.clone();
        if message.role == "assistant" && !preserve_reasoning_content {
            message.reasoning_content = None;
        }
        message
    }

    pub fn has_reasoning_content(&self) -> bool {
        self.message
            .reasoning_content
            .as_deref()
            .is_some_and(|content| !content.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptToolResult {
    pub id: MessageId,
    pub tool_call_id: String,
    pub content: String,
    pub metadata: ToolResultMetadata,
}

impl TranscriptToolResult {
    pub fn to_message(&self) -> ChatMessage {
        ChatMessage::tool(self.content.clone(), &self.tool_call_id)
    }
}

#[derive(Debug, Clone)]
pub struct ToolExchange {
    pub assistant_message: TranscriptMessage,
    pub tool_results: Vec<TranscriptToolResult>,
    pub summary: Option<ToolExchangeSummary>,
    pub status: ToolExchangeStatus,
}

impl ToolExchange {
    pub fn tool_calls(&self) -> &[ToolCall] {
        self.assistant_message
            .message
            .tool_calls
            .as_deref()
            .unwrap_or_default()
    }

    pub fn accepts_tool_result(&self, tool_call_id: &str) -> bool {
        self.status == ToolExchangeStatus::Pending
            && self
                .tool_calls()
                .iter()
                .any(|tool_call| tool_call.id == tool_call_id)
    }

    pub fn is_complete(&self) -> bool {
        let tool_calls = self.tool_calls();
        !tool_calls.is_empty()
            && tool_calls.iter().all(|tool_call| {
                self.tool_results
                    .iter()
                    .any(|result| result.tool_call_id == tool_call.id)
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExchangeStatus {
    Pending,
    Completed,
}

#[derive(Debug, Clone)]
pub struct ToolExchangeSummary {
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct TurnSummary {
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionClass {
    Recent,
    Summarized,
}

fn choice_message_to_chat_message(message: ChoiceMessage) -> ChatMessage {
    let has_tool_calls = message
        .tool_calls
        .as_ref()
        .is_some_and(|tool_calls| !tool_calls.is_empty());

    ChatMessage {
        role: "assistant".into(),
        content: if has_tool_calls {
            Some(message.content.unwrap_or_default())
        } else {
            Some(message.content.unwrap_or_default())
        },
        reasoning_content: message.reasoning_content,
        tool_calls: message.tool_calls,
        tool_call_id: None,
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", ulid::Ulid::new())
}

#[cfg(test)]
mod tests {
    use crate::api::{ChoiceMessage, FunctionCall, ToolCall};

    use super::ConversationTranscript;

    #[test]
    fn transcript_keeps_tool_exchange_adjacent_in_legacy_messages() {
        let mut transcript = ConversationTranscript::new("system".into());
        transcript.record_user("read it".into());
        transcript.record_assistant(ChoiceMessage {
            role: "assistant".into(),
            content: None,
            reasoning_content: Some("Need a tool.".into()),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                kind: "function".into(),
                function: FunctionCall {
                    name: "read_file".into(),
                    arguments: "{}".into(),
                },
            }]),
        });
        transcript.record_tool_result(
            "call_1".into(),
            "tool output".into(),
            crate::tool_result_processor::ToolResultMetadata {
                original_chars: 11,
                injected_chars: 11,
                truncated: false,
                artifact_path: None,
            },
        );

        let messages = transcript.to_legacy_messages();

        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[2].role, "assistant");
        assert_eq!(messages[3].role, "tool");
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(
            messages[2].reasoning_content.as_deref(),
            Some("Need a tool.")
        );
    }
}
