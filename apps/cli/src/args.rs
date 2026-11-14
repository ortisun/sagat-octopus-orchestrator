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
                         [--business-id <ID> [--if-exists fail|use-existing] [--reuse allow-duplicate|allow-duplicate-failed-only|reject-duplicate]]
                         [--start-delay <SECONDS>]
                          an identifier of your own makes the run findable and unique while it is open;
                          --start-delay holds the run's first step for up to 366 days
    workflow signal-with-start <NAME> --business-id <ID> --signal <NAME> [--payload <JSON|@FILE>] [--input <JSON|@FILE>] [--reuse <POLICY>] [--key <KEY>]
                          signal the open run of the identifier, or start one that waits for the signal
    workflow update-with-start <NAME> --business-id <ID> --update <NAME> --update-key <KEY> [--args <JSON|@FILE>] [--input <JSON|@FILE>]
                         [--if-exists fail|use-existing] [--reuse <POLICY>] [--start-delay <SECONDS>] [--expected-revision <N>] [--key <KEY>]
                          apply a declared Update to the open run of the identifier, or start one and apply it
    workflow reset-points <RUN>          where a run that ended can be reset to
    workflow reset-preview <RUN> --sequence <N> [--reapply none|signals|updates|all]
                          what a reset to that point keeps and runs again; --reapply also lists the signals and
                          Updates the run received after the point that the reset would deliver again
    workflow reset <RUN> --sequence <N> --reason <TEXT> [--acknowledge] [--drop-business-id]
                         [--reapply none|signals|updates|all] [--cancel-source] [--key <KEY>]
                          a new run from that point of a run that ended; --acknowledge says you know the steps
                          that completed will run again; --reapply delivers again, in order, the signals the run
                          received after the point and the Updates that kept their arguments, and reports each;
                          --cancel-source also resets a run that has not ended: it is cancelled, and the new run is
                          made when it has ended (the answer says it waits; `workflow resets` says how it went);
                          there --acknowledge is also needed for a step in flight, and without it nothing is cancelled
    workflow resets <RUN>                the resets made from a run, and the ones that wait for it to end
    workflow runs <NAME> [--limit <N>]   the latest runs of a workflow (the ones an event or a schedule started too)
    workflow inspect <RUN> [--values [--reveal-pii] [--sequence N]]
                                          the status, journal and attempts; optionally disclosed values
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
    namespace reveal-grants list
    namespace reveal-grants grant <SUBJECT>
    namespace reveal-grants revoke <SUBJECT>
                          list or change the namespace's temporary PII reveal grants

SCHEDULES AND BATCH JOBS:
    schedule list [--state active|paused] [--limit <N>]
    schedule get <ID>                    the schedule, its next times and its latest actions
    schedule create --definition <FILE> [--business-id-prefix <TEXT> [--if-exists fail|use-existing] [--reuse <POLICY>]] [--key <KEY>]
    schedule update <ID> --definition <FILE> --revision <N> [--business-id-prefix <TEXT> [--if-exists ..] [--reuse ..]] [--key <KEY>]
    schedule delete <ID> --revision <N> [--key <KEY>]
    schedule pause <ID> --note <TEXT> [--key <KEY>]
    schedule resume <ID> --note <TEXT> [--key <KEY>]
    schedule trigger <ID> [--overlap skip|buffer-one|buffer-all|cancel-other|allow-all] [--key <KEY>]
    schedule backfill <ID> --from <TIME> --to <TIME> [--overlap <POLICY>] [--key <KEY>]
    schedule actions <ID> [--limit <N>]
                          a schedule is a JSON document of the contract; --revision is the one you saw; with
                          --business-id-prefix its runs get the id <TEXT>-<time of the action> (RFC 3339, UTC), the
                          flags going over what the file says
    batch preview (--selector <JSON|@FILE> | --business-id-prefix <TEXT>) [--max-items <N>]
    batch create (--selector <JSON|@FILE> | --business-id-prefix <TEXT>) --action <ACTION> --reason <TEXT> [--max-items <N>] [--rate <PER SECOND>]
                 [--signal <NAME> [--payload <JSON|@FILE>]] [--step-id <STEP>] [--drop-business-id] [--key <KEY>]
    batch get <ID> | batch list [--state <STATE>] [--limit <N>] | batch cancel <ID> --reason <TEXT> [--key <KEY>]
    batch items <ID> [--state <STATE>] [--limit <N>]
                          one action (cancel, signal, rerun, retry-step, pause, resume, mark-manual-review) over
                          the runs a selector names, at a rate the platform bounds

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
        options: StartOptions,
    },
    /// Signals the open run of a business identifier, or starts one and signals it (feature 018).
    WorkflowSignalWithStart {
        name: String,
        business_id: String,
        signal: String,
        payload: Option<String>,
        input: Option<String>,
        reuse: Option<Reuse>,
        key: Option<String>,
    },
    /// Applies a declared Update to the open run of a business identifier, or starts one and applies it.
    WorkflowUpdateWithStart {
        name: String,
        business_id: String,
        update: String,
        update_key: String,
        args: Option<String>,
        input: Option<String>,
        if_exists: Option<IfExists>,
        reuse: Option<Reuse>,
        start_delay_secs: Option<u64>,
        expected_revision: Option<u64>,
        key: Option<String>,
    },
    /// The places a run that ended can be reset to.
    WorkflowResetPoints {
        run: String,
    },
    /// What a reset to a point would keep and run again.
    WorkflowResetPreview {
        run: String,
        sequence: u64,
        reapply: Option<Reapply>,
    },
    /// A new run from a point of a run that ended.
    WorkflowReset {
        run: String,
        sequence: u64,
        reason: String,
        acknowledge: bool,
        drop_business_id: bool,
        reapply: Option<Reapply>,
        /// Cancel the run if it has not ended and make the new run when it has (`sourcePolicy: CANCEL_SOURCE`).
        cancel_source: bool,
        key: Option<String>,
    },
    /// The resets made from a run, with how each went.
    WorkflowResets {
        run: String,
    },
    WorkflowRuns {
        name: String,
        limit: u32,
    },
    WorkflowInspect {
        run: String,
        values: bool,
        reveal_pii: bool,
        sequence: Option<u64>,
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
    RevealGrantsList,
    RevealGrant {
        subject: String,
    },
    RevealRevoke {
        subject: String,
    },
    Schedule(ScheduleCommand),
    Batch(BatchCommand),
}

/// What a schedule's action says about the business id of the runs it starts (feature 018, `FR-018-016`): the prefix, and what to
/// do when an open run holds the id (`--if-exists`) or a closed one still remembers it (`--reuse`). What the flags give goes
/// over what the file says; the platform decides whether it is valid.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionBusiness {
    pub prefix: Option<String>,
    pub if_exists: Option<IfExists>,
    pub reuse: Option<Reuse>,
}

/// `sagat schedule …` (feature 018): the schedules of the namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleCommand {
    List {
        state: Option<String>,
        limit: u32,
    },
    Get {
        id: String,
    },
    Create {
        file: PathBuf,
        key: Option<String>,
        business: ActionBusiness,
    },
    Update {
        id: String,
        file: PathBuf,
        revision: u64,
        key: Option<String>,
        business: ActionBusiness,
    },
    Delete {
        id: String,
        revision: u64,
        key: Option<String>,
    },
    Pause {
        id: String,
        note: String,
        key: Option<String>,
    },
    Resume {
        id: String,
        note: String,
        key: Option<String>,
    },
    Trigger {
        id: String,
        overlap: Option<String>,
        key: Option<String>,
    },
    Backfill {
        id: String,
        from: String,
        to: String,
        overlap: Option<String>,
        key: Option<String>,
    },
    Actions {
        id: String,
        limit: u32,
    },
}

/// What a batch job does to each run it selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRequest {
    /// The selector, as JSON or `@FILE`; none when the runs are named by `--business-id-prefix` alone.
    pub selector: Option<String>,
    /// The runs whose business id starts with this text, joined to the filter of the selector (`G2027`).
    pub business_id_prefix: Option<String>,
    pub action: String,
    pub reason: String,
    pub max_items: Option<u32>,
    /// Items per second.
    pub rate: Option<String>,
    pub signal: Option<String>,
    pub payload: Option<String>,
    pub update: Option<String>,
    pub args: Option<String>,
    pub step_id: Option<String>,
    pub drop_business_id: bool,
    pub key: Option<String>,
}

/// `sagat batch …` (feature 018): one action over many runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchCommand {
    Preview {
        selector: Option<String>,
        business_id_prefix: Option<String>,
        max_items: Option<u32>,
    },
    Create(Box<BatchRequest>),
    Get {
        id: String,
    },
    List {
        state: Option<String>,
        limit: u32,
    },
    Items {
        id: String,
        state: Option<String>,
        limit: u32,
    },
    Cancel {
        id: String,
        /// Why the job is stopped (8 to 2,000 characters; the platform checks it and keeps it with the job).
        reason: String,
        key: Option<String>,
    },
}

/// What to do when an open run already holds the business identifier (`--if-exists`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfExists {
    /// Refuse and start nothing (the default).
    Fail,
    /// Answer the run that holds it.
    UseExisting,
}

impl IfExists {
    /// The name the API uses.
    #[must_use]
    pub const fn api(self) -> &'static str {
        match self {
            Self::Fail => "FAIL",
            Self::UseExisting => "USE_EXISTING",
        }
    }
}

/// What a reset delivers again to the run it makes (`--reapply`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reapply {
    None,
    Signals,
    Updates,
    All,
}

impl Reapply {
    /// The name the API uses.
    #[must_use]
    pub const fn api(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Signals => "SIGNALS",
            Self::Updates => "UPDATES",
            Self::All => "ALL",
        }
    }
}

/// What to do when a run that ended still remembers the business identifier (`--reuse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reuse {
    AllowDuplicate,
    AllowDuplicateFailedOnly,
    RejectDuplicate,
}

impl Reuse {
    /// The name the API uses.
    #[must_use]
    pub const fn api(self) -> &'static str {
        match self {
            Self::AllowDuplicate => "ALLOW_DUPLICATE",
            Self::AllowDuplicateFailedOnly => "ALLOW_DUPLICATE_FAILED_ONLY",
            Self::RejectDuplicate => "REJECT_DUPLICATE",
        }
    }
}

/// What a start says about the business identifier of the run and about holding its entry (feature 018).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartOptions {
    pub business_id: Option<String>,
    pub if_exists: Option<IfExists>,
    pub reuse: Option<Reuse>,
    /// Seconds the entry of the run is held for.
    pub start_delay_secs: Option<u64>,
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
const WITH_VALUE: [&str; 52] = [
    "--definition",
    "--note",
    "--revision",
    "--from",
    "--to",
    "--overlap",
    "--selector",
    "--business-id-prefix",
    "--action",
    "--max-items",
    "--rate",
    "--step-id",
    "--business-id",
    "--if-exists",
    "--reuse",
    "--reapply",
    "--start-delay",
    "--signal",
    "--payload",
    "--update",
    "--update-key",
    "--args",
    "--expected-revision",
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
    "--sequence",
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

/// A business identifier as the platform takes it: 1 to 200 visible ASCII characters. It is the caller's own
/// text, so a refusal says what the rule is and never what was typed.
fn business_id(text: String) -> Result<String, UsageError> {
    if (1..=200).contains(&text.len()) && text.bytes().all(|byte| (b'!'..=b'~').contains(&byte)) {
        Ok(text)
    } else {
        Err(UsageError(
            "--business-id is 1 to 200 visible ASCII characters".to_owned(),
        ))
    }
}

/// A business-id prefix as a schedule's action or a batch filter takes it: 1 to 160 visible ASCII characters (the id it makes
/// is the prefix, a dash and 20 characters of time, inside the 200 of an id; the filter takes up to 200 of them). It is the
/// caller's own text, so a refusal says what the rule is and never what was typed. A convenience, not the rule: the platform checks
/// the same limits when the schedule is written or the selector is used (a `422`), so this only says so before a request is made,
/// and a change of the limits there is a change of the numbers here.
fn business_id_prefix(text: String, longest: usize) -> Result<String, UsageError> {
    if (1..=longest).contains(&text.len()) && text.bytes().all(|byte| (b'!'..=b'~').contains(&byte))
    {
        Ok(text)
    } else {
        Err(UsageError(format!(
            "--business-id-prefix is 1 to {longest} visible ASCII characters"
        )))
    }
}

/// What a schedule's action takes about the business id of its runs.
fn action_business(flags: &mut Flags) -> Result<ActionBusiness, UsageError> {
    let prefix = flags
        .take_value(&["--business-id-prefix"])
        .map(|text| business_id_prefix(text, 160))
        .transpose()?;
    // the policies go with a prefix: the flag's, or the one in the file (which is looked at when the document is read)
    Ok(ActionBusiness {
        prefix,
        if_exists: if_exists(flags)?,
        reuse: reuse(flags)?,
    })
}

/// The selector of a batch command: `--selector`, `--business-id-prefix`, or both (the prefix joins the filter), and at least
/// one of them.
fn batch_selector(flags: &mut Flags) -> Result<(Option<String>, Option<String>), UsageError> {
    let selector = flags.take_value(&["--selector"]);
    let prefix = flags
        .take_value(&["--business-id-prefix"])
        .map(|text| business_id_prefix(text, 200))
        .transpose()?;
    if selector.is_none() && prefix.is_none() {
        return Err(UsageError(
            "--selector is required (or --business-id-prefix, for the runs whose business id starts with it)"
                .to_owned(),
        ));
    }
    Ok((selector, prefix))
}

/// The id of a schedule or of a batch job: a UUID, which goes into a path, so nothing else is let through.
fn resource_id(text: &str, what: &str) -> Result<String, UsageError> {
    request_id(text).map_err(|_| UsageError(format!("the {what} is given by its id (a UUID)")))
}

/// `--definition <FILE>`: the file with the schedule to write.
fn definition_file(flags: &mut Flags) -> Result<PathBuf, UsageError> {
    flags
        .required("--definition")
        .map(PathBuf::from)
        .map_err(|_| UsageError("--definition is required: the file with the schedule".to_owned()))
}

/// `--overlap`, in the words of the command line.
fn overlap(flags: &mut Flags) -> Result<Option<String>, UsageError> {
    flags
        .take_value(&["--overlap"])
        .map(|value| match value.as_str() {
            "skip" => Ok("SKIP"),
            "buffer-one" => Ok("BUFFER_ONE"),
            "buffer-all" => Ok("BUFFER_ALL"),
            "cancel-other" => Ok("CANCEL_OTHER"),
            "allow-all" => Ok("ALLOW_ALL"),
            _ => Err(UsageError(
                "--overlap is skip, buffer-one, buffer-all, cancel-other or allow-all".to_owned(),
            )),
        })
        .map(|result| result.map(str::to_owned))
        .transpose()
}

/// A filter by state: `--state` in lower case, the name the API uses in upper case.
fn state_filter(flags: &mut Flags, allowed: &[&str]) -> Result<Option<String>, UsageError> {
    flags
        .take_value(&["--state"])
        .map(|value| {
            let name = value.to_ascii_uppercase();
            if allowed.contains(&name.as_str()) {
                Ok(name)
            } else {
                Err(UsageError(format!(
                    "--state is one of {}",
                    allowed
                        .iter()
                        .map(|state| state.to_ascii_lowercase())
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
        })
        .transpose()
}

/// `--limit`, 1 to 200, with the default of the command.
fn list_limit(flags: &mut Flags, default: u32) -> Result<u32, UsageError> {
    Ok(flags
        .take_value(&["--limit"])
        .map(|value| number(&value, "--limit"))
        .transpose()?
        .unwrap_or(default)
        .min(200))
}

/// `--revision`: the revision the caller saw, which a change of a schedule must name.
fn revision(flags: &mut Flags) -> Result<u64, UsageError> {
    flags
        .required("--revision")?
        .parse::<u64>()
        .ok()
        .filter(|revision| *revision >= 1)
        .ok_or_else(|| UsageError("--revision is the revision you saw, from 1".to_owned()))
}

/// The action of a batch job, in either case, with `-` for `_`.
fn batch_action(text: &str) -> Result<String, UsageError> {
    let name = text.to_ascii_uppercase().replace('-', "_");
    const ACTIONS: [&str; 9] = [
        "CANCEL",
        "SIGNAL",
        "UPDATE",
        "RERUN",
        "RETRY_STEP",
        "PAUSE",
        "RESUME",
        "MARK_MANUAL_REVIEW",
        "REPIN",
    ];
    if ACTIONS.contains(&name.as_str()) {
        Ok(name)
    } else {
        Err(UsageError(format!(
            "--action is one of {}",
            ACTIONS
                .iter()
                .map(|action| action.to_ascii_lowercase().replace('_', "-"))
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

fn if_exists(flags: &mut Flags) -> Result<Option<IfExists>, UsageError> {
    flags
        .take_value(&["--if-exists"])
        .map(|value| match value.as_str() {
            "fail" => Ok(IfExists::Fail),
            "use-existing" => Ok(IfExists::UseExisting),
            _ => Err(UsageError("--if-exists is fail or use-existing".to_owned())),
        })
        .transpose()
}

/// `--reapply`, in the words of the other policies (the names of the API in lower case are accepted too).
fn reapply(flags: &mut Flags) -> Result<Option<Reapply>, UsageError> {
    flags
        .take_value(&["--reapply"])
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "none" => Ok(Reapply::None),
            "signals" => Ok(Reapply::Signals),
            "updates" => Ok(Reapply::Updates),
            "all" => Ok(Reapply::All),
            _ => Err(UsageError(
                "--reapply is none, signals, updates or all".to_owned(),
            )),
        })
        .transpose()
}

fn reuse(flags: &mut Flags) -> Result<Option<Reuse>, UsageError> {
    flags
        .take_value(&["--reuse"])
        .map(|value| match value.as_str() {
            "allow-duplicate" => Ok(Reuse::AllowDuplicate),
            "allow-duplicate-failed-only" => Ok(Reuse::AllowDuplicateFailedOnly),
            "reject-duplicate" => Ok(Reuse::RejectDuplicate),
            _ => Err(UsageError(
                "--reuse is allow-duplicate, allow-duplicate-failed-only or reject-duplicate"
                    .to_owned(),
            )),
        })
        .transpose()
}

/// `--start-delay`, in seconds: 1 to 366 days.
fn start_delay(flags: &mut Flags) -> Result<Option<u64>, UsageError> {
    flags
        .take_value(&["--start-delay"])
        .map(|value| seconds(&value, "--start-delay", 31_622_400))
        .transpose()
}

/// A policy about the business identifier means nothing without one.
fn needs_business_id(
    business: &Option<String>,
    policies: &[(&str, bool)],
) -> Result<(), UsageError> {
    match (business, policies.iter().find(|(_, given)| *given)) {
        (None, Some((name, _))) => Err(UsageError(format!("{name} needs --business-id"))),
        _ => Ok(()),
    }
}

/// What a plain start takes about the business identifier and the delay.
fn start_options(flags: &mut Flags) -> Result<StartOptions, UsageError> {
    let business = flags
        .take_value(&["--business-id"])
        .map(business_id)
        .transpose()?;
    let if_exists = if_exists(flags)?;
    let reuse = reuse(flags)?;
    needs_business_id(
        &business,
        &[
            ("--if-exists", if_exists.is_some()),
            ("--reuse", reuse.is_some()),
        ],
    )?;
    Ok(StartOptions {
        business_id: business,
        if_exists,
        reuse,
        start_delay_secs: start_delay(flags)?,
    })
}

fn sequence(text: &str) -> Result<u64, UsageError> {
    text.parse::<u64>()
        .ok()
        .filter(|n| (1..=i64::MAX as u64).contains(n))
        .ok_or_else(|| {
            UsageError(format!(
                "--sequence is a positive sequence up to {}",
                i64::MAX
            ))
        })
}

fn subject(text: &str) -> Result<String, UsageError> {
    sagat_domain::identity::Subject::parse(text)
        .map(|subject| subject.as_str().to_owned())
        .map_err(|_| UsageError("the subject is not a valid caller identity".to_owned()))
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
                options: start_options(&mut flags)?,
            },
            Some((sub, tail)) if sub == "signal-with-start" => Command::WorkflowSignalWithStart {
                name: argument(tail, "the name of the workflow")?.to_owned(),
                business_id: business_id(flags.required("--business-id")?)?,
                signal: flags.required("--signal")?,
                payload: flags.take_value(&["--payload"]),
                input: flags.take_value(&["--input"]),
                reuse: reuse(&mut flags)?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "update-with-start" => Command::WorkflowUpdateWithStart {
                name: argument(tail, "the name of the workflow")?.to_owned(),
                business_id: business_id(flags.required("--business-id")?)?,
                update: flags.required("--update")?,
                update_key: flags.required("--update-key")?,
                args: flags.take_value(&["--args"]),
                input: flags.take_value(&["--input"]),
                if_exists: if_exists(&mut flags)?,
                reuse: reuse(&mut flags)?,
                start_delay_secs: start_delay(&mut flags)?,
                expected_revision: flags
                    .take_value(&["--expected-revision"])
                    .map(|value| {
                        value.parse::<u64>().map_err(|_| {
                            UsageError("--expected-revision is a revision number".to_owned())
                        })
                    })
                    .transpose()?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "reset-points" => Command::WorkflowResetPoints {
                run: run_id(argument(tail, "the run (its id)")?)?,
            },
            Some((sub, tail)) if sub == "reset-preview" => Command::WorkflowResetPreview {
                run: run_id(argument(tail, "the run (its id)")?)?,
                sequence: sequence(&flags.required("--sequence")?)?,
                reapply: reapply(&mut flags)?,
            },
            Some((sub, tail)) if sub == "reset" => Command::WorkflowReset {
                run: run_id(argument(tail, "the run (its id)")?)?,
                sequence: sequence(&flags.required("--sequence")?)?,
                reason: flags.required("--reason")?,
                acknowledge: flags.take_switch("--acknowledge"),
                drop_business_id: flags.take_switch("--drop-business-id"),
                reapply: reapply(&mut flags)?,
                cancel_source: flags.take_switch("--cancel-source"),
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "resets" => Command::WorkflowResets {
                run: run_id(argument(tail, "the run (its id)")?)?,
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
            Some((sub, tail)) if sub == "inspect" => {
                let values = flags.take_switch("--values");
                let reveal_pii = flags.take_switch("--reveal-pii");
                let sequence = flags
                    .take_value(&["--sequence"])
                    .map(|value| sequence(&value))
                    .transpose()?;
                if reveal_pii && !values {
                    return Err(UsageError("--reveal-pii needs --values".to_owned()));
                }
                if sequence.is_some() && !values {
                    return Err(UsageError("--sequence needs --values".to_owned()));
                }
                Command::WorkflowInspect {
                    run: run_id(argument(tail, "the run (its id)")?)?,
                    values,
                    reveal_pii,
                    sequence,
                }
            }
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
                    "'workflow {other}' is not a command (validate, publish, run, runs, inspect, cancel, rerun, signal-with-start, update-with-start, reset-points, reset-preview, reset, resets)"
                )));
            }
            None => {
                return Err(UsageError(
                    "workflow needs a command (validate, publish, run, runs, inspect, cancel, rerun, signal-with-start, update-with-start, reset-points, reset-preview, reset, resets)"
                        .to_owned(),
                ));
            }
        },
        Some("schedule") => Command::Schedule(match rest.split_first() {
            Some((sub, tail)) if sub == "list" => {
                nothing(tail)?;
                ScheduleCommand::List {
                    state: state_filter(&mut flags, &["ACTIVE", "PAUSED"])?,
                    limit: list_limit(&mut flags, 50)?,
                }
            }
            Some((sub, tail)) if sub == "get" => ScheduleCommand::Get {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
            },
            Some((sub, tail)) if sub == "create" => {
                nothing(tail)?;
                ScheduleCommand::Create {
                    file: definition_file(&mut flags)?,
                    business: action_business(&mut flags)?,
                    key: flags.take_value(&["--key"]),
                }
            }
            Some((sub, tail)) if sub == "update" => ScheduleCommand::Update {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
                file: definition_file(&mut flags)?,
                revision: revision(&mut flags)?,
                business: action_business(&mut flags)?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "delete" => ScheduleCommand::Delete {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
                revision: revision(&mut flags)?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "pause" => ScheduleCommand::Pause {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
                note: flags.required("--note")?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "resume" => ScheduleCommand::Resume {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
                note: flags.required("--note")?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "trigger" => ScheduleCommand::Trigger {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
                overlap: overlap(&mut flags)?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "backfill" => ScheduleCommand::Backfill {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
                from: flags.required("--from")?,
                to: flags.required("--to")?,
                overlap: overlap(&mut flags)?,
                key: flags.take_value(&["--key"]),
            },
            Some((sub, tail)) if sub == "actions" => ScheduleCommand::Actions {
                id: resource_id(argument(tail, "the schedule (its id)")?, "schedule")?,
                limit: list_limit(&mut flags, 20)?,
            },
            Some((other, _)) => {
                return Err(UsageError(format!(
                    "'schedule {other}' is not a command (list, get, create, update, delete, pause, resume, trigger, backfill, actions)"
                )));
            }
            None => {
                return Err(UsageError(
                    "schedule needs a command (list, get, create, update, delete, pause, resume, trigger, backfill, actions)"
                        .to_owned(),
                ));
            }
        }),
        Some("batch") => Command::Batch(match rest.split_first() {
            Some((sub, tail)) if sub == "preview" => {
                nothing(tail)?;
                let (selector, business_id_prefix) = batch_selector(&mut flags)?;
                BatchCommand::Preview {
                    selector,
                    business_id_prefix,
                    max_items: flags
                        .take_value(&["--max-items"])
                        .map(|value| number(&value, "--max-items"))
                        .transpose()?,
                }
            }
            Some((sub, tail)) if sub == "create" => {
                nothing(tail)?;
                let (selector, business_id_prefix) = batch_selector(&mut flags)?;
                BatchCommand::Create(Box::new(BatchRequest {
                    selector,
                    business_id_prefix,
                    action: batch_action(&flags.required("--action")?)?,
                    reason: flags.required("--reason")?,
                    max_items: flags
                        .take_value(&["--max-items"])
                        .map(|value| number(&value, "--max-items"))
                        .transpose()?,
                    rate: flags.take_value(&["--rate"]),
                    signal: flags.take_value(&["--signal"]),
                    payload: flags.take_value(&["--payload"]),
                    update: flags.take_value(&["--update"]),
                    args: flags.take_value(&["--args"]),
                    step_id: flags.take_value(&["--step-id"]),
                    drop_business_id: flags.take_switch("--drop-business-id"),
                    key: flags.take_value(&["--key"]),
                }))
            }
            Some((sub, tail)) if sub == "get" => BatchCommand::Get {
                id: resource_id(argument(tail, "the job (its id)")?, "job")?,
            },
            Some((sub, tail)) if sub == "list" => {
                nothing(tail)?;
                BatchCommand::List {
                    state: state_filter(
                        &mut flags,
                        &[
                            "SELECTING",
                            "RUNNING",
                            "CANCELLING",
                            "COMPLETED",
                            "CANCELLED",
                            "FAILED",
                        ],
                    )?,
                    limit: list_limit(&mut flags, 20)?,
                }
            }
            Some((sub, tail)) if sub == "items" => BatchCommand::Items {
                id: resource_id(argument(tail, "the job (its id)")?, "job")?,
                state: state_filter(
                    &mut flags,
                    &[
                        "PENDING",
                        "DEFERRED",
                        "SUCCEEDED",
                        "SKIPPED",
                        "FAILED",
                        "CANCELLED",
                    ],
                )?,
                limit: list_limit(&mut flags, 50)?,
            },
            Some((sub, tail)) if sub == "cancel" => BatchCommand::Cancel {
                id: resource_id(argument(tail, "the job (its id)")?, "job")?,
                reason: flags.required("--reason")?,
                key: flags.take_value(&["--key"]),
            },
            Some((other, _)) => {
                return Err(UsageError(format!(
                    "'batch {other}' is not a command (preview, create, get, list, items, cancel)"
                )));
            }
            None => {
                return Err(UsageError(
                    "batch needs a command (preview, create, get, list, items, cancel)".to_owned(),
                ));
            }
        }),
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
        Some("namespace") => match rest {
            [group, command] if group == "reveal-grants" && command == "list" => {
                Command::RevealGrantsList
            }
            [group, command, value] if group == "reveal-grants" && command == "grant" => {
                Command::RevealGrant {
                    subject: subject(value)?,
                }
            }
            [group, command, value] if group == "reveal-grants" && command == "revoke" => {
                Command::RevealRevoke {
                    subject: subject(value)?,
                }
            }
            [group, command]
                if group == "reveal-grants" && matches!(command.as_str(), "grant" | "revoke") =>
            {
                return Err(UsageError("the subject is required".to_owned()));
            }
            [group, command, ..] if group == "reveal-grants" => {
                return Err(UsageError(format!(
                    "'namespace reveal-grants {command}' is not a command (list, grant, revoke)"
                )));
            }
            [group] if group == "reveal-grants" => {
                return Err(UsageError(
                    "namespace reveal-grants needs a command (list, grant, revoke)".to_owned(),
                ));
            }
            [other, ..] => {
                return Err(UsageError(format!(
                    "'namespace {other}' is not a command (reveal-grants)"
                )));
            }
            [] => {
                return Err(UsageError(
                    "namespace needs a command (reveal-grants)".to_owned(),
                ));
            }
        },
        Some(other) => return Err(UsageError(format!("'{other}' is not a command"))),
    };
    flags.finish()?;
    Ok(Invocation { common, command })
}
