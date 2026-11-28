//! The publication hold and the registry of replay floors of ADR-0112, with the fairness gate of its amendment of 2026-11-27
//! (design 054's Tasks 054-18 and 054-20, built first by design 045's Task 045-21; Owner decisions 2026-11-26-540 and
//! 2026-11-26-564; migration `20270218000000_requirement_hold`).
//!
//! The two held-lock tests at the end settle with evidence whether publications that hold a requirement can starve the
//! operator: four publication sessions relay holds so that, at every instant but while the operator's command runs, one of them
//! holds the requirement epoch `FOR SHARE`; the relay starts before the command and lasts until the command returns and for at
//! least four times the command's lock bound. No sleep decides anything: each step waits for a grant, or for a backend seen
//! waiting for a lock in `pg_stat_activity`, polling every few milliseconds. Whether a hold was granted before the command
//! committed is read from the database, not from a clock: the command moves the epoch when it commits, and a hold that holds
//! the epoch `FOR SHARE` keeps the command from committing, so a hold that reads the old epoch was granted first.
#![allow(clippy::panic)]

use std::time::{Duration, Instant};

use sagat_application::error::StoreError;
use sagat_application::instances::{Activated, Activation, Capability, InstanceKind, Removed};
use sagat_domain::identity::Subject;
use sagat_infra_postgres::{
    HOLD_LOCK_TIMEOUT, Held, OperatorBounds, PgInstanceStore, REQUIREMENT_BUSY, hold_requirement,
};
use sqlx::PgPool;
use tokio::sync::oneshot;

use crate::support::instances::nothing;
use crate::support::{TestDb, migrated_db};

/// The capability the publications hold in these tests (a capability of today's closed lists, given a test floor).
const HELD: Capability = Capability::PayloadEncodingV1;
/// The capability the operator's command activates or removes while the publications hold [`HELD`].
const OTHER: Capability = Capability::AccessNarrowingV1;
const BOTH: [InstanceKind; 2] = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];
const RESTRICT_VIOLATION: &str = "23001";
const INVALID_PARAMETER_VALUE: &str = "22023";
/// The gate of ADR-0112's amendment, as every function that takes it writes it (followed by its mode).
const GATE: &str = "LOCK TABLE public.platform_requirement_gate IN ";

/// A floor for [`HELD`] that holds while the one row of `test_floor_state` says so (it starts `false`), registered as a
/// design's migration registers its own: a `SECURITY DEFINER` function with a fixed search path and no grant.
async fn floor_read_from_a_table(pool: &PgPool) {
    for statement in [
        "CREATE TABLE test_floor_state (held boolean NOT NULL)",
        "INSERT INTO test_floor_state (held) VALUES (false)",
        "CREATE FUNCTION test_floor_holds() RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER \
         SET search_path = pg_catalog, pg_temp AS 'SELECT held FROM public.test_floor_state'",
        "REVOKE ALL ON FUNCTION test_floor_holds() FROM PUBLIC",
        "INSERT INTO platform_requirement_floors (capability, holds) \
         VALUES ('payload-encoding-v1', 'test_floor_holds()')",
    ] {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }
}

/// A floor for [`OTHER`] that never holds.
async fn floor_that_never_holds(pool: &PgPool) {
    for statement in [
        "CREATE FUNCTION test_floor_never_holds() RETURNS boolean LANGUAGE sql IMMUTABLE SECURITY DEFINER \
         SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
        "REVOKE ALL ON FUNCTION test_floor_never_holds() FROM PUBLIC",
        "INSERT INTO platform_requirement_floors (capability, holds) \
         VALUES ('access-narrowing-v1', 'test_floor_never_holds()')",
    ] {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }
}

async fn set_floor(pool: &PgPool, held: bool) {
    sqlx::query("UPDATE test_floor_state SET held = $1")
        .bind(held)
        .execute(pool)
        .await
        .expect("the floor's state");
}

/// Records `capability` for both kinds, as the operator's activation does.
async fn record(pool: &PgPool, capability: Capability, kinds: &[&str]) {
    let outcome: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_activate($1, $2, 'operator')",
    )
    .bind(capability.as_str())
    .bind(kinds)
    .fetch_one(pool)
    .await
    .expect("activate");
    assert_eq!(outcome, "RECORDED");
}

async fn record_for_both(pool: &PgPool, capability: Capability) {
    record(pool, capability, &["CONTROL_PLANE", "EXECUTION_ENGINE"]).await;
}

async fn is_recorded(pool: &PgPool, capability: Capability) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM platform_requirements WHERE capability = $1)")
        .bind(capability.as_str())
        .fetch_one(pool)
        .await
        .expect("the requirement")
}

async fn epoch(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT epoch FROM platform_requirement_epoch")
        .fetch_one(pool)
        .await
        .expect("the epoch")
}

fn code_of(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|db| db.code())
        .map(|code| code.to_string())
        .unwrap_or_default()
}

/// Whether the backend `pid` waits for a lock.
async fn waits_for_a_lock(monitor: &PgPool, pid: i32) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')",
    )
    .bind(pid)
    .fetch_one(monitor)
    .await
    .expect("pg_stat_activity")
}

/// Polls `check` every two milliseconds until it answers, for at most `bound` (a barrier on an observed state, never a sleep
/// that decides an order).
async fn until<T, F, Fut>(what: &str, bound: Duration, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + bound;
    loop {
        if let Some(value) = check().await {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// A removal on a connection of its own, as `PgInstanceStore::remove` runs it (the epoch lock, then the removal), run until it
/// ends: its backend and its outcome or its error as `(SQLSTATE, text)`.
async fn removal(
    pool: &PgPool,
    capability: Capability,
) -> (
    i32,
    tokio::task::JoinHandle<Result<String, (String, String)>>,
) {
    let pool = pool.clone();
    let (pid_sender, pid) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let mut tx = pool.begin().await.expect("a transaction");
        let own: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .expect("pid");
        let _ = pid_sender.send(own);
        let outcome = async {
            sqlx::query("SELECT platform_requirement_lock()")
                .execute(&mut *tx)
                .await?;
            sqlx::query_scalar::<_, String>(
                "SELECT out_outcome FROM platform_requirement_remove($1, 'operator')",
            )
            .bind(capability.as_str())
            .fetch_one(&mut *tx)
            .await
        }
        .await;
        match outcome {
            Ok(outcome) => {
                tx.commit().await.expect("the removal commits");
                Ok(outcome)
            }
            Err(error) => Err((code_of(&error), error.to_string())),
        }
    });
    (pid.await.expect("the removal's backend"), handle)
}

/// Task 054-18: the hold runs as its owner with a fixed search path; only the runtime role may call it; the floors are the
/// migration role's alone; the floor trigger runs as its caller with a fixed search path and fires under every replication role.
#[tokio::test]
async fn the_hold_runs_as_its_owner_and_only_the_runtime_role_may_call_it() {
    let db = migrated_db().await;
    let (definer, config): (bool, Vec<String>) = sqlx::query_as(
        "SELECT prosecdef, coalesce(proconfig, '{}') FROM pg_proc \
          WHERE oid = 'platform_requirement_hold(text)'::regprocedure",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the hold");
    assert!(definer, "the hold runs as its owner");
    assert_eq!(config, vec!["search_path=pg_catalog, pg_temp".to_owned()]);
    let privileges: (bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT has_function_privilege('public', 'platform_requirement_hold(text)', 'EXECUTE'), \
                has_function_privilege('sagat_app', 'platform_requirement_hold(text)', 'EXECUTE'), \
                has_table_privilege('sagat_app', 'platform_requirement_floors', 'SELECT'), \
                has_table_privilege('sagat_app', 'platform_requirement_floors', 'INSERT, UPDATE, DELETE, TRUNCATE'), \
                has_function_privilege('public', 'platform_requirement_floor()', 'EXECUTE')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("privileges");
    assert_eq!(
        privileges,
        (false, true, false, false, false),
        "(PUBLIC calls the hold, the runtime role calls it, reads the floors, writes them, PUBLIC calls the trigger)"
    );
    let (trigger_definer, trigger_config, enabled): (bool, Vec<String>, String) = sqlx::query_as(
        "SELECT p.prosecdef, coalesce(p.proconfig, '{}'), t.tgenabled::text FROM pg_trigger t \
           JOIN pg_proc p ON p.oid = t.tgfoid \
          WHERE t.tgname = 'platform_requirement_floor' AND t.tgrelid = 'platform_requirements'::regclass",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the floor trigger");
    assert!(
        !trigger_definer,
        "the floor trigger runs as whoever deletes the requirement"
    );
    assert_eq!(
        trigger_config,
        vec!["search_path=pg_catalog, pg_temp".to_owned()]
    );
    assert_eq!(
        enabled, "A",
        "enabled ALWAYS, so it fires under session_replication_role = replica"
    );
}

/// ADR-0112 decision 3: the hold's body is the text of the amendment, the text every later migration compares with.
#[tokio::test]
async fn the_hold_is_the_text_of_the_architecture_decision() {
    let db = migrated_db().await;
    let source: String = sqlx::query_scalar(
        "SELECT prosrc FROM pg_proc WHERE oid = 'platform_requirement_hold(text)'::regprocedure",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the hold");
    let record = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../docs/adr/ADR-0112-fleet-requirements-share-one-publication-hold-and-one-registry-of-replay-floors.md"
    ))
    .expect("ADR-0112");
    let amendment = record
        .find("**The hold's body**")
        .expect("the amendment's body");
    let block = &record[amendment..];
    let start = block.find("AS $$").expect("the body starts") + "AS $$".len();
    let end = start + block[start..].find("$$;").expect("the body ends");
    let words = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(words(&source), words(&block[start..end]));
}

/// ADR-0112's amendment, decisions 1 and 2: the gate comes first, exclusive in the four functions that take the epoch `FOR UPDATE`
/// (the operator's three and the automatic handover, which then calls the activation) and shared in the two that wait to share it.
#[tokio::test]
async fn the_gate_comes_before_the_epoch_in_every_function_that_waits_for_it() {
    let db = migrated_db().await;
    for (function, lock) in [
        ("platform_requirement_lock()", "EXCLUSIVE MODE;"),
        (
            "platform_requirement_activate(text, text[], text)",
            "EXCLUSIVE MODE;",
        ),
        ("platform_requirement_remove(text, text)", "EXCLUSIVE MODE;"),
        ("platform_requirement_hold(text)", "SHARE MODE;"),
        ("schedule_trigger_handover_auto(uuid)", "EXCLUSIVE MODE;"),
        (
            "instance_register(uuid, bytea, text, text, text, text[])",
            "SHARE MODE;",
        ),
    ] {
        let source: String =
            sqlx::query_scalar("SELECT prosrc FROM pg_proc WHERE oid = $1::regprocedure")
                .bind(function)
                .fetch_one(&db.pool)
                .await
                .unwrap_or_else(|error| panic!("{function}: {error}"));
        let gate = format!("{GATE}{lock}");
        let at = source
            .find(&gate)
            .unwrap_or_else(|| panic!("{function} takes the gate with {lock}"));
        let epoch = source
            .find("FROM public.platform_requirement_epoch e")
            .unwrap_or_else(|| panic!("{function} reads the epoch"));
        assert!(at < epoch, "{function} takes the gate before the epoch");
    }
}

/// The hold answers only for a capability that has a floor, so the runtime role can fence on nothing else; the store says so
/// as a defect of the build.
#[tokio::test]
async fn the_hold_refuses_a_capability_that_has_no_floor() {
    let db = migrated_db().await;
    let runtime = db.runtime_pool().await;
    let refused = sqlx::query_scalar::<_, bool>("SELECT platform_requirement_hold('schedules-v1')")
        .fetch_one(&runtime)
        .await
        .expect_err("no floor");
    assert_eq!(code_of(&refused), INVALID_PARAMETER_VALUE, "{refused}");
    let mut tx = runtime.begin().await.expect("a transaction");
    let refused = hold_requirement(&mut tx, Capability::SchedulesV1)
        .await
        .expect_err("no floor");
    assert!(
        matches!(refused, StoreError::Internal { .. }),
        "{refused:?}"
    );
}

/// ADR-0112 decision 1: true only when the requirement is recorded for control planes and engines.
#[tokio::test]
async fn the_hold_answers_true_only_for_a_requirement_of_both_kinds() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    let runtime = db.runtime_pool().await;
    let held = |runtime: PgPool| async move {
        let mut tx = runtime.begin().await.expect("a transaction");
        let active = hold_requirement(&mut tx, HELD).await.expect("the hold") == Held::Active;
        tx.commit().await.expect("commit");
        active
    };
    assert!(!held(runtime.clone()).await, "not recorded");
    record(&db.pool, HELD, &["CONTROL_PLANE"]).await;
    assert!(!held(runtime.clone()).await, "recorded for one kind only");
    let removed: String =
        sqlx::query_scalar("SELECT out_outcome FROM platform_requirement_remove($1, 'operator')")
            .bind(HELD.as_str())
            .fetch_one(&db.pool)
            .await
            .expect("remove");
    assert_eq!(removed, "REMOVED");
    record_for_both(&db.pool, HELD).await;
    assert!(held(runtime.clone()).await, "recorded for both kinds");
}

/// The hold sets its own lock bound only for itself: the transaction's bound is what it was before, after the hold.
#[tokio::test]
async fn the_hold_restores_the_lock_bound_of_its_transaction() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    let runtime = db.runtime_pool().await;
    let mut tx = runtime.begin().await.expect("a transaction");
    sqlx::query("SET LOCAL lock_timeout = '7s'")
        .execute(&mut *tx)
        .await
        .expect("a bound of the transaction");
    assert_eq!(
        hold_requirement(&mut tx, HELD).await.expect("the hold"),
        Held::NotActive
    );
    let bound: String = sqlx::query_scalar("SELECT current_setting('lock_timeout')")
        .fetch_one(&mut *tx)
        .await
        .expect("the bound");
    assert_eq!(bound, "7s");
}

/// Task 054-18, a hold against a concurrent removal, first order: the publication holds the requirement and writes what the
/// floor reads; the removal waits for it and, once it commits, sees it and is refused `FLOOR_HELD`. The requirement stays.
#[tokio::test]
async fn a_removal_that_waits_for_a_hold_sees_what_the_publication_committed() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    let mut publication = db.pool.begin().await.expect("a transaction");
    // the version the publication writes, which the floor reads
    sqlx::query("UPDATE test_floor_state SET held = true")
        .execute(&mut *publication)
        .await
        .expect("the version");
    assert_eq!(
        hold_requirement(&mut publication, HELD)
            .await
            .expect("the hold"),
        Held::Active
    );
    let (pid, removal) = removal(&db.pool, HELD).await;
    until(
        "the removal waits for the hold",
        Duration::from_secs(5),
        || {
            let monitor = db.pool.clone();
            async move { waits_for_a_lock(&monitor, pid).await.then_some(()) }
        },
    )
    .await;
    publication.commit().await.expect("the publication commits");
    let (code, text) = removal
        .await
        .expect("the removal ended")
        .expect_err("the removal is refused");
    assert_eq!(code, RESTRICT_VIOLATION, "{text}");
    assert!(text.contains("FLOOR_HELD"), "{text}");
    assert!(is_recorded(&db.pool, HELD).await, "the requirement stays");
}

/// The other order: a removal that holds the registry makes the hold wait, and once it commits the hold finds no requirement,
/// so the publication is refused.
#[tokio::test]
async fn a_hold_that_waits_for_a_removal_finds_no_requirement() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    let mut operator = db.pool.begin().await.expect("a transaction");
    let removed: String =
        sqlx::query_scalar("SELECT out_outcome FROM platform_requirement_remove($1, 'operator')")
            .bind(HELD.as_str())
            .fetch_one(&mut *operator)
            .await
            .expect("remove");
    assert_eq!(removed, "REMOVED");
    let runtime = db.runtime_pool().await;
    let (pid_sender, pid) = oneshot::channel();
    let hold = tokio::spawn(async move {
        let mut tx = runtime.begin().await.expect("a transaction");
        let own: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .expect("pid");
        let _ = pid_sender.send(own);
        // the function itself, with no lock bound: the order is what is tested, not the bound (it has its own test)
        sqlx::query_scalar::<_, bool>("SELECT platform_requirement_hold($1)")
            .bind(HELD.as_str())
            .fetch_one(&mut *tx)
            .await
            .expect("the hold")
    });
    let pid = pid.await.expect("the hold's backend");
    until(
        "the hold waits for the removal",
        Duration::from_secs(5),
        || {
            let monitor = db.pool.clone();
            async move { waits_for_a_lock(&monitor, pid).await.then_some(()) }
        },
    )
    .await;
    operator.commit().await.expect("the removal commits");
    assert!(
        !hold.await.expect("the hold ended"),
        "no requirement: the publication is refused"
    );
}

/// Design 054, Technical Plan 15: a hold that cannot get the registry within its bound is `UNAVAILABLE` with the reason
/// `REQUIREMENT_BUSY`, and gives up within one bound per lock it waits for.
#[tokio::test]
async fn a_hold_that_waits_past_its_bound_is_refused_requirement_busy() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    let mut operator = db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT platform_requirement_lock()")
        .execute(&mut *operator)
        .await
        .expect("the operator holds the registry");
    let runtime = db.runtime_pool().await;
    let mut publication = runtime.begin().await.expect("a transaction");
    let began = Instant::now();
    let refused = hold_requirement(&mut publication, HELD)
        .await
        .expect_err("refused within its bound");
    let waited = began.elapsed();
    match &refused {
        StoreError::Unavailable { detail } => {
            assert!(detail.starts_with(REQUIREMENT_BUSY), "{detail}");
        }
        other => panic!("expected UNAVAILABLE REQUIREMENT_BUSY, got {other:?}"),
    }
    // one bound for the gate, which the operator holds; ten bounds would mean the hold has none
    assert!(waited < HOLD_LOCK_TIMEOUT * 10, "{waited:?}");
    operator.rollback().await.expect("rolled back");
}

/// Task 054-18: a registered floor refuses the removal while it holds and allows it once it clears.
#[tokio::test]
async fn a_floor_refuses_the_removal_while_it_holds_and_allows_it_once_it_clears() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    set_floor(&db.pool, true).await;
    let (_, refused) = removal(&db.pool, HELD).await;
    let (code, text) = refused
        .await
        .expect("the removal ended")
        .expect_err("refused while the floor holds");
    assert_eq!(code, RESTRICT_VIOLATION, "{text}");
    assert!(
        text.contains("FLOOR_HELD") && text.contains("test_floor_holds()"),
        "{text}"
    );
    assert!(is_recorded(&db.pool, HELD).await);
    set_floor(&db.pool, false).await;
    let (_, allowed) = removal(&db.pool, HELD).await;
    assert_eq!(
        allowed.await.expect("the removal ended").expect("removed"),
        "REMOVED"
    );
    assert!(!is_recorded(&db.pool, HELD).await);
}

/// Task 054-18: two capabilities registered at once each answer through their own floor.
#[tokio::test]
async fn two_capabilities_registered_at_once_each_answer_through_their_own_floor() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    floor_that_never_holds(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    record_for_both(&db.pool, OTHER).await;
    set_floor(&db.pool, true).await;
    let (_, other) = removal(&db.pool, OTHER).await;
    assert_eq!(other.await.expect("ended").expect("removed"), "REMOVED");
    let (_, held) = removal(&db.pool, HELD).await;
    let (_, text) = held.await.expect("ended").expect_err("refused");
    assert!(
        text.contains("FLOOR_HELD") && text.contains("payload-encoding-v1"),
        "{text}"
    );
}

/// ADR-0112 decision 2 and decision 4 of its amendment: a removal that reads a floor runs only under READ COMMITTED; a
/// capability with no floor is removed under any level, as before.
#[tokio::test]
async fn a_floor_is_read_only_under_read_committed() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    record(&db.pool, Capability::SchedulesV1, &["EXECUTION_ENGINE"]).await;
    for level in ["REPEATABLE READ", "SERIALIZABLE"] {
        let mut tx = db.pool.begin().await.expect("a transaction");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET TRANSACTION ISOLATION LEVEL {level}"
        )))
        .execute(&mut *tx)
        .await
        .expect("isolation level");
        let refused = sqlx::query_scalar::<_, String>(
            "SELECT out_outcome FROM platform_requirement_remove($1, 'operator')",
        )
        .bind(HELD.as_str())
        .fetch_one(&mut *tx)
        .await
        .expect_err("refused outside READ COMMITTED");
        assert_eq!(code_of(&refused), RESTRICT_VIOLATION, "{level}: {refused}");
        assert!(
            refused
                .to_string()
                .contains("is removed only under READ COMMITTED"),
            "{level}: {refused}"
        );
    }
    let mut tx = db.pool.begin().await.expect("a transaction");
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .expect("isolation level");
    let removed: String = sqlx::query_scalar(
        "SELECT out_outcome FROM platform_requirement_remove('schedules-v1', 'operator')",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("a capability with no floor is removed under any level");
    assert_eq!(removed, "REMOVED");
    tx.commit().await.expect("commit");
}

/// ADR-0112 decisions 2 and 3 (and the security review of 2026-11-27): a floor names a plain function with no argument, a boolean
/// result and the owner of the floor trigger, run as its owner with the fixed search path, and with no overload.
#[tokio::test]
async fn a_floor_names_only_an_admissible_function() {
    let db = migrated_db().await;
    for statement in [
        "CREATE FUNCTION test_floor_with_argument(n integer) RETURNS boolean LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
        "CREATE FUNCTION test_floor_of_text() RETURNS text LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS 'SELECT ''false'''",
        "CREATE FUNCTION test_floor_of_a_set() RETURNS SETOF boolean LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
        "CREATE FUNCTION test_floor_as_its_caller() RETURNS boolean LANGUAGE sql SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
        "CREATE FUNCTION test_floor_with_any_path() RETURNS boolean LANGUAGE sql SECURITY DEFINER AS 'SELECT false'",
        "CREATE FUNCTION test_floor_overloaded() RETURNS boolean LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
        "CREATE FUNCTION test_floor_overloaded(n integer DEFAULT 0) RETURNS boolean LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
        "CREATE FUNCTION test_floor_of_another_owner() RETURNS boolean LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
        "DO $$ BEGIN \
         PERFORM pg_advisory_xact_lock(hashtextextended('sagat:test:floor-owner', 0)); \
         CREATE ROLE sagat_test_floor_owner NOLOGIN; EXCEPTION WHEN duplicate_object THEN NULL; END $$",
        "ALTER FUNCTION test_floor_of_another_owner() OWNER TO sagat_test_floor_owner",
        "CREATE FUNCTION test_floor_fine() RETURNS boolean LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS 'SELECT false'",
    ] {
        sqlx::query(statement)
            .execute(&db.pool)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }
    for holds in [
        "test_floor_with_argument(integer)",
        "test_floor_of_text()",
        "test_floor_of_a_set()",
        "test_floor_as_its_caller()",
        "test_floor_with_any_path()",
        "test_floor_overloaded()",
        "test_floor_of_another_owner()",
    ] {
        let refused = sqlx::query(
            "INSERT INTO platform_requirement_floors (capability, holds) VALUES ('schedules-v1', $1::regprocedure)",
        )
        .bind(holds)
        .execute(&db.pool)
        .await
        .expect_err("refused");
        assert_eq!(
            code_of(&refused),
            INVALID_PARAMETER_VALUE,
            "{holds}: {refused}"
        );
    }
    sqlx::query(
        "INSERT INTO platform_requirement_floors (capability, holds) VALUES ('schedules-v1', 'test_floor_fine()')",
    )
    .execute(&db.pool)
    .await
    .expect("admitted");
    let refused = sqlx::query(
        "UPDATE platform_requirement_floors SET holds = 'test_floor_of_text()' WHERE capability = 'schedules-v1'",
    )
    .execute(&db.pool)
    .await
    .expect_err("an update is checked too");
    assert_eq!(code_of(&refused), INVALID_PARAMETER_VALUE, "{refused}");
    let unknown = sqlx::query(
        "INSERT INTO platform_requirement_floors (capability, holds) VALUES ('no-such-capability-v1', 'test_floor_fine()')",
    )
    .execute(&db.pool)
    .await
    .expect_err("the closed list");
    assert_eq!(code_of(&unknown), "23514", "{unknown}");
}

/// The security review of 2026-11-27: a floor is removed only by a migration, never by a hand edit or a truncation, and a floor
/// whose function stopped being admissible after it was registered (here an overload that would make the call ambiguous) refuses
/// the removal instead of running something else.
#[tokio::test]
async fn a_floor_is_removed_only_by_a_migration_and_checked_again_before_each_call() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    for statement in [
        "DELETE FROM platform_requirement_floors",
        "TRUNCATE platform_requirement_floors",
    ] {
        let refused = sqlx::query(statement)
            .execute(&db.pool)
            .await
            .expect_err("a hand edit is refused");
        assert_eq!(
            code_of(&refused),
            RESTRICT_VIOLATION,
            "{statement}: {refused}"
        );
    }
    sqlx::query(
        "CREATE FUNCTION test_floor_holds(n integer DEFAULT 0) RETURNS boolean LANGUAGE sql AS 'SELECT false'",
    )
    .execute(&db.pool)
    .await
    .expect("an overload made after the floor was registered");
    let (_, removal) = removal(&db.pool, HELD).await;
    let (code, text) = removal
        .await
        .expect("the removal ended")
        .expect_err("refused: the floor is no longer admissible");
    assert_eq!(code, RESTRICT_VIOLATION, "{text}");
    assert!(
        text.contains("no longer a function the registry admits"),
        "{text}"
    );
    assert!(is_recorded(&db.pool, HELD).await);
    let mut migration = db.pool.begin().await.expect("a transaction");
    sqlx::query("SET LOCAL sagat.floor_change = 'migration'")
        .execute(&mut *migration)
        .await
        .expect("the flag of a down migration");
    sqlx::query("DELETE FROM platform_requirement_floors WHERE capability = 'payload-encoding-v1'")
        .execute(&mut *migration)
        .await
        .expect("a down migration removes its floor");
    migration.commit().await.expect("commit");
}

// -------------------------------------------------------------------------------------------------------------------------
// The held-lock tests of Task 045-21 and Task 054-20 (Owner decision 2026-11-26-564).
// -------------------------------------------------------------------------------------------------------------------------

/// One publication of the relay: a transaction of its own as the runtime role that holds [`HELD`], then reads the epoch, and
/// commits when told.
struct Hold {
    pid: i32,
    /// The epoch the publication read just after its hold was granted, once received.
    granted: oneshot::Receiver<i64>,
    epoch: Option<i64>,
    commit: oneshot::Sender<()>,
    done: tokio::task::JoinHandle<()>,
    /// Whether it was requested after the operator's command was seen waiting.
    after_the_command_waited: bool,
}

async fn request_hold(pool: &PgPool, after_the_command_waited: bool) -> Hold {
    let pool = pool.clone();
    let (pid_sender, pid) = oneshot::channel();
    let (granted_sender, granted) = oneshot::channel();
    let (commit, committed) = oneshot::channel::<()>();
    let done = tokio::spawn(async move {
        let mut tx = pool.begin().await.expect("a transaction");
        let own: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .expect("pid");
        let _ = pid_sender.send(own);
        assert_eq!(
            hold_requirement(&mut tx, HELD).await.expect("the hold"),
            Held::Active,
            "the publication's requirement stays recorded"
        );
        let seen: i64 = sqlx::query_scalar("SELECT epoch FROM platform_requirement_epoch")
            .fetch_one(&mut *tx)
            .await
            .expect("the epoch");
        let _ = granted_sender.send(seen);
        let _ = committed.await;
        tx.commit().await.expect("the publication commits");
    });
    Hold {
        pid: pid.await.expect("the publication's backend"),
        granted,
        epoch: None,
        commit,
        done,
        after_the_command_waited,
    }
}

impl Hold {
    /// The epoch it read, if its hold is granted by now.
    fn granted_now(&mut self) -> Option<i64> {
        if self.epoch.is_none() {
            self.epoch = self.granted.try_recv().ok();
        }
        self.epoch
    }

    async fn granted_eventually(&mut self) -> i64 {
        if let Some(epoch) = self.epoch {
            return epoch;
        }
        let epoch = tokio::time::timeout(Duration::from_secs(10), &mut self.granted)
            .await
            .expect("the hold is granted once the command is done")
            .expect("the publication reports its epoch");
        self.epoch = Some(epoch);
        epoch
    }

    async fn commit(self) {
        let _ = self.commit.send(());
        self.done.await.expect("the publication ended");
    }
}

struct Relay {
    /// Holds requested in all.
    holds: usize,
    /// Holds requested after the command was seen waiting and granted before it committed (they read the old epoch).
    granted_ahead: usize,
    /// Whether the command was seen waiting for a lock: a command that never waits proves nothing.
    command_waited: bool,
    /// Holds requested while the command was seen waiting and had not returned.
    requested_while_waiting: usize,
    relay_lasted: Duration,
    command: Result<(), String>,
    command_took: Duration,
}

/// Runs the relay of Task 045-21 around `command`, which must move the epoch when it commits.
async fn relay_around<C>(db: &TestDb, command: C, waiting_statement: &'static str) -> Relay
where
    C: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    let bound = OperatorBounds::default().lock_timeout;
    assert_eq!(
        bound,
        Duration::from_millis(2_000),
        "the command's lock bound"
    );
    let at_least = bound * 4;
    let publications = db.runtime_pool_with(4).await;
    let before = epoch(&db.pool).await;
    let started = Instant::now();
    let mut current = request_hold(&publications, false).await;
    assert_eq!(current.granted_eventually().await, before);
    let mut holds = 1;
    let command = tokio::spawn(async move {
        let began = Instant::now();
        let outcome = command.await;
        (outcome, began.elapsed())
    });
    // the command waits for the publication that holds the epoch
    let command_waited = until(
        "the command waits for a lock",
        Duration::from_secs(5),
        || {
            let finished = command.is_finished();
            let monitor = db.pool.clone();
            async move {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                      WHERE datname = current_database() AND pid <> pg_backend_pid() \
                        AND wait_event_type = 'Lock' AND query LIKE $1)",
                )
                .bind(waiting_statement)
                .fetch_one(&monitor)
                .await
                .expect("pg_stat_activity");
                (waiting || finished).then_some(waiting)
            }
        },
    )
    .await;
    let mut granted_ahead = 0;
    let mut requested_while_waiting = 0;
    loop {
        let pending = !command.is_finished();
        let mut next = request_hold(&publications, true).await;
        holds += 1;
        if command_waited && pending {
            requested_while_waiting += 1;
        }
        // the next publication is granted, or seen waiting, before the current one commits
        until(
            "the next hold is granted or waits",
            Duration::from_secs(5),
            || {
                let granted = next.granted_now().is_some();
                let pid = next.pid;
                let monitor = db.pool.clone();
                async move { (granted || waits_for_a_lock(&monitor, pid).await).then_some(()) }
            },
        )
        .await;
        current.commit().await;
        current = next;
        let seen = current.granted_eventually().await;
        if current.after_the_command_waited && seen == before && !command.is_finished() {
            granted_ahead += 1;
        }
        if command.is_finished() && started.elapsed() >= at_least {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the relay ends"
        );
    }
    current.commit().await;
    let relay_lasted = started.elapsed();
    let (outcome, command_took) = command.await.expect("the command ended");
    Relay {
        holds,
        granted_ahead,
        command_waited,
        requested_while_waiting,
        relay_lasted,
        command: outcome,
        command_took,
    }
}

fn assert_the_operator_is_not_starved(relay: &Relay) {
    // the relay runs until the command returns and for at least four times its lock bound (the loop's exit); what is asserted
    // is that the command did wait for the relay, and that holds kept being requested while it waited
    assert!(
        relay.command_waited,
        "the command was seen waiting for a lock (relay of {} holds over {:?})",
        relay.holds, relay.relay_lasted
    );
    assert!(
        relay.requested_while_waiting > 0,
        "holds were requested while the command waited: {} of {}",
        relay.requested_while_waiting,
        relay.holds
    );
    assert_eq!(
        relay.granted_ahead, 0,
        "no hold requested after the command waited is granted before it commits ({} of {} were; the command: {:?} after {:?})",
        relay.granted_ahead, relay.holds, relay.command, relay.command_took
    );
    assert_eq!(
        relay.command,
        Ok(()),
        "the command commits, without {REQUIREMENT_BUSY}"
    );
    assert!(
        relay.command_took < Duration::from_millis(2_000),
        "the command commits within its lock bound: {:?}",
        relay.command_took
    );
}

/// Task 045-21 and Task 054-20: the operator's activation commits within its lock bound while publications keep the epoch held
/// `FOR SHARE`, and no hold requested after it waits is granted before it.
#[tokio::test]
async fn an_activation_commits_under_a_continuous_stream_of_holds() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    let store = PgInstanceStore::new(db.pool.clone());
    let command = async move {
        let actor = Subject::operator();
        match store
            .activate(
                Activation {
                    capability: OTHER,
                    kinds: &BOTH,
                    actor: &actor,
                },
                nothing,
            )
            .await
        {
            Ok(Activated::Recorded { .. }) => Ok(()),
            other => Err(format!("{other:?}")),
        }
    };
    let relay = relay_around(&db, command, "%platform_requirement_activate%").await;
    assert_the_operator_is_not_starved(&relay);
    assert!(is_recorded(&db.pool, OTHER).await);
}

/// Task 045-21 and Task 054-20: the same for the operator's removal.
#[tokio::test]
async fn a_removal_commits_under_a_continuous_stream_of_holds() {
    let db = migrated_db().await;
    floor_read_from_a_table(&db.pool).await;
    record_for_both(&db.pool, HELD).await;
    record_for_both(&db.pool, OTHER).await;
    let store = PgInstanceStore::new(db.pool.clone());
    let command = async move {
        let actor = Subject::operator();
        match store.remove(OTHER, &actor, nothing).await {
            Ok(Removed::Removed { .. }) => Ok(()),
            other => Err(format!("{other:?}")),
        }
    };
    let relay = relay_around(&db, command, "%platform_requirement_lock%").await;
    assert_the_operator_is_not_starved(&relay);
    assert!(!is_recorded(&db.pool, OTHER).await);
}
