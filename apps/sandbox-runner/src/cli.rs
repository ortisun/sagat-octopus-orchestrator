//! Command-line parsing: two subcommands and no flags.

/// What the process was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Take builds, test runs and script steps from the control plane (the default).
    Serve,
    /// Serve the lake sandbox API on a node-local socket and nothing else (feature 035, ADR-0079).
    Lake,
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
        Some("lake") => Command::Lake,
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
    lake          Serve the lake sandbox API on a node-local socket: lake containers only, no control-plane token
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

ENVIRONMENT (lake; none of the variables above that are named SAGAT_RUNNER_* may be set):
    SAGAT_LAKE_SOCKET_DIR             directory of the socket; owned by this process, mode 0750 (required)
    SAGAT_LAKE_RUNNER_UID             the one user the socket serves; never 0 or 65534 (required)
    SAGAT_LAKE_RUNNER_GID             the lake runner's group; the group of the socket and of the containers (required)
    SAGAT_LAKE_WORK_VOLUME            the work volume, by its name for the engine (required)
    SAGAT_LAKE_WORK_ROOT              where this process sees the work volume, read only (required)
    SAGAT_LAKE_IMAGE                  the lake runtime image, by digest (required)
    SAGAT_LAKE_NODE_MEMORY_MIB        what the reserved budgets of the live containers may add up to (required)
    SAGAT_LAKE_OWNER                  label of the containers of this lake runner      [lake-runner]
    SAGAT_LAKE_CEILING                containers at once on the node                   [4]
    SAGAT_LAKE_SMALL_MEMORY_MIB       budget of the small class (memory, then _PROCESSES, _CPU_MILLIS,
                                      _WORK_MIB, _TMP_MIB)                             [2048, 128, 1500, 512, 64]
    SAGAT_LAKE_LARGE_MEMORY_MIB       the large class exists only when this is set (and the same limits as _SMALL_*)
    SAGAT_LAKE_MAX_ATTEMPT_SECS       the longest attempt a start may ask for; longer is refused [3600]
    SAGAT_LAKE_DEADLINE_GRACE_SECS    added to an attempt's deadline for the label that ends the container [30]
    SAGAT_LAKE_RETAIN_SECS            an ended container is kept this long after its deadline [300]
    SAGAT_LAKE_SWEEP_SECS             how often what is past its deadline is ended      [30]
    SAGAT_LAKE_DRAIN_SECS             after the stop signal, starts are refused and waits and kills are answered [30]
    SAGAT_ADMIN_ADDR                  /metrics and /livez listener, or 'off'            [127.0.0.1:9092]
";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    #[test]
    fn lake_is_a_command_of_its_own_and_serve_stays_the_default() {
        assert_eq!(parse(&args(&[])), Ok(Command::Serve));
        assert_eq!(parse(&args(&["serve"])), Ok(Command::Serve));
        assert_eq!(parse(&args(&["lake"])), Ok(Command::Lake));
        assert_eq!(parse(&args(&["healthcheck"])), Ok(Command::Healthcheck));
    }

    /// The two modes are never one process: there is no flag that makes `serve` also be `lake`, and no argument after either.
    #[test]
    fn nothing_makes_a_command_both_modes_or_takes_an_argument() {
        for bad in [
            &["lake", "serve"][..],
            &["serve", "lake"][..],
            &["lake", "--also-scripts"][..],
            &["lake-and-serve"][..],
            &["Lake"][..],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_usage_names_the_lake_command_and_its_settings() {
        assert!(USAGE.contains("lake "), "{USAGE}");
        assert!(USAGE.contains("SAGAT_LAKE_RUNNER_UID"));
    }
}
