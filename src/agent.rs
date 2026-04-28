use anyhow::Result;

use crate::{
    api::{ChatCompletionRequest, ChatMessage, ChoiceMessage, ThinkingConfig},
    client::DeepSeekClient,
    config::AppConfig,
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

        for _ in 0..self.config.max_tool_rounds {
            let request = self.build_request();
            let response = self.client.chat_completion(&request).await?;

            let Some(choice) = response.choices.first() else {
                eprintln!("Error: empty choices in response");
                return Ok(());
            };

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
            self.messages.push(ChatMessage {
                role: "assistant".into(),
                content: Some(String::new()),
                reasoning_content: message.reasoning_content.clone(),
                tool_calls: message.tool_calls.clone(),
                tool_call_id: None,
            });

            for result in self.tool_registry.execute_all(tool_calls).await {
                self.messages
                    .push(ChatMessage::tool(result.content, &result.tool_call_id));
            }

            return TurnStatus::Continue;
        }

        if let Some(content) = &message.content {
            println!("agent> {content}");
            self.messages.push(ChatMessage::assistant(content));
        }

        TurnStatus::Complete
    }
}

enum TurnStatus {
    Continue,
    Complete,
}
