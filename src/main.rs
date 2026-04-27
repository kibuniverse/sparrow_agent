mod api;
mod client;
mod tools;
use std::env;
use std::io::{self, Write};

use api::{ChatCompletionRequest, ChatMessage, ThinkingConfig};
use client::DeepSeekClient;
use serde_json::json;

use crate::api::ToolDef;

const DEFAULT_MODEL: &str = "deepseek-v4-flash";
const SYSTEM_PROMPT: &str = "You are a helpful assistant.";

#[tokio::main]
async fn main() {
    let api_key =
        env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY environment variable is not set");

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

        let mut get_weather_tool =
            ToolDef::function("getWeather", "Get the weather for a given location.");
        get_weather_tool.function.parameters = Some(json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The location to get the weather for."
                }
            },
            "required": ["location"]
        }));

        const MAX_TOOL_ROUNDS: usize = 6;
        for _ in 0..MAX_TOOL_ROUNDS {
            println!("messages: {:?}", messages);
            let request = ChatCompletionRequest {
                model: DEFAULT_MODEL.to_string(),
                messages: messages.clone(),
                tools: Some(vec![get_weather_tool.clone()]),
                thinking: Some(ThinkingConfig::enabled()),
                reasoning_effort: Some("high".into()),
                stream: None,
            };

            match client.chat_completion(&request).await {
                Ok(response) => {
                    let Some(choice) = response.choices.first() else {
                        eprintln!("Error: empty choices in response");
                        break;
                    };

                    let message = &choice.message;
                    if let Some(tool_calls) = &message.tool_calls {
                        messages.push(ChatMessage {
                            role: "assistant".into(),
                            content: Some("".to_string()),
                            reasoning_content: message.reasoning_content.clone(),
                            tool_calls: message.tool_calls.clone(),
                            tool_call_id: None
                        });

                        for tool_call in tool_calls {
                            let func = &tool_call.function;
                            println!("Tool call: {func:#?}");
                            let result = match func.name.as_str() {
                                "getWeather" => tools::get_weather("123"),
                                _ => format!("unknown tool: {}", func.name),
                            };
                            messages.push(ChatMessage::tool(result, &tool_call.id));
                        }
                    } else if let Some(content) = &choice.message.content {
                        println!("agent> {content}");
                        messages.push(ChatMessage::assistant(content));
                        break;
                    } else {
                        // Model returned neither tool_calls nor content — nothing to do.
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    break;
                }
            }
        }
    }
}
