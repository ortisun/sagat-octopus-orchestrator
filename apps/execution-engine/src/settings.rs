//! Settings of `serve`, read from environment variables like the control plane's (the shared parts
//! come from `sagat_control_plane::config`).

use std::net::SocketAddr;
use std::time::Duration;

use sagat_api_grpc::GrpcConfig;
use sagat_api_grpc::TlsConfig;
use sagat_api_http::{AuthConfig, RateLimitConfig};
use sagat_application::worker::WorkerSettings;
use sagat_control_plane::config::{
    ConfigError, Environment, Lookup, SUBJECT_RATE_VARS, TENANT_RATE_VARS, flag, optional,
    rate_limit_error, read_auth, read_rate_limit, read_secrets,
};
use sagat_infra_nats::NatsConfig;
use sagat_scheduler::{DispatchSettings, SchedulerSettings};

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
    pub scheduler: SchedulerSettings,
    /// Pause between scheduler ticks when the last one found nothing to do.
    pub tick_interval: Duration,
    pub housekeeping_interval: Duration,
    /// How long the scheduler loop may go without finishing a tick before `/livez` reports it
    /// stuck (and the orchestrator restarts the process).
    pub liveness_grace: Duration,
    pub dispatch: DispatchSettings,
    /// Pause between dispatch rounds when the last one found nothing to send.
    pub outbox_poll: Duration,
    /// `None`: hints are not published (workers poll; the outbox accumulates).
    pub nats: Option<NatsConfig>,
    /// The key ring that opens secrets for workers (`SAGAT_SECRETS_KEYS`); `None` turns the
    /// `ResolveSecrets` call off.
    pub secrets: Option<sagat_infra_secrets::SecretsConfig>,
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
            max_in_flight: capacity(get, "SAGAT_GRPC_MAX_IN_FLIGHT", grpc_defaults.max_in_flight)?,
            tls: read_tls(get)?,
            ..grpc_defaults
        };

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
            .map_err(|_| ConfigError::Invalid {
                name: "SAGAT_SCHEDULER_BATCH",
                reason: "is too large".to_owned(),
            })?,
            ..scheduler_defaults
        };
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
            .map_err(|_| ConfigError::Invalid {
                name: "SAGAT_OUTBOX_BATCH",
                reason: "is too large".to_owned(),
            })?,
            ..dispatch_defaults
        };
        let outbox_poll = Duration::from_millis(positive(
            "SAGAT_OUTBOX_POLL_MS",
            optional(get, "SAGAT_OUTBOX_POLL_MS", 250u64)?,
        )?);

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
        // Secret values travel over the worker API: plaintext is acceptable only in development, or
        // when the operator states that TLS is terminated in front of the engine.
        if secrets.is_some()
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
            scheduler,
            tick_interval,
            housekeeping_interval,
            liveness_grace,
            dispatch,
            outbox_poll,
            nats,
            secrets,
        })
    }
}
