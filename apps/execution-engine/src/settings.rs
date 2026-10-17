//! Settings of `serve`, read from environment variables like the control plane's (the shared parts
//! come from `sagat_control_plane::config`).

use std::net::SocketAddr;
use std::time::Duration;

use sagat_api_grpc::{BlobTransfer, GrpcConfig, TlsConfig};
use sagat_api_http::{AuthConfig, RateLimitConfig};
use sagat_application::execution::BreakerPolicy;
use sagat_application::operations::WorkerPool;
use sagat_application::worker::WorkerSettings;
use sagat_control_plane::config::{
    ConfigError, Environment, Lookup, SUBJECT_RATE_VARS, TENANT_RATE_VARS, flag, optional,
    rate_limit_error, read_auth, read_rate_limit, read_secrets,
};
use sagat_infra_blobstore::{BlobSettings, BlobStoreConfig, ConfigError as BlobConfigError};
use sagat_infra_nats::NatsConfig;
use sagat_scheduler::{
    DeliverySettings, DispatchSettings, EventDispatchSettings, KeeperSettings, MonitorSettings,
    ProjectorSettings, SchedulerSettings,
};
use sagat_telemetry::OtlpConfig;

/// The projection of the journal into spans and measurements (ADR-0017).
#[derive(Debug, Clone)]
pub struct TelemetrySettings {
    /// Where spans go (the standard `OTEL_*` variables); `None`: the measurements are made and the
    /// spans are not sent.
    pub otlp: Option<OtlpConfig>,
    pub projector: ProjectorSettings,
    /// Pause between rounds when the last one found nothing to do.
    pub poll: Duration,
}

/// The monitor of operational conditions (FR-008-005).
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub settings: MonitorSettings,
    /// How often it looks.
    pub interval: Duration,
}

/// Everything `serve` needs beyond the shared database settings.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub grpc_addr: SocketAddr,
    /// Where `/metrics`, `/livez` and `/readyz` are served; `None` disables the listener.
    pub admin_addr: Option<SocketAddr>,
    pub auth: AuthConfig,
    /// Per-tenant budget of worker calls, charged after authentication.
    pub rate_limit: RateLimitConfig,
    /// Per-identity budget, charged before any database lookup.
    pub subject_rate_limit: RateLimitConfig,
    pub grpc: GrpcConfig,
    pub worker: WorkerSettings,
    /// When the compensations of a provider operation that keeps failing stop being leased.
    pub breaker: BreakerPolicy,
    pub scheduler: SchedulerSettings,
    /// How the replica holds its shards of the scheduler's work (`FR-010-001`).
    pub shards: KeeperSettings,
    /// Names the replica in the leases (`SAGAT_REPLICA_ID`, often the pod name); a random suffix is
    /// added at start so that a restarted process is never mistaken for the one it replaces.
    pub replica: String,
    /// Pause between scheduler ticks when the last one found nothing to do.
    pub tick_interval: Duration,
    pub housekeeping_interval: Duration,
    /// How long the scheduler loop may go without finishing a tick before `/livez` reports it
    /// stuck (and the orchestrator restarts the process).
    pub liveness_grace: Duration,
    pub dispatch: DispatchSettings,
    /// Pause between dispatch rounds when the last one found nothing to send.
    pub outbox_poll: Duration,
    /// The dispatcher that turns admitted events into started runs and woken steps.
    pub events: EventDispatchSettings,
    /// Pause between event dispatch rounds when the last one found nothing to do.
    pub event_poll: Duration,
    /// The worker that sends outbound webhooks.
    pub deliveries: DeliverySettings,
    /// Pause between delivery rounds when the last one found nothing to send.
    pub delivery_poll: Duration,
    /// How often the connectors are looked at, to start and stop their consumers.
    pub connector_refresh: Duration,
    /// The telemetry projector: how often it looks, how much it takes, where it exports.
    pub telemetry: TelemetrySettings,
    /// The monitor that raises operational alerts and samples the queues.
    pub monitor: MonitorConfig,
    /// `None`: hints are not published (workers poll; the outbox accumulates).
    pub nats: Option<NatsConfig>,
    /// The key ring that opens secrets for workers (`SAGAT_SECRETS_KEYS`); `None` turns the
    /// `ResolveSecrets` call off.
    pub secrets: Option<sagat_infra_secrets::SecretsConfig>,
    pub openbao: Option<sagat_infra_secrets::openbao::OpenBaoSettings>,
    pub blobs: Option<BlobSettings>,
    pub blob_transfer: BlobTransfer,
}

/// The most candidates of one pass a scheduler acts on at once; each holds a database connection.
const MAX_SCHEDULER_CONCURRENCY: usize = 64;
/// The most candidates one pass asks for (`SAGAT_SCHEDULER_BATCH`) and messages one dispatch round claims
/// (`SAGAT_OUTBOX_BATCH`): each query is `LIMIT batch` for every shard the replica holds, and a search for
/// other tenants reads up to sixteen such pages, so a mistyped value is the database's load.
const MAX_BATCH: u32 = 5_000;
const MIN_SHARD_LEASE: Duration = Duration::from_secs(3);
const MAX_SHARD_LEASE: Duration = Duration::from_secs(600);
const MAX_REPLICA_ID: usize = 64;
/// The longest a search for other tenants may be rested: a tenant that arrives behind a burst waits
/// for it.
const MAX_DISCOVERY_REST_MS: u64 = 60_000;
/// Pools whose queues are measured, and activities in all of them: each pool is two series.
const MAX_POOLS: usize = 32;
const MAX_POOL_ACTIVITIES: usize = 512;

/// `SAGAT_WORKER_POOLS`: `name=activity,activity;name=activity`. The name is what the pool's gauges
/// are labelled with, so it is the operator's, short and made of letters, digits and `. _ -`; an
/// activity is a name a workflow uses, with no separator in it.
fn worker_pools(get: Lookup<'_>) -> Result<Vec<WorkerPool>, ConfigError> {
    const NAME: &str = "SAGAT_WORKER_POOLS";
    let invalid = |reason: String| ConfigError::Invalid { name: NAME, reason };
    let Some(spec) = get(NAME)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
    else {
        return Ok(Vec::new());
    };
    let mut pools: Vec<WorkerPool> = Vec::new();
    let mut activities = 0;
    for part in spec.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, list) = part
            .split_once('=')
            .ok_or_else(|| invalid("each pool is name=activity,activity".to_owned()))?;
        let name = name.trim();
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
            })
        {
            return Err(invalid(
                "a pool name is 1 to 64 lowercase letters, digits or . _ -".to_owned(),
            ));
        }
        if pools.iter().any(|p| p.name == name) {
            return Err(invalid(format!("the pool {name} is named twice")));
        }
        let list: Vec<String> = list
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .collect();
        if list.is_empty() {
            return Err(invalid(format!("the pool {name} serves no activity")));
        }
        if let Some(bad) = list
            .iter()
            .find(|a| a.len() > 128 || !a.bytes().all(|b| b.is_ascii_graphic()))
        {
            return Err(invalid(format!(
                "an activity of the pool {name} is too long or has a character that is not printable ASCII ({} bytes)",
                bad.len()
            )));
        }
        // An activity named twice in a pool is one.
        let mut list = list;
        let mut seen = std::collections::BTreeSet::new();
        list.retain(|a| seen.insert(a.clone()));
        activities += list.len();
        pools.push(WorkerPool {
            name: name.to_owned(),
            activities: list,
        });
    }
    if pools.len() > MAX_POOLS || activities > MAX_POOL_ACTIVITIES {
        return Err(invalid(format!(
            "at most {MAX_POOLS} pools and {MAX_POOL_ACTIVITIES} activities in all"
        )));
    }
    Ok(pools)
}

/// `SAGAT_REPLICA_ID`: printable ASCII without spaces, short; `engine` when it is not set.
fn replica_name(get: Lookup<'_>) -> Result<String, ConfigError> {
    let Some(name) = get("SAGAT_REPLICA_ID")
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
    else {
        return Ok("engine".to_owned());
    };
    if name.len() > MAX_REPLICA_ID || !name.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(ConfigError::Invalid {
            name: "SAGAT_REPLICA_ID",
            reason: format!(
                "must be at most {MAX_REPLICA_ID} printable ASCII characters without spaces"
            ),
        });
    }
    Ok(name)
}

fn positive(name: &'static str, value: u64) -> Result<u64, ConfigError> {
    if value == 0 {
        return Err(ConfigError::Invalid {
            name,
            reason: "must be greater than zero".to_owned(),
        });
    }
    Ok(value)
}

/// A PEM file named by `name`, if the variable is set.
fn pem_file(get: Lookup<'_>, name: &'static str) -> Result<Option<Vec<u8>>, ConfigError> {
    let Some(path) = get(name)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    let bytes = std::fs::read(&path).map_err(|e| ConfigError::Invalid {
        name,
        reason: format!("cannot be read ({})", e.kind()),
    })?;
    if !bytes.windows(10).any(|w| w == b"-----BEGIN") {
        return Err(ConfigError::Invalid {
            name,
            reason: "is not a PEM file".to_owned(),
        });
    }
    Ok(Some(bytes))
}

/// TLS for the worker API, from `SAGAT_GRPC_TLS_CERT_FILE` and `SAGAT_GRPC_TLS_KEY_FILE` (both or
/// neither) and, for mutual TLS, `SAGAT_GRPC_TLS_CLIENT_CA_FILE`.
fn read_tls(get: Lookup<'_>) -> Result<Option<TlsConfig>, ConfigError> {
    let certificate = pem_file(get, "SAGAT_GRPC_TLS_CERT_FILE")?;
    let key = pem_file(get, "SAGAT_GRPC_TLS_KEY_FILE")?;
    let client_ca = pem_file(get, "SAGAT_GRPC_TLS_CLIENT_CA_FILE")?;
    match (certificate, key, client_ca) {
        (None, None, None) => Ok(None),
        (Some(certificate_pem), Some(private_key_pem), client_ca_pem) => Ok(Some(TlsConfig {
            certificate_pem,
            private_key_pem,
            client_ca_pem,
        })),
        (Some(_), None, _) => Err(ConfigError::Invalid {
            name: "SAGAT_GRPC_TLS_KEY_FILE",
            reason: "is required together with SAGAT_GRPC_TLS_CERT_FILE".to_owned(),
        }),
        (None, Some(_), _) => Err(ConfigError::Invalid {
            name: "SAGAT_GRPC_TLS_CERT_FILE",
            reason: "is required together with SAGAT_GRPC_TLS_KEY_FILE".to_owned(),
        }),
        (None, None, Some(_)) => Err(ConfigError::Invalid {
            name: "SAGAT_GRPC_TLS_CLIENT_CA_FILE",
            reason:
                "needs a server certificate: set SAGAT_GRPC_TLS_CERT_FILE and SAGAT_GRPC_TLS_KEY_FILE"
                    .to_owned(),
        }),
    }
}

/// A positive count read from `name`, or `default`.
fn seconds(
    get: Lookup<'_>,
    name: &'static str,
    default: Duration,
) -> Result<Duration, ConfigError> {
    Ok(Duration::from_secs(positive(
        name,
        optional(get, name, default.as_secs())?,
    )?))
}

fn capacity(get: Lookup<'_>, name: &'static str, default: usize) -> Result<usize, ConfigError> {
    let value = positive(name, optional(get, name, default as u64)?)?;
    usize::try_from(value).map_err(|_| ConfigError::Invalid {
        name,
        reason: "is too large".to_owned(),
    })
}

impl EngineConfig {
    /// # Errors
    /// [`ConfigError`] naming the first missing or invalid variable.
    pub fn from_lookup(get: Lookup<'_>, environment: Environment) -> Result<Self, ConfigError> {
        let auth = read_auth(get)?;
        // Workers poll continuously, so their budgets are an order of magnitude above the REST
        // API's; they are still finite, because a leaked worker credential must not be a free
        // load generator.
        let rate_limit = read_rate_limit(
            get,
            TENANT_RATE_VARS,
            RateLimitConfig {
                per_second: 2000.0,
                burst: 4000.0,
            },
        )?;
        let subject_rate_limit = read_rate_limit(
            get,
            SUBJECT_RATE_VARS,
            RateLimitConfig {
                per_second: 200.0,
                burst: 400.0,
            },
        )?;
        // Re-validated here because the limiter must never run with a budget that could panic.
        rate_limit
            .validate()
            .map_err(|e| rate_limit_error(TENANT_RATE_VARS, e))?;
        subject_rate_limit
            .validate()
            .map_err(|e| rate_limit_error(SUBJECT_RATE_VARS, e))?;

        let admin_addr =
            match get("SAGAT_ADMIN_ADDR").as_deref().map(str::trim) {
                None | Some("") => Some(SocketAddr::from(([127, 0, 0, 1], 9091))),
                Some("off") => None,
                Some(raw) => Some(raw.parse().map_err(|e: std::net::AddrParseError| {
                    ConfigError::Invalid {
                        name: "SAGAT_ADMIN_ADDR",
                        reason: e.to_string(),
                    }
                })?),
            };

        let grpc_defaults = GrpcConfig::default();
        let grpc = GrpcConfig {
            request_timeout: Duration::from_secs(positive(
                "SAGAT_GRPC_REQUEST_TIMEOUT_SECS",
                optional(
                    get,
                    "SAGAT_GRPC_REQUEST_TIMEOUT_SECS",
                    grpc_defaults.request_timeout.as_secs(),
                )?,
            )?),
            max_connections: capacity(
                get,
                "SAGAT_GRPC_MAX_CONNECTIONS",
                grpc_defaults.max_connections,
            )?,
            max_connections_per_peer: capacity(
                get,
                "SAGAT_GRPC_MAX_CONNECTIONS_PER_PEER",
                grpc_defaults.max_connections_per_peer,
            )?,
            first_byte_timeout: seconds(
                get,
                "SAGAT_GRPC_FIRST_BYTE_TIMEOUT_SECS",
                grpc_defaults.first_byte_timeout,
            )?,
            idle_timeout: seconds(
                get,
                "SAGAT_GRPC_IDLE_TIMEOUT_SECS",
                grpc_defaults.idle_timeout,
            )?,
            handshake_timeout: seconds(
                get,
                "SAGAT_GRPC_TLS_HANDSHAKE_TIMEOUT_SECS",
                grpc_defaults.handshake_timeout,
            )?,
            max_connection_age: seconds(
                get,
                "SAGAT_GRPC_MAX_CONNECTION_AGE_SECS",
                grpc_defaults.max_connection_age,
            )?,
            max_in_flight: capacity(get, "SAGAT_GRPC_MAX_IN_FLIGHT", grpc_defaults.max_in_flight)?,
            transfer_timeout: seconds(
                get,
                "SAGAT_BLOB_TRANSFER_TIMEOUT_SECS",
                grpc_defaults.transfer_timeout,
            )?,
            tls: read_tls(get)?,
            ..grpc_defaults
        };

        let blob_defaults = BlobTransfer::default();
        let blob_transfer = BlobTransfer {
            max_concurrent: capacity(
                get,
                "SAGAT_BLOB_MAX_TRANSFERS",
                blob_defaults.max_concurrent,
            )?,
            timeout: grpc.transfer_timeout,
        };
        let blobs = read_blobs(get, environment)?;
        let worker_defaults = WorkerSettings::default();
        let worker = WorkerSettings {
            lease_ttl: Duration::from_secs(positive(
                "SAGAT_LEASE_TTL_SECS",
                optional(
                    get,
                    "SAGAT_LEASE_TTL_SECS",
                    worker_defaults.lease_ttl.as_secs(),
                )?,
            )?),
            session_ttl: Duration::from_secs(positive(
                "SAGAT_SESSION_TTL_SECS",
                optional(
                    get,
                    "SAGAT_SESSION_TTL_SECS",
                    worker_defaults.session_ttl.as_secs(),
                )?,
            )?),
            idle_retry: Duration::from_millis(positive(
                "SAGAT_POLL_IDLE_RETRY_MS",
                optional(
                    get,
                    "SAGAT_POLL_IDLE_RETRY_MS",
                    u64::try_from(worker_defaults.idle_retry.as_millis()).unwrap_or(500),
                )?,
            )?),
        };

        let breaker_defaults = BreakerPolicy::default();
        let breaker = BreakerPolicy {
            // Zero turns the breaker off.
            threshold: u32::try_from(optional(
                get,
                "SAGAT_COMPENSATION_BREAKER_THRESHOLD",
                u64::from(breaker_defaults.threshold),
            )?)
            .map_err(|_| ConfigError::Invalid {
                name: "SAGAT_COMPENSATION_BREAKER_THRESHOLD",
                reason: "is too large".to_owned(),
            })?,
            open_for: seconds(
                get,
                "SAGAT_COMPENSATION_BREAKER_OPEN_SECS",
                breaker_defaults.open_for,
            )?,
            max_open: seconds(
                get,
                "SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS",
                breaker_defaults.max_open,
            )?,
        };
        if breaker.max_open < breaker.open_for {
            return Err(ConfigError::Invalid {
                name: "SAGAT_COMPENSATION_BREAKER_MAX_OPEN_SECS",
                reason: "must not be smaller than SAGAT_COMPENSATION_BREAKER_OPEN_SECS".to_owned(),
            });
        }

        let scheduler_defaults = SchedulerSettings::default();
        let scheduler = SchedulerSettings {
            batch: u32::try_from(positive(
                "SAGAT_SCHEDULER_BATCH",
                optional(
                    get,
                    "SAGAT_SCHEDULER_BATCH",
                    u64::from(scheduler_defaults.batch),
                )?,
            )?)
            .ok()
            .filter(|batch| *batch <= MAX_BATCH)
            .ok_or_else(|| ConfigError::Invalid {
                name: "SAGAT_SCHEDULER_BATCH",
                reason: format!("must be at most {MAX_BATCH}"),
            })?,
            concurrency: capacity(
                get,
                "SAGAT_SCHEDULER_CONCURRENCY",
                scheduler_defaults.concurrency,
            )?
            .min(MAX_SCHEDULER_CONCURRENCY),
            // On unless it is said to be off: it is a diagnostic to turn off, never a thing to turn on.
            fair: match get("SAGAT_SCHEDULER_FAIRNESS").as_deref().map(str::trim) {
                None | Some("") => true,
                Some(_) => flag(get, "SAGAT_SCHEDULER_FAIRNESS")?,
            },
            ..scheduler_defaults
        };
        // How long a pass rests its search for other tenants after one that found nobody (0: never).
        let discovery_rest = Duration::from_millis(
            optional(
                get,
                "SAGAT_SCHEDULER_DISCOVERY_REST_MS",
                u64::try_from(scheduler_defaults.discovery_rest.as_millis()).unwrap_or(100),
            )?
            .min(MAX_DISCOVERY_REST_MS),
        );
        let scheduler = SchedulerSettings {
            discovery_rest,
            ..scheduler
        };
        let shard_defaults = KeeperSettings::default();
        let shards = KeeperSettings {
            ttl: seconds(get, "SAGAT_SHARD_LEASE_SECS", shard_defaults.ttl)?,
        };
        // Shorter than this, a pause of the garbage collector or of a network hop loses the shards;
        // longer, a crashed replica's work waits that long before somebody takes it over.
        if !(MIN_SHARD_LEASE..=MAX_SHARD_LEASE).contains(&shards.ttl) {
            return Err(ConfigError::Invalid {
                name: "SAGAT_SHARD_LEASE_SECS",
                reason: format!(
                    "must be between {} and {} seconds",
                    MIN_SHARD_LEASE.as_secs(),
                    MAX_SHARD_LEASE.as_secs()
                ),
            });
        }
        let replica = replica_name(get)?;
        let tick_interval = Duration::from_millis(positive(
            "SAGAT_SCHEDULER_INTERVAL_MS",
            optional(get, "SAGAT_SCHEDULER_INTERVAL_MS", 500u64)?,
        )?);
        let housekeeping_interval = Duration::from_secs(positive(
            "SAGAT_HOUSEKEEPING_INTERVAL_SECS",
            optional(get, "SAGAT_HOUSEKEEPING_INTERVAL_SECS", 300u64)?,
        )?);

        let liveness_grace = Duration::from_secs(positive(
            "SAGAT_LIVENESS_GRACE_SECS",
            optional(get, "SAGAT_LIVENESS_GRACE_SECS", 30u64)?,
        )?);

        let dispatch_defaults = DispatchSettings::default();
        let dispatch = DispatchSettings {
            batch: u32::try_from(positive(
                "SAGAT_OUTBOX_BATCH",
                optional(
                    get,
                    "SAGAT_OUTBOX_BATCH",
                    u64::from(dispatch_defaults.batch),
                )?,
            )?)
            .ok()
            .filter(|batch| *batch <= MAX_BATCH)
            .ok_or_else(|| ConfigError::Invalid {
                name: "SAGAT_OUTBOX_BATCH",
                reason: format!("must be at most {MAX_BATCH}"),
            })?,
            discovery_rest,
            ..dispatch_defaults
        };
        let outbox_poll = Duration::from_millis(positive(
            "SAGAT_OUTBOX_POLL_MS",
            optional(get, "SAGAT_OUTBOX_POLL_MS", 250u64)?,
        )?);

        let event_defaults = EventDispatchSettings::default();
        let events = EventDispatchSettings {
            batch: u32::try_from(positive(
                "SAGAT_EVENT_BATCH",
                optional(get, "SAGAT_EVENT_BATCH", u64::from(event_defaults.batch))?,
            )?)
            .map_err(|_| ConfigError::Invalid {
                name: "SAGAT_EVENT_BATCH",
                reason: "is too large".to_owned(),
            })?,
            ..event_defaults
        };
        let event_poll = Duration::from_millis(positive(
            "SAGAT_EVENT_POLL_MS",
            optional(get, "SAGAT_EVENT_POLL_MS", 200u64)?,
        )?);

        let delivery_defaults = DeliverySettings::default();
        let deliveries = DeliverySettings {
            batch: u32::try_from(positive(
                "SAGAT_DELIVERY_BATCH",
                optional(
                    get,
                    "SAGAT_DELIVERY_BATCH",
                    u64::from(delivery_defaults.batch),
                )?,
            )?)
            .map_err(|_| ConfigError::Invalid {
                name: "SAGAT_DELIVERY_BATCH",
                reason: "is too large".to_owned(),
            })?,
            retention: Duration::from_secs(
                positive(
                    "SAGAT_DELIVERY_RETENTION_DAYS",
                    optional(get, "SAGAT_DELIVERY_RETENTION_DAYS", 14u64)?,
                )?
                .saturating_mul(24 * 3600),
            ),
            ..delivery_defaults
        };
        let delivery_poll = Duration::from_millis(positive(
            "SAGAT_DELIVERY_POLL_MS",
            optional(get, "SAGAT_DELIVERY_POLL_MS", 250u64)?,
        )?);

        let connector_refresh = Duration::from_secs(positive(
            "SAGAT_CONNECTOR_REFRESH_SECS",
            optional(get, "SAGAT_CONNECTOR_REFRESH_SECS", 5u64)?,
        )?);

        let projector_defaults = ProjectorSettings::default();
        let telemetry = TelemetrySettings {
            otlp: OtlpConfig::from_lookup(get, "sagat-execution-engine")?,
            projector: ProjectorSettings {
                batch_runs: u32::try_from(positive(
                    "SAGAT_TELEMETRY_BATCH_RUNS",
                    optional(
                        get,
                        "SAGAT_TELEMETRY_BATCH_RUNS",
                        u64::from(projector_defaults.batch_runs),
                    )?,
                )?)
                .map_err(|_| ConfigError::Invalid {
                    name: "SAGAT_TELEMETRY_BATCH_RUNS",
                    reason: "is too large".to_owned(),
                })?,
                lag_budget: seconds(
                    get,
                    "SAGAT_TELEMETRY_LAG_BUDGET_SECS",
                    projector_defaults.lag_budget,
                )?,
                ..projector_defaults
            },
            poll: Duration::from_millis(positive(
                "SAGAT_TELEMETRY_POLL_MS",
                optional(get, "SAGAT_TELEMETRY_POLL_MS", 1000u64)?,
            )?),
        };

        let monitor_defaults = MonitorSettings::default();
        let monitor = MonitorConfig {
            settings: MonitorSettings {
                thresholds: sagat_application::operations::Thresholds {
                    outbox_age: seconds(
                        get,
                        "SAGAT_ALERT_OUTBOX_AGE_SECS",
                        monitor_defaults.thresholds.outbox_age,
                    )?,
                    lease_grace: seconds(
                        get,
                        "SAGAT_ALERT_LEASE_GRACE_SECS",
                        monitor_defaults.thresholds.lease_grace,
                    )?,
                    unclaimed: seconds(
                        get,
                        "SAGAT_ALERT_UNCLAIMED_SECS",
                        monitor_defaults.thresholds.unclaimed,
                    )?,
                    review: seconds(
                        get,
                        "SAGAT_ALERT_REVIEW_SECS",
                        monitor_defaults.thresholds.review,
                    )?,
                },
                bounds: monitor_defaults.bounds,
                runbook_base: get("SAGAT_RUNBOOK_BASE_URL")
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
                    .unwrap_or(monitor_defaults.runbook_base),
                pools: worker_pools(get)?,
                retention: Duration::from_secs(
                    positive(
                        "SAGAT_ALERT_RETENTION_DAYS",
                        optional(get, "SAGAT_ALERT_RETENTION_DAYS", 30u64)?,
                    )?
                    .saturating_mul(24 * 3600),
                ),
            },
            interval: seconds(get, "SAGAT_MONITOR_INTERVAL_SECS", Duration::from_secs(30))?,
        };

        let nats = match get("SAGAT_NATS_URL")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some(url) => {
                let mut nats = NatsConfig::new(url);
                nats.stream = optional(get, "SAGAT_NATS_STREAM", nats.stream)?;
                nats.credentials_file = get("SAGAT_NATS_CREDENTIALS_FILE")
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty());
                // Plaintext NATS is a development convenience only, like plaintext JWKS.
                nats.require_tls = match get("SAGAT_NATS_REQUIRE_TLS").as_deref().map(str::trim) {
                    None | Some("") => environment != Environment::Dev,
                    Some(_) => flag(get, "SAGAT_NATS_REQUIRE_TLS")?,
                };
                Some(nats)
            }
        };

        let secrets = read_secrets(get)?;
        let openbao = sagat_control_plane::config::read_openbao(get, environment)?;
        // Secret values travel over the worker API: plaintext is acceptable only in development, or
        // when the operator states that TLS is terminated in front of the engine.
        if (secrets.is_some() || openbao.is_some())
            && grpc.tls.is_none()
            && environment != Environment::Dev
            && !flag(get, "SAGAT_GRPC_ALLOW_PLAINTEXT_SECRETS")?
        {
            return Err(ConfigError::Invalid {
                name: "SAGAT_SECRETS_KEYS",
                reason: "secrets are handed to workers over the worker API, which would be plaintext: \
                         set SAGAT_GRPC_TLS_CERT_FILE and SAGAT_GRPC_TLS_KEY_FILE, or set \
                         SAGAT_GRPC_ALLOW_PLAINTEXT_SECRETS=true if TLS is terminated in front of the engine"
                    .to_owned(),
            });
        }
        Ok(Self {
            grpc_addr: optional(
                get,
                "SAGAT_GRPC_ADDR",
                SocketAddr::from(([127, 0, 0, 1], 9000)),
            )?,
            admin_addr,
            auth,
            rate_limit,
            subject_rate_limit,
            grpc,
            worker,
            breaker,
            scheduler,
            shards,
            replica,
            tick_interval,
            housekeeping_interval,
            liveness_grace,
            dispatch,
            outbox_poll,
            events,
            event_poll,
            deliveries,
            delivery_poll,
            connector_refresh,
            telemetry,
            monitor,
            nats,
            secrets,
            openbao,
            blobs,
            blob_transfer,
        })
    }
}

/// The blob store settings, with the rules that depend on where the process runs.
fn read_blobs(
    get: Lookup<'_>,
    environment: Environment,
) -> Result<Option<BlobSettings>, ConfigError> {
    let settings = BlobSettings::from_lookup(|name| get(name)).map_err(|error| match error {
        BlobConfigError::Missing(name) => ConfigError::Missing(name),
        BlobConfigError::Invalid { name, reason } => ConfigError::Invalid {
            name,
            reason: reason.to_owned(),
        },
        BlobConfigError::Incomplete(reason) => ConfigError::Invalid {
            name: "SAGAT_BLOB_S3_ACCESS_KEY_ID",
            reason: reason.to_owned(),
        },
        BlobConfigError::InsecureEndpoint => ConfigError::Invalid {
            name: "SAGAT_BLOB_S3_ENDPOINT",
            reason: "plain http needs SAGAT_BLOB_S3_ALLOW_HTTP=true".to_owned(),
        },
        BlobConfigError::Build => ConfigError::Invalid {
            name: "SAGAT_BLOB_PROVIDER",
            reason: "the object store could not be set up".to_owned(),
        },
    })?;
    if let Some(settings) = &settings {
        // Memory is private to one process, and the control plane and every engine replica are
        // different processes: a blob written by one could not be read by another, and a restart
        // would lose them all. It exists for tests and for a single-process development setup.
        if matches!(settings.store, BlobStoreConfig::Memory) && environment != Environment::Dev {
            return Err(ConfigError::Invalid {
                name: "SAGAT_BLOB_PROVIDER",
                reason: "memory is only allowed when SAGAT_ENV=dev".to_owned(),
            });
        }
        if let BlobStoreConfig::S3(s3) = &settings.store
            && s3.allow_http
            && environment != Environment::Dev
        {
            return Err(ConfigError::Invalid {
                name: "SAGAT_BLOB_S3_ALLOW_HTTP",
                reason: "plain http to the object store is only allowed when SAGAT_ENV=dev"
                    .to_owned(),
            });
        }
    }
    Ok(settings)
}
