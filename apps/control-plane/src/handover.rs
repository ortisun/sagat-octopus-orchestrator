//! `control-plane schedule-trigger-handover` (feature 018, `T018-061`, `FR-018-017`, `FR-018-052`, G2002): the audited
//! operator command that hands the cron triggers over from the legacy cron pass to the schedule pass.
//!
//! It is the first activation of the instance-capability registry: in one transaction it requires `schedules-v1` of every
//! live execution engine (refusing, by name, the engines that do not list it) and moves
//! `schedule_trigger_handover` from `LEGACY` to `HANDED_OVER`, recording the operator's name as the command line gave it
//! and the database role of the session, on the database's clock. The engines then stop their legacy cron pass within
//! about two seconds (a handover is read at most that often), and trigger-owned schedules accept pause, resume, trigger
//! and backfill.
//!
//! It runs with the schema-owning role, as `provision-tenant` does: the functions it calls are not executable by the
//! runtime role. It is idempotent (a second run says it is already handed over and exits 0), has a dry run that verifies
//! everything and changes nothing, and takes the number of live engines the operator expects, because an activation that
//! finds none would pass vacuously and leave a platform whose triggers nothing fires.

use std::fmt;

use sagat_application::error::StoreError;
use sagat_application::instances::{
    Activated, Activation, Capability, ChangeRefused, InstanceKind,
};
use sagat_application::schedule_triggers::HandoverState;
use sagat_domain::identity::Subject;
use sagat_infra_postgres::{
    Census, PgInstanceStore, engine_census, handover_step, may_hand_over, read_handover_row,
};

use crate::config::{ConfigError, DatabaseConfig};
use crate::{RunError, open_pool};

/// The request a command line makes: the handover has no way back, so it is made only when `--apply` is given; without it (and
/// without `--dry-run`) the command verifies and says so (D2052).
#[must_use]
pub fn request_of(args: &crate::cli::HandoverArgs) -> HandoverRequest {
    HandoverRequest {
        operator: args.operator.clone(),
        expect_live: args.expect_live,
        dry_run: args.dry_run || !args.apply,
    }
}

/// The command as `main` runs it, and the words it prints: what [`schedule_trigger_handover`] did, and, when neither `--apply` nor
/// `--dry-run` was given and the answer is "nothing stops it", that nothing was done because `--apply` was not given. The
/// sentence is the library's, so that the test of the `--apply` path is the test of what the binary prints (D2077).
///
/// # Errors
/// As [`schedule_trigger_handover`].
pub async fn run_handover_command(
    db: &DatabaseConfig,
    args: &crate::cli::HandoverArgs,
) -> Result<String, RunError> {
    let outcome = schedule_trigger_handover(db, &request_of(args)).await?;
    let mut text = outcome.to_string();
    if matches!(outcome, HandoverOutcome::WouldHandOver { .. }) && !args.dry_run && !args.apply {
        text.push_str(
            "\nThis was a dry run: --apply was not given. Run it again with --apply to hand the cron triggers over.",
        );
    }
    Ok(text)
}

/// What the operator asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoverRequest {
    /// Who is doing it, as the command line says (`--operator`); recorded with the database role of the session.
    pub operator: Option<String>,
    /// How many live execution engines the operator expects (`--expect-live`).
    pub expect_live: Option<u32>,
    /// Verify everything, change nothing (`--dry-run`).
    pub dry_run: bool,
}

/// What the command did, in words for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoverOutcome {
    /// The state moved to `HANDED_OVER` in this run.
    HandedOver { live: u32, epoch: i64 },
    /// It had been handed over before; nothing was done.
    AlreadyHandedOver {
        by: Option<String>,
        db_role: Option<String>,
    },
    /// A dry run found nothing that would stop the handover.
    WouldHandOver { live: u32 },
}

impl fmt::Display for HandoverOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandedOver { live, epoch } => write!(
                f,
                "handed over: the cron triggers fire from the schedule pass now ({live} live execution engine(s) \
                 list schedules-v1, requirement epoch {epoch}). The legacy cron pass of every engine stops within \
                 about two seconds. There is no command to go back: read the rollback section of docs/runbooks/schedules.md."
            ),
            Self::AlreadyHandedOver { by, db_role } => write!(
                f,
                "already handed over (by {} as database role {}); nothing was changed",
                shown(by.as_deref().unwrap_or("an operator that was not recorded")),
                shown(db_role.as_deref().unwrap_or("not recorded"))
            ),
            Self::WouldHandOver { live } => write!(
                f,
                "dry run: nothing stops the handover ({live} live execution engine(s), all listing schedules-v1, and this \
                 database role may run it). Nothing was changed."
            ),
        }
    }
}

/// Whether a character could make a record read as somebody else, or as nothing, wherever it is shown: a control character, a
/// bidirectional override or isolate, or a zero-width one.
fn unprintable_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// A text from the database or the environment as it is printed: every character that could rewrite the line (an escape
/// sequence, a newline, a right-to-left override) is shown as `?`. The record of a handover written before the database refused
/// such names, a host name, a user name: none of them is trusted to be printable (D2076).
#[must_use]
pub fn shown(text: &str) -> String {
    text.chars()
        .map(|c| if unprintable_char(c) { '?' } else { c })
        .collect()
}

/// Why the operator is told no, as a sentence that says what to do next.
fn declined(message: impl Into<String>) -> RunError {
    RunError::Declined(message.into())
}

fn store_failure(error: &StoreError) -> RunError {
    let text = error.to_string();
    if text.contains("permission denied") {
        declined(
            "permission denied: the handover calls functions the runtime role may not execute. Run it with the \
             schema-owning database role (the one that runs `migrate` and `provision-tenant`), not the one the API and the \
             engines use.",
        )
    } else {
        RunError::Database(text)
    }
}

fn lacking(missing: u32, examples: &[uuid::Uuid]) -> RunError {
    let names = examples
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    declined(format!(
        "{missing} live execution engine(s) do not list schedules-v1 (instance ids: {names}). An engine lists it only when it \
         runs the schedule pass: start it with SAGAT_SCHEDULES_ENABLED=true (or stop it), wait for it to register, and run \
         the command again. Nothing was changed."
    ))
}

/// The count of live engines against what the operator expects. Without `--expect-live` none live is a refusal: an
/// activation that finds no engine verifies nothing.
fn check_live(live: u32, expect: Option<u32>) -> Result<(), RunError> {
    match expect {
        Some(expected) if live != expected => Err(declined(format!(
            "{live} execution engine(s) are live (seen within 30 seconds) and --expect-live says {expected}. If fewer are \
             live, an engine has not registered yet or is down; if more, one is not yours or is not stopped. Nothing was \
             changed."
        ))),
        None if live == 0 => Err(declined(
            "no execution engine is live (seen within 30 seconds), so there is nothing to verify and the handover would \
             leave the cron triggers with nothing that fires them if the engines are started without the schedule pass. \
             Start the engines (with SAGAT_SCHEDULES_ENABLED=true) and run the command again, or pass --expect-live 0 if \
             that is what you mean. Nothing was changed.",
        )),
        _ => Ok(()),
    }
}

/// Whether the live engines counted under the lock of the epoch are not what the first count and the operator's expectation
/// said: an engine registered or went silent between the check and the lock, so nothing is recorded.
const fn live_changed(live: u32, expect: Option<u32>) -> bool {
    match expect {
        Some(expected) => live != expected,
        None => live == 0,
    }
}

/// Why an operator name cannot be recorded as typed: it must be printable. A control character, a bidirectional override or a
/// zero-width character could make the record read as somebody else, or as nothing, wherever it is shown (D2052).
#[must_use]
pub fn unprintable(name: &str) -> Option<&'static str> {
    if name.starts_with('-') {
        return Some("a name does not start with `-`: that is an option");
    }
    if name.trim() != name {
        return Some("a name has no space at its ends");
    }
    if name.chars().any(unprintable_char) {
        return Some("a name has no control, bidirectional or zero-width character");
    }
    None
}

/// Hands the cron triggers over to the schedule pass.
///
/// # Errors
/// [`RunError::Declined`] when the operator is told no (the engines that lack `schedules-v1`, a count that is not the
/// one expected, a role that may not run it), with the sentence that says what to do; [`RunError::Config`] for an
/// operator name that is not a subject; [`RunError::Database`] otherwise.
pub async fn schedule_trigger_handover(
    db: &DatabaseConfig,
    request: &HandoverRequest,
) -> Result<HandoverOutcome, RunError> {
    let operator = match (&request.operator, request.dry_run) {
        (Some(name), _) => {
            if let Some(reason) = unprintable(name) {
                return Err(ConfigError::Invalid {
                    name: "--operator",
                    reason: reason.to_owned(),
                }
                .into());
            }
            Some(Subject::parse(name).map_err(|error| ConfigError::Invalid {
                name: "--operator",
                reason: error.to_string(),
            })?)
        }
        (None, true) => None,
        (None, false) => {
            return Err(declined(
                "--operator NAME is required: the handover is audited, and it records who made it",
            ));
        }
    };
    let pool = open_pool(db, db.statement_timeout).await?;
    let result = run(&pool, request, operator.as_ref()).await;
    pool.close().await;
    result
}

/// What the operator is told when the cron triggers have not converged: nothing would fire them once the legacy pass stops.
fn unconverged(count: u32, examples: &[uuid::Uuid]) -> RunError {
    let names = examples
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    declined(format!(
        "{count} cron trigger(s) have no schedule or no next time, or have a schedule that was paused before the handover \
         (trigger ids: {names}). The legacy cron pass stops at the \
         handover, so nothing would fire them. The reconciliation makes them within seconds in every engine that runs the \
         schedule pass: wait and run the command again; if the count does not reach zero look in the engines' logs for the \
         trigger ids (an expression or zone this build cannot read, or every name taken). Nothing was changed."
    ))
}

/// The count and the ids of the cron triggers that have no schedule equal to them, read now, outside the lock: the step found
/// that there was one and rolled back.
async fn unconverged_now(pool: &sagat_infra_postgres::PgPool) -> RunError {
    let Ok(mut conn) = pool.acquire().await else {
        return declined(
            "a cron trigger has no schedule equal to it, and the details could not be read. Nothing was changed; run the command with --dry-run.",
        );
    };
    match engine_census(&mut conn, false).await {
        Ok(census) if census.unconverged > 0 => {
            unconverged(census.unconverged, &census.unconverged_examples)
        }
        Ok(_) => declined(
            "a cron trigger had no schedule equal to it when the handover counted, and has one now. Nothing was changed; run the command again.",
        ),
        Err(error) => store_failure(&error),
    }
}

/// The same check inside the activation's transaction. It records why in `why` (the numbers do not travel through a store
/// error) and refuses with nothing recorded.
type Why = std::sync::Arc<std::sync::Mutex<Option<Refusal>>>;

/// What the step found under the lock. Under the lock the check is whether a cron trigger has no schedule equal to it, and
/// nothing more, because publications wait while the lock is held; the count and the ids the operator is told are read after the
/// transaction ended (D2071).
enum Refusal {
    Said(RunError),
    Unconverged,
}

/// The live engines the step counted under the lock: what the command reports, and not the count of the first check.
type Seen = std::sync::Arc<std::sync::atomic::AtomicU32>;

/// What the step says of the census it took under the lock: whether to refuse, and why (kept in `why`, because the numbers do not
/// travel through a store error). The registry runs it with the requirement epoch held: it counts again under the lock (an engine
/// that registered or went silent since the first check, a trigger that has no schedule yet), and only then is the state moved.
fn judge(
    expect: Option<u32>,
    why: Why,
    seen: Seen,
) -> impl FnOnce(&Census) -> bool + Send + 'static {
    move |census| {
        seen.store(census.live, std::sync::atomic::Ordering::Relaxed);
        let refusal = if live_changed(census.live, expect) {
            Some(Refusal::Said(declined(
                "the number of live execution engines changed while the handover was being made. Nothing was changed; run the command again.",
            )))
        } else if census.lacking > 0 {
            Some(Refusal::Said(lacking(
                census.lacking,
                &census.lacking_examples,
            )))
        } else if census.unconverged > 0 {
            Some(Refusal::Unconverged)
        } else {
            None
        };
        let refused = refusal.is_some();
        if let Some(refusal) = refusal {
            *why.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(refusal);
        }
        refused
    }
}

/// The os user and host that ran the command, for the line the platform logs: the operator's name is what they typed and the
/// database role is shared by whoever has the owner's login (D2042).
fn who_ran_it() -> (String, String) {
    who_from(|name| std::env::var(name).ok())
}

/// The same, from any lookup: what the environment says is printed through [`shown`], and a name that is empty is `unknown`.
fn who_from(lookup: impl Fn(&str) -> Option<String>) -> (String, String) {
    let var = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| lookup(name).filter(|value| !value.is_empty()))
            .map_or_else(|| "unknown".to_owned(), |value| shown(&value))
    };
    (var(&["USER", "LOGNAME"]), var(&["HOSTNAME", "HOST"]))
}

async fn run(
    pool: &sagat_infra_postgres::PgPool,
    request: &HandoverRequest,
    operator: Option<&Subject>,
) -> Result<HandoverOutcome, RunError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| RunError::Database(error.to_string()))?;
    let (state, by, db_role) = read_handover_row(&mut conn)
        .await
        .map_err(|error| store_failure(&error))?;
    if state.is_handed_over() {
        return Ok(HandoverOutcome::AlreadyHandedOver { by, db_role });
    }
    if let HandoverState::Other(other) = &state {
        return Err(declined(format!(
            "the handover state is `{other}`, which this build does not know: run the handover with the build that wrote it"
        )));
    }
    let census = engine_census(&mut conn, false)
        .await
        .map_err(|error| store_failure(&error))?;
    check_live(census.live, request.expect_live)?;
    if census.lacking > 0 {
        return Err(lacking(census.lacking, &census.lacking_examples));
    }
    if census.unconverged > 0 {
        return Err(unconverged(
            census.unconverged,
            &census.unconverged_examples,
        ));
    }
    let live = census.live;
    if request.dry_run {
        if !may_hand_over(&mut conn)
            .await
            .map_err(|error| store_failure(&error))?
        {
            return Err(store_failure(&StoreError::Internal {
                detail: "permission denied for function platform_requirement_activate".to_owned(),
            }));
        }
        return Ok(HandoverOutcome::WouldHandOver { live });
    }
    drop(conn);
    let Some(operator) = operator else {
        return Err(declined("--operator NAME is required"));
    };
    let why: Why = Why::default();
    let seen: Seen = Seen::default();
    let store = PgInstanceStore::new(pool.clone());
    let activated = store
        .activate(
            Activation {
                capability: Capability::SchedulesV1,
                kinds: &[InstanceKind::ExecutionEngine],
                actor: operator,
            },
            handover_step(
                operator.as_str().to_owned(),
                judge(request.expect_live, Why::clone(&why), Seen::clone(&seen)),
            ),
        )
        .await;
    let (os_user, host) = who_ran_it();
    match activated {
        Ok(Activated::Recorded { epoch }) => {
            let live = seen.load(std::sync::atomic::Ordering::Relaxed);
            tracing::info!(operator = %operator, %os_user, %host, epoch, live, "the cron triggers were handed over to the schedule pass");
            Ok(HandoverOutcome::HandedOver { live, epoch })
        }
        Ok(Activated::AlreadyRecorded) => {
            // The requirement was recorded before. With the handover done there is nothing left to do. With the state still
            // `LEGACY` it was recorded by something other than this command: the same checks, and the handover itself, are made
            // now in one transaction under the epoch's lock, and a refusal changes nothing (D2051).
            let mut conn = pool
                .acquire()
                .await
                .map_err(|error| RunError::Database(error.to_string()))?;
            let (state, by, db_role) = read_handover_row(&mut conn)
                .await
                .map_err(|error| store_failure(&error))?;
            drop(conn);
            if state.is_handed_over() {
                return Ok(HandoverOutcome::AlreadyHandedOver { by, db_role });
            }
            let completed = store
                .run_locked(handover_step(
                    operator.as_str().to_owned(),
                    judge(request.expect_live, Why::clone(&why), Seen::clone(&seen)),
                ))
                .await;
            match completed {
                Ok(()) => {
                    let live = seen.load(std::sync::atomic::Ordering::Relaxed);
                    tracing::info!(operator = %operator, %os_user, %host, live, "the cron triggers were handed over to the schedule pass (the requirement had been recorded before)");
                    let epoch: i64 = sqlx_epoch(pool).await?;
                    Ok(HandoverOutcome::HandedOver { live, epoch })
                }
                Err(error) => Err(refused(error, &why, pool).await),
            }
        }
        Err(error) => Err(refused(error, &why, pool).await),
    }
}

async fn sqlx_epoch(pool: &sagat_infra_postgres::PgPool) -> Result<i64, RunError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| RunError::Database(error.to_string()))?;
    sagat_infra_postgres::read_epoch(&mut conn)
        .await
        .map_err(|error| store_failure(&error))
}

/// What a refused activation or completion is told to the operator as: what the step recorded under the lock, or the registry's
/// own refusal.
async fn refused(error: ChangeRefused, why: &Why, pool: &sagat_infra_postgres::PgPool) -> RunError {
    let found = why
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    match found {
        Some(Refusal::Said(reason)) => return reason,
        Some(Refusal::Unconverged) => return unconverged_now(pool).await,
        None => {}
    }
    match error {
        ChangeRefused::InstancesLackCapability { missing, examples } => lacking(missing, &examples),
        ChangeRefused::KindsDiffer => declined(
            "schedules-v1 is already required of other kinds of instance; remove that requirement first. Nothing was changed.",
        ),
        ChangeRefused::Store(StoreError::StateConflict(reason)) => declined(format!(
            "{reason}. Nothing was changed; run the command again."
        )),
        ChangeRefused::Store(error) => {
            let text = error.to_string();
            if text.contains("HANDOVER_BUSY") {
                declined(
                    "a publication, or a cron pass, was writing the cron triggers and the handover waited two seconds for it, \
                     which is as long as it holds them back. Nothing was changed; run the command again.",
                )
            } else if text.contains("HANDOVER_UNCONVERGED") {
                declined(format!(
                    "{text}. The legacy cron pass stops at the handover, so nothing would fire them; wait for the reconciliation and run the command again. Nothing was changed."
                ))
            } else {
                store_failure(&error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(error: RunError) -> String {
        error.to_string()
    }

    #[test]
    fn what_is_printed_of_a_stored_or_an_environment_text_cannot_rewrite_the_line() {
        // the record of a handover written before the database refused such names, a host name, a user name
        let hostile = "ana\u{1b}[2K\u{202E}evil\u{200B}\nroot\u{7}";
        let printed = shown(hostile);
        assert!(!printed.chars().any(unprintable_char), "{printed:?}");
        assert_eq!(
            printed, "ana?[2K?evil??root?",
            "each such character is shown as a question mark"
        );
        assert_eq!(
            shown("Jos\u{e9} M\u{fc}ller"),
            "Jos\u{e9} M\u{fc}ller",
            "letters of other alphabets are text"
        );
        let said = HandoverOutcome::AlreadyHandedOver {
            by: Some(hostile.to_owned()),
            db_role: Some("owner\u{202E}".to_owned()),
        }
        .to_string();
        assert!(!said.chars().any(unprintable_char), "{said:?}");
        assert!(said.contains("ana?[2K?evil??root?"), "{said}");
    }

    #[test]
    fn the_os_user_and_host_of_the_log_line_are_printed_through_the_same_filter() {
        let (user, host) = who_from(|name| match name {
            "USER" => Some("ana\u{202E}evil".to_owned()),
            "HOSTNAME" => Some("host\u{1b}[31m".to_owned()),
            _ => None,
        });
        assert_eq!((user.as_str(), host.as_str()), ("ana?evil", "host?[31m"));
        let (user, host) = who_from(|_| None);
        assert_eq!((user.as_str(), host.as_str()), ("unknown", "unknown"));
        let (user, _) = who_from(|name| (name == "LOGNAME").then(|| "ana".to_owned()));
        assert_eq!(user, "ana", "LOGNAME when USER is not set");
        let (user, _) = who_from(|name| (name == "USER").then(String::new));
        assert_eq!(user, "unknown", "an empty value says nothing");
    }

    #[test]
    fn the_number_of_live_engines_must_be_the_one_expected_and_none_is_never_vacuous() {
        assert!(check_live(3, Some(3)).is_ok());
        assert!(check_live(3, None).is_ok());
        assert!(
            check_live(0, Some(0)).is_ok(),
            "zero, when the operator says so"
        );
        let fewer = message(check_live(2, Some(3)).expect_err("fewer"));
        assert!(
            fewer.contains("2 execution engine(s) are live")
                && fewer.contains("--expect-live says 3")
        );
        let more = message(check_live(4, Some(3)).expect_err("more"));
        assert!(more.contains("4 execution engine(s) are live"));
        let none = message(check_live(0, None).expect_err("vacuous"));
        assert!(none.contains("no execution engine is live") && none.contains("--expect-live 0"));
    }

    #[test]
    fn the_count_under_the_lock_is_held_to_the_same_expectation() {
        assert!(!live_changed(3, Some(3)));
        assert!(live_changed(2, Some(3)), "an engine went silent");
        assert!(live_changed(4, Some(3)), "an engine registered");
        assert!(!live_changed(1, None));
        assert!(
            live_changed(0, None),
            "no expectation and none live is never a vacuous pass"
        );
        assert!(!live_changed(0, Some(0)));
    }

    #[test]
    fn a_missing_privilege_is_said_in_words_and_any_other_failure_is_a_database_error() {
        let denied = StoreError::Internal {
            detail: "error returned from database: permission denied for function platform_requirement_activate"
                .to_owned(),
        };
        let text = message(store_failure(&denied));
        assert!(text.contains("schema-owning database role"), "{text}");
        let other = message(store_failure(&StoreError::Internal {
            detail: "something else".to_owned(),
        }));
        assert!(other.starts_with("database error"), "{other}");
    }

    #[test]
    fn the_refusal_names_the_engines_that_lack_the_capability_and_why_an_engine_lacks_it() {
        let ids = [uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2)];
        let text = message(lacking(2, &ids));
        assert!(text.contains("2 live execution engine(s) do not list schedules-v1"));
        assert!(text.contains(&ids[0].to_string()) && text.contains(&ids[1].to_string()));
        assert!(text.contains("SAGAT_SCHEDULES_ENABLED"));
    }

    #[test]
    fn the_outcomes_are_told_in_words() {
        let done = HandoverOutcome::HandedOver { live: 3, epoch: 7 }.to_string();
        assert!(done.contains("3 live execution engine(s)") && done.contains("epoch 7"));
        let again = HandoverOutcome::AlreadyHandedOver {
            by: Some("ana".to_owned()),
            db_role: Some("sagat_owner".to_owned()),
        }
        .to_string();
        assert!(again.contains("already handed over") && again.contains("ana"));
        assert!(
            HandoverOutcome::WouldHandOver { live: 1 }
                .to_string()
                .contains("Nothing was changed")
        );
    }
}
