//! The command line, parsed by hand: a verb, the flags it takes, nothing else.

use std::path::PathBuf;

pub const USAGE: &str = "\
sagat: write, check, publish, run and roll back workflows, from a laptop or a pipeline

USAGE:
    sagat [GLOBAL FLAGS] <COMMAND> [FLAGS]

GLOBAL FLAGS:
    --server <URL>        the platform's API, e.g. https://sagat.example.com/v1   (SAGAT_SERVER)
    --token-file <PATH>   a file with a bearer token                              (SAGAT_TOKEN holds the token itself)
    --namespace <SLUG>    the namespace to work in (the manifest names one; this overrides it)   (SAGAT_NAMESPACE)
    --grpc <URL>          the worker endpoint, e.g. http://127.0.0.1:19000        (SAGAT_GRPC)
    -f, --file <PATH>     the manifest (default: sagat.yaml)

GETTING STARTED:
    doctor [--json] [--nats <URL>]
                          check the address, the token, the namespace, the versions and the endpoints,
                          and say what to do about each thing that is wrong
    new <TEMPLATE> [DIR] [--force]       write a workflow from a template (new --list shows them)

ONE WORKFLOW:
    workflow validate <FILE>             check a document: where each problem is, what to do, where it is explained
    workflow publish <FILE> [--notes <TEXT>] [--dry-run] [--accept-breaking]
    workflow run <NAME> [--input <JSON|@FILE>] [--wait] [--timeout <SECONDS>] [--key <KEY>]
    workflow runs <NAME> [--limit <N>]   the latest runs of a workflow (the ones an event or a schedule started too)
    workflow inspect <RUN>               the status, the journal and the attempts of a run
    workflow cancel <RUN> --reason <TEXT>
    workflow rerun <RUN> --reason <TEXT>
                          start the same version again, with the same input, from a run that ended
    script test <FILE> [--input <JSON|@FILE>] [--language <typescript|python>] [--name <NAME>] [--timeout <SECONDS>]
    script publish <FILE> [--language <typescript|python>] [--name <NAME>] [--timeout <SECONDS>]
    worker dev --mocks <FILE> [--workflow <FILE>] [--bind] [--show-payloads] [--idle-exit <SECONDS>] [--max-tasks <N>] [--allow-non-dev]
                          serve the activities of a workflow from a file of canned answers and failures
                          (only in a dev namespace unless --allow-non-dev: it answers real tasks)
    dev token [--idp <URL>] [--subject <SUBJECT>] [--tenant <SLUG>] [--ttl <SECONDS>] [--out <FILE>]
                          a token from the local stack's development identity provider (dev-admin of
                          tenant dev unless said otherwise); only the token is printed, or written
                          with --out for --token-file
    worker bind [--workflow <FILE>] [<ACTIVITY>...] [--allow-non-dev]
                          let the identity of the token serve those activities (and the ones the workflow
                          names), so that an SDK worker can; it replaces what the identity served there

A MANIFEST KEPT IN GIT:
    validate              check the manifest and every document: locally, and against the server
    diff [--check]        what publish would change; with --check exit 5 when anything would
    export --out <DIR>    write the active version of every workflow of the namespace and a manifest
    publish [--dry-run] [--accept-breaking]
                          create or update the drafts and publish what changed; where the namespace
                          asks for approvals, open a request (or publish the approved one) and say so
    promote --workflow <NAME> --to <NAMESPACE> --reason <TEXT> [--version <N>] [--accept-breaking]
    rollback --workflow <NAME> --reason <TEXT> [--to <N>] [--accept-breaking]
    history --workflow <NAME> [--limit <N>]
    requests --workflow <NAME> [--state <STATE>]     the requests to publish a workflow, newest first
    approve --request <ID> --reason <TEXT>           approve a request (never your own)
    reject --request <ID> --reason <TEXT>            reject a request: it ends it
    help | version

EXIT CODES:
    0 done   1 failed   2 usage   3 waiting for approval   4 refused as breaking (see --accept-breaking)
    5 differences (diff --check)   6 it did not finish in time, or waits for a person (workflow run --wait, script test|publish)
";

/// A usage error: the message, and the usage text is printed after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

/// The flags every command takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Common {
    pub server: Option<String>,
    pub token_file: Option<PathBuf>,
    pub namespace: Option<String>,
    pub file: Option<PathBuf>,
    pub grpc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Validate,
    Diff {
        check: bool,
    },
    Export {
        out: PathBuf,
    },
    Publish {
        dry_run: bool,
        accept_breaking: bool,
    },
    Promote {
        workflow: String,
        to: String,
        reason: String,
        version: Option<u32>,
        accept_breaking: bool,
    },
    Rollback {
        workflow: String,
        reason: String,
        to: Option<u32>,
        accept_breaking: bool,
    },
    History {
        workflow: String,
        limit: u32,
    },
    Requests {
        workflow: String,
        state: Option<String>,
    },
    Decide {
        request: String,
        reason: String,
        approve: bool,
    },
    Doctor {
        json: bool,
        nats: Option<String>,
    },
    New {
        template: Option<String>,
        dir: Option<PathBuf>,
        force: bool,
    },
    WorkflowValidate {
        file: PathBuf,
    },
    WorkflowPublish {
        file: PathBuf,
        notes: Option<String>,
        dry_run: bool,
        accept_breaking: bool,
    },
    WorkflowRun {
        name: String,
        input: Option<String>,
        wait: bool,
        timeout: u64,
        key: Option<String>,
    },
    WorkflowRuns {
        name: String,
        limit: u32,
    },
    WorkflowInspect {
        run: String,
    },
    WorkflowCancel {
        run: String,
        reason: String,
    },
    WorkflowRerun {
        run: String,
        reason: String,
    },
    ScriptTest {
        file: PathBuf,
        input: Option<String>,
        language: Option<String>,
        name: Option<String>,
        timeout: u64,
    },
    ScriptPublish {
        file: PathBuf,
        language: Option<String>,
        name: Option<String>,
        timeout: u64,
    },
    WorkerDev {
        mocks: PathBuf,
        workflow: Option<PathBuf>,
        bind: bool,
        idle_exit: Option<u64>,
        max_tasks: Option<u64>,
        show_payloads: bool,
        allow_non_dev: bool,
    },
    WorkerBind {
        workflow: Option<PathBuf>,
        activities: Vec<String>,
        allow_non_dev: bool,
    },
    DevToken {
        idp: Option<String>,
        subject: String,
        tenant: String,
        ttl: Option<u64>,
        out: Option<PathBuf>,
    },
}

/// What was asked: the common flags and the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub common: Common,
    pub command: Command,
}

struct Flags {
    values: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Flags {
    fn take_value(&mut self, names: &[&str]) -> Option<String> {
        let at = self
            .values
            .iter()
            .position(|(n, _)| names.contains(&n.as_str()))?;
        Some(self.values.remove(at).1)
    }

    fn take_switch(&mut self, name: &str) -> bool {
        match self.switches.iter().position(|s| s == name) {
            Some(at) => {
                self.switches.remove(at);
                true
            }
            None => false,
        }
    }

    fn required(&mut self, name: &str) -> Result<String, UsageError> {
        self.take_value(&[name])
            .ok_or_else(|| UsageError(format!("{name} is required")))
    }

    fn finish(self) -> Result<(), UsageError> {
        match self
            .values
            .first()
            .map(|(n, _)| n)
            .or(self.switches.first())
        {
            Some(unknown) => Err(UsageError(format!(
                "{unknown} is not a flag of this command"
            ))),
            None => Ok(()),
        }
    }
}

/// Flags that take a value; every other `--flag` is a switch.
const WITH_VALUE: [&str; 28] = [
    "--server",
    "--token-file",
    "--namespace",
    "--file",
    "-f",
    "--out",
    "--workflow",
    "--to",
    "--reason",
    "--version",
    "--limit",
    "--request",
    "--state",
    "--grpc",
    "--input",
    "--timeout",
    "--key",
    "--notes",
    "--language",
    "--name",
    "--mocks",
    "--nats",
    "--idle-exit",
    "--max-tasks",
    "--idp",
    "--subject",
    "--tenant",
    "--ttl",
];

fn split(args: &[String]) -> Result<(Vec<String>, Flags), UsageError> {
    let mut words = Vec::new();
    let mut flags = Flags {
        values: Vec::new(),
        switches: Vec::new(),
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some((name, value)) = arg.split_once('=').filter(|(n, _)| n.starts_with("--")) {
            if !WITH_VALUE.contains(&name) {
                return Err(UsageError(format!("{name} does not take a value")));
            }
            flags.values.push((name.to_owned(), value.to_owned()));
        } else if WITH_VALUE.contains(&arg.as_str()) {
            let value = iter
                .next()
                .ok_or_else(|| UsageError(format!("{arg} needs a value")))?;
            flags.values.push((arg.clone(), value.clone()));
        } else if arg.starts_with('-') {
            flags.switches.push(arg.clone());
        } else {
            words.push(arg.clone());
        }
    }
    Ok((words, flags))
}

/// The word that is a command's argument, and no more of them than it takes.
fn argument<'a>(rest: &'a [String], what: &str) -> Result<&'a str, UsageError> {
    match rest {
        [one] => Ok(one),
        [] => Err(UsageError(format!("{what} is required"))),
        [_, extra, ..] => Err(UsageError(format!("unexpected argument '{extra}'"))),
    }
}

/// A command that takes no word.
fn nothing(rest: &[String]) -> Result<(), UsageError> {
    match rest.first() {
        Some(extra) => Err(UsageError(format!("unexpected argument '{extra}'"))),
        None => Ok(()),
    }
}

/// A word that is optional, and no more of them than one.
fn maybe(rest: &[String]) -> Result<Option<&str>, UsageError> {
    match rest {
        [] => Ok(None),
        [one] => Ok(Some(one)),
        [_, extra, ..] => Err(UsageError(format!("unexpected argument '{extra}'"))),
    }
}

/// A run id: the same shape as a request id, and it goes into a path too.
fn run_id(text: &str) -> Result<String, UsageError> {
    request_id(text).map_err(|_| UsageError("the run is given by its id (a UUID)".to_owned()))
}

fn script_timeout(flags: &mut Flags) -> Result<u64, UsageError> {
    Ok(flags
        .take_value(&["--timeout"])
        .map(|v| seconds(&v, "--timeout", 1800))
        .transpose()?
        .unwrap_or(150))
}

fn seconds(text: &str, name: &str, most: u64) -> Result<u64, UsageError> {
    text.parse::<u64>()
        .ok()
        .filter(|n| (1..=most).contains(n))
        .ok_or_else(|| UsageError(format!("{name} is a number of seconds from 1 to {most}")))
}

/// A request id as the platform writes it: 36 characters of hexadecimal digits and hyphens. It
/// goes into a path, so nothing else is let through.
fn request_id(text: &str) -> Result<String, UsageError> {
    let shaped = text.len() == 36
        && text.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
    if shaped {
        Ok(text.to_ascii_lowercase())
    } else {
        Err(UsageError(
            "--request is the id of a request (a UUID)".to_owned(),
        ))
    }
}

fn number(text: &str, name: &str) -> Result<u32, UsageError> {
    text.parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .ok_or_else(|| UsageError(format!("{name} is a number from 1")))
}

/// Parses the arguments after the program name.
///
/// # Errors
/// A [`UsageError`] naming what is wrong.
pub fn parse(args: &[String]) -> Result<Invocation, UsageError> {
    // `--version` alone is the tool's version, although it is a flag with a value for `promote`.
    if matches!(args, [only] if only == "--version" || only == "-V") {
        return Ok(Invocation {
            common: Common::default(),
            command: Command::Version,
        });
    }
    let (words, mut flags) = split(args)?;
    if flags.take_switch("--help") || flags.take_switch("-h") {
        return Ok(Invocation {
            common: Common::default(),
            command: Command::Help,
        });
    }
    let common = Common {
        server: flags.take_value(&["--server"]),
        token_file: flags.take_value(&["--token-file"]).map(PathBuf::from),
        namespace: flags.take_value(&["--namespace"]),
        file: flags.take_value(&["--file", "-f"]).map(PathBuf::from),
        grpc: flags.take_value(&["--grpc"]),
    };
    let (verb, rest) = match words.split_first() {
        Some((verb, rest)) => (Some(verb.as_str()), rest),
        None => (None, &words[..]),
    };
    let command = match verb {
        None | Some("help") => {
            nothing(rest)?;
            Command::Help
        }
        Some("version") => {
            nothing(rest)?;
            Command::Version
        }
        Some("validate") => {
            nothing(rest)?;
            Command::Validate
        }
        Some("diff") => {
            nothing(rest)?;
            Command::Diff {
                check: flags.take_switch("--check"),
            }
        }
        Some("export") => {
            nothing(rest)?;
            Command::Export {
                out: PathBuf::from(flags.required("--out")?),
            }
        }
        Some("publish") => {
            nothing(rest)?;
            Command::Publish {
                dry_run: flags.take_switch("--dry-run"),
                accept_breaking: flags.take_switch("--accept-breaking"),
            }
        }
        Some("promote") => {
            nothing(rest)?;
            Command::Promote {
                workflow: flags.required("--workflow")?,
                to: flags.required("--to")?,
                reason: flags.required("--reason")?,
                version: flags
                    .take_value(&["--version"])
                    .map(|v| number(&v, "--version"))
                    .transpose()?,
                accept_breaking: flags.take_switch("--accept-breaking"),
            }
        }
        Some("rollback") => {
            nothing(rest)?;
            Command::Rollback {
                workflow: flags.required("--workflow")?,
                reason: flags.required("--reason")?,
                to: flags
                    .take_value(&["--to"])
                    .map(|v| number(&v, "--to"))
                    .transpose()?,
                accept_breaking: flags.take_switch("--accept-breaking"),
            }
        }
        Some("history") => {
            nothing(rest)?;
            Command::History {
                workflow: flags.required("--workflow")?,
                limit: flags
                    .take_value(&["--limit"])
                    .map(|v| number(&v, "--limit"))
                    .transpose()?
                    .unwrap_or(20)
                    .min(200),
            }
        }
        Some("requests") => {
            nothing(rest)?;
            Command::Requests {
                workflow: flags.required("--workflow")?,
                state: flags.take_value(&["--state"]),
            }
        }
        Some(verb @ ("approve" | "reject")) => {
            nothing(rest)?;
            Command::Decide {
                request: request_id(&flags.required("--request")?)?,
                reason: flags.required("--reason")?,
                approve: verb == "approve",
            }
        }
        Some("doctor") => {
            nothing(rest)?;
            Command::Doctor {
                json: flags.take_switch("--json"),
                nats: flags.take_value(&["--nats"]),
            }
        }
        Some("new") => {
            let list = flags.take_switch("--list");
            match rest {
                [] if list => Command::New {
                    template: None,
                    dir: None,
                    force: false,
                },
                [] => {
                    return Err(UsageError(
                        "a template is required (new --list shows them)".to_owned(),
                    ));
                }
                [template, dir @ ..] => Command::New {
                    template: Some(template.clone()),
                    dir: maybe(dir)?.map(PathBuf::from),
                    force: flags.take_switch("--force"),
                },
            }
        }
        Some("workflow") => match rest.split_first() {
            Some((sub, tail)) if sub == "validate" => Command::WorkflowValidate {
                file: PathBuf::from(argument(tail, "the document (a file)")?),
            },
            Some((sub, tail)) if sub == "publish" => Command::WorkflowPublish {
                file: PathBuf::from(argument(tail, "the document (a file)")?),
                notes: flags.take_value(&["--notes"]),
                dry_run: flags.take_switch("--dry-run"),
                accept_breaking: flags.take_switch("--accept-breaking"),
            },
            Some((sub, tail)) if sub == "run" => Command::WorkflowRun {
                name: argument(tail, "the name of the workflow")?.to_owned(),
                input: flags.take_value(&["--input"]),
                wait: flags.take_switch("--wait"),
                timeout: flags
                    .take_value(&["--timeout"])
                    .map(|v| seconds(&v, "--timeout", 86_400))
                    .transpose()?
                    .unwrap_or(300),
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "runs" => Command::WorkflowRuns {
                name: argument(tail, "the name of the workflow")?.to_owned(),
                limit: flags
                    .take_value(&["--limit"])
                    .map(|v| number(&v, "--limit"))
                    .transpose()?
                    .unwrap_or(10)
                    .min(200),
            },
            Some((sub, tail)) if sub == "inspect" => Command::WorkflowInspect {
                run: run_id(argument(tail, "the run (its id)")?)?,
            },
            Some((sub, tail)) if sub == "cancel" => Command::WorkflowCancel {
                run: run_id(argument(tail, "the run (its id)")?)?,
                reason: flags.required("--reason")?,
            },
            Some((sub, tail)) if sub == "rerun" => Command::WorkflowRerun {
                run: run_id(argument(tail, "the run (its id)")?)?,
                reason: flags.required("--reason")?,
            },
            Some((other, _)) => {
                return Err(UsageError(format!(
                    "'workflow {other}' is not a command (validate, publish, run, runs, inspect, cancel, rerun)"
                )));
            }
            None => {
                return Err(UsageError(
                    "workflow needs a command (validate, publish, run, runs, inspect, cancel, rerun)"
                        .to_owned(),
                ));
            }
        },
        Some("script") => match rest.split_first() {
            Some((sub, tail)) if sub == "test" => Command::ScriptTest {
                file: PathBuf::from(argument(tail, "the script (a file)")?),
                input: flags.take_value(&["--input"]),
                language: flags.take_value(&["--language"]),
                name: flags.take_value(&["--name"]),
                timeout: script_timeout(&mut flags)?,
            },
            Some((sub, tail)) if sub == "publish" => Command::ScriptPublish {
                file: PathBuf::from(argument(tail, "the script (a file)")?),
                language: flags.take_value(&["--language"]),
                name: flags.take_value(&["--name"]),
                timeout: script_timeout(&mut flags)?,
            },
            Some((other, _)) => {
                return Err(UsageError(format!(
                    "'script {other}' is not a command (test, publish)"
                )));
            }
            None => {
                return Err(UsageError(
                    "script needs a command (test, publish)".to_owned(),
                ));
            }
        },
        Some("worker") => match rest.split_first() {
            Some((sub, tail)) if sub == "dev" => {
                nothing(tail)?;
                Command::WorkerDev {
                    mocks: PathBuf::from(flags.required("--mocks")?),
                    workflow: flags.take_value(&["--workflow"]).map(PathBuf::from),
                    bind: flags.take_switch("--bind"),
                    show_payloads: flags.take_switch("--show-payloads"),
                    allow_non_dev: flags.take_switch("--allow-non-dev"),
                    idle_exit: flags
                        .take_value(&["--idle-exit"])
                        .map(|v| seconds(&v, "--idle-exit", 86_400))
                        .transpose()?,
                    max_tasks: flags
                        .take_value(&["--max-tasks"])
                        .map(|v| number(&v, "--max-tasks").map(u64::from))
                        .transpose()?,
                }
            }
            Some((sub, tail)) if sub == "bind" => {
                let workflow = flags.take_value(&["--workflow"]).map(PathBuf::from);
                if workflow.is_none() && tail.is_empty() {
                    return Err(UsageError(
                        "worker bind needs a --workflow or the names of the activities".to_owned(),
                    ));
                }
                Command::WorkerBind {
                    workflow,
                    activities: tail.to_vec(),
                    allow_non_dev: flags.take_switch("--allow-non-dev"),
                }
            }
            Some((other, _)) => {
                return Err(UsageError(format!(
                    "'worker {other}' is not a command (dev, bind)"
                )));
            }
            None => return Err(UsageError("worker needs a command (dev, bind)".to_owned())),
        },
        Some("dev") => match rest.split_first() {
            Some((sub, tail)) if sub == "token" => {
                nothing(tail)?;
                Command::DevToken {
                    idp: flags.take_value(&["--idp"]),
                    subject: flags
                        .take_value(&["--subject"])
                        .unwrap_or_else(|| "dev-admin".to_owned()),
                    tenant: flags
                        .take_value(&["--tenant"])
                        .unwrap_or_else(|| "dev".to_owned()),
                    ttl: flags
                        .take_value(&["--ttl"])
                        .map(|v| seconds(&v, "--ttl", 3600))
                        .transpose()?,
                    out: flags.take_value(&["--out"]).map(PathBuf::from),
                }
            }
            Some((other, _)) => {
                return Err(UsageError(format!(
                    "'dev {other}' is not a command (token)"
                )));
            }
            None => return Err(UsageError("dev needs a command (token)".to_owned())),
        },
        Some(other) => return Err(UsageError(format!("'{other}' is not a command"))),
    };
    flags.finish()?;
    Ok(Invocation { common, command })
}
