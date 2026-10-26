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
mod dev;
pub mod diagnostics;
pub mod doctor;
mod export;
pub mod manifest;
pub mod mocks;
pub mod new;
mod remote;
mod reveal;
mod script;
mod sync;
pub mod telemetry;
mod validate;
mod versions;
mod worker;
mod workflow;

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
/// `workflow run --wait`, `script test` or `script publish` did not see the end within the time it was
/// given (a run that waits for a person is reported at once).
pub const EXIT_TIMEOUT: u8 = 6;

/// What the process provides besides the arguments.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub server: Option<String>,
    /// The bearer token itself (never accepted as an argument: arguments are visible to every user
    /// of the machine).
    pub token: Option<String>,
    pub namespace: Option<String>,
    /// The worker endpoint (`SAGAT_GRPC`).
    pub grpc: Option<String>,
    /// The development identity provider (`SAGAT_DEV_IDP`).
    pub idp: Option<String>,
    /// Where the documentation is published (`SAGAT_DOCS_URL`), if somebody said: the links in the
    /// help of a problem then lead there, and to the file in the repository otherwise.
    pub docs_url: Option<String>,
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
            grpc: var("SAGAT_GRPC"),
            idp: var("SAGAT_DEV_IDP"),
            docs_url: var("SAGAT_DOCS_URL"),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

/// The two streams a command writes to.
pub(crate) struct Io<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    /// Where the documentation is published, if somebody said.
    docs: Option<String>,
    /// What stops a long-running command, when the caller gave one (a test, or a program that
    /// embeds the tool); otherwise the process's own signals do.
    stop: Option<tokio::sync::watch::Receiver<bool>>,
}

/// Whether a character may be shown as it is: not a control character, and none of the format
/// characters that reorder or hide text (the bidirectional overrides and isolates, the zero-width
/// characters, the line and paragraph separators, the invisible "tag" characters that can carry
/// text nobody sees, the variation selectors, the soft hyphen and the combining grapheme joiner).
fn shown_as_is(c: char) -> bool {
    !c.is_control()
        && !matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
                | '\u{E0100}'..='\u{E01EF}'
        )
}

/// A line as it may be shown on a terminal. Much of what is printed was written by somebody else (a
/// reason typed by an approver, a name or a message from the platform): a control character in it
/// (an escape sequence that rewrites the screen, a carriage return that hides the start of the
/// line) or a character that reorders or hides text is shown as `?`. New lines and tabs are the
/// layout of the tool's own text and stay; text written by somebody else goes through
/// [`one_line`] first.
#[must_use]
pub fn printable(line: &str) -> std::borrow::Cow<'_, str> {
    if line
        .chars()
        .all(|c| shown_as_is(c) || c == '\n' || c == '\t')
    {
        return std::borrow::Cow::Borrowed(line);
    }
    std::borrow::Cow::Owned(
        line.chars()
            .map(|c| {
                if shown_as_is(c) || c == '\n' || c == '\t' {
                    c
                } else {
                    '?'
                }
            })
            .collect(),
    )
}

/// Text somebody else wrote, as one line of ours: a new line in it would forge another line of the
/// tool's output (a row of a history that never happened).
#[must_use]
pub fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .collect()
}

impl Io<'_> {
    /// What ends the command: the one it was given, or the process's own signals.
    pub(crate) fn stop(&mut self) -> tokio::sync::watch::Receiver<bool> {
        self.stop.take().unwrap_or_else(crate::worker::stop_signal)
    }

    /// Where the documentation is published, if somebody said (`SAGAT_DOCS_URL`).
    pub(crate) fn docs(&self) -> Option<&str> {
        self.docs.as_deref()
    }

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

/// Where the documentation is published, if somebody said (`SAGAT_DOCS_URL`): the links in the help
/// of a problem then lead there, and to the file in the repository otherwise.
fn docs_base(env: &Env) -> Option<String> {
    env.docs_url
        .clone()
        .filter(|v| v.starts_with("https://") || v.starts_with("http://"))
}

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
    run_with(args, env, out, err, None).await
}

/// Runs a command line that a long-running command (`worker dev`) ends when `stop` becomes true,
/// instead of when the process is signalled.
pub async fn run_until(
    args: &[String],
    env: &Env,
    out: &mut dyn Write,
    err: &mut dyn Write,
    stop: tokio::sync::watch::Receiver<bool>,
) -> u8 {
    run_with(args, env, out, err, Some(stop)).await
}

async fn run_with(
    args: &[String],
    env: &Env,
    out: &mut dyn Write,
    err: &mut dyn Write,
    stop: Option<tokio::sync::watch::Receiver<bool>>,
) -> u8 {
    let mut io = Io {
        out,
        err,
        docs: docs_base(env),
        stop,
    };
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
        Command::WorkflowValidate { file } => {
            workflow::validate(&common, env, &file, &mut io).await
        }
        Command::WorkflowPublish {
            file,
            notes,
            dry_run,
            accept_breaking,
        } => {
            workflow::publish(
                &common,
                env,
                &file,
                notes.as_deref(),
                dry_run,
                accept_breaking,
                &mut io,
            )
            .await
        }
        Command::WorkflowRun {
            name,
            input,
            wait,
            timeout,
            key,
        } => {
            workflow::run(
                &common,
                env,
                &workflow::RunAsk {
                    name: &name,
                    input: input.as_deref(),
                    wait,
                    timeout,
                    key: key.as_deref(),
                },
                &mut io,
            )
            .await
        }
        Command::WorkflowRuns { name, limit } => {
            workflow::runs(&common, env, &name, limit, &mut io).await
        }
        Command::WorkflowInspect {
            run,
            values,
            reveal_pii,
            sequence,
        } => {
            workflow::inspect(
                &common,
                env,
                &workflow::InspectAsk {
                    run: &run,
                    values,
                    reveal_pii,
                    sequence,
                },
                &mut io,
            )
            .await
        }
        Command::WorkflowCancel { run, reason } => {
            workflow::cancel(&common, env, &run, &reason, &mut io).await
        }
        Command::WorkflowRerun { run, reason } => {
            workflow::rerun(&common, env, &run, &reason, &mut io).await
        }
        Command::RevealGrantsList => reveal::list(&common, env, &mut io).await,
        Command::RevealGrant { subject } => {
            reveal::grant_subject(&common, env, &subject, &mut io).await
        }
        Command::RevealRevoke { subject } => {
            reveal::revoke_subject(&common, env, &subject, &mut io).await
        }
        Command::Doctor { json, nats } => {
            doctor::run(&common, env, json, nats.as_deref(), &mut io).await
        }
        Command::WorkerDev {
            mocks,
            workflow,
            bind,
            idle_exit,
            max_tasks,
            show_payloads,
            allow_non_dev,
        } => {
            worker::dev(
                &common,
                env,
                &worker::Ask {
                    mocks: &mocks,
                    workflow: workflow.as_deref(),
                    bind,
                    idle_exit,
                    max_tasks,
                    show_payloads,
                    allow_non_dev,
                },
                &mut io,
            )
            .await
        }
        Command::DevToken {
            idp,
            subject,
            tenant,
            ttl,
            out,
        } => {
            dev::token(
                env,
                &dev::Ask {
                    idp: idp.as_deref(),
                    subject: &subject,
                    tenant: &tenant,
                    ttl,
                    out: out.as_deref(),
                },
                &mut io,
            )
            .await
        }
        Command::WorkerBind {
            workflow,
            activities,
            allow_non_dev,
        } => {
            worker::bind_command(
                &common,
                env,
                workflow.as_deref(),
                &activities,
                allow_non_dev,
                &mut io,
            )
            .await
        }
        Command::New {
            template,
            dir,
            force,
        } => new::run(env, template.as_deref(), dir.as_deref(), force, &mut io),
        Command::ScriptTest {
            file,
            input,
            language,
            name,
            timeout,
        } => {
            script::test(
                &common,
                env,
                &file,
                input.as_deref(),
                language.as_deref(),
                name.as_deref(),
                timeout,
                &mut io,
            )
            .await
        }
        Command::ScriptPublish {
            file,
            language,
            name,
            timeout,
        } => {
            script::publish(
                &common,
                env,
                &file,
                language.as_deref(),
                name.as_deref(),
                timeout,
                &mut io,
            )
            .await
        }
    };
    match result {
        Ok(code) => code,
        Err(why) => {
            io.warn(format!("sagat: {why}"));
            EXIT_FAILED
        }
    }
}
