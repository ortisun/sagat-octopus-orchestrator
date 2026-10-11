//! The execution engine process: the worker gRPC API, the scheduler loop and the outbox dispatcher
//! over one PostgreSQL database.
//!
//! `main.rs` only parses arguments and the environment; everything testable lives here.
//!
//! Replicas are interchangeable. Claims are `FOR UPDATE SKIP LOCKED`, scheduler commands are
//! re-checked under the run's row lock, and outbox messages are claimed with a visibility timeout,
//! so running several engines is safe (only redundant where work is shared).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use sagat_api_grpc::{WorkerGrpc, serve as serve_grpc, worker_service};
use sagat_api_http::{AppState, HttpJwksFetcher, TokenVerifier};
use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::identity::IdentityService;
use sagat_application::worker::WorkerService;
use sagat_control_plane::config::{self, Config, Environment};
use sagat_control_plane::{
    METRICS_UPKEEP_INTERVAL, RunError, admin_router_with, open_pool, spawn_metrics_upkeep, stopped,
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_nats::NatsPublisher;
use sagat_infra_postgres::{PgIdentityStore, PgRunStore, spawn_pool_metrics};
use sagat_scheduler::{
    DeliveryWorker, EventDispatcher, Monitor, OutboxDispatcher, ProjectorSettings, Scheduler,
    TelemetryProjector,
};
use sagat_telemetry::Metrics;
use tokio::net::TcpListener;
use tokio::sync::watch;

pub mod cli;
pub mod loops;
pub mod settings;

pub use sagat_control_plane::{probe, shutdown_signal};
pub use settings::EngineConfig;

use loops::{
    EngineDeliveries, EngineDispatcher, EngineEvents, EngineMonitor, EngineProjector,
    EngineScheduler, Health,
};

/// Parsed workflow versions kept in memory (they are immutable, so they never go stale).
const DEFINITION_CACHE_ENTRIES: usize = 1024;

/// Runs the engine until `shutdown` completes, then stops the loops and drains in-flight calls.
///
/// # Errors
/// Fails fast, before listening, on an unusable database, role or authentication setup.
pub async fn serve(
    config: &Config,
    engine: &EngineConfig,
    grpc_listener: TcpListener,
    admin_listener: Option<TcpListener>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), RunError> {
    // Plaintext key URLs let anyone on the path forge identities: development only.
    if engine.auth.allow_insecure_jwks_http && config.environment != Environment::Dev {
        return Err(RunError::Refused(
            "SAGAT_ALLOW_INSECURE_JWKS_HTTP is only allowed when SAGAT_ENV=dev",
        ));
    }
    // Everything checkable without the database comes first.
    let fetcher = HttpJwksFetcher::new(&engine.auth).map_err(|e| RunError::Auth(e.to_string()))?;
    let verifier = TokenVerifier::new(Arc::new(fetcher), &engine.auth);
    let pool = open_pool(&config.database, config.database.statement_timeout).await?;
    let identity = PgIdentityStore::new(pool.clone());
    if config.environment == Environment::Dev {
        tracing::debug!("database role privileges are not checked when SAGAT_ENV=dev");
    } else {
        if let Err(error) = identity.verify_least_privilege().await {
            pool.close().await;
            return Err(error.into());
        }
        tracing::info!("database role verified: it cannot rewrite the audit trail");
    }

    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), DEFINITION_CACHE_ENTRIES);
    let store = PgRunStore::new(pool.clone(), cache.clone()).with_breaker(engine.breaker);
    let state = AppState::with_rate_limit(
        IdentityService::new(identity, SystemClock),
        verifier,
        engine.rate_limit,
    )
    .map_err(|e| config::rate_limit_error(config::TENANT_RATE_VARS, e))?
    .with_subject_rate_limit(engine.subject_rate_limit)
    .map_err(|e| config::rate_limit_error(config::SUBJECT_RATE_VARS, e))?;
    let mut workers = WorkerService::new(
        store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        engine.worker,
    );
    if let Some(secrets) = &engine.secrets {
        workers = workers.with_vault(secrets.cipher());
    }
    let scheduler: Arc<EngineScheduler> = Arc::new(
        Scheduler::new(
            store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            engine.scheduler,
        )
        .with_children(cache.clone()),
    );
    let events: Arc<EngineEvents> = Arc::new(
        EventDispatcher::new(
            store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            cache,
            engine.events,
        )
        .with_cron(Arc::new(sagat_infra_ir::cron::CronCalendar)),
    );
    // Deliveries are signed with a secret of the namespace, so without a key ring there is nothing to
    // deliver with.
    let deliveries: Option<Arc<EngineDeliveries>> = engine.secrets.as_ref().map(|keys| {
        Arc::new(DeliveryWorker::new(
            store.clone(),
            sagat_infra_connectors::webhook::GuardedSender::new(store.clone()),
            SystemEntropy,
            keys.cipher(),
            engine.deliveries,
        ))
    });
    // The consumers of the connectors read brokers with credentials that are secrets of the
    // namespace, so they need the key ring too.
    let connectors = engine.secrets.as_ref().map(|keys| {
        Arc::new(sagat_infra_connectors::supervisor::Supervisor::new(
            Arc::new(store.clone()),
            sagat_infra_connectors::guard::PolicyGuard::new(store.clone()),
            sagat_infra_connectors::webhook::Clients::new(store.clone()),
            keys.cipher(),
            Arc::new(JsonEvaluator),
            format!("engine-{}", uuid::Uuid::new_v4()),
            engine.connector_refresh,
        ))
    });
    let dispatcher: Option<Arc<EngineDispatcher>> = match &engine.nats {
        Some(nats) => {
            let publisher = NatsPublisher::connect(nats.clone()).await.map_err(|e| {
                RunError::Refused(match e {
                    sagat_infra_nats::NatsError::Config(reason) => reason,
                    sagat_infra_nats::NatsError::Connect => "could not set up the NATS client",
                })
            })?;
            Some(Arc::new(OutboxDispatcher::new(
                store.clone(),
                publisher,
                SystemEntropy,
                engine.dispatch,
            )))
        }
        None => {
            tracing::warn!(
                "SAGAT_NATS_URL is not set: task hints are not published, workers must poll, \
                 and the outbox accumulates until a dispatcher is configured"
            );
            None
        }
    };

    // The journal becomes spans and measurements here, off the path of every request and every
    // transition. With no collector configured the measurements are still made.
    let exporter = match engine.telemetry.otlp.clone() {
        Some(otlp) => Some(
            sagat_telemetry::OtlpClient::new(otlp)
                .map_err(|_| RunError::Refused("the telemetry exporter could not be set up"))?,
        ),
        None => None,
    };
    // The sampler that decides which live spans are kept decides which projected ones are too.
    let projector_settings = ProjectorSettings {
        sample_ratio: engine
            .telemetry
            .otlp
            .as_ref()
            .map_or(1.0, |otlp| otlp.sample_ratio),
        ..engine.telemetry.projector
    };
    let projector: Arc<EngineProjector> = Arc::new(TelemetryProjector::new(
        store.clone(),
        exporter,
        projector_settings,
    ));

    let monitor: Arc<EngineMonitor> =
        Arc::new(Monitor::new(store.clone(), engine.monitor.settings.clone()));

    let metrics_upkeep = spawn_metrics_upkeep(Metrics::global(), METRICS_UPKEEP_INTERVAL);
    let pool_metrics = spawn_pool_metrics(pool.clone(), Duration::from_secs(5));
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::spawn(async move {
        shutdown.await;
        let _ = stop_tx.send(true);
    });

    let health = Arc::new(Health::new(
        (engine.tick_interval * 10).max(engine.liveness_grace),
    ));
    let mut tasks = vec![
        tokio::spawn(loops::run_scheduler(
            {
                let scheduler = Arc::clone(&scheduler);
                move || {
                    let scheduler = Arc::clone(&scheduler);
                    async move { scheduler.tick().await }
                }
            },
            engine.tick_interval,
            Arc::clone(&health),
            stop_rx.clone(),
        )),
        tokio::spawn(loops::run_housekeeping(
            {
                let scheduler = Arc::clone(&scheduler);
                let events = Arc::clone(&events);
                let deliveries = deliveries.clone();
                let store = store.clone();
                move || {
                    let scheduler = Arc::clone(&scheduler);
                    let events = Arc::clone(&events);
                    let deliveries = deliveries.clone();
                    let store = store.clone();
                    async move {
                        scheduler.housekeeping().await;
                        events.housekeeping().await;
                        // The notes of runs that finished and were projected a day ago.
                        if let Err(error) = store
                            .purge_projected(Duration::from_secs(24 * 3600), 1000)
                            .await
                        {
                            tracing::warn!(%error, "could not purge the telemetry notes of finished runs");
                        }
                        if let Some(deliveries) = deliveries {
                            deliveries.housekeeping().await;
                        }
                    }
                }
            },
            engine.housekeeping_interval,
            stop_rx.clone(),
        )),
        tokio::spawn(loops::run_events(
            {
                let events = Arc::clone(&events);
                move || {
                    let events = Arc::clone(&events);
                    async move { events.tick().await }
                }
            },
            engine.event_poll,
            stop_rx.clone(),
        )),
    ];
    tasks.push(tokio::spawn(loops::run_telemetry(
        {
            let projector = Arc::clone(&projector);
            move || {
                let projector = Arc::clone(&projector);
                async move { projector.run_once().await }
            }
        },
        engine.telemetry.poll,
        stop_rx.clone(),
    )));
    tasks.push(tokio::spawn(loops::run_monitor(
        {
            let monitor = Arc::clone(&monitor);
            move || {
                let monitor = Arc::clone(&monitor);
                async move { monitor.run_once().await }
            }
        },
        engine.monitor.interval,
        stop_rx.clone(),
    )));
    if let Some(connectors) = connectors {
        tasks.push(tokio::spawn(connectors.run(stop_rx.clone())));
    }
    if let Some(deliveries) = deliveries {
        tasks.push(tokio::spawn(loops::run_deliveries(
            move || {
                let deliveries = Arc::clone(&deliveries);
                async move { deliveries.tick().await }
            },
            engine.delivery_poll,
            stop_rx.clone(),
        )));
    }
    if let Some(dispatcher) = dispatcher {
        tasks.push(tokio::spawn(loops::run_dispatcher(
            move || {
                let dispatcher = Arc::clone(&dispatcher);
                async move { dispatcher.run_once().await }
            },
            engine.dispatch.batch,
            engine.outbox_poll,
            stop_rx.clone(),
        )));
    }
    let admin = match admin_listener {
        Some(listener) => {
            let addr = listener.local_addr()?;
            tracing::info!(%addr, "admin listener (metrics and probes)");
            if config::admin_addr_is_exposed(addr) {
                tracing::warn!(
                    %addr,
                    "the admin endpoints are unauthenticated and listen on a non-loopback address; \
                     restrict them with a NetworkPolicy or firewall rule"
                );
            }
            Some(tokio::spawn(sagat_api_http::serve(
                listener,
                admin_router_with(probes(store.clone(), Arc::clone(&health))),
                sagat_api_http::ServerConfig::default(),
                stopped(stop_rx.clone()),
            )))
        }
        None => None,
    };

    tracing::info!(
        addr = %grpc_listener.local_addr()?,
        environment = ?config.environment,
        issuer = %config::redact_url(&engine.auth.issuer),
        audience = %engine.auth.audience,
        "execution engine listening (worker gRPC API)"
    );
    let outcome = serve_grpc(
        grpc_listener,
        worker_service(WorkerGrpc::new(state, workers)),
        engine.grpc.clone(),
        stopped(stop_rx.clone()),
    )
    .await
    .map_err(|e| RunError::Io(std::io::Error::other(e.to_string())));

    // Whatever ended the server, tell the loops to stop and wait for them.
    drop(stop_rx);
    for task in tasks {
        let _ = task.await;
    }
    if let Some(admin) = admin {
        let _ = admin.await;
    }
    tracing::info!("execution engine stopped");
    pool_metrics.abort();
    metrics_upkeep.abort();
    pool.close().await;
    outcome
}

/// `/livez` (the scheduler loop is ticking) and `/readyz` (and the database is usable).
fn probes(store: PgRunStore, health: Arc<Health>) -> Router {
    let live = {
        let health = Arc::clone(&health);
        move || {
            let health = Arc::clone(&health);
            async move {
                if health.scheduler_alive() {
                    (StatusCode::OK, "alive")
                } else {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "the scheduler loop stopped ticking",
                    )
                }
            }
        }
    };
    let ready = move || {
        let store = store.clone();
        let health = Arc::clone(&health);
        async move {
            if !health.scheduler_alive() {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the scheduler loop stopped ticking",
                );
            }
            match tokio::time::timeout(Duration::from_secs(2), store.ready()).await {
                Ok(Ok(())) => (StatusCode::OK, "ready"),
                _ => (StatusCode::SERVICE_UNAVAILABLE, "the database is not ready"),
            }
        }
    };
    Router::new()
        .route("/livez", get(live))
        .route("/readyz", get(ready))
}
