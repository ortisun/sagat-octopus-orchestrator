//! `sagat`: workflows kept in Git, published, promoted and rolled back through the platform's API
//! (feature 011).
//!
//! The library holds everything the executable does so that the whole of it can be tested in
//! process: [`run`] takes the arguments, the environment and the two output streams, talks to the
//! platform only through its public REST API, and answers an exit code. What a command cannot
//! prove on its own (a secret the caller may not use, a script nobody built, the approvals a
//! namespace asks for) is left to the platform, which says no in words this tool prints.
//!
//! Exit codes: `0` done, `1` failed, `2` usage, `3` waiting for approval, `4` refused as
//! breaking, `5` differences (`diff --check`).

pub mod args;
pub mod client;
mod export;
pub mod manifest;
mod remote;
mod sync;
mod validate;
mod versions;

use std::io::Write;
use std::path::{Path, PathBuf};

use args::{Command, Common, Invocation, USAGE};
use client::Client;

/// Done.
pub const EXIT_DONE: u8 = 0;
/// Something went wrong; the message says what.
pub const EXIT_FAILED: u8 = 1;
/// The command line is wrong.
pub const EXIT_USAGE: u8 = 2;
/// Nothing failed, but something waits for approvals before it takes effect.
pub const EXIT_PENDING: u8 = 3;
/// The platform (or `publish`, before sending) refused a change that breaks callers or emitters.
pub const EXIT_BREAKING: u8 = 4;
/// `diff --check` found differences.
pub const EXIT_DIFFERENCES: u8 = 5;

/// What the process provides besides the arguments.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub server: Option<String>,
    /// The bearer token itself (never accepted as an argument: arguments are visible to every user
    /// of the machine).
    pub token: Option<String>,
    pub namespace: Option<String>,
    /// Relative paths are resolved against it.
    pub cwd: PathBuf,
}

impl Env {
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            server: var("SAGAT_SERVER"),
            token: var("SAGAT_TOKEN"),
            namespace: var("SAGAT_NAMESPACE"),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

/// The two streams a command writes to.
pub(crate) struct Io<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
}

/// A line as it may be shown on a terminal. Much of what is printed was written by somebody else (a
/// reason typed by an approver, a name or a message from the platform): a control character in it
/// (an escape sequence that rewrites the screen, a carriage return that hides the start of the
/// line) is shown as `?`. New lines and tabs are the layout of the tool's own text and stay.
#[must_use]
pub fn printable(line: &str) -> std::borrow::Cow<'_, str> {
    if line
        .chars()
        .all(|c| !c.is_control() || c == '\n' || c == '\t')
    {
        return std::borrow::Cow::Borrowed(line);
    }
    std::borrow::Cow::Owned(
        line.chars()
            .map(|c| {
                if c.is_control() && c != '\n' && c != '\t' {
                    '?'
                } else {
                    c
                }
            })
            .collect(),
    )
}

impl Io<'_> {
    /// A line of the result.
    pub(crate) fn say(&mut self, line: impl AsRef<str>) {
        let _ = writeln!(self.out, "{}", printable(line.as_ref()));
    }

    /// A line the person must see even when the result is piped away.
    pub(crate) fn warn(&mut self, line: impl AsRef<str>) {
        let _ = writeln!(self.err, "{}", printable(line.as_ref()));
    }
}

/// The largest token file read.
const MAX_TOKEN_BYTES: u64 = 16 * 1024;

fn resolve(env: &Env, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env.cwd.join(path)
    }
}

/// The manifest path: the flag, or `sagat.yaml`.
pub(crate) fn manifest_path(common: &Common, env: &Env) -> PathBuf {
    resolve(
        env,
        common
            .file
            .as_deref()
            .unwrap_or_else(|| Path::new("sagat.yaml")),
    )
}

/// The platform's address and the token, from the flags or the environment.
fn credentials(common: &Common, env: &Env) -> Result<(String, String), String> {
    let server = common
        .server
        .clone()
        .or_else(|| env.server.clone())
        .ok_or("the platform's address is missing: pass --server or set SAGAT_SERVER")?;
    let token = match &common.token_file {
        Some(path) => {
            let path = resolve(env, path);
            let meta = std::fs::metadata(&path)
                .map_err(|e| format!("the token file {}: {e}", path.display()))?;
            if !meta.is_file() || meta.len() > MAX_TOKEN_BYTES {
                return Err(format!(
                    "the token file {} is not a file of at most {MAX_TOKEN_BYTES} bytes",
                    path.display()
                ));
            }
            std::fs::read_to_string(&path)
                .map_err(|e| format!("the token file {}: {e}", path.display()))?
                .trim()
                .to_owned()
        }
        None => env
            .token
            .clone()
            .ok_or("the token is missing: pass --token-file or set SAGAT_TOKEN")?,
    };
    Ok((server, token))
}

/// A connection to the platform for one namespace.
pub(crate) struct Session {
    pub(crate) client: Client,
}

pub(crate) fn session(common: &Common, env: &Env, namespace: &str) -> Result<Session, String> {
    let (server, token) = credentials(common, env)?;
    Ok(Session {
        client: Client::new(&server, &token, namespace)?,
    })
}

/// The namespace a command that does not need the manifest works in: the flag, the environment,
/// or the manifest if there is one.
pub(crate) fn namespace_of(common: &Common, env: &Env) -> Result<String, String> {
    if let Some(slug) = common.namespace.clone().or_else(|| env.namespace.clone()) {
        return Ok(slug);
    }
    let path = manifest_path(common, env);
    if let Ok(text) = manifest::read_limited(&path, "manifest")
        && let Ok(found) = manifest::Manifest::parse(&text, manifest::is_yaml(&path))
    {
        return Ok(found.namespace);
    }
    Err(
        "the namespace is missing: pass --namespace, set SAGAT_NAMESPACE or run next to a manifest"
            .to_owned(),
    )
}

/// Runs a command line.
pub async fn run(args: &[String], env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let mut io = Io { out, err };
    let Invocation { common, command } = match args::parse(args) {
        Ok(invocation) => invocation,
        Err(args::UsageError(why)) => {
            io.warn(format!("sagat: {why}\n"));
            io.warn(USAGE);
            return EXIT_USAGE;
        }
    };
    let result = match command {
        Command::Help => {
            io.say(USAGE);
            Ok(EXIT_DONE)
        }
        Command::Version => {
            io.say(format!("sagat {}", env!("CARGO_PKG_VERSION")));
            Ok(EXIT_DONE)
        }
        Command::Validate => validate::run(&common, env, &mut io).await,
        Command::Diff { check } => sync::diff(&common, env, check, &mut io).await,
        Command::Publish {
            dry_run,
            accept_breaking,
        } => sync::publish(&common, env, dry_run, accept_breaking, &mut io).await,
        Command::Export { out } => export::run(&common, env, &out, &mut io).await,
        Command::Promote {
            workflow,
            to,
            reason,
            version,
            accept_breaking,
        } => {
            versions::promote(
                &common,
                env,
                &versions::Promote {
                    workflow: &workflow,
                    to: &to,
                    reason: &reason,
                    version,
                    accept_breaking,
                },
                &mut io,
            )
            .await
        }
        Command::Rollback {
            workflow,
            reason,
            to,
            accept_breaking,
        } => {
            versions::rollback(
                &common,
                env,
                &versions::Rollback {
                    workflow: &workflow,
                    reason: &reason,
                    to,
                    accept_breaking,
                },
                &mut io,
            )
            .await
        }
        Command::History { workflow, limit } => {
            versions::history(&common, env, &workflow, limit, &mut io).await
        }
        Command::Requests { workflow, state } => {
            versions::requests(&common, env, &workflow, state.as_deref(), &mut io).await
        }
        Command::Decide {
            request,
            reason,
            approve,
        } => versions::decide(&common, env, &request, &reason, approve, &mut io).await,
    };
    match result {
        Ok(code) => code,
        Err(why) => {
            io.warn(format!("sagat: {why}"));
            EXIT_FAILED
        }
    }
}
