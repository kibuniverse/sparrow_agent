use anyhow::Result;
use sparrow_agent::{
    agent::Agent,
    cli_observer::{inspect_addr_from_env_value, run_cli_with_browser_trace},
    config::AppConfig,
    console::{is_exit_command, read_user_input},
    server::run_server,
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

    let args = std::env::args().collect::<Vec<_>>();

    if args.iter().any(|arg| arg == "--server") {
        let addr = std::env::var("SPARROW_SERVER_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8787".into())
            .parse()?;
        return run_server(config, addr).await;
    }

    if args
        .iter()
        .any(|arg| arg == "--inspect" || arg == "--browser-trace")
    {
        let addr = inspect_addr_from_env_value(
            std::env::var("SPARROW_INSPECT_ADDR").ok().as_deref(),
        )?;
        return run_cli_with_browser_trace(config, addr).await;
    }

    let mut agent = Agent::new(config).await?;

    println!("Sparrow Agent ready. Type 'exit' or 'quit' to stop.");
    loop {
        let context_usage_line = agent.context_usage_line();
        let Some(input) = read_user_input(">>> ", Some(&context_usage_line))? else {
            break;
        };

        if is_exit_command(&input) {
            break;
        }

        agent.handle_user_input(input).await?;
    }

    Ok(())
}
