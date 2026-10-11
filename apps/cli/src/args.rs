//! The command line, parsed by hand: a verb, the flags it takes, nothing else.

use std::path::PathBuf;

pub const USAGE: &str = "\
sagat: validate, diff, export, publish, promote and roll back workflows kept in Git

USAGE:
    sagat [GLOBAL FLAGS] <COMMAND> [FLAGS]

GLOBAL FLAGS:
    --server <URL>        the platform's API, e.g. https://sagat.example.com/v1   (SAGAT_SERVER)
    --token-file <PATH>   a file with a bearer token                              (SAGAT_TOKEN holds the token itself)
    --namespace <SLUG>    the namespace to work in (the manifest names one; this overrides it)
    -f, --file <PATH>     the manifest (default: sagat.yaml)

COMMANDS:
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
    0 done   1 failed   2 usage   3 waiting for approval   4 refused as breaking (see --accept-breaking)   5 differences (diff --check)
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
const WITH_VALUE: [&str; 13] = [
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
];

fn split(args: &[String]) -> Result<(Option<String>, Flags), UsageError> {
    let mut verb = None;
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
        } else if verb.is_none() {
            verb = Some(arg.clone());
        } else {
            return Err(UsageError(format!("unexpected argument '{arg}'")));
        }
    }
    Ok((verb, flags))
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
    let (verb, mut flags) = split(args)?;
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
    };
    let command = match verb.as_deref() {
        None | Some("help") => Command::Help,
        Some("version") => Command::Version,
        Some("validate") => Command::Validate,
        Some("diff") => Command::Diff {
            check: flags.take_switch("--check"),
        },
        Some("export") => Command::Export {
            out: PathBuf::from(flags.required("--out")?),
        },
        Some("publish") => Command::Publish {
            dry_run: flags.take_switch("--dry-run"),
            accept_breaking: flags.take_switch("--accept-breaking"),
        },
        Some("promote") => Command::Promote {
            workflow: flags.required("--workflow")?,
            to: flags.required("--to")?,
            reason: flags.required("--reason")?,
            version: flags
                .take_value(&["--version"])
                .map(|v| number(&v, "--version"))
                .transpose()?,
            accept_breaking: flags.take_switch("--accept-breaking"),
        },
        Some("rollback") => Command::Rollback {
            workflow: flags.required("--workflow")?,
            reason: flags.required("--reason")?,
            to: flags
                .take_value(&["--to"])
                .map(|v| number(&v, "--to"))
                .transpose()?,
            accept_breaking: flags.take_switch("--accept-breaking"),
        },
        Some("history") => Command::History {
            workflow: flags.required("--workflow")?,
            limit: flags
                .take_value(&["--limit"])
                .map(|v| number(&v, "--limit"))
                .transpose()?
                .unwrap_or(20)
                .min(200),
        },
        Some("requests") => Command::Requests {
            workflow: flags.required("--workflow")?,
            state: flags.take_value(&["--state"]),
        },
        Some(verb @ ("approve" | "reject")) => Command::Decide {
            request: request_id(&flags.required("--request")?)?,
            reason: flags.required("--reason")?,
            approve: verb == "approve",
        },
        Some(other) => return Err(UsageError(format!("'{other}' is not a command"))),
    };
    flags.finish()?;
    Ok(Invocation { common, command })
}
