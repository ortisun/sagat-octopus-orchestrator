//! `control-plane schedule-trigger-handover --auto` (feature 018, `T018-084`, ADR-0091): the deployment's one-shot job that hands
//! the cron triggers over once the fleet is stable and converged.
//!
//! It runs as `sagat_handover`, which can call five functions, read two tables and one column (OD-2026-11-15-1). **The database decides**: every round goes
//! through the round function, which computes every condition and counts the stable rounds in the attempt row; the activation goes
//! through the wrapper, which computes them all again under the epoch lock, takes the mode from `session_user` and the actor from
//! the run id. This loop only asks, waits and says why:
//!
//! * no transaction is open across a wait: a round is one short transaction, the waits hold no connection (decision 3);
//! * what the database answers is typed by SQLSTATE and by its closed outcome columns, never by text (decision 4): a lock or
//!   statement timeout, the step bound and a refusal under the lock are retried with a backoff of 15 s doubling to 5 min, and three
//!   statement timeouts in a row are `CHECK_TOO_SLOW`;
//! * after SIGTERM or SIGINT it starts no activation: it ends the round it is in, writes `INTERRUPTED` and exits 4 (decision 1);
//! * after the commit it writes the tenant audit from the recorded set (decision 7), also when it finds the state handed over, and,
//!   with `--retire-role`, retires the role (D2265).
//!
//! Exit codes: 0 handed over, already handed over, switched off, or a retired role that cannot log in; 2 a reason no wait cures;
//! 3 the deadline, with the last reason; 4 interrupted; 1 a database failure.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sagat_application::schedule_triggers::HandoverBlock;
use sagat_infra_postgres::{
    AutoActivated, AutoAsk, HandoverDbError, OperatorBounds, PgPool, PoolConfig,
    ROUND_STATEMENT_TIMEOUT, Retirement, Round, RoundOutcome, complete_handover_audit,
    connect_handover, handover_activate, handover_note, handover_round, last_attempt,
    may_auto_hand_over, read_handover_mode, retire_handover_role,
};
use uuid::Uuid;

use crate::RunError;
use crate::cli::{HandoverArgs, auto_bounds};
use crate::config::{ConfigError, DatabaseConfig};
use crate::handover::shown;

/// How far apart two rounds are (one heartbeat): the database counts a stable round only that long after the last one counted.
pub const SPACING: Duration = Duration::from_secs(15);
/// The first wait of the backoff, and its ceiling.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(15);
pub const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Statement timeouts in a row that are reported as `CHECK_TOO_SLOW` rather than `BUSY`.
pub const TOO_SLOW_AFTER: u32 = 3;

/// The backoff of what a wait cures: 15 s, doubling, at most 5 min (ADR-0091 decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    /// The wait before the next try, and one more failure counted.
    pub fn next_wait(&mut self) -> Duration {
        let doubled = BACKOFF_FIRST.saturating_mul(1_u32 << self.failures.min(8));
        self.failures = self.failures.saturating_add(1);
        doubled.min(BACKOFF_MAX)
    }

    /// A round that answered: the backoff starts over.
    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

/// Whether SIGTERM or SIGINT was received, and a way to wake whoever waits.
#[derive(Debug, Clone, Default)]
pub struct Interrupt(Arc<(AtomicBool, tokio::sync::Notify)>);

impl Interrupt {
    /// Says the process was asked to stop.
    pub fn raise(&self) {
        self.0.0.store(true, Ordering::SeqCst);
        self.0.1.notify_waiters();
    }

    #[must_use]
    pub fn is_raised(&self) -> bool {
        self.0.0.load(Ordering::SeqCst)
    }

    /// Returns once the process was asked to stop.
    pub async fn raised(&self) {
        loop {
            let notified = self.0.1.notified();
            if self.is_raised() {
                return;
            }
            notified.await;
        }
    }
}

/// Raises `interrupt` on SIGTERM or SIGINT (a drained node, `helm rollback`, `docker compose stop`, Ctrl-C).
///
/// The handlers are installed before this returns, not when the task that waits on them is first polled (G2283): once the job
/// has made a round, a signal is the interrupt and never the default action that kills the process without `INTERRUPTED`.
pub fn raise_on_signals(interrupt: &Interrupt) {
    let interrupt = interrupt.clone();
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let terminate = signal(SignalKind::terminate());
        let ctrl_c = signal(SignalKind::interrupt());
        tokio::spawn(async move {
            match (terminate, ctrl_c) {
                (Ok(mut terminate), Ok(mut ctrl_c)) => {
                    tokio::select! {
                        _ = terminate.recv() => {}
                        _ = ctrl_c.recv() => {}
                    }
                }
                // a handler the system refused: the one that is installed still raises, as `shutdown_signal` does
                (Ok(mut one), Err(_)) | (Err(_), Ok(mut one)) => {
                    one.recv().await;
                }
                (Err(_), Err(_)) => std::future::pending::<()>().await,
            }
            tracing::warn!(
                "SIGTERM or SIGINT: the automatic handover starts no activation and stops after this round"
            );
            interrupt.raise();
        });
    }
    #[cfg(not(unix))]
    tokio::spawn(async move {
        crate::shutdown_signal().await;
        tracing::warn!(
            "SIGTERM or SIGINT: the automatic handover starts no activation and stops after this round"
        );
        interrupt.raise();
    });
}

/// A boxed future of the pacing.
pub type Paced<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// How the loop waits. The process waits for real and wakes on an interrupt; a test makes the waits instant (and moves the
/// database's times back as the owner) and holds the loop at its barriers.
pub trait Pace: Send + Sync {
    /// Waits `wait`, or until the interrupt is raised.
    fn wait<'a>(&'a self, wait: Duration, interrupt: &'a Interrupt) -> Paced<'a>;

    /// Between a stable round and its activation.
    fn before_activation(&self) -> Paced<'_> {
        Box::pin(async {})
    }
}

/// The real waits.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sleep;

impl Pace for Sleep {
    fn wait<'a>(&'a self, wait: Duration, interrupt: &'a Interrupt) -> Paced<'a> {
        Box::pin(async move {
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = interrupt.raised() => {}
            }
        })
    }
}

/// What the job was asked for: what it asks of each round (`ask`), and what it does around them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRequest {
    pub ask: AutoAsk,
    pub retire_role: bool,
    pub dry_run: bool,
    /// Who the deployment says runs it (the release), for the log line only: the actor recorded is the run's.
    pub operator: Option<String>,
    /// The bounds of the activation's transaction (ADR-0091 decision 4): lock wait 2 s, statement 3 s, step 3 s.
    pub activation: OperatorBounds,
}

impl AutoRequest {
    /// The request of a command line, with the run id given.
    #[must_use]
    pub fn of(args: &HandoverArgs, run: Uuid) -> Self {
        Self {
            ask: AutoAsk {
                run,
                stable: args.stable.unwrap_or(auto_bounds::STABLE),
                horizon: args.horizon.unwrap_or(auto_bounds::HORIZON),
                min_live: args.min_live.unwrap_or(auto_bounds::MIN_LIVE),
                wait: args.wait.unwrap_or(auto_bounds::WAIT),
            },
            retire_role: args.retire_role,
            dry_run: args.dry_run,
            operator: args.operator.clone(),
            activation: OperatorBounds::default(),
        }
    }
}

/// What the job did, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoOutcome {
    /// Handed over by this run.
    HandedOver {
        epoch: i64,
        live: u32,
        tenants: u32,
        audited: u32,
        /// The tenant audit could not be completed (a conflict, a failure): the handover is made, the engines count what is left.
        audit_pending: bool,
        retired: Option<Retirement>,
    },
    /// It had been handed over before; the audit was completed if it was not.
    AlreadyHandedOver {
        mode: Option<String>,
        audited: u32,
        audit_pending: bool,
        retired: Option<Retirement>,
    },
    /// `SAGAT_SCHEDULES_AUTO_HANDOVER=false`.
    SwitchedOff,
    /// With `--retire-role`, the role cannot log in: it was retired, or `pg_hba` refuses it (the engines alert on that).
    RoleRetired,
    /// The dry run: the role may make it; the last reason recorded, if any.
    WouldWait { last_reason: Option<String> },
}

fn pending_words(pending: bool) -> &'static str {
    if pending {
        " WARNING: the tenant audit of the handover is not complete (see the log); the engines export \
         trigger_schedule_handover_audit_pending and SagatTriggerHandoverAuditPending fires on it (docs/runbooks/schedules.md, 6.3)."
    } else {
        ""
    }
}

fn retired_words(retired: Option<Retirement>) -> &'static str {
    match retired {
        None => "",
        Some(Retirement::Retired) => {
            " The role sagat_handover was retired (it cannot log in any more)."
        }
        Some(Retirement::RecordedNotAltered) => {
            " The retirement of sagat_handover was recorded, but this database refused to take its login away: run ALTER ROLE \
             sagat_handover NOLOGIN as the role's creator."
        }
        Some(Retirement::NotHandedOver | Retirement::AuditPending) => " The role was not retired.",
    }
}

impl fmt::Display for AutoOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandedOver {
                epoch,
                live,
                tenants,
                audited,
                audit_pending,
                retired,
            } => write!(
                f,
                "handed over automatically: the cron triggers fire from the schedule pass now ({live} live execution engine(s), \
                 requirement epoch {epoch}, {tenants} tenant(s) affected, {audited} audit record(s) written). There is no \
                 command to go back.{}{}",
                pending_words(*audit_pending),
                retired_words(*retired)
            ),
            Self::AlreadyHandedOver {
                mode,
                audited,
                audit_pending,
                retired,
            } => write!(
                f,
                "already handed over (mode {}); {audited} audit record(s) completed now; nothing else was changed.{}{}",
                shown(mode.as_deref().unwrap_or("not recorded")),
                pending_words(*audit_pending),
                retired_words(*retired)
            ),
            Self::SwitchedOff => f.write_str(
                "the automatic handover is switched off (SAGAT_SCHEDULES_AUTO_HANDOVER=false): nothing was done; the operator \
                 hands over with --apply (docs/runbooks/schedules.md, 6.2)",
            ),
            Self::RoleRetired => f.write_str(
                "sagat_handover cannot log in, and --retire-role was given: the role may be retired. If the cron triggers are \
                 not handed over, look at pg_hba and the role (SagatTriggerHandoverNotAttempted fires on it).",
            ),
            Self::WouldWait { last_reason } => write!(
                f,
                "dry run: this role may make the automatic handover; the last reason recorded is {}. Nothing was changed.",
                last_reason
                    .as_deref()
                    .map_or_else(|| "none (no round yet, or the rounds are counting)".to_owned(), shown)
            ),
        }
    }
}

fn database(error: &HandoverDbError) -> RunError {
    RunError::Database(error.to_string())
}

/// A refusal that no wait cures (exit 2), in words.
fn declined(message: impl Into<String>) -> RunError {
    RunError::Declined(message.into())
}

fn denied() -> RunError {
    // refusal test: the_dry_run_checks_the_wrappers_privilege_and_says_the_last_reason
    declined(
        "permission denied: the automatic handover runs as sagat_handover logged in as itself, with the grants migrate gives it \
         (run migrate again; a role wider than the design is given none). The owner's handover is --apply. Nothing was changed.",
    )
}

/// What the deadline is told as: the last reason that blocked.
fn deadline(last: Option<HandoverBlock>) -> RunError {
    let reason = last.map_or("none recorded", HandoverBlock::code);
    RunError::Deadline(format!(
        "the automatic handover waited its whole --wait and the database never found the fleet stable and converged; the last \
         reason was {reason} (docs/runbooks/schedules.md, 6.3, #{}). Nothing was changed; the state is LEGACY.",
        last.map_or_else(
            || "63-why-a-handover-is-blocked".to_owned(),
            HandoverBlock::anchor
        )
    ))
}

fn interrupted() -> RunError {
    RunError::Interrupted(
        "SIGTERM or SIGINT: the automatic handover started no activation and stopped (INTERRUPTED). The state is LEGACY; the \
         next run of the job begins again."
            .to_owned(),
    )
}

/// The command as `main` runs it: the switch, the password, the run id, then [`auto_handover`] with the real waits.
///
/// # Errors
/// As [`auto_handover`]; [`RunError::Declined`] when the job is on and `SAGAT_HANDOVER_PASSWORD` is not set, or the run id is not
/// a uuid.
pub async fn run_auto_command(
    db: &DatabaseConfig,
    args: &HandoverArgs,
    env: &dyn Fn(&str) -> Option<String>,
    interrupt: &Interrupt,
) -> Result<String, RunError> {
    match env("SAGAT_SCHEDULES_AUTO_HANDOVER")
        .as_deref()
        .map(str::trim)
    {
        None | Some("" | "true" | "1") => {}
        Some("false" | "0") => return Ok(AutoOutcome::SwitchedOff.to_string()),
        Some(_) => {
            return Err(ConfigError::Invalid {
                name: "SAGAT_SCHEDULES_AUTO_HANDOVER",
                reason: "must be true or false".to_owned(),
            }
            .into());
        }
    }
    let Some(password) = env("SAGAT_HANDOVER_PASSWORD")
        .filter(|password| !password.is_empty())
        .map(zeroize::Zeroizing::new)
    else {
        // refusal test: the_job_switched_off_exits_zero_at_once_and_without_a_password_exits_two_naming_the_variable
        return Err(declined(
            "SAGAT_HANDOVER_PASSWORD is not set: the automatic handover logs in as sagat_handover with that password, given apart \
             from DATABASE_URL. Set it (and give it to migrate, which sets the role's login), or switch the job off with \
             SAGAT_SCHEDULES_AUTO_HANDOVER=false. Nothing was changed.",
        ));
    };
    let run = run_id(args.run_id, env)?;
    let mut request = AutoRequest::of(args, run);
    // compose cannot add a flag on a condition: the setting stands for `--retire-role` (D2265)
    match env("SAGAT_HANDOVER_RETIRE_ROLE").as_deref().map(str::trim) {
        None | Some("" | "false" | "0") => {}
        Some("true" | "1") => request.retire_role = true,
        Some(_) => {
            return Err(ConfigError::Invalid {
                name: "SAGAT_HANDOVER_RETIRE_ROLE",
                reason: "must be true or false".to_owned(),
            }
            .into());
        }
    }
    auto_handover(db, Some(password.as_str()), &request, &Sleep, interrupt)
        .await
        .map(|outcome| outcome.to_string())
}

/// The run id: `--run-id`, or its environment form `SAGAT_HANDOVER_RUN_ID` (in Kubernetes the Job's UID from the downward API),
/// or drawn at start (compose, which does not retry the job). Two that differ are refused: which run a retried pod resumes must
/// not depend on which of them a deployment happens to set.
///
/// # Errors
/// [`RunError::Config`] (exit 2) for a variable that is not a uuid or one that differs from the flag.
pub fn run_id(flag: Option<Uuid>, env: &dyn Fn(&str) -> Option<String>) -> Result<Uuid, RunError> {
    let variable = match env("SAGAT_HANDOVER_RUN_ID").filter(|id| !id.trim().is_empty()) {
        Some(id) => Some(
            Uuid::parse_str(id.trim()).map_err(|_| ConfigError::Invalid {
                name: "SAGAT_HANDOVER_RUN_ID",
                reason: "must be a uuid (in Kubernetes, the Job's UID from the downward API)"
                    .to_owned(),
            })?,
        ),
        None => None,
    };
    match (flag, variable) {
        (Some(flag), Some(variable)) if flag != variable => Err(ConfigError::Invalid {
            name: "--run-id",
            reason:
                "differs from SAGAT_HANDOVER_RUN_ID: give one of them, or the same uuid in both"
                    .to_owned(),
        }
        .into()),
        (Some(id), _) | (None, Some(id)) => Ok(id),
        (None, None) => Ok(Uuid::new_v4()),
    }
}

/// The automatic handover.
///
/// # Errors
/// [`RunError::Declined`] for a reason no wait cures (exit 2), [`RunError::Deadline`] (3), [`RunError::Interrupted`] (4),
/// [`RunError::Database`] (1).
pub async fn auto_handover(
    db: &DatabaseConfig,
    password: Option<&str>,
    request: &AutoRequest,
    pace: &dyn Pace,
    interrupt: &Interrupt,
) -> Result<AutoOutcome, RunError> {
    let mut config = PoolConfig::new(db.url.clone());
    config.max_connections = 2;
    config.statement_timeout = ROUND_STATEMENT_TIMEOUT;
    // the job's own deadline, from its start until a round says the run's (correctness review Major 1)
    let mut clock = Deadline::from_wait(request.ask.wait);
    let mut backoff = Backoff::default();
    let pool = loop {
        match connect_handover(&config, password).await {
            Ok(pool) => break pool,
            // only 28000 is a role that may not log in; a wrong password (28P01) is never a retirement
            Err(HandoverDbError::CannotLogIn) if request.retire_role => {
                return Ok(AutoOutcome::RoleRetired);
            }
            Err(HandoverDbError::CannotLogIn | HandoverDbError::WrongPassword) => {
                // refusal test: the_role_is_ensured_granted_logged_in_refused_when_wider_and_retired_for_good
                return Err(declined(
                    "sagat_handover cannot log in (SQLSTATE 28000: no login or a pg_hba refusal; 28P01: a wrong password): give \
                     migrate SAGAT_HANDOVER_PASSWORD so that it sets the login, put the same password in the job's Secret, check \
                     pg_hba, or create the login once (docs/runbooks/schedules.md, 6.1). Nothing was changed.",
                ));
            }
            // the database cannot be reached: tried again with the backoff while --wait lasts
            Err(HandoverDbError::Unavailable(detail)) => {
                if clock.passed() {
                    // refusal test: a_database_the_job_cannot_reach_ends_it_at_its_deadline_with_three_or_at_an_interrupt_with_four
                    return Err(deadline_unreached(&detail));
                }
                tracing::warn!(%detail, "the database of the automatic handover cannot be reached; trying again");
                pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                if interrupt.is_raised() {
                    // refusal test: a_database_the_job_cannot_reach_ends_it_at_its_deadline_with_three_or_at_an_interrupt_with_four
                    return Err(interrupted());
                }
            }
            Err(error) => return Err(database(&error)),
        }
    };
    let result = if request.dry_run {
        dry_run(&pool).await
    } else {
        drive(&pool, request, pace, interrupt, &mut clock).await
    };
    pool.close().await;
    result
}

fn deadline_unreached(detail: &str) -> RunError {
    RunError::Deadline(format!(
        "the automatic handover waited its whole --wait and could not reach the database ({}). Nothing was changed.",
        shown(detail)
    ))
}

/// The job's deadline on its own clock: from its start plus `--wait` until a round gives the run's (kept by the database from the
/// run's first round, so that a retried pod keeps it), then that one.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    at: tokio::time::Instant,
}

impl Deadline {
    #[must_use]
    pub fn from_wait(wait: Duration) -> Self {
        Self {
            at: tokio::time::Instant::now() + wait,
        }
    }

    /// The run's deadline as a round reports it, on the database's clock.
    pub fn follow(&mut self, round: &Round) {
        let left = Duration::try_from(round.deadline - round.now).unwrap_or(Duration::ZERO);
        self.at = tokio::time::Instant::now() + left;
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        tokio::time::Instant::now() >= self.at
    }

    /// `wanted`, but never past the deadline by more than a second (every wait goes through this).
    #[must_use]
    pub fn clamp(&self, wanted: Duration) -> Duration {
        let left = self
            .at
            .saturating_duration_since(tokio::time::Instant::now())
            + Duration::from_secs(1);
        wanted.min(left)
    }
}

async fn dry_run(pool: &PgPool) -> Result<AutoOutcome, RunError> {
    if !may_auto_hand_over(pool)
        .await
        .map_err(|error| database(&error))?
    {
        // refusal test: the_dry_run_checks_the_wrappers_privilege_and_says_the_last_reason
        return Err(denied());
    }
    let (state, mode, _) = read_handover_mode(pool)
        .await
        .map_err(|error| database(&error))?;
    if state == "HANDED_OVER" {
        return Ok(AutoOutcome::AlreadyHandedOver {
            mode,
            audited: 0,
            audit_pending: false,
            retired: None,
        });
    }
    let last = last_attempt(pool).await.map_err(|error| database(&error))?;
    Ok(AutoOutcome::WouldWait {
        last_reason: last.and_then(|(reason, _)| reason),
    })
}

/// Writes a reason only the job can see; a failure to write it is logged and changes nothing of what the job does.
async fn note(pool: &PgPool, ask: &AutoAsk, reason: HandoverBlock) {
    if let Err(error) = handover_note(pool, ask, reason).await {
        tracing::warn!(%error, reason = reason.code(), "the reason of the automatic handover could not be written");
    }
}

/// Statement timeouts in a row of one kind of call (the rounds, or the activation: correctness review Major 2): `BUSY` until
/// [`TOO_SLOW_AFTER`], then `CHECK_TOO_SLOW`. Reset only by a call of the same kind that answered.
#[derive(Debug, Clone, Copy, Default)]
struct Timeouts(u32);

impl Timeouts {
    fn one_more(&mut self) -> HandoverBlock {
        self.0 += 1;
        if self.0 >= TOO_SLOW_AFTER {
            HandoverBlock::CheckTooSlow
        } else {
            HandoverBlock::Busy
        }
    }

    fn answered(&mut self) {
        self.0 = 0;
    }
}

async fn drive(
    pool: &PgPool,
    request: &AutoRequest,
    pace: &dyn Pace,
    interrupt: &Interrupt,
    clock: &mut Deadline,
) -> Result<AutoOutcome, RunError> {
    let ask = request.ask;
    let mut backoff = Backoff::default();
    let mut round_timeouts = Timeouts::default();
    let mut activation_timeouts = Timeouts::default();
    let mut last: Option<HandoverBlock> = None;
    tracing::info!(run = %ask.run, operator = request.operator.as_deref().map(shown).as_deref().unwrap_or("not given"),
        "the automatic handover starts its rounds");
    loop {
        if interrupt.is_raised() {
            note(pool, &ask, HandoverBlock::Interrupted).await;
            // refusal test: a_fleet_with_no_backlog_waits_a_horizon_after_the_first_round_and_an_interrupt_ends_it_with_nothing_changed
            return Err(interrupted());
        }
        if clock.passed() {
            note(pool, &ask, HandoverBlock::Deadline).await;
            // refusal test: the_job_ends_at_its_deadline_with_exit_three_even_when_every_round_fails
            return Err(deadline(last));
        }
        let round = match handover_round(pool, &ask).await {
            Ok(round) => {
                round_timeouts.answered();
                clock.follow(&round);
                round
            }
            Err(HandoverDbError::StatementTimeout) => {
                let reason = round_timeouts.one_more();
                note(pool, &ask, reason).await;
                last = Some(reason);
                pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                continue;
            }
            Err(HandoverDbError::LockTimeout) => {
                note(pool, &ask, HandoverBlock::Busy).await;
                last = Some(HandoverBlock::Busy);
                pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                continue;
            }
            Err(HandoverDbError::Unavailable(detail)) => {
                tracing::warn!(%detail, "the database of the automatic handover cannot be reached; trying again");
                pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                continue;
            }
            Err(HandoverDbError::TooManyRuns) => {
                // refusal test: a_seventeenth_live_run_is_refused_and_a_stopped_run_gives_way
                return Err(declined(
                    "TOO_MANY_RUNS: sixteen runs of the automatic handover are before their deadline; another job is running. \
                     Nothing was changed.",
                ));
            }
            // refusal test: a_role_other_than_sagat_handover_is_refused_at_the_round_or_at_the_activation_with_exit_two
            Err(HandoverDbError::Denied) => return Err(denied()),
            Err(error) => return Err(database(&error)),
        };
        match round.outcome {
            RoundOutcome::HandedOver => return finish(pool, request, None).await,
            RoundOutcome::UnknownState => {
                // refusal test: a_state_this_build_does_not_know_stops_the_job_at_its_activation_or_its_round_with_exit_two
                return Err(declined(
                    "the handover state is one this build does not know: run the handover with the build that wrote it",
                ));
            }
            // refusal test: a_run_whose_deadline_the_database_says_has_passed_ends_with_exit_three
            RoundOutcome::Deadline => return Err(deadline(round.reason.or(last))),
            RoundOutcome::Blocked | RoundOutcome::Counting => {
                if round.reason.is_some() {
                    last = round.reason;
                }
                backoff.reset();
                tracing::info!(run = %ask.run, reason = round.reason.map_or("counting", HandoverBlock::code),
                    stable = round.stable, wanted = round.wanted, live = round.live, examples = ?round.examples,
                    "a round of the automatic handover");
                pace.wait(clock.clamp(SPACING), interrupt).await;
            }
            RoundOutcome::Stable => {
                if interrupt.is_raised() {
                    note(pool, &ask, HandoverBlock::Interrupted).await;
                    // refusal test: an_interrupt_during_the_stable_round_starts_no_activation_and_exits_four
                    return Err(interrupted());
                }
                pace.before_activation().await;
                if interrupt.is_raised() {
                    note(pool, &ask, HandoverBlock::Interrupted).await;
                    // refusal test: an_interrupt_between_the_stable_round_and_the_activation_starts_no_activation_and_exits_four
                    return Err(interrupted());
                }
                // an activation begun is not cut by an interrupt: it commits or rolls back within its bounds
                match handover_activate(pool, ask.run, request.activation).await {
                    Ok(AutoActivated::HandedOver {
                        epoch,
                        live,
                        tenants,
                    }) => {
                        return finish(pool, request, Some((epoch, live, tenants))).await;
                    }
                    Ok(AutoActivated::Already { .. }) => return finish(pool, request, None).await,
                    Ok(AutoActivated::Refused(reason)) => {
                        // refused under the lock, nothing recorded: the next round sees it (an engine that left, a publication)
                        activation_timeouts.answered();
                        last = Some(reason);
                        pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                    }
                    Ok(AutoActivated::KindsDiffer) => {
                        // refusal test: schedules_v1_required_of_other_kinds_ends_the_job_with_exit_two
                        return Err(declined(
                            "schedules-v1 is already required of other kinds of instance; remove that requirement first. \
                             Nothing was changed.",
                        ));
                    }
                    Ok(AutoActivated::UnknownState) => {
                        // refusal test: a_state_this_build_does_not_know_stops_the_job_at_its_activation_or_its_round_with_exit_two
                        return Err(declined(
                            "the handover state is one this build does not know: run the handover with the build that wrote it",
                        ));
                    }
                    Err(HandoverDbError::LockTimeout) => {
                        note(pool, &ask, HandoverBlock::Busy).await;
                        last = Some(HandoverBlock::Busy);
                        pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                    }
                    // the step bound and the statement timeout end the same slow activation (both 3 s, and the client's clock
                    // fires first): three of either in a row are CHECK_TOO_SLOW (correctness review of 2026-11-15, m1)
                    Err(HandoverDbError::StatementTimeout | HandoverDbError::StepBound) => {
                        let reason = activation_timeouts.one_more();
                        note(pool, &ask, reason).await;
                        last = Some(reason);
                        pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                    }
                    Err(HandoverDbError::Unavailable(detail)) => {
                        tracing::warn!(%detail, "the activation lost its connection and rolled back; trying again");
                        pace.wait(clock.clamp(backoff.next_wait()), interrupt).await;
                    }
                    // refusal test: a_role_other_than_sagat_handover_is_refused_at_the_round_or_at_the_activation_with_exit_two
                    Err(HandoverDbError::Denied) => return Err(denied()),
                    Err(error) => return Err(database(&error)),
                }
            }
        }
    }
}

/// After the commit (or on finding it handed over): the tenant audit from the recorded set, the log line, the retirement. The
/// handover is made, so nothing here fails the job: an audit that cannot be completed is a warning, and what is left is exported by
/// the engines and alerted on (correctness review); the next run completes it.
async fn finish(
    pool: &PgPool,
    request: &AutoRequest,
    made: Option<(i64, u32, u32)>,
) -> Result<AutoOutcome, RunError> {
    let audit = match complete_handover_audit(pool).await {
        Ok(audit) => Some(audit),
        Err(HandoverDbError::AuditConflict) => {
            tracing::warn!(
                "AUDIT_CONFLICT: an audit record exists under an id drawn for a tenant of the handover and differs from the \
                 handover's; the handover is made, the tenant audit is not complete (docs/runbooks/schedules.md, 6.3)"
            );
            None
        }
        Err(error) => {
            tracing::warn!(%error, "the tenant audit of the handover could not be completed; the next run completes it");
            None
        }
    };
    let (mode, epoch) = match &audit {
        Some(audit) => (audit.mode.clone(), audit.epoch),
        None => {
            let (_, mode, epoch) = read_handover_mode(pool)
                .await
                .map_err(|error| database(&error))?;
            (mode, epoch)
        }
    };
    tracing::info!(
        mode = mode.as_deref().unwrap_or("not recorded"),
        epoch = epoch.unwrap_or_default(),
        engines = made.map_or(0, |(_, live, _)| live),
        tenants = audit.as_ref().map_or(0, |audit| audit.tenants),
        run = %request.ask.run,
        "the cron triggers were handed over to the schedule pass"
    );
    let retired = if request.retire_role {
        match retire_handover_role(pool).await {
            Ok(Retirement::RecordedNotAltered) => {
                tracing::warn!(
                    "the retirement of sagat_handover was recorded but the database refused to take its login away (42501): run \
                     ALTER ROLE sagat_handover NOLOGIN PASSWORD NULL as the role's creator"
                );
                Some(Retirement::RecordedNotAltered)
            }
            Ok(Retirement::AuditPending) => {
                tracing::warn!(
                    "sagat_handover is not retired: the tenant audit of the handover is not complete"
                );
                Some(Retirement::AuditPending)
            }
            Ok(other) => Some(other),
            Err(error) => {
                tracing::warn!(%error, "sagat_handover could not be retired: run ALTER ROLE sagat_handover NOLOGIN PASSWORD NULL");
                None
            }
        }
    } else {
        None
    };
    let audited = audit.as_ref().map_or(0, |audit| audit.written);
    let audit_pending = audit.is_none();
    Ok(match made {
        Some((epoch, live, tenants)) => AutoOutcome::HandedOver {
            epoch,
            live,
            tenants,
            audited,
            audit_pending,
            retired,
        },
        None => AutoOutcome::AlreadyHandedOver {
            mode,
            audited,
            audit_pending,
            retired,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_starts_at_a_heartbeat_doubles_and_stops_at_five_minutes() {
        let mut backoff = Backoff::default();
        let waits: Vec<u64> = (0..8).map(|_| backoff.next_wait().as_secs()).collect();
        assert_eq!(waits, [15, 30, 60, 120, 240, 300, 300, 300]);
        backoff.reset();
        assert_eq!(backoff.next_wait(), BACKOFF_FIRST);
    }

    #[test]
    fn each_refusal_has_its_exit_code() {
        assert_eq!(denied().exit_code(), 2);
        assert_eq!(declined("x").exit_code(), 2);
        assert_eq!(
            deadline(Some(HandoverBlock::EngineUnregistered)).exit_code(),
            3
        );
        assert_eq!(interrupted().exit_code(), 4);
        assert_eq!(
            database(&HandoverDbError::Other("x".to_owned())).exit_code(),
            1
        );
    }

    /// G2307, ADR-0091 decision 5: each call site of a refusal, in the operator's command and in the job, names on the line above it
    /// the test that reaches that site and holds its exit code (`// refusal test: NAME`), or says why no honest test can
    /// (`// refusal test: none — why`), and the named test exists. A new site without a marker fails here, and so does a marker whose
    /// test was renamed or removed.
    #[test]
    fn every_refusal_call_site_names_a_test_that_exists() {
        const CALLS: [&str; 5] = [
            "declined(",
            "denied()",
            "deadline(",
            "deadline_unreached(",
            "interrupted()",
        ];
        const MARKER: &str = "// refusal test: ";
        let sources = [
            ("handover.rs", include_str!("handover.rs")),
            ("handover_auto.rs", include_str!("handover_auto.rs")),
        ];
        let mut sites = 0;
        let mut named: Vec<(&str, usize, String)> = Vec::new();
        for (file, text) in sources {
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            let lines: Vec<&str> = code.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                let trimmed = line.trim_start();
                let a_site = !trimmed.starts_with("//")
                    && !trimmed.split_whitespace().any(|word| word == "fn")
                    && CALLS.iter().any(|call| line.contains(call));
                if !a_site {
                    continue;
                }
                sites += 1;
                let above = index.checked_sub(1).map_or("", |i| lines[i].trim());
                let Some(name) = above.strip_prefix(MARKER) else {
                    panic!(
                        "{file}:{}: a refusal without `{MARKER}NAME` on the line above it: {}",
                        index + 1,
                        line.trim()
                    );
                };
                if let Some(why) = name.strip_prefix("none \u{2014} ") {
                    assert!(
                        !why.trim().is_empty(),
                        "{file}:{}: none, and why?",
                        index + 1
                    );
                    continue;
                }
                assert!(
                    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "{file}:{}: `{name}` is not the name of a test",
                    index + 1
                );
                named.push((file, index + 1, name.to_owned()));
            }
        }
        assert_eq!(
            sites, 32,
            "the refusal call sites of handover.rs and handover_auto.rs: a new one needs its marker and its test"
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut corpus = String::new();
        let mut pending = vec![root.join("src"), root.join("../../tests")];
        while let Some(dir) = pending.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
            for entry in entries {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    corpus.push_str(
                        &std::fs::read_to_string(&path)
                            .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
                    );
                    corpus.push('\n');
                }
            }
        }
        for (file, line, name) in named {
            assert!(
                corpus.contains(&format!("fn {name}(")),
                "{file}:{line}: no test `{name}` in apps/control-plane/src or tests"
            );
        }
    }

    #[test]
    fn the_deadline_names_the_last_reason_and_its_runbook_row() {
        let said = deadline(Some(HandoverBlock::ReconciliationRecent)).to_string();
        assert!(said.contains("RECONCILIATION_RECENT"), "{said}");
        assert!(
            said.contains("#handover-reason-reconciliation-recent"),
            "{said}"
        );
        assert!(deadline(None).to_string().contains("none recorded"));
    }

    #[test]
    fn the_run_id_is_the_flag_or_the_variable_and_never_two_that_differ() {
        let flag = Uuid::from_u128(7);
        let other = Uuid::from_u128(8);
        let none = |_: &str| None;
        let variable = |text: &'static str| {
            move |name: &str| (name == "SAGAT_HANDOVER_RUN_ID").then(|| text.to_owned())
        };
        assert_eq!(run_id(Some(flag), &none).expect("the flag"), flag);
        assert_eq!(
            run_id(None, &variable("00000000-0000-0000-0000-000000000008")).expect("the variable"),
            other
        );
        assert_eq!(
            run_id(
                Some(flag),
                &variable("00000000-0000-0000-0000-000000000007")
            )
            .expect("the same twice"),
            flag
        );
        let differ = run_id(
            Some(flag),
            &variable("00000000-0000-0000-0000-000000000008"),
        )
        .expect_err("two run ids");
        assert_eq!(differ.exit_code(), 2);
        assert!(differ.to_string().contains("--run-id"), "{differ}");
        let bad = run_id(None, &variable("job-7")).expect_err("not a uuid");
        assert_eq!(bad.exit_code(), 2);
        assert!(bad.to_string().contains("SAGAT_HANDOVER_RUN_ID"), "{bad}");
        let drawn = run_id(None, &variable("")).expect("drawn");
        assert_ne!(
            drawn,
            run_id(None, &none).expect("drawn again"),
            "drawn at start, new each time"
        );
    }

    #[tokio::test]
    async fn an_interrupt_wakes_a_wait_at_once() {
        let interrupt = Interrupt::default();
        let raiser = interrupt.clone();
        let waiting = tokio::spawn(async move {
            Sleep.wait(Duration::from_secs(3600), &interrupt).await;
        });
        raiser.raise();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("woken")
            .expect("joined");
    }
}
