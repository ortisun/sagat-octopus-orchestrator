//! `sandbox-runner` executable: argument and environment handling only.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use sagat_infra_sandbox::DockerSandbox;
use sagat_sandbox_runner::cli::{self, Command, USAGE, UsageError};
use sagat_sandbox_runner::{Runner, RunnerConfig};
use sagat_telemetry::{LogFormat, init_logging};

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
            println!("sandbox-runner {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        other => match run(other).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::from(1)
            }
        },
    }
}

async fn run(command: Command) -> Result<(), String> {
    if command == Command::Healthcheck {
        let addr =
            env_lookup("SAGAT_HEALTHCHECK_ADDR").unwrap_or_else(|| "127.0.0.1:9092".to_owned());
        return sagat_control_plane::probe(&addr, "/livez")
            .await
            .map_err(|e| e.to_string());
    }
    let log_format = match env_lookup("SAGAT_LOG_FORMAT").as_deref().map(str::trim) {
        None | Some("" | "json") => LogFormat::Json,
        Some("pretty") => LogFormat::Pretty,
        Some(_) => return Err("SAGAT_LOG_FORMAT is json or pretty".to_owned()),
    };
    init_logging(log_format);
    let config = RunnerConfig::from_lookup(&env_lookup).map_err(|e| e.to_string())?;
    let sandbox = DockerSandbox::connect(config.images.clone())
        .map_err(|_| "the container engine could not be reached".to_owned())?;
    let admin_addr = config.admin_addr;
    let sweep_after = config.sweep_after;
    // Install the metrics recorder before anything runs so the first samples are kept; idle series
    // are forgotten when the page is rendered, so a process nobody scrapes needs this timer.
    let metrics_upkeep = sagat_control_plane::spawn_metrics_upkeep(
        sagat_telemetry::Metrics::global(),
        sagat_control_plane::METRICS_UPKEEP_INTERVAL,
    );
    let runner = Runner::start(config, sandbox)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(languages = ?runner.languages(), "sandbox runner started");

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        sagat_control_plane::shutdown_signal().await;
        let _ = stop_tx.send(true);
    });
    let admin = match admin_addr {
        Some(addr) => {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| e.to_string())?;
            let live = axum_live();
            Some(tokio::spawn(sagat_api_http::serve(
                listener,
                sagat_control_plane::admin_router_with(live),
                sagat_api_http::ServerConfig::default(),
                sagat_control_plane::stopped(stop_rx.clone()),
            )))
        }
        None => None,
    };
    // Containers a runner that died left behind are removed.
    let sweeper = {
        let runner = Arc::clone(&runner);
        tokio::spawn(async move {
            loop {
                let removed = runner.sandbox().sweep(sweep_after).await;
                if removed > 0 {
                    tracing::warn!(removed, "old sandbox containers were removed");
                }
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        })
    };
    runner.serve(sagat_control_plane::stopped(stop_rx)).await;
    sweeper.abort();
    metrics_upkeep.abort();
    if let Some(admin) = admin {
        let _ = admin.await;
    }
    Ok(())
}

/// `/livez`: the process answers.
fn axum_live() -> axum::Router {
    axum::Router::new().route("/livez", axum::routing::get(|| async { "ok" }))
}
