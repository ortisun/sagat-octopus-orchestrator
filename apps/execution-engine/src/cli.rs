//! Command-line parsing: two subcommands and no flags.

/// What the process was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Run the worker API, the scheduler and the outbox dispatcher (the default).
    Serve,
    /// Probe the local liveness endpoint; for container health checks.
    Healthcheck,
    Help,
    Version,
}

/// The arguments were not understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

/// Parses `argv` without the program name.
///
/// # Errors
/// [`UsageError`] for an unknown subcommand or extra arguments.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let mut args = args.iter().map(String::as_str);
    let command = match args.next() {
        None | Some("serve") => Command::Serve,
        Some("healthcheck") => Command::Healthcheck,
        Some("help" | "--help" | "-h") => Command::Help,
        Some("version" | "--version" | "-V") => Command::Version,
        Some(other) => return Err(UsageError(format!("unknown command '{other}'"))),
    };
    match args.next() {
        None => Ok(command),
        Some(extra) => Err(UsageError(format!("unexpected argument '{extra}'"))),
    }
}

pub const USAGE: &str = "\
Sagat Octopus Orchestrator execution engine

USAGE:
    execution-engine [COMMAND]

COMMANDS:
    serve         Run the worker gRPC API, the scheduler and the outbox dispatcher (default)
    healthcheck   Probe the local liveness endpoint (GET /livez); exit 0 when alive
    version       Print the version
    help          Print this message

ENVIRONMENT (shared with the control plane):
    DATABASE_URL                      PostgreSQL URL (required; never logged). Use the runtime role
                                      (sagat_app): outside SAGAT_ENV=dev the engine refuses to start
                                      with a role that could rewrite the audit trail.
    SAGAT_ENV                         dev | staging | prod            [prod]
    SAGAT_LOG_FORMAT                  json | pretty                   [json]
    SAGAT_DB_MAX_CONNECTIONS          pool size                       [20]
    SAGAT_DB_STATEMENT_TIMEOUT_MS     per-statement server timeout    [5000]
    SAGAT_OIDC_ISSUER / _AUDIENCE / _JWKS_URI   worker identity verification (required)
    SAGAT_OIDC_TENANT_CLAIM, SAGAT_OIDC_REQUIRED_TYP, SAGAT_ALLOW_INSECURE_JWKS_HTTP (dev only)

ENVIRONMENT (serve):
    SAGAT_GRPC_ADDR                   worker API listen address       [127.0.0.1:9000]
    SAGAT_GRPC_REQUEST_TIMEOUT_SECS   upper bound for one call        [30]
    SAGAT_GRPC_MAX_CONNECTIONS        open connections, all clients   [2048]
    SAGAT_GRPC_MAX_IN_FLIGHT          calls running at once; polls shed at 3/4 [1024]
    SAGAT_GRPC_TLS_CERT_FILE / _KEY_FILE   serve the worker API over TLS (PEM files)
    SAGAT_GRPC_TLS_CLIENT_CA_FILE     also require client certificates signed by this CA (mTLS)
    SAGAT_SECRETS_KEYS                key ring that opens secrets for workers: id:base64,id2:base64
    SAGAT_GRPC_ALLOW_PLAINTEXT_SECRETS  allow secrets without TLS here (TLS terminated in front)
    SAGAT_RATE_LIMIT_RPS / _BURST     per-tenant worker budget        [2000 / 4000]
    SAGAT_SUBJECT_RATE_LIMIT_RPS / _BURST   per-identity budget       [200 / 400]
    SAGAT_LEASE_TTL_SECS              lease without a heartbeat       [30]
    SAGAT_SESSION_TTL_SECS            idle worker session lifetime    [3600]
    SAGAT_POLL_IDLE_RETRY_MS          retry hint when nothing is ready [500]
    SAGAT_SCHEDULER_INTERVAL_MS       pause between idle scheduler ticks [500]
    SAGAT_SCHEDULER_BATCH             candidates per pass per tick    [200]
    SAGAT_HOUSEKEEPING_INTERVAL_SECS  purge expired records every     [300]
    SAGAT_LIVENESS_GRACE_SECS         /livez fails when no scheduler tick finished for this long [30]
    SAGAT_NATS_URL                    JetStream URL; unset disables hint publishing (the outbox
                                      then only accumulates, and workers must poll)
    SAGAT_NATS_STREAM                 stream name                     [SAGAT]
    SAGAT_NATS_CREDENTIALS_FILE       .creds file for NATS authentication
    SAGAT_NATS_REQUIRE_TLS            refuse plaintext NATS           [true outside dev]
    SAGAT_OUTBOX_BATCH                messages per dispatch round     [100]
    SAGAT_OUTBOX_POLL_MS              pause between idle dispatch rounds [250]
    SAGAT_ADMIN_ADDR                  /metrics, /livez, /readyz listener, or 'off' [127.0.0.1:9091]

ENVIRONMENT (healthcheck):
    SAGAT_HEALTHCHECK_ADDR            address to probe                [127.0.0.1:9091]
";
