//! Command-line parsing. Deliberately tiny: a handful of subcommands and no flags.

/// What the process was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Run the HTTP API (the default).
    Serve,
    /// Apply pending schema migrations and exit.
    Migrate,
    /// Create a development tenant and administrator, then exit. Refused outside `SAGAT_ENV=dev`.
    SeedDev,
    /// Create a tenant and its first administrator in any environment. Both values are explicit.
    ProvisionTenant,
    /// Re-seal every stored secret version under the active key of `SAGAT_SECRETS_KEYS`, then exit.
    ResealSecrets,
    /// Grant a tenant a quota ceiling, or withdraw it, then exit. Values are explicit.
    QuotaCeiling,
    /// Print a new signing key for scripts (`SAGAT_SCRIPT_SIGNING_KEY`) and exit. Needs no database.
    ScriptKey,
    /// Probe the local API's liveness endpoint; for container health checks.
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
        Some("migrate") => Command::Migrate,
        Some("seed-dev") => Command::SeedDev,
        Some("provision-tenant") => Command::ProvisionTenant,
        Some("reseal-secrets") => Command::ResealSecrets,
        Some("quota-ceiling") => Command::QuotaCeiling,
        Some("script-key") => Command::ScriptKey,
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
Sagat Octopus Orchestrator control plane

USAGE:
    control-plane [COMMAND]

COMMANDS:
    serve         Run the HTTP API (default)
    migrate       Apply pending schema migrations and exit
    seed-dev      Create a development tenant and admin (only when SAGAT_ENV=dev)
    provision-tenant  Create a tenant and its first admin (any environment; values are explicit)
    reseal-secrets  Re-seal stored secrets under the active key of SAGAT_SECRETS_KEYS (after a key rotation)
    quota-ceiling  Grant a tenant a quota ceiling (SAGAT_CEILING_TENANT_ID, _RESOURCE, _VALUE; a number or default)
    script-key    Print a new Ed25519 key for SAGAT_SCRIPT_SIGNING_KEY (it signs the artifacts of scripts)
    healthcheck   Probe the local API (GET /v1/health); exit 0 when healthy
    version       Print the version
    help          Print this message

ENVIRONMENT (all commands):
    DATABASE_URL                      PostgreSQL connection URL (required; never logged)
    SAGAT_ENV                         dev | staging | prod            [prod]
    SAGAT_LOG_FORMAT                  json | pretty                   [json]
    SAGAT_DB_MAX_CONNECTIONS          pool size                       [10]
    SAGAT_DB_STATEMENT_TIMEOUT_MS     per-statement server timeout    [5000]

ENVIRONMENT (serve):
    Outside SAGAT_ENV=dev, serve checks its database role at start and exits with code 2 if the
    role is a superuser, owns audit_records, or may UPDATE/DELETE/TRUNCATE it: use the
    least-privilege runtime role (sagat_app) here and the owner role only for migrate and
    provision-tenant.
    SAGAT_OIDC_ISSUER                expected token issuer (required)
    SAGAT_OIDC_AUDIENCE               expected token audience (required)
    SAGAT_OIDC_JWKS_URI               signing keys URL, https (required)
    SAGAT_OIDC_TENANT_CLAIM           claim holding the tenant slug   [tenant]
    SAGAT_BIND_ADDR                   listen address                  [127.0.0.1:8080]
    SAGAT_MIGRATE_ON_START            apply migrations before serving, dev only [false]
    SAGAT_REQUEST_TIMEOUT_SECS        request timeout                 [15]
    SAGAT_MAX_BODY_BYTES              request body limit              [65536]
    SAGAT_HEADER_READ_TIMEOUT_SECS    max time to send a request head [10]
    SAGAT_MAX_CONNECTIONS             simultaneous connections        [1024]
    SAGAT_SHUTDOWN_GRACE_SECS         drain time on shutdown          [30]
    SAGAT_WRITE_STALL_TIMEOUT_SECS    a client that takes no byte of a response this long is dropped [30]
    SAGAT_STREAMS_MAX                 open event streams in all, at most half of SAGAT_MAX_CONNECTIONS [min(2000, max_connections / 2)]
    SAGAT_STREAMS_PER_TENANT          open event streams per tenant   [200]
    SAGAT_STREAMS_PER_SUBJECT         open event streams per subject  [20]
    SAGAT_MAX_IN_FLIGHT               requests served at once         [512]
    SAGAT_MAX_IN_FLIGHT_PER_TENANT    requests of one tenant at once  [max_in_flight / 4, min 1]
    SAGAT_BODY_IDLE_TIMEOUT_SECS      max wait for the next body part [5]
    SAGAT_RATE_LIMIT_RPS              per-tenant sustained req/s, >= 0.001 [100]
    SAGAT_RATE_LIMIT_BURST            per-tenant burst, >= 1          [200]
    SAGAT_SUBJECT_RATE_LIMIT_RPS      per-subject req/s, charged before any DB lookup, >= 0.001 [20]
    SAGAT_SUBJECT_RATE_LIMIT_BURST    per-subject burst, >= 1         [40]
    SAGAT_ADMIN_ADDR                  /metrics listener, or 'off'     [127.0.0.1:9090]
    SAGAT_OIDC_REQUIRED_TYP           required JOSE typ; at+jwt and application/at+jwt are the
                                      same type, case-insensitive, whitespace trimmed [unset]
    SAGAT_ALLOW_INSECURE_JWKS_HTTP    allow an http JWKS URL (dev only) [false]
    SAGAT_SCRIPT_SIGNING_KEY          Ed25519 key (PKCS#8, base64) that signs artifacts; without it the
                                      script routes do not exist (make one with `script-key`)
    SAGAT_SCRIPT_CEILING              the most any script may be given, as
                                      v1;memory=<bytes>;cpu=<millis>;wall=<secs>;pids=<n>;output=<bytes>;scratch=<bytes>;egress=none|namespace
                                      [v1;memory=268435456;cpu=1000;wall=30;pids=64;output=65536;scratch=33554432;egress=none]

ENVIRONMENT (seed-dev):
    SAGAT_SEED_TENANT                 tenant slug                     [dev]
    SAGAT_SEED_ADMIN_SUBJECT          administrator subject           [dev-admin]
    SAGAT_SEED_NAMESPACE              also leave a namespace with this slug (and the workspace
                                      below) for the administrator; unset: none
    SAGAT_SEED_WORKSPACE              workspace slug of that namespace [dev]
    SAGAT_SEED_ENVIRONMENT            its environment: dev, staging or prod [dev]

ENVIRONMENT (provision-tenant):
    SAGAT_PROVISION_TENANT            tenant slug (required)
    SAGAT_PROVISION_ADMIN             subject of the first administrator (required)

ENVIRONMENT (healthcheck):
    SAGAT_HEALTHCHECK_ADDR            address to probe                [127.0.0.1:8080]
";
