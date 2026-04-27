mod api;
mod client;
mod tools;

use std::env;
use std::error::Error;
use std::io::{self, Write};

use api::{ChatCompletionRequest, ChatMessage, ChoiceMessage, ThinkingConfig, ToolCall, ToolDef};
use client::DeepSeekClient;
use serde::Deserialize;
use serde_json::json;

const DEFAULT_MODEL: &str = "deepseek-v4-flash";
const SYSTEM_PROMPT: &str = "You are a helpful assistant.";
const REASONING_EFFORT: &str = "high";
const MAX_TOOL_ROUNDS: usize = 6;
const GET_WEATHER_TOOL: &str = "getWeather";

type AppResult<T> = Result<T, Box<dyn Error>>;

struct AppConfig {
    api_key: String,
    model: String,
    system_prompt: String,
    reasoning_effort: String,
    max_tool_rounds: usize,
}

impl AppConfig {
    fn from_env() -> AppResult<Self> {
        let api_key = env::var("DEEPSEEK_API_KEY").map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "DEEPSEEK_API_KEY environment variable is not set",
            )
        })?;

        Ok(Self {
            api_key,
            model: DEFAULT_MODEL.into(),
            system_prompt: SYSTEM_PROMPT.into(),
            reasoning_effort: REASONING_EFFORT.into(),
            max_tool_rounds: MAX_TOOL_ROUNDS,
        })
    }
}

enum TurnStatus {
    Continue,
    Complete,
}

#[derive(Deserialize)]
struct WeatherArgs {
    location: String,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let config = AppConfig::from_env()?;
    let client = DeepSeekClient::new(&config.api_key);
    let tools = available_tools();
    let mut messages = vec![ChatMessage::system(&config.system_prompt)];

    println!("Sparrow Agent ready. Type 'exit' or 'quit' to stop.");
    while let Some(input) = read_user_input("you> ")? {
        if should_exit(&input) {
            return Ok(());
        }
        messages.push(ChatMessage::user(input));
        run_agent_turn(&client, &config, &tools, &mut messages).await?;
    }

    Ok(())
}

fn available_tools() -> Vec<ToolDef> {
    vec![weather_tool()]
}

fn weather_tool() -> ToolDef {
    let mut tool = ToolDef::function(GET_WEATHER_TOOL, "Get the weather for a given location.");
    tool.function.parameters = Some(json!({
        "type": "object",
        "properties": {
            "location": {
                "type": "string",
                "description": "The location to get the weather for."
            }
        },
        "required": ["location"]
    }));
    tool
}

fn read_user_input(prompt: &str) -> io::Result<Option<String>> {
    loop {
        print!("{prompt}");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            return Ok(None);
        }

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        return Ok(Some(input));
    }
}

fn should_exit(input: &str) -> bool {
    input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit")
}

async fn run_agent_turn(
    client: &DeepSeekClient,
    config: &AppConfig,
    tools: &[ToolDef],
    messages: &mut Vec<ChatMessage>,
) -> AppResult<()> {
    for _ in 0..config.max_tool_rounds {
        let request = build_chat_request(config, messages, tools);
        let response = client.chat_completion(&request).await?;

        let Some(choice) = response.choices.first() else {
            eprintln!("Error: empty choices in response");
            return Ok(());
        };

        match handle_assistant_message(&choice.message, messages) {
            TurnStatus::Continue => continue,
            TurnStatus::Complete => return Ok(()),
        }
    }

    eprintln!(
        "Error: reached the maximum number of tool rounds ({})",
        config.max_tool_rounds
    );
    Ok(())
}

fn build_chat_request(
    config: &AppConfig,
    messages: &[ChatMessage],
    tools: &[ToolDef],
) -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: config.model.clone(),
        messages: messages.to_vec(),
        tools: Some(tools.to_vec()),
        thinking: Some(ThinkingConfig::enabled()),
        reasoning_effort: Some(config.reasoning_effort.clone()),
        stream: None,
    }
}

fn handle_assistant_message(
    message: &ChoiceMessage,
    messages: &mut Vec<ChatMessage>,
) -> TurnStatus {
    if let Some(tool_calls) = message.tool_calls.as_deref() {
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: Some(String::new()),
            reasoning_content: message.reasoning_content.clone(),
            tool_calls: message.tool_calls.clone(),
            tool_call_id: None,
        });

        for tool_call in tool_calls {
            let result = execute_tool_call(tool_call);
            messages.push(ChatMessage::tool(result, &tool_call.id));
        }

        return TurnStatus::Continue;
    }

    if let Some(content) = &message.content {
        println!("agent> {content}");
        messages.push(ChatMessage::assistant(content));
    }

    TurnStatus::Complete
}

fn execute_tool_call(tool_call: &ToolCall) -> String {
    let function = &tool_call.function;

    match function.name.as_str() {
        GET_WEATHER_TOOL => call_weather_tool(&function.arguments),
        unknown_tool => format!("unknown tool: {unknown_tool}"),
    }
}

fn call_weather_tool(arguments: &str) -> String {
    match serde_json::from_str::<WeatherArgs>(arguments) {
        Ok(args) => tools::get_weather(&args.location),
        Err(error) => format!("invalid arguments for {GET_WEATHER_TOOL}: {error}"),
    }
}
