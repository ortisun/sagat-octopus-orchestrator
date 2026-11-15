//! `control-plane schedule-trigger-handover --auto` (feature 018, `T018-084`, ADR-0091): the deployment's one-shot job that hands
//! the cron triggers over once the fleet is stable and converged.
//!
//! It runs as `sagat_handover`, which can call four functions and read two tables. **The database decides**: every round goes
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
    connect_handover, handover_activate, handover_round, last_attempt, may_auto_hand_over,
    read_handover_mode, retire_handover_role,
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
pub fn raise_on_signals(interrupt: &Interrupt) {
    let interrupt = interrupt.clone();
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

/// What the job was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoRequest {
    pub run: Uuid,
    pub stable: u32,
    pub horizon: Duration,
    pub min_live: u32,
    pub wait: Duration,
    pub retire_role: bool,
    pub dry_run: bool,
    /// Who the deployment says runs it (the release), for the log line only: the actor recorded is the run's.
    pub operator: Option<String>,
}

impl AutoRequest {
    /// The request of a command line, with the run id given.
    #[must_use]
    pub fn of(args: &HandoverArgs, run: Uuid) -> Self {
        Self {
            run,
            stable: args.stable.unwrap_or(auto_bounds::STABLE),
            horizon: args.horizon.unwrap_or(auto_bounds::HORIZON),
            min_live: args.min_live.unwrap_or(auto_bounds::MIN_LIVE),
            wait: args.wait.unwrap_or(auto_bounds::WAIT),
            retire_role: args.retire_role,
            dry_run: args.dry_run,
            operator: args.operator.clone(),
        }
    }

    fn ask(&self) -> AutoAsk {
        AutoAsk {
            run: self.run,
            stable: self.stable,
            horizon: self.horizon,
            min_live: self.min_live,
            wait: self.wait,
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
        retired: Option<Retirement>,
    },
    /// It had been handed over before; the audit was completed if it was not.
    AlreadyHandedOver {
        mode: Option<String>,
        audited: u32,
        retired: Option<Retirement>,
    },
    /// `SAGAT_SCHEDULES_AUTO_HANDOVER=false`.
    SwitchedOff,
    /// With `--retire-role`, the role cannot log in: it was retired, or `pg_hba` refuses it (the engines alert on that).
    RoleRetired,
    /// The dry run: the role may make it; the last reason recorded, if any.
    WouldWait { last_reason: Option<String> },
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
                retired,
            } => write!(
                f,
                "handed over automatically: the cron triggers fire from the schedule pass now ({live} live execution engine(s), \
                 requirement epoch {epoch}, {tenants} tenant(s) affected, {audited} audit record(s) written). There is no \
                 command to go back.{}",
                retired_words(*retired)
            ),
            Self::AlreadyHandedOver {
                mode,
                audited,
                retired,
            } => write!(
                f,
                "already handed over (mode {}); {audited} audit record(s) completed now; nothing else was changed.{}",
                shown(mode.as_deref().unwrap_or("not recorded")),
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
    let Some(password) = env("SAGAT_HANDOVER_PASSWORD").filter(|password| !password.is_empty())
    else {
        return Err(declined(
            "SAGAT_HANDOVER_PASSWORD is not set: the automatic handover logs in as sagat_handover with that password, given apart \
             from DATABASE_URL. Set it (and give it to migrate, which sets the role's login), or switch the job off with \
             SAGAT_SCHEDULES_AUTO_HANDOVER=false. Nothing was changed.",
        ));
    };
    let run = match env("SAGAT_HANDOVER_RUN_ID").filter(|id| !id.is_empty()) {
        Some(id) => Uuid::parse_str(id.trim()).map_err(|_| ConfigError::Invalid {
            name: "SAGAT_HANDOVER_RUN_ID",
            reason: "must be a uuid (in Kubernetes, the Job's UID from the downward API)"
                .to_owned(),
        })?,
        None => Uuid::new_v4(),
    };
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
    auto_handover(db, Some(&password), &request, &Sleep, interrupt)
        .await
        .map(|outcome| outcome.to_string())
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
    let pool = match connect_handover(&config, password).await {
        Ok(pool) => pool,
        Err(HandoverDbError::CannotLogIn) if request.retire_role => {
            return Ok(AutoOutcome::RoleRetired);
        }
        Err(HandoverDbError::CannotLogIn) => {
            return Err(declined(
                "sagat_handover cannot log in (SQLSTATE 28000 or 28P01): give migrate SAGAT_HANDOVER_PASSWORD so that it sets the \
                 login, check pg_hba, or create the login once (docs/runbooks/schedules.md, 6.1). Nothing was changed.",
            ));
        }
        Err(error) => return Err(database(&error)),
    };
    let result = if request.dry_run {
        dry_run(&pool).await
    } else {
        drive(&pool, request, pace, interrupt).await
    };
    pool.close().await;
    result
}

async fn dry_run(pool: &PgPool) -> Result<AutoOutcome, RunError> {
    if !may_auto_hand_over(pool)
        .await
        .map_err(|error| database(&error))?
    {
        return Err(denied());
    }
    let (state, mode, _) = read_handover_mode(pool)
        .await
        .map_err(|error| database(&error))?;
    if state == "HANDED_OVER" {
        return Ok(AutoOutcome::AlreadyHandedOver {
            mode,
            audited: 0,
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
    if let Err(error) = handover_round(pool, ask, Some(reason)).await {
        tracing::warn!(%error, reason = reason.code(), "the reason of the automatic handover could not be written");
    }
}

/// The wait before the next round: `wanted`, but never past the run's deadline by more than a second.
fn until_next(wanted: Duration, round: &Round) -> Duration {
    let left = round.deadline - round.now;
    let left = Duration::try_from(left).unwrap_or(Duration::ZERO) + Duration::from_secs(1);
    wanted.min(left)
}

async fn drive(
    pool: &PgPool,
    request: &AutoRequest,
    pace: &dyn Pace,
    interrupt: &Interrupt,
) -> Result<AutoOutcome, RunError> {
    let ask = request.ask();
    let mut backoff = Backoff::default();
    let mut timeouts = 0_u32;
    let mut last: Option<HandoverBlock> = None;
    tracing::info!(run = %request.run, operator = request.operator.as_deref().map(shown).as_deref().unwrap_or("not given"),
        "the automatic handover starts its rounds");
    loop {
        if interrupt.is_raised() {
            note(pool, &ask, HandoverBlock::Interrupted).await;
            return Err(interrupted());
        }
        let round = match handover_round(pool, &ask, None).await {
            Ok(round) => {
                timeouts = 0;
                round
            }
            Err(HandoverDbError::StatementTimeout) => {
                timeouts += 1;
                let reason = if timeouts >= TOO_SLOW_AFTER {
                    HandoverBlock::CheckTooSlow
                } else {
                    HandoverBlock::Busy
                };
                note(pool, &ask, reason).await;
                last = Some(reason);
                pace.wait(backoff.next_wait(), interrupt).await;
                continue;
            }
            Err(HandoverDbError::LockTimeout) => {
                note(pool, &ask, HandoverBlock::Busy).await;
                last = Some(HandoverBlock::Busy);
                pace.wait(backoff.next_wait(), interrupt).await;
                continue;
            }
            Err(HandoverDbError::Denied) => return Err(denied()),
            Err(error) => return Err(database(&error)),
        };
        match round.outcome {
            RoundOutcome::HandedOver => return finish(pool, request, None).await,
            RoundOutcome::UnknownState => {
                return Err(declined(
                    "the handover state is one this build does not know: run the handover with the build that wrote it",
                ));
            }
            RoundOutcome::Deadline => {
                return Err(deadline(round.reason.or(last)));
            }
            RoundOutcome::Blocked | RoundOutcome::Counting | RoundOutcome::Noted => {
                if round.reason.is_some() {
                    last = round.reason;
                }
                backoff.reset();
                tracing::info!(run = %request.run, reason = round.reason.map_or("counting", HandoverBlock::code),
                    stable = round.stable, wanted = round.wanted, live = round.live, "a round of the automatic handover");
                let wait = until_next(SPACING, &round);
                pace.wait(wait, interrupt).await;
            }
            RoundOutcome::Stable => {
                if interrupt.is_raised() {
                    note(pool, &ask, HandoverBlock::Interrupted).await;
                    return Err(interrupted());
                }
                pace.before_activation().await;
                if interrupt.is_raised() {
                    note(pool, &ask, HandoverBlock::Interrupted).await;
                    return Err(interrupted());
                }
                match handover_activate(pool, request.run, OperatorBounds::default()).await {
                    Ok(AutoActivated::HandedOver {
                        epoch,
                        live,
                        tenants,
                    }) => return finish(pool, request, Some((epoch, live, tenants))).await,
                    Ok(AutoActivated::Already { .. }) => return finish(pool, request, None).await,
                    Ok(AutoActivated::Refused(reason)) => {
                        // refused under the lock, nothing recorded: the next round sees it (an engine that left, a publication)
                        last = Some(reason);
                        let wait = until_next(backoff.next_wait(), &round);
                        pace.wait(wait, interrupt).await;
                    }
                    Ok(AutoActivated::KindsDiffer) => {
                        return Err(declined(
                            "schedules-v1 is already required of other kinds of instance; remove that requirement first. \
                             Nothing was changed.",
                        ));
                    }
                    Ok(AutoActivated::UnknownState) => {
                        return Err(declined(
                            "the handover state is one this build does not know: run the handover with the build that wrote it",
                        ));
                    }
                    Err(HandoverDbError::LockTimeout | HandoverDbError::StepBound) => {
                        note(pool, &ask, HandoverBlock::Busy).await;
                        last = Some(HandoverBlock::Busy);
                        pace.wait(backoff.next_wait(), interrupt).await;
                    }
                    Err(HandoverDbError::StatementTimeout) => {
                        timeouts += 1;
                        let reason = if timeouts >= TOO_SLOW_AFTER {
                            HandoverBlock::CheckTooSlow
                        } else {
                            HandoverBlock::Busy
                        };
                        note(pool, &ask, reason).await;
                        last = Some(reason);
                        pace.wait(backoff.next_wait(), interrupt).await;
                    }
                    Err(HandoverDbError::Denied) => return Err(denied()),
                    Err(error) => return Err(database(&error)),
                }
            }
        }
    }
}

/// After the commit (or on finding it handed over): the tenant audit from the recorded set, the log line, the retirement.
async fn finish(
    pool: &PgPool,
    request: &AutoRequest,
    made: Option<(i64, u32, u32)>,
) -> Result<AutoOutcome, RunError> {
    let audit = match complete_handover_audit(pool).await {
        Ok(audit) => audit,
        Err(HandoverDbError::AuditConflict) => {
            return Err(declined(
                "AUDIT_CONFLICT: an audit record exists under an id drawn for a tenant of the handover and differs from the \
                 handover's: it was not written by the handover. The handover itself is made; look at the record and at who \
                 could write audit_records (docs/runbooks/schedules.md, 6.3).",
            ));
        }
        Err(HandoverDbError::Denied) => return Err(denied()),
        Err(error) => return Err(database(&error)),
    };
    let mode = audit.mode.clone();
    tracing::info!(
        mode = mode.as_deref().unwrap_or("not recorded"),
        epoch = audit.epoch.unwrap_or_default(),
        engines = made.map_or(0, |(_, live, _)| live),
        tenants = audit.tenants,
        run = %request.run,
        "the cron triggers were handed over to the schedule pass"
    );
    let retired = if request.retire_role {
        match retire_handover_role(pool).await {
            Ok(Retirement::RecordedNotAltered) => {
                tracing::warn!(
                    "the retirement of sagat_handover was recorded but the database refused to take its login away (42501): run \
                     ALTER ROLE sagat_handover NOLOGIN as the role's creator"
                );
                Some(Retirement::RecordedNotAltered)
            }
            Ok(other) => Some(other),
            Err(error) => {
                tracing::warn!(%error, "sagat_handover could not be retired: run ALTER ROLE sagat_handover NOLOGIN");
                None
            }
        }
    } else {
        None
    };
    Ok(match made {
        Some((epoch, live, tenants)) => AutoOutcome::HandedOver {
            epoch,
            live,
            tenants,
            audited: audit.written,
            retired,
        },
        None => AutoOutcome::AlreadyHandedOver {
            mode,
            audited: audit.written,
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
