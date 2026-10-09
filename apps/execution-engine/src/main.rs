//! `execution-engine` executable: argument and environment handling only.

use std::process::ExitCode;

use sagat_control_plane::RunError;
use sagat_control_plane::config::Config;
use sagat_execution_engine::cli::{self, Command, USAGE, UsageError};
use sagat_execution_engine::{EngineConfig, probe, serve, shutdown_signal};
use sagat_telemetry::init_logging;

fn env_lookup(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(command) => command,
        Err(UsageError(message)) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("execution-engine {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        other => match run(other).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::from(error.exit_code())
            }
        },
    }
}

async fn run(command: Command) -> Result<(), RunError> {
    if command == Command::Healthcheck {
        let addr =
            env_lookup("SAGAT_HEALTHCHECK_ADDR").unwrap_or_else(|| "127.0.0.1:9091".to_owned());
        return probe(&addr, "/livez").await;
    }
    let config = Config::from_lookup(&env_lookup)?;
    init_logging(config.log_format);
    let engine = EngineConfig::from_lookup(&env_lookup, config.environment)?;
    let grpc = tokio::net::TcpListener::bind(engine.grpc_addr).await?;
    let admin = match engine.admin_addr {
        Some(addr) => Some(tokio::net::TcpListener::bind(addr).await?),
        None => None,
    };
    serve(&config, &engine, grpc, admin, shutdown_signal()).await
}
