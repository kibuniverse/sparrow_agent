mod api;
mod client;

use std::env;
use std::io::{self, Write};

use api::{ChatCompletionRequest, ChatMessage, ThinkingConfig};
use client::DeepSeekClient;

const DEFAULT_MODEL: &str = "deepseek-v4-flash";
const SYSTEM_PROMPT: &str = "You are a helpful assistant.";

#[tokio::main]
async fn main() {
    let api_key = env::var("DEEPSEEK_API_KEY")
        .expect("DEEPSEEK_API_KEY environment variable is not set");

    let client = DeepSeekClient::new(&api_key);
    let mut messages = vec![ChatMessage::system(SYSTEM_PROMPT)];

    println!("Sparrow Agent ready. Type 'exit' or 'quit' to stop.");
    loop {
        print!("you> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).unwrap() == 0 {
            break;
        }

        let input = input.trim();
        if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
            break;
        }
        if input.is_empty() {
            continue;
        }

        messages.push(ChatMessage::user(input));

        let request = ChatCompletionRequest {
            model: DEFAULT_MODEL.to_string(),
            messages: messages.clone(),
            thinking: Some(ThinkingConfig::enabled()),
            reasoning_effort: Some("high".into()),
            stream: None,
        };

        match client.chat_completion(&request).await {
            Ok(response) => {
                if let Some(choice) = response.choices.first()
                    && let Some(content) = &choice.message.content
                {
                    println!("agent> {content}");
                    messages.push(ChatMessage::assistant(content));
                }
            }
            Err(e) => eprintln!("Error: {e}"),
        }
    }
}
