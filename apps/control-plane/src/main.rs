//! `control-plane` executable: argument and environment handling only.

use std::process::ExitCode;

use sagat_control_plane::cli::{self, Command, USAGE, UsageError};
use sagat_control_plane::config::{
    Config, NamespaceSeed, ProvisionConfig, SeedConfig, ServeConfig,
};
use sagat_control_plane::{RunError, SeedOutcome};

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
    if command == Command::ScriptKey {
        // The key goes to standard output and nowhere else: not to a log, not to the environment.
        let key = sagat_control_plane::new_script_key()?;
        println!("{key}");
        return Ok(());
    }
    if command == Command::Healthcheck {
        let addr =
            env_lookup("SAGAT_HEALTHCHECK_ADDR").unwrap_or_else(|| "127.0.0.1:8080".to_owned());
        let tls = sagat_control_plane::HealthcheckTlsConfig::from_lookup(env_lookup)?;
        return match tls {
            Some(tls) => {
                let addr = addr.parse().map_err(|_| {
                    sagat_control_plane::config::ConfigError::Invalid {
                        name: "SAGAT_HEALTHCHECK_ADDR",
                        reason: "must be an exact IP socket address for a TLS healthcheck"
                            .to_owned(),
                    }
                })?;
                sagat_control_plane::healthcheck_tls(addr, &tls).await
            }
            None => sagat_control_plane::healthcheck(&addr).await,
        };
    }

    let config = Config::from_lookup(&env_lookup)?;
    let telemetry = sagat_telemetry::init(sagat_control_plane::config::telemetry(
        &env_lookup,
        "sagat-control-plane",
        config.log_format,
    )?);
    let result = execute(command, &config).await;
    telemetry
        .shutdown(sagat_control_plane::config::TELEMETRY_FLUSH)
        .await;
    result
}

async fn execute(command: Command, config: &Config) -> Result<(), RunError> {
    match command {
        Command::Migrate => {
            sagat_control_plane::run_migrations(&config.database).await?;
            tracing::info!("migrations are up to date");
            Ok(())
        }
        Command::SeedDev => {
            let seed = SeedConfig::from_lookup(&env_lookup)?;
            match sagat_control_plane::seed_dev(config, &seed).await? {
                SeedOutcome::Created => {
                    tracing::info!(tenant = %seed.tenant_slug, "development tenant created")
                }
                SeedOutcome::AlreadyPresent => {
                    tracing::info!(tenant = %seed.tenant_slug, "development tenant already present");
                }
            }
            if let Some(wanted) = NamespaceSeed::from_lookup(&env_lookup)? {
                match sagat_control_plane::seed_dev_namespace(config, &seed, &wanted).await? {
                    SeedOutcome::Created => {
                        tracing::info!(namespace = %wanted.namespace_slug, "development namespace created");
                    }
                    SeedOutcome::AlreadyPresent => {
                        tracing::info!(namespace = %wanted.namespace_slug, "development namespace already present");
                    }
                }
            }
            Ok(())
        }
        Command::ResealSecrets => {
            let secrets = sagat_control_plane::config::read_secrets(&env_lookup)?.ok_or(
                sagat_control_plane::config::ConfigError::Missing("SAGAT_SECRETS_KEYS"),
            )?;
            let moved = sagat_control_plane::reseal_secrets(config, &secrets).await?;
            tracing::info!(moved, "secret versions re-sealed under the active key");
            Ok(())
        }
        Command::QuotaCeiling => {
            let request = sagat_control_plane::config::CeilingConfig::from_lookup(&env_lookup)?;
            sagat_control_plane::grant_quota_ceiling(config, &request).await?;
            tracing::info!(
                tenant = %request.tenant_id,
                resource = %request.resource,
                "quota ceiling changed"
            );
            Ok(())
        }
        Command::SchedulingWeight => {
            let request = sagat_control_plane::config::WeightConfig::from_lookup(&env_lookup)?;
            sagat_control_plane::set_scheduling_weight(config, &request).await?;
            tracing::info!(tenant = %request.tenant_id, "scheduling weight changed");
            Ok(())
        }
        Command::ProvisionTenant => {
            let request = ProvisionConfig::from_lookup(&env_lookup)?;
            sagat_control_plane::provision_tenant(config, &request).await?;
            // The administrator's subject can be personal data; only the tenant is logged.
            tracing::info!(tenant = %request.tenant_slug, "tenant provisioned");
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
                config,
                &serve,
                listener,
                admin,
                sagat_control_plane::shutdown_signal(),
            )
            .await
        }
        Command::Help | Command::Version | Command::Healthcheck | Command::ScriptKey => Ok(()),
    }
}
