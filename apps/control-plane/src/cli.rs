//! Command-line parsing. Deliberately tiny: four subcommands and no flags.

/// What the process was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Run the HTTP API (the default).
    Serve,
    /// Apply pending schema migrations and exit.
    Migrate,
    /// Create a development tenant and administrator, then exit. Refused outside `SAGAT_ENV=dev`.
    SeedDev,
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
    SAGAT_OIDC_ISSUER                 expected token issuer (required)
    SAGAT_OIDC_AUDIENCE               expected token audience (required)
    SAGAT_OIDC_JWKS_URI               signing keys URL, https (required)
    SAGAT_OIDC_TENANT_CLAIM           claim holding the tenant slug   [tenant]
    SAGAT_BIND_ADDR                   listen address                  [127.0.0.1:8080]
    SAGAT_MIGRATE_ON_START            apply migrations before serving [false]
    SAGAT_REQUEST_TIMEOUT_SECS        request timeout                 [15]
    SAGAT_MAX_BODY_BYTES              request body limit              [65536]
    SAGAT_HEADER_READ_TIMEOUT_SECS    max time to send a request head [10]
    SAGAT_MAX_CONNECTIONS             simultaneous connections        [1024]
    SAGAT_SHUTDOWN_GRACE_SECS         drain time on shutdown          [30]
    SAGAT_ALLOW_INSECURE_JWKS_HTTP    allow an http JWKS URL (dev only) [false]

ENVIRONMENT (seed-dev):
    SAGAT_SEED_TENANT                 tenant slug                     [dev]
    SAGAT_SEED_ADMIN_SUBJECT          administrator subject           [dev-admin]

ENVIRONMENT (healthcheck):
    SAGAT_HEALTHCHECK_ADDR            address to probe                [127.0.0.1:8080]
";
