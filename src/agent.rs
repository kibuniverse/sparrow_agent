use anyhow::Result;

use crate::{
    api::{ChatCompletionRequest, ChatMessage, ChoiceMessage, ThinkingConfig},
    client::DeepSeekClient,
    config::AppConfig,
    debug_log,
    tool_registry::ToolRegistry,
};

pub struct Agent {
    client: DeepSeekClient,
    config: AppConfig,
    messages: Vec<ChatMessage>,
    tool_registry: ToolRegistry,
}

impl Agent {
    pub fn new(config: AppConfig) -> Self {
        let client = DeepSeekClient::new(&config.api_key);
        let messages = vec![ChatMessage::system(&config.system_prompt)];
        let tool_registry = ToolRegistry::new(&config.tavily_api_key);

        Self {
            client,
            config,
            messages,
            tool_registry,
        }
    }

    pub async fn handle_user_input(&mut self, input: impl Into<String>) -> Result<()> {
        self.messages.push(ChatMessage::user(input));

        for round in 0..self.config.max_tool_rounds {
            debug_log!("=== Tool round {round} ===");
            debug_log!("Message count: {}", self.messages.len());
            for (i, msg) in self.messages.iter().enumerate() {
                let content_str = msg.content.as_deref().unwrap_or("<None>");
                let preview_len = content_str.len().min(80);
                debug_log!(
                    "msg[{i}] role={}, content={:?}..., tool_calls={}, tool_call_id={}",
                    msg.role,
                    &content_str[..preview_len],
                    msg.tool_calls.as_ref().map(|tc| tc.len()).unwrap_or(0),
                    msg.tool_call_id.as_deref().unwrap_or("<None>"),
                );
            }

            let request = self.build_request();
            let response = self.client.chat_completion(&request).await?;

            let Some(choice) = response.choices.first() else {
                debug_log!("Empty choices in response");
                return Ok(());
            };

            debug_log!(
                "Response: finish_reason={}, has_content={}, has_tool_calls={}",
                choice.finish_reason,
                choice.message.content.is_some(),
                choice.message.tool_calls.is_some(),
            );

            match self.handle_assistant_message(&choice.message).await {
                TurnStatus::Continue => continue,
                TurnStatus::Complete => return Ok(()),
            }
        }

        eprintln!(
            "Error: reached the maximum number of tool rounds ({})",
            self.config.max_tool_rounds
        );
        Ok(())
    }

    fn build_request(&self) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: self.messages.clone(),
            tools: Some(self.tool_registry.definitions().to_vec()),
            thinking: Some(ThinkingConfig::enabled()),
            reasoning_effort: Some(self.config.reasoning_effort.clone()),
            stream: None,
        }
    }

    async fn handle_assistant_message(&mut self, message: &ChoiceMessage) -> TurnStatus {
        if let Some(tool_calls) = message.tool_calls.as_deref() {
            debug_log!(
                "Assistant requests {} tool call(s): {:?}",
                tool_calls.len(),
                tool_calls
                    .iter()
                    .map(|tc| &tc.function.name)
                    .collect::<Vec<_>>(),
            );

            self.messages.push(ChatMessage {
                role: "assistant".into(),
                content: Some(String::new()),
                reasoning_content: message.reasoning_content.clone(),
                tool_calls: message.tool_calls.clone(),
                tool_call_id: None,
            });

            let results = self.tool_registry.execute_all(tool_calls).await;
            for result in &results {
                let preview_len = result.content.len().min(120);
                debug_log!(
                    "Tool result: id={}, content={:?}...",
                    result.tool_call_id,
                    &result.content[..preview_len],
                );
            }
            for result in results {
                self.messages
                    .push(ChatMessage::tool(result.content, &result.tool_call_id));
            }

            return TurnStatus::Continue;
        }

        if let Some(content) = &message.content {
            println!("agent> {content}");
            self.messages.push(ChatMessage::assistant(
                content,
                message.reasoning_content.clone(),
            ));
        }

        TurnStatus::Complete
    }
}

enum TurnStatus {
    Continue,
    Complete,
}
