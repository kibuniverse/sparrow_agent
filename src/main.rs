use anyhow::Result;
use sparrow_agent::{
    agent::Agent,
    config::AppConfig,
    console::{is_exit_command, read_user_input},
};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    sparrow_agent::debug::init();
    let config = AppConfig::load_or_initialize()?;
    let mut agent = Agent::new(config);

    println!("Sparrow Agent ready. Type 'exit' or 'quit' to stop.");
    loop {
        let context_usage_line = agent.context_usage_line();
        let Some(input) = read_user_input("you> ", Some(&context_usage_line))? else {
            break;
        };

        if is_exit_command(&input) {
            break;
        }

        agent.handle_user_input(input).await?;
    }

    Ok(())
}
