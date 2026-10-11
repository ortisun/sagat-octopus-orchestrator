//! Command-line parsing: two subcommands and no flags.

/// What the process was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Take builds, test runs and script steps from the control plane (the default).
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
Sagat Octopus Orchestrator sandbox runner

USAGE:
    sandbox-runner [COMMAND]

COMMANDS:
    serve         Take builds, test runs and script steps and run them in containers (default)
    healthcheck   Probe the local liveness endpoint; exit 0 when healthy
    version       Print the version
    help          Print this message

The runner needs the container engine (DOCKER_HOST or the local socket). Whoever can reach that
socket is root on the machine: run the runner on a host that holds nothing else, with a
rootless or sandboxed engine where you can.

ENVIRONMENT (serve):
    SAGAT_RUNNER_API_URL              control plane API root, https (required), e.g. https://cp.example/v1
    SAGAT_RUNNER_GRPC_URL             worker protocol endpoint, https (required)
    SAGAT_RUNNER_TOKEN_FILE           file holding the token of the runner identity, read at every
                                      call so a rotated token is used at once (or SAGAT_RUNNER_TOKEN)
    SAGAT_RUNNER_CA_FILE              extra root certificate (PEM) for the control plane and the engine
    SAGAT_RUNNER_NAME                 name of the worker sessions                    [sandbox-runner]
    SAGAT_RUNNER_NAMESPACES           script steps are served in: slug=id,slug=id    [none: builds and tests only]
    SAGAT_RUNNER_MAX_CONCURRENT       containers at once                              [4]
    SAGAT_RUNNER_POLL_MS              pause when there was nothing to do              [500]
    SAGAT_RUNNER_CEILING              the most any script is given here, as the platform writes a policy
    SAGAT_RUNNER_TRUSTED_KEYS         public keys (base64, comma separated) whose signatures are trusted;
                                      without them the control plane is asked for its key (trust on first use)
    SAGAT_RUNNER_IMAGE_TYPESCRIPT     image that builds and runs TypeScript           [node:24-alpine]
    SAGAT_RUNNER_IMAGE_PYTHON         image that builds and runs Python               [python:3.13-alpine]
    SAGAT_RUNNER_SWEEP_AFTER_SECS     containers of this runner older than this are removed [3600]
    SAGAT_ADMIN_ADDR                  /metrics and /livez listener, or 'off'          [127.0.0.1:9092]
    SAGAT_ENV                         dev | staging | prod                            [prod]
    SAGAT_LOG_FORMAT                  json | pretty                                   [json]
";
