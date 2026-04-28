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
    let config = AppConfig::from_env()?;
    let mut agent = Agent::new(config);

    println!("Sparrow Agent ready. Type 'exit' or 'quit' to stop.");
    while let Some(input) = read_user_input("you> ")? {
        if is_exit_command(&input) {
            break;
        }

        agent.handle_user_input(input).await?;
    }

    Ok(())
}
