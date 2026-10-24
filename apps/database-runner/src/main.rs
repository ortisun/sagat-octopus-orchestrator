//! An isolated executable: the platform database and OpenBao tokens are never configured here.
use sagat_database_runner::{Runner, RunnerError};
use sagat_telemetry::{LogFormat, Metrics, OtlpConfig, TelemetryConfig};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // A library panic may carry SQL or server diagnostics. Preserve a closed incident signal.
    std::panic::set_hook(Box::new(|_| eprintln!("database runner task panicked")));
    let result = run().await;
    if let Err(error) = result {
        eprintln!("{error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

async fn run() -> Result<(), RunnerError> {
    let path = std::env::var_os("SAGAT_DATABASE_RUNNER_CONFIG")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .ok_or(RunnerError::Configuration)?;
    let addr: SocketAddr = std::env::var("SAGAT_DATABASE_RUNNER_METRICS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9092".into())
        .parse()
        .map_err(|_| RunnerError::Configuration)?;
    let otlp = OtlpConfig::from_lookup(&|key| std::env::var(key).ok(), "database-runner")
        .map_err(|_| RunnerError::Configuration)?;
    let telemetry = sagat_telemetry::init(TelemetryConfig {
        format: LogFormat::Json,
        service: "database-runner".into(),
        otlp,
    });
    let metrics = Metrics::global();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|_| RunnerError::Configuration)?;
    let runner = Runner::start(path).await?;
    let stop = CancellationToken::new();
    let shutdown = stop.clone();
    let mut serving = tokio::spawn(runner.serve(stop.clone()));
    let router = axum::Router::new().route(
        "/metrics",
        axum::routing::get(move || async move {
            (
                [("content-type", "text/plain; version=0.0.4")],
                metrics.render(),
            )
        }),
    );
    let mut monitoring = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    });
    let mut upkeep = tokio::time::interval(Duration::from_secs(60));
    let signal = shutdown_signal();
    tokio::pin!(signal);
    let outcome = loop {
        tokio::select! {
            ()=&mut signal => break Ok(()),
            _=&mut monitoring => break Err(RunnerError::Transport),
            _=&mut serving => break Err(RunnerError::Transport),
            _=upkeep.tick()=>metrics.prune_idle(),
        }
    };
    stop.cancel();
    if !serving.is_finished() {
        let _ = (&mut serving).await;
    }
    if !monitoring.is_finished()
        && tokio::time::timeout(Duration::from_secs(2), &mut monitoring)
            .await
            .is_err()
    {
        monitoring.abort();
        let _ = monitoring.await;
    }
    telemetry.shutdown(Duration::from_secs(2)).await;
    outcome
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=terminate.recv()=>{} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
