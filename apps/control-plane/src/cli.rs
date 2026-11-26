//! Command-line parsing. Deliberately tiny: a handful of subcommands and no flags.

/// What the process was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Retire one bounded batch of unreferenced OpenBao credentials, then exit.
    CleanupSecrets,
    /// Grant a tenant a quota ceiling, or withdraw it, then exit. Values are explicit.
    QuotaCeiling,
    /// Set the weight of a tenant in the scheduler's passes, or withdraw it, then exit.
    SchedulingWeight,
    /// Hand the cron triggers over from the legacy cron pass to the schedule pass (feature 018), then exit. Needs the
    /// schema-owning database role.
    ScheduleTriggerHandover(HandoverArgs),
    /// Record a fleet requirement of the instance-capability registry (feature 006, decision D006-034), then exit. Needs the
    /// schema-owning database role.
    CapabilityActivate(crate::capability_activation::ActivationArgs),
    /// List the stored workflow versions whose `inputSchema` is outside the supported subset (design 043), then exit
    /// 0 (none), 1 (at least one) or 2 (the command failed). Reads only; it runs with the process's database settings (the runtime role is enough).
    /// A superseded version nothing starts never counts; `--all` (`all`) lists those as well (OD-2026-11-16-5).
    InputSchemaInventory {
        all: bool,
    },
    /// Print a new signing key for scripts (`SAGAT_SCRIPT_SIGNING_KEY`) and exit. Needs no database.
    ScriptKey,
    /// Probe the local API's liveness endpoint; for container health checks.
    Healthcheck,
    Help,
    Version,
}

/// The flags of `schedule-trigger-handover`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HandoverArgs {
    /// `--operator NAME`: who is making the handover, recorded with the database role of the session.
    pub operator: Option<String>,
    /// `--expect-live N`: how many live execution engines the operator expects.
    pub expect_live: Option<u32>,
    /// `--dry-run`: verify and change nothing.
    pub dry_run: bool,
    /// `--apply`: do it. Without it the command only verifies and says so (the handover has no way back).
    pub apply: bool,
    /// `--auto`: the deployment's job (ADR-0091). It waits, bounded, until the database says the fleet is stable and converged,
    /// and hands over as `sagat_handover`; it implies `--apply` (with `--dry-run` it only checks the role and says the last reason).
    pub auto: bool,
    /// `--stable N`: consecutive stable rounds, at least 15 s apart (at least 3; the database floors it).
    pub stable: Option<u32>,
    /// `--horizon DURATION`: how far back the engines, the memberships and the reconciliation are looked at (10m to 1h).
    pub horizon: Option<std::time::Duration>,
    /// `--min-live N`: the fewest live engines the job hands over with (at least 1).
    pub min_live: Option<u32>,
    /// `--wait DURATION`: how long the job waits in all (at most 24h).
    pub wait: Option<std::time::Duration>,
    /// `--retire-role`: after the handover and its audit, take the login of `sagat_handover` away.
    pub retire_role: bool,
    /// `--run-id UUID`: the run (ADR-0091 decision 1); `SAGAT_HANDOVER_RUN_ID` is its environment form, and without either the
    /// run id is drawn at start.
    pub run_id: Option<uuid::Uuid>,
}

/// The defaults and bounds of `--auto` (ADR-0091 decisions 2 and 3).
pub mod auto_bounds {
    use std::time::Duration;

    pub const STABLE: u32 = 3;
    pub const HORIZON: Duration = Duration::from_secs(600);
    pub const HORIZON_MAX: Duration = Duration::from_secs(3600);
    pub const MIN_LIVE: u32 = 1;
    pub const WAIT: Duration = Duration::from_secs(2 * 3600);
    pub const WAIT_MAX: Duration = Duration::from_secs(24 * 3600);
}

/// A duration as the flags take it: a whole number followed by `s`, `m` or `h` (`600s`, `10m`, `2h`).
///
/// # Errors
/// [`UsageError`] for any other text.
pub fn parse_duration(name: &str, text: &str) -> Result<std::time::Duration, UsageError> {
    let wrong = || {
        UsageError(format!(
            "{name} needs a duration such as 600s, 10m or 2h, not '{text}'"
        ))
    };
    let split = text.len().checked_sub(1).ok_or_else(wrong)?;
    let (number, unit) = text.split_at(split);
    let number: u64 = number.parse().map_err(|_| wrong())?;
    let seconds = match unit {
        "s" => number,
        "m" => number.checked_mul(60).ok_or_else(wrong)?,
        "h" => number.checked_mul(3600).ok_or_else(wrong)?,
        _ => return Err(wrong()),
    };
    Ok(std::time::Duration::from_secs(seconds))
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
    let first = args.next();
    if first == Some("schedule-trigger-handover") {
        return parse_handover(args);
    }
    if first == Some("capability-activate") {
        return parse_activation(args);
    }
    if first == Some("input-schema-inventory") {
        return match (args.next(), args.next()) {
            (None, _) => Ok(Command::InputSchemaInventory { all: false }),
            (Some("--all"), None) => Ok(Command::InputSchemaInventory { all: true }),
            (Some("--all"), Some(extra)) | (Some(extra), _) => {
                Err(UsageError(format!("unexpected argument '{extra}'")))
            }
        };
    }
    let command = match first {
        None | Some("serve") => Command::Serve,
        Some("migrate") => Command::Migrate,
        Some("seed-dev") => Command::SeedDev,
        Some("provision-tenant") => Command::ProvisionTenant,
        Some("reseal-secrets") => Command::ResealSecrets,
        Some("cleanup-secrets") => Command::CleanupSecrets,
        Some("quota-ceiling") => Command::QuotaCeiling,
        Some("scheduling-weight") => Command::SchedulingWeight,
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

fn parse_handover<'a>(mut args: impl Iterator<Item = &'a str>) -> Result<Command, UsageError> {
    let mut parsed = HandoverArgs::default();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (arg, None),
        };
        let mut value = |name: &str| -> Result<String, UsageError> {
            let given = inline
                .map(str::to_owned)
                .or_else(|| args.next().map(str::to_owned))
                .ok_or_else(|| UsageError(format!("{name} needs a value")))?;
            // an option is never a value: `--operator --dry-run` must not record `--dry-run` as who did it
            if given.starts_with('-') {
                return Err(UsageError(format!(
                    "{name} needs a value, not the option '{given}'"
                )));
            }
            Ok(given)
        };
        match flag {
            "--operator" => parsed.operator = Some(value("--operator")?),
            "--expect-live" => {
                let text = value("--expect-live")?;
                parsed.expect_live = Some(text.parse::<u32>().map_err(|_| {
                    UsageError(format!(
                        "--expect-live needs a whole number of engines, not '{text}'"
                    ))
                })?);
            }
            "--dry-run" if inline.is_none() => parsed.dry_run = true,
            "--apply" if inline.is_none() => parsed.apply = true,
            "--auto" if inline.is_none() => parsed.auto = true,
            "--retire-role" if inline.is_none() => parsed.retire_role = true,
            "--stable" => {
                let text = value("--stable")?;
                parsed.stable = Some(text.parse::<u32>().map_err(|_| {
                    UsageError(format!(
                        "--stable needs a whole number of rounds, not '{text}'"
                    ))
                })?);
            }
            "--min-live" => {
                let text = value("--min-live")?;
                parsed.min_live = Some(text.parse::<u32>().map_err(|_| {
                    UsageError(format!(
                        "--min-live needs a whole number of engines, not '{text}'"
                    ))
                })?);
            }
            "--run-id" => {
                let text = value("--run-id")?;
                parsed.run_id = Some(uuid::Uuid::parse_str(text.trim()).map_err(|_| {
                    UsageError(format!(
                        "--run-id needs a uuid (in Kubernetes, the Job's UID), not '{text}'"
                    ))
                })?);
            }
            "--horizon" => {
                let text = value("--horizon")?;
                parsed.horizon = Some(parse_duration("--horizon", &text)?);
            }
            "--wait" => {
                let text = value("--wait")?;
                parsed.wait = Some(parse_duration("--wait", &text)?);
            }
            other => {
                return Err(UsageError(format!(
                    "unexpected argument '{other}' for schedule-trigger-handover"
                )));
            }
        }
    }
    if parsed.apply && parsed.dry_run {
        return Err(UsageError(
            "--apply and --dry-run contradict each other: choose one".to_owned(),
        ));
    }
    check_auto(&parsed)?;
    Ok(Command::ScheduleTriggerHandover(parsed))
}

/// The flags of `--auto` go with `--auto` and keep to their bounds; `--expect-live` and `--apply` go with the operator's command.
fn check_auto(parsed: &HandoverArgs) -> Result<(), UsageError> {
    use auto_bounds::{HORIZON, HORIZON_MAX, STABLE, WAIT_MAX};
    let auto_only = parsed.stable.is_some()
        || parsed.horizon.is_some()
        || parsed.min_live.is_some()
        || parsed.wait.is_some()
        || parsed.retire_role
        || parsed.run_id.is_some();
    if !parsed.auto {
        if auto_only {
            return Err(UsageError(
                "--stable, --horizon, --min-live, --wait, --retire-role and --run-id go with --auto"
                    .to_owned(),
            ));
        }
        return Ok(());
    }
    if parsed.apply {
        return Err(UsageError(
            "--auto implies --apply: give one of them".to_owned(),
        ));
    }
    if parsed.expect_live.is_some() {
        return Err(UsageError(
            "--expect-live is the operator's (--apply); the job takes --min-live".to_owned(),
        ));
    }
    if parsed.stable.is_some_and(|stable| stable < STABLE) {
        return Err(UsageError(format!("--stable is at least {STABLE}")));
    }
    if parsed
        .horizon
        .is_some_and(|horizon| horizon < HORIZON || horizon > HORIZON_MAX)
    {
        return Err(UsageError(
            "--horizon is between 10m and 1h: expired scheduler memberships are deleted after an hour".to_owned(),
        ));
    }
    if parsed.min_live == Some(0) {
        return Err(UsageError("--min-live is at least 1".to_owned()));
    }
    if parsed.wait.is_some_and(|wait| wait > WAIT_MAX) {
        return Err(UsageError("--wait is at most 24h".to_owned()));
    }
    // a wait that cannot hold one horizon and the stable rounds after it can only end at its deadline (correctness review)
    let floor = parsed.horizon.unwrap_or(HORIZON)
        + std::time::Duration::from_secs(15) * parsed.stable.unwrap_or(STABLE);
    if parsed.wait.is_some_and(|wait| wait <= floor) {
        return Err(UsageError(format!(
            "--wait must be longer than --horizon plus 15 s for each --stable round ({} s here)",
            floor.as_secs()
        )));
    }
    Ok(())
}

fn parse_activation<'a>(mut args: impl Iterator<Item = &'a str>) -> Result<Command, UsageError> {
    let mut parsed = crate::capability_activation::ActivationArgs::default();
    match args.next() {
        Some(token) if !token.starts_with('-') => {
            if crate::capability_activation::activatable(token).is_none() {
                return Err(UsageError(crate::capability_activation::not_activatable(
                    token,
                )));
            }
            token.clone_into(&mut parsed.capability);
        }
        _ => {
            return Err(UsageError(
                "capability-activate needs the capability first: capability-activate <capability> --operator NAME \
                 [--expect-live N] [--dry-run]"
                    .to_owned(),
            ));
        }
    }
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (arg, None),
        };
        let mut value = |name: &str| -> Result<String, UsageError> {
            let given = inline
                .map(str::to_owned)
                .or_else(|| args.next().map(str::to_owned))
                .ok_or_else(|| UsageError(format!("{name} needs a value")))?;
            if given.starts_with('-') {
                return Err(UsageError(format!(
                    "{name} needs a value, not the option '{given}'"
                )));
            }
            Ok(given)
        };
        match flag {
            "--operator" => parsed.operator = Some(value("--operator")?),
            "--expect-live" => {
                let text = value("--expect-live")?;
                parsed.expect_live = Some(text.parse::<u32>().map_err(|_| {
                    UsageError(format!(
                        "--expect-live needs a whole number of processes, not '{text}'"
                    ))
                })?);
            }
            "--dry-run" if inline.is_none() => parsed.dry_run = true,
            other => {
                return Err(UsageError(format!(
                    "unexpected argument '{other}' for capability-activate"
                )));
            }
        }
    }
    Ok(Command::CapabilityActivate(parsed))
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
    cleanup-secrets  Retire expired unreferenced OpenBao credentials (one batch; 60 second deadline)
    quota-ceiling  Grant a tenant a quota ceiling (SAGAT_CEILING_TENANT_ID, _RESOURCE, _VALUE; a number or default)
    scheduling-weight  Set a tenant's weight in the scheduler's passes, 1 to 16 (SAGAT_WEIGHT_TENANT_ID, SAGAT_WEIGHT_VALUE; a number or default)
    schedule-trigger-handover  Hand the cron triggers over from the legacy cron pass to the schedule pass; audited, once, with the
                  owner role: schedule-trigger-handover --operator NAME [--expect-live N] [--dry-run | --apply];
                  the deployment's job: schedule-trigger-handover --auto [--stable N] [--horizon D] [--min-live N]
                  [--wait D] [--retire-role] [--run-id UUID] [--dry-run], as sagat_handover
    capability-activate  Record a fleet requirement of the instance-capability registry (lease-withdrawal-v1: the engines
                  withdraw the leases they never handed to a host; lease-withdrawal-v2, after it: the floor of protocol
                  1.8; flow-control-concurrency-v1 and flow-control-rate-v1: concurrency keys and rate limits, once a
                  digest key version is active and a process of each kind lists them); audited, with the owner role:
                  capability-activate <capability> --operator NAME [--expect-live N] [--dry-run]
    input-schema-inventory [--all]  List the stored workflow versions whose inputSchema is outside the supported subset (every
                  start of them is refused in enforce mode), and for information those with an inputSchema and a trigger;
                  reads only, with the process's database settings. A superseded version nothing starts (not active, no
                  open run, no run in 30 days) is only counted; --all lists those too. Exit 0: none would be refused; 1:
                  at least one; 2: it failed
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
    least-privilege runtime role (sagat_app) here and the owner role only for migrate,
    provision-tenant and schedule-trigger-handover (and any later command that records a
    requirement of the instance registry).
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
    SAGAT_REDIS_CALLBACKS_ENABLED     enable Redis callback creation after a cold upgrade [false]
    SAGAT_SCHEDULES_ENABLED           serve the schedule routes (the engine fires them under the same flag) [true]
    SAGAT_BATCHES_ENABLED             serve the batch operation routes (the engine runs the jobs under the same flag) [false]
    SAGAT_RESET_REAPPLY_ENABLED       let a reset deliver signals and Updates again (reapply SIGNALS, UPDATES, ALL) and
                                      let a workflow declare an Update with reapplyOnReset (the engine needs nothing) [false]
    SAGAT_RESET_CANCEL_SOURCE_ENABLED let a reset cancel a run that has not ended and wait for it (sourcePolicy
                                      CANCEL_SOURCE); the engine completes those resets under the same flag, and it
                                      alone expires one whose run does not end (SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS,
                                      a setting of the engine only: nothing here reads it) [false]
    SAGAT_LAKE_ENABLED                let a workflow hold an Apache Iceberg write (publication still waits for its checks) [false]
    SAGAT_RUN_INPUT_VALIDATION        enforce | report: a run input its version's inputSchema refuses, and an inputSchema
                                      outside the supported subset at publication, are refused, or counted, audited and
                                      admitted (published with a warning); anything else stops the process [enforce]
    SAGAT_COLLECTION_OPERATORS_ENABLED  let a forEach declare operators; every replica must run this release first [false]
    SAGAT_SECRETS_KEYS                key ring that seals secrets: id:base64,id2:base64; without it the
                                      secret routes do not exist. The engine needs the same ring
    SAGAT_API_ALLOW_PLAINTEXT_SECRETS  with a key ring, outside dev, say that TLS is terminated in
                                      front of this API (it receives secret values) [false]
    SAGAT_HTTP_TLS_CERT / _KEY         serve the public API over native TLS (PEM files)
    SAGAT_HTTP_TLS_CLIENT_CA           CA for optional verified webhook client certificates (PEM)
    SAGAT_WEBHOOK_OIDC_PROFILES_FILE   bounded JSON array of deployment-approved webhook issuers
    SAGAT_STUDIO_DIR                  directory of the built Studio, served under /studio/; unset or empty:
                                      the Studio is not served (the image ships it in /srv/studio)
    SAGAT_STUDIO_TRACE_URL            optional tracing-tool URL template with one {traceId}; absolute
                                      https, or http on a loopback address, with no URL credentials
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

ENVIRONMENT (cleanup-secrets):
    Use the least-privilege runtime database role and a separate OpenBao cleanup token.
    SAGAT_SECRET_CLEANUP_BATCH        maximum operations and terminal runs per batch, 1..128 [32]
    SAGAT_OPENBAO_ADDRESS             provider URL, HTTPS outside explicit loopback development
    SAGAT_OPENBAO_TOKEN_FILE          absolute token-file path; cleanup policy only
    SAGAT_OPENBAO_MOUNT               KV v2 mount [secret]
    SAGAT_OPENBAO_CA_FILE             optional PEM trust roots
    SAGAT_OPENBAO_TIMEOUT_MS          whole provider operation deadline, 1..10000 [5000]
    SAGAT_OPENBAO_DEVELOPMENT         allow numeric loopback HTTP, only when SAGAT_ENV=dev [false]
    No local secret key ring or OpenBao intent key is required. Exit 1 means retry later.

ENVIRONMENT (provision-tenant):
    SAGAT_PROVISION_TENANT            tenant slug (required)
    SAGAT_PROVISION_ADMIN             subject of the first administrator (required)

ENVIRONMENT (schedule-trigger-handover):
    Run it with the schema-owning database role (DATABASE_URL of the owner, as for migrate). It requires
    schedules-v1 of every live execution engine and records the handover in one transaction.
    --operator NAME       who is making the handover (required, except with --dry-run); recorded with the database role
    --expect-live N       how many live execution engines you expect (seen within 30 seconds); the command refuses when the
                          count differs, and, without it, when none is live
    --apply               do it. Without it the command only verifies, as --dry-run does, and says so: the handover has no way back
    --dry-run             verify everything (engines, capability, convergence of the triggers, privilege) and change nothing
    A second run says it is already handed over and exits 0. There is no command to go back.
    --auto                the deployment's job (ADR-0091): rounds every 15 s, through the database's round function, until the
                          fleet is stable and converged for --stable rounds and one horizon has passed since the first round;
                          then the handover, made by the database as sagat_handover (mode AUTOMATIC), and the tenant audit.
                          Exit 0 handed over, already handed over or switched off; 2 a reason no wait cures; 3 the deadline;
                          4 interrupted (SIGTERM or SIGINT: no activation is started after it); 1 a database failure
    --stable N            stable rounds in a row, at least 15 s apart [3, at least 3]
    --horizon D           how far back engines, memberships and reconciliation work are looked at [10m, 10m to 1h]
    --min-live N          the fewest live engines it hands over with [1]
    --wait D              how long it waits in all [2h, at most 24h]
    --retire-role         after the handover and its audit, take the login of sagat_handover away
    --run-id UUID         the run id (a uuid; in Kubernetes the Job's UID); SAGAT_HANDOVER_RUN_ID is the same, and the two
                          must agree when both are given; drawn at start when neither is
    SAGAT_HANDOVER_PASSWORD   the password of sagat_handover, given apart from DATABASE_URL (required with --auto); for
                          migrate, the password it sets (from a SCRAM verifier it computes) while the state is LEGACY and the
                          role was never retired
    SAGAT_HANDOVER_RUN_ID     the run id (a uuid; in Kubernetes the Job's UID); drawn at start when unset
    SAGAT_SCHEDULES_AUTO_HANDOVER  false: --auto exits 0 at once and hands nothing over [true]
    SAGAT_HANDOVER_RETIRE_ROLE     true: as --retire-role (for compose, which cannot add a flag on a condition) [false]
    SAGAT_HANDOVER_MIN_LIVE        for migrate: the owner's floor of the live engines the job counts with, 1 to 1000; a
                          caller of the round function cannot ask for fewer [unset: 1]

ENVIRONMENT (healthcheck):
    SAGAT_HEALTHCHECK_ADDR            address to probe                [127.0.0.1:8080]
    SAGAT_HEALTHCHECK_TLS_SERVER_NAME certificate DNS name/SNI when native TLS is configured [localhost]
    SAGAT_HEALTHCHECK_TLS_CA_FILE     optional server trust roots, PEM, at most 256 KiB
";

#[cfg(test)]
mod tests {
    use super::*;

    fn handover(args: &[&str]) -> Result<Command, UsageError> {
        let mut all = vec!["schedule-trigger-handover".to_owned()];
        all.extend(args.iter().map(|arg| (*arg).to_owned()));
        parse(&all)
    }

    const RUN: &str = "0b7c4c1e-2f4d-4f7a-9a0e-3c1b2d4e5f60";

    #[test]
    fn the_run_id_is_a_flag_of_the_job_and_a_uuid() {
        let Ok(Command::ScheduleTriggerHandover(args)) = handover(&["--auto", "--run-id", RUN])
        else {
            panic!("--auto --run-id UUID is understood");
        };
        assert_eq!(args.run_id, Some(uuid::Uuid::parse_str(RUN).expect("uuid")));
        let Ok(Command::ScheduleTriggerHandover(inline)) =
            handover(&["--auto", &format!("--run-id={RUN}")])
        else {
            panic!("--run-id=UUID is understood");
        };
        assert_eq!(inline.run_id, args.run_id);
        let not_a_uuid = handover(&["--auto", "--run-id", "job-7"]).expect_err("not a uuid");
        assert!(
            not_a_uuid.0.contains("--run-id needs a uuid"),
            "{not_a_uuid:?}"
        );
        let operator = handover(&["--operator", "ana", "--apply", "--run-id", RUN])
            .expect_err("the operator's handover has no run");
        assert!(operator.0.contains("--run-id"), "{operator:?}");
        let missing = handover(&["--auto", "--run-id"]).expect_err("a value is needed");
        assert!(missing.0.contains("--run-id"), "{missing:?}");
    }
}
