//! `control-plane` executable: argument and environment handling only.

use std::process::ExitCode;

use sagat_control_plane::cli::{self, Command, USAGE, UsageError};
use sagat_control_plane::config::{Config, SeedConfig, ServeConfig};
use sagat_control_plane::{RunError, SeedOutcome};
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
            println!("control-plane {}", env!("CARGO_PKG_VERSION"));
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
            env_lookup("SAGAT_HEALTHCHECK_ADDR").unwrap_or_else(|| "127.0.0.1:8080".to_owned());
        return sagat_control_plane::healthcheck(&addr).await;
    }

    let config = Config::from_lookup(&env_lookup)?;
    init_logging(config.log_format);

    match command {
        Command::Migrate => {
            sagat_control_plane::run_migrations(&config.database).await?;
            tracing::info!("migrations are up to date");
            Ok(())
        }
        Command::SeedDev => {
            let seed = SeedConfig::from_lookup(&env_lookup)?;
            match sagat_control_plane::seed_dev(&config, &seed).await? {
                SeedOutcome::Created => {
                    tracing::info!(tenant = %seed.tenant_slug, "development tenant created")
                }
                SeedOutcome::AlreadyPresent => {
                    tracing::info!(tenant = %seed.tenant_slug, "development tenant already present");
                }
            }
            Ok(())
        }
        Command::Serve => {
            let serve = ServeConfig::from_lookup(&env_lookup)?;
            let listener = tokio::net::TcpListener::bind(serve.bind_addr).await?;
            let admin = match serve.admin_addr {
                Some(addr) => Some(tokio::net::TcpListener::bind(addr).await?),
                None => None,
            };
            sagat_control_plane::serve_with_admin(
                &config,
                &serve,
                listener,
                admin,
                sagat_control_plane::shutdown_signal(),
            )
            .await
        }
        Command::Help | Command::Version | Command::Healthcheck => Ok(()),
    }
}
