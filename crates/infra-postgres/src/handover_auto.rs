//! The automatic handover of the cron triggers in PostgreSQL (feature 018, `T018-082`, `T018-084`, ADR-0091; migration
//! `20270117000000_trigger_handover_automatic`).
//!
//! The job (`control-plane schedule-trigger-handover --auto`) runs as the role `sagat_handover`, which may execute four functions
//! and read two tables, and nothing else: the round, the wrapper, the audit completion and the retirement. **The database
//! decides**: the round computes every condition and counts the stable rounds in the attempt row, the wrapper computes them again
//! under the epoch lock; what this module does is ask, bound each call, and type what the database answers, by its SQLSTATE and its
//! closed outcome columns, never by matching text.
//!
//! The owner side is what `migrate` runs every time after the migrations: ensure the role, grant it again (refusing a role wider
//! than the design), and set its login from a SCRAM verifier the caller computed, only while the handover is `LEGACY` and the role
//! was never retired (D2264, D2269, D2272).

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use sagat_application::schedule_triggers::HandoverBlock;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::instances::OperatorBounds;
use crate::pool::{BeginSafely, PoolConfig};

/// The name of the role, fixed: the wrapper compares `session_user` with it.
pub const HANDOVER_ROLE: &str = "sagat_handover";

/// The statement timeout of a round (ADR-0091 decision 4): outside the epoch, longer than the 4 s the convergence count gives itself.
pub const ROUND_STATEMENT_TIMEOUT: Duration = Duration::from_secs(5);

/// What the database answered that is not an outcome: typed by SQLSTATE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoverDbError {
    /// `55P03`: a lock wait ended (the epoch, the triggers' table: `HANDOVER_BUSY`). A wait cures it.
    LockTimeout,
    /// `57014`: a statement ran past its timeout. A wait cures it, three in a row are `CHECK_TOO_SLOW`.
    StatementTimeout,
    /// The activation's step ran past its bound and was rolled back. A wait cures it.
    StepBound,
    /// `42501`: the role may not do this (the runtime role's credential, a role without the grants, a member of the role).
    Denied,
    /// `28000` or `28P01`: the role cannot log in (retired, a `pg_hba` refusal, a wrong password).
    CannotLogIn,
    /// `23505` from the audit completion: an audit record with a drawn id exists and differs (`AUDIT_CONFLICT`).
    AuditConflict,
    /// The database could not be reached.
    Unavailable(String),
    /// Anything else, as the driver says it (never a connection URL).
    Other(String),
}

impl fmt::Display for HandoverDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LockTimeout => f.write_str("a lock wait ended (SQLSTATE 55P03)"),
            Self::StatementTimeout => f.write_str("a statement ran past its timeout (SQLSTATE 57014)"),
            Self::StepBound => f.write_str("the activation ran past its step bound and was rolled back"),
            Self::Denied => f.write_str("permission denied (SQLSTATE 42501)"),
            Self::CannotLogIn => f.write_str("the role cannot log in (SQLSTATE 28000 or 28P01)"),
            Self::AuditConflict => f.write_str(
                "AUDIT_CONFLICT: an audit record with an id drawn for a tenant of the handover exists and differs",
            ),
            Self::Unavailable(detail) => write!(f, "the database could not be reached: {detail}"),
            Self::Other(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for HandoverDbError {}

/// The SQLSTATE of a driver error, when it is a database error.
fn sqlstate(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(db) => db.code().map(|code| code.to_string()),
        _ => None,
    }
}

/// Types a driver error by its SQLSTATE. `audit` says the call was the audit completion, the only one whose `23505` is a conflict.
fn classify(error: &sqlx::Error, audit: bool) -> HandoverDbError {
    use sqlx::Error;
    match sqlstate(error).as_deref() {
        Some("55P03") => return HandoverDbError::LockTimeout,
        Some("57014") => return HandoverDbError::StatementTimeout,
        Some("42501") => return HandoverDbError::Denied,
        Some("28000" | "28P01") => return HandoverDbError::CannotLogIn,
        Some("23505") if audit => return HandoverDbError::AuditConflict,
        Some(code) if code.starts_with("08") || matches!(code, "57P01" | "57P02" | "57P03") => {
            return HandoverDbError::Unavailable(error.to_string());
        }
        _ => {}
    }
    match error {
        Error::Io(_)
        | Error::Tls(_)
        | Error::PoolTimedOut
        | Error::PoolClosed
        | Error::WorkerCrashed => HandoverDbError::Unavailable(error.to_string()),
        _ => HandoverDbError::Other(error.to_string()),
    }
}

fn typed(error: sqlx::Error) -> HandoverDbError {
    classify(&error, false)
}

fn millis(duration: Duration) -> String {
    duration.as_millis().to_string()
}

/// Connects the job's pool: the URL carries no password, which is set apart on the connection options, so that `@`, `/` or `#`
/// in it cannot break an interpolated URL (D2273).
///
/// # Errors
/// [`HandoverDbError::CannotLogIn`] when the role cannot log in; the others as [`classify`] types them.
pub async fn connect_handover(
    config: &PoolConfig,
    password: Option<&str>,
) -> Result<PgPool, HandoverDbError> {
    let mut options = PgConnectOptions::from_str(&config.url)
        .map_err(|error| HandoverDbError::Other(error.to_string()))?
        .application_name("sagat-schedule-trigger-handover")
        .options([(
            "statement_timeout",
            format!("{}ms", config.statement_timeout.as_millis()),
        )]);
    if let Some(password) = password {
        options = options.password(password);
    }
    PgPoolOptions::new()
        .max_connections(config.max_connections.max(2))
        .acquire_timeout(config.acquire_timeout)
        .connect_with(options)
        .await
        .map_err(typed)
}

/// What the job asks of a round: its run id and its own stricter values (the database floors each and keeps those of the run's
/// first round).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoAsk {
    pub run: Uuid,
    pub stable: u32,
    pub horizon: Duration,
    pub min_live: u32,
    pub wait: Duration,
}

/// The closed outcome of a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundOutcome {
    /// It was handed over (by this job, another, or the operator).
    HandedOver,
    /// The state is one this build does not know.
    UnknownState,
    /// A note of the job's own was written.
    Noted,
    /// A condition failed; `reason` says which.
    Blocked,
    /// Every condition held; the stable rounds are being counted.
    Counting,
    /// Every condition held for the rounds asked: the wrapper may be called now.
    Stable,
    /// The run's wait ended; `reason` is what the round found.
    Deadline,
}

impl RoundOutcome {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "HANDED_OVER" => Self::HandedOver,
            "UNKNOWN_STATE" => Self::UnknownState,
            "NOTED" => Self::Noted,
            "BLOCKED" => Self::Blocked,
            "COUNTING" => Self::Counting,
            "STABLE" => Self::Stable,
            "DEADLINE" => Self::Deadline,
            _ => return None,
        })
    }
}

/// What a round answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub outcome: RoundOutcome,
    pub reason: Option<HandoverBlock>,
    pub stable: u32,
    pub wanted: u32,
    pub live: u32,
    pub first_round_at: OffsetDateTime,
    pub deadline: OffsetDateTime,
    pub now: OffsetDateTime,
    /// Up to sixteen engines that lack the capability.
    pub examples: Vec<Uuid>,
}

fn count(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

fn reason_of(text: Option<String>) -> Result<Option<HandoverBlock>, HandoverDbError> {
    match text {
        None => Ok(None),
        Some(code) => HandoverBlock::from_code(&code).map(Some).ok_or_else(|| {
            HandoverDbError::Other(format!(
                "the database gave a reason this build does not know: {code:.32}"
            ))
        }),
    }
}

/// One round of the job, through the round function, in a transaction of its own with a statement timeout of 5 s; nothing is
/// held after it returns. `note` writes a reason only the job can see (`BUSY`, `CHECK_TOO_SLOW`, `INTERRUPTED`, `DEADLINE`) and
/// counts nothing.
///
/// # Errors
/// As [`classify`] types them.
pub async fn handover_round(
    pool: &PgPool,
    ask: &AutoAsk,
    note: Option<HandoverBlock>,
) -> Result<Round, HandoverDbError> {
    let mut tx = pool.begin_safely().await.map_err(typed)?;
    sqlx::query("SELECT set_config('statement_timeout', $1, true)")
        .bind(millis(ROUND_STATEMENT_TIMEOUT))
        .execute(&mut *tx)
        .await
        .map_err(typed)?;
    let row = sqlx::query(
        "SELECT out_outcome, out_reason, out_stable, out_wanted, out_live, out_first_round_at, out_deadline, out_now, \
                out_examples \
           FROM schedule_trigger_handover_round($1, $2, make_interval(secs => $3::float8), $4, \
                                                make_interval(secs => $5::float8), $6)",
    )
    .bind(ask.run)
    .bind(i32::try_from(ask.stable).unwrap_or(i32::MAX))
    .bind(ask.horizon.as_secs_f64())
    .bind(i32::try_from(ask.min_live).unwrap_or(i32::MAX))
    .bind(ask.wait.as_secs_f64())
    .bind(note.map(HandoverBlock::code))
    .fetch_one(&mut *tx)
    .await
    .map_err(typed)?;
    tx.commit().await.map_err(typed)?;
    let outcome: String = row.try_get("out_outcome").map_err(typed)?;
    Ok(Round {
        outcome: RoundOutcome::parse(&outcome).ok_or_else(|| {
            HandoverDbError::Other(format!(
                "the database gave an outcome this build does not know: {outcome:.32}"
            ))
        })?,
        reason: reason_of(row.try_get("out_reason").map_err(typed)?)?,
        stable: count(row.try_get("out_stable").map_err(typed)?),
        wanted: count(row.try_get("out_wanted").map_err(typed)?),
        live: count(row.try_get("out_live").map_err(typed)?),
        first_round_at: row.try_get("out_first_round_at").map_err(typed)?,
        deadline: row.try_get("out_deadline").map_err(typed)?,
        now: row.try_get("out_now").map_err(typed)?,
        examples: row.try_get("out_examples").map_err(typed)?,
    })
}

/// What the wrapper did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoActivated {
    /// Handed over in this transaction, which committed.
    HandedOver { epoch: i64, live: u32, tenants: u32 },
    /// It had been handed over before.
    Already { epoch: Option<i64> },
    /// A condition failed under the lock; nothing was recorded.
    Refused(HandoverBlock),
    /// `schedules-v1` is already required of other kinds of instance: no wait cures it.
    KindsDiffer,
    /// The state is one this build does not know.
    UnknownState,
}

/// The activation through the wrapper: one bounded transaction (lock wait 2 s, statement 3 s, step 3 s: ADR-0091 decision 4) that
/// commits only when the wrapper handed over. Called only after a stable round.
///
/// # Errors
/// [`HandoverDbError::LockTimeout`], [`HandoverDbError::StatementTimeout`] and [`HandoverDbError::StepBound`], which a wait cures;
/// [`HandoverDbError::Denied`] for a session that is not `sagat_handover`; the others as [`classify`] types them. Nothing was
/// recorded in any of them.
pub async fn handover_activate(
    pool: &PgPool,
    run: Uuid,
    bounds: OperatorBounds,
) -> Result<AutoActivated, HandoverDbError> {
    let mut tx = pool.begin_safely().await.map_err(typed)?;
    sqlx::query(
        "SELECT set_config('lock_timeout', $1, true), set_config('statement_timeout', $2, true)",
    )
    .bind(millis(bounds.lock_timeout))
    .bind(millis(bounds.statement_timeout))
    .execute(&mut *tx)
    .await
    .map_err(typed)?;
    let call = sqlx::query(
        "SELECT out_outcome, out_reason, out_epoch, out_live, out_tenants FROM schedule_trigger_handover_auto($1)",
    )
    .bind(run)
    .fetch_one(&mut *tx);
    let row = match tokio::time::timeout(bounds.step, call).await {
        Ok(row) => row.map_err(typed)?,
        Err(_) => return Err(HandoverDbError::StepBound),
    };
    let outcome: String = row.try_get("out_outcome").map_err(typed)?;
    let epoch: Option<i64> = row.try_get("out_epoch").map_err(typed)?;
    let live = count(row.try_get("out_live").map_err(typed)?);
    let tenants = count(row.try_get("out_tenants").map_err(typed)?);
    let answer = match outcome.as_str() {
        "HANDED_OVER" => {
            tx.commit().await.map_err(typed)?;
            return Ok(AutoActivated::HandedOver {
                epoch: epoch.unwrap_or_default(),
                live,
                tenants,
            });
        }
        "ALREADY_HANDED_OVER" => AutoActivated::Already { epoch },
        "REFUSED" => AutoActivated::Refused(
            reason_of(row.try_get("out_reason").map_err(typed)?)?
                .unwrap_or(HandoverBlock::FleetChanging),
        ),
        "KINDS_DIFFER" => AutoActivated::KindsDiffer,
        "UNKNOWN_STATE" => AutoActivated::UnknownState,
        other => {
            return Err(HandoverDbError::Other(format!(
                "the database gave an outcome this build does not know: {other:.32}"
            )));
        }
    };
    tx.rollback().await.map_err(typed)?;
    Ok(answer)
}

/// What the audit completion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditCompleted {
    /// Records written by this call.
    pub written: u32,
    /// Tenants in the recorded set.
    pub tenants: u32,
    pub epoch: Option<i64>,
    pub mode: Option<String>,
}

/// Writes, after the commit of a handover, the audit record of every tenant of its recorded set not yet audited (ADR-0091 decision
/// 7). Idempotent: a second call writes nothing.
///
/// # Errors
/// [`HandoverDbError::AuditConflict`] when a record with a drawn id exists and differs; the others as [`classify`] types them.
pub async fn complete_handover_audit(pool: &PgPool) -> Result<AuditCompleted, HandoverDbError> {
    let row = sqlx::query("SELECT out_written, out_tenants, out_epoch, out_mode FROM schedule_trigger_handover_audit()")
        .fetch_one(pool)
        .await
        .map_err(|error| classify(&error, true))?;
    Ok(AuditCompleted {
        written: count(row.try_get("out_written").map_err(typed)?),
        tenants: count(row.try_get("out_tenants").map_err(typed)?),
        epoch: row.try_get("out_epoch").map_err(typed)?,
        mode: row.try_get("out_mode").map_err(typed)?,
    })
}

/// What the retirement did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retirement {
    /// Recorded, and the role cannot log in any more.
    Retired,
    /// Recorded (so `migrate` will not log the role in again), but the owner may not alter the role: the operator runs the
    /// statement (D2274).
    RecordedNotAltered,
    /// Not handed over yet: nothing done.
    NotHandedOver,
    /// The audit of the handover is not complete: nothing done.
    AuditPending,
}

/// Retires the role `sagat_handover` (`--retire-role`, D2265).
///
/// # Errors
/// As [`classify`] types them.
pub async fn retire_handover_role(pool: &PgPool) -> Result<Retirement, HandoverDbError> {
    let outcome: String = sqlx::query_scalar("SELECT schedule_trigger_handover_retire()")
        .fetch_one(pool)
        .await
        .map_err(typed)?;
    match outcome.as_str() {
        "RETIRED" => Ok(Retirement::Retired),
        "RECORDED_NOT_ALTERED" => Ok(Retirement::RecordedNotAltered),
        "NOT_HANDED_OVER" => Ok(Retirement::NotHandedOver),
        "AUDIT_PENDING" => Ok(Retirement::AuditPending),
        other => Err(HandoverDbError::Other(format!(
            "the database gave an outcome this build does not know: {other:.32}"
        ))),
    }
}

/// Whether the role of this session may make the automatic handover: execute the round, the wrapper and the audit completion
/// (the dry run of `--auto`, D2266). The wrapper itself also requires `session_user` to be `sagat_handover`.
///
/// # Errors
/// As [`classify`] types them.
pub async fn may_auto_hand_over(pool: &PgPool) -> Result<bool, HandoverDbError> {
    sqlx::query_scalar(
        "SELECT has_function_privilege('schedule_trigger_handover_auto(uuid)', 'EXECUTE') \
            AND has_function_privilege('schedule_trigger_handover_round(uuid, integer, interval, integer, interval, text)', 'EXECUTE') \
            AND has_function_privilege('schedule_trigger_handover_audit()', 'EXECUTE') \
            AND session_user = 'sagat_handover'",
    )
    .fetch_one(pool)
    .await
    .map_err(typed)
}

/// The state of the handover and its mode, as the job reads it.
///
/// # Errors
/// As [`classify`] types them.
pub async fn read_handover_mode(
    pool: &PgPool,
) -> Result<(String, Option<String>, Option<i64>), HandoverDbError> {
    sqlx::query_as("SELECT state, changed_mode, changed_epoch FROM schedule_trigger_handover")
        .fetch_one(pool)
        .await
        .map_err(typed)
}

/// The most recent attempt (its reason and how many seconds ago), for the dry run of `--auto`.
///
/// # Errors
/// As [`classify`] types them.
pub async fn last_attempt(pool: &PgPool) -> Result<Option<(Option<String>, f64)>, HandoverDbError> {
    sqlx::query_as(
        "SELECT last_reason, extract(epoch FROM clock_timestamp() - last_round_at)::float8 \
           FROM schedule_trigger_handover_attempts ORDER BY last_round_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(typed)
}

// --- the owner side, run by `migrate` --------------------------------------------------------------------------------------

/// What `migrate` found of the role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleEnsured {
    Exists,
    Created,
    /// Missing, and the migrating role may not create roles: the operator must (G2249).
    NotPermitted,
}

/// What the grant function did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleGrants {
    Granted,
    /// The role is wider than the design (an attribute beyond LOGIN, a membership, an object it owns): it was given nothing (D2275).
    Wider,
    Missing,
}

/// Ensures `sagat_handover` and grants it everything it needs, every run (D2264).
///
/// # Errors
/// The driver's.
pub async fn ensure_handover_role(pool: &PgPool) -> Result<(RoleEnsured, RoleGrants), sqlx::Error> {
    let ensured: String = sqlx::query_scalar("SELECT schedule_trigger_handover_role_ensure()")
        .fetch_one(pool)
        .await?;
    let granted: String = sqlx::query_scalar("SELECT schedule_trigger_handover_role_grants()")
        .fetch_one(pool)
        .await?;
    let ensured = match ensured.as_str() {
        "CREATED" => RoleEnsured::Created,
        "NOT_PERMITTED" => RoleEnsured::NotPermitted,
        _ => RoleEnsured::Exists,
    };
    let granted = match granted.as_str() {
        "GRANTED" => RoleGrants::Granted,
        "WIDER" => RoleGrants::Wider,
        _ => RoleGrants::Missing,
    };
    Ok((ensured, granted))
}

/// What setting the login did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginSet {
    /// `LOGIN` and the verifier are set.
    Set,
    /// The role was retired: left as it is (D2269).
    Retired,
    /// The cron triggers were handed over: left as it is (D2269).
    HandedOver,
    /// The role does not exist.
    NoRole,
    /// The migrating role may not alter it (`42501`, PostgreSQL 16 without `ADMIN OPTION`, D2274).
    NotPermitted,
}

/// Whether `text` is a SCRAM-SHA-256 verifier as PostgreSQL stores one: `SCRAM-SHA-256$<iterations>:<salt>$<StoredKey>:<ServerKey>`,
/// with base64 parts. Nothing else may reach the statement that sets the password, which cannot take a bind parameter.
#[must_use]
pub fn is_scram_verifier(text: &str) -> bool {
    let b64 = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    };
    let Some(rest) = text.strip_prefix("SCRAM-SHA-256$") else {
        return false;
    };
    let Some((head, keys)) = rest.split_once('$') else {
        return false;
    };
    let Some((iterations, salt)) = head.split_once(':') else {
        return false;
    };
    let Some((stored, server)) = keys.split_once(':') else {
        return false;
    };
    !iterations.is_empty()
        && iterations.bytes().all(|b| b.is_ascii_digit())
        && b64(salt)
        && b64(stored)
        && b64(server)
        && text.len() <= 512
}

/// Sets `LOGIN` and the password of `sagat_handover` from a SCRAM verifier, only while the handover is `LEGACY` and the role was
/// never retired, in one transaction that reads both (D2269). The clear password never reaches the server (D2272).
///
/// # Errors
/// The driver's; [`sqlx::Error::Protocol`] for a text that is not a verifier.
pub async fn set_handover_login(pool: &PgPool, verifier: &str) -> Result<LoginSet, sqlx::Error> {
    if !is_scram_verifier(verifier) {
        return Err(sqlx::Error::Protocol(
            "the password of sagat_handover is set only from a SCRAM-SHA-256 verifier".into(),
        ));
    }
    let mut tx = pool.begin_safely().await?;
    let state: String = sqlx::query_scalar("SELECT state FROM schedule_trigger_handover FOR SHARE")
        .fetch_one(&mut *tx)
        .await?;
    let retired: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM schedule_trigger_handover_retirement)")
            .fetch_one(&mut *tx)
            .await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_handover')",
    )
    .fetch_one(&mut *tx)
    .await?;
    let answer = if retired {
        LoginSet::Retired
    } else if state != "LEGACY" {
        LoginSet::HandedOver
    } else if !exists {
        LoginSet::NoRole
    } else {
        // the verifier is checked above to hold only the characters of base64, digits, `:` and `$`: it cannot leave the literal
        let statement = format!("ALTER ROLE sagat_handover LOGIN PASSWORD '{verifier}'");
        match sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&mut *tx)
            .await
        {
            Ok(_) => LoginSet::Set,
            Err(error) if sqlstate(&error).as_deref() == Some("42501") => {
                tx.rollback().await?;
                return Ok(LoginSet::NotPermitted);
            }
            Err(error) => return Err(error),
        }
    };
    tx.commit().await?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_scram_verifier_reaches_the_statement() {
        assert!(is_scram_verifier(
            "SCRAM-SHA-256$4096:c2FsdA==$c3RvcmVk/+=:c2VydmVy"
        ));
        for hostile in [
            "",
            "md5abc",
            "SCRAM-SHA-256$4096:salt$stored",
            "SCRAM-SHA-256$40x6:c2FsdA==$a:b",
            "SCRAM-SHA-256$4096:c2FsdA==$a:b'; DROP ROLE x; --",
            "SCRAM-SHA-256$4096:c2Fs dA==$a:b",
            "plain password",
        ] {
            assert!(!is_scram_verifier(hostile), "{hostile:?}");
        }
    }

    #[test]
    fn every_outcome_of_a_round_is_known_and_nothing_else_is() {
        for (text, outcome) in [
            ("HANDED_OVER", RoundOutcome::HandedOver),
            ("UNKNOWN_STATE", RoundOutcome::UnknownState),
            ("NOTED", RoundOutcome::Noted),
            ("BLOCKED", RoundOutcome::Blocked),
            ("COUNTING", RoundOutcome::Counting),
            ("STABLE", RoundOutcome::Stable),
            ("DEADLINE", RoundOutcome::Deadline),
        ] {
            assert_eq!(RoundOutcome::parse(text), Some(outcome));
        }
        assert_eq!(RoundOutcome::parse("stable"), None);
    }
}
