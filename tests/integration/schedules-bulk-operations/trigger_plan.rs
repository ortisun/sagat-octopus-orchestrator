//! The check the handover makes under its lock does not depend on the plan the statistics give it (G2049, D2117): the fitness
//! function of that rule. It runs in a pull request only when a file of this target changes (`tests/AGENTS.md`); a change of the
//! migration alone does not select it, and it runs for a release.
//!
//! The benchmark found the view `schedule_trigger_drift` asked under `EXISTS` (what `schedule_trigger_unconverged()` does while
//! the handover holds the table lock) took 2.85 s at 10,000 triggers, 1.0 s at 5,000 and 28 ms at 15,000: the planner optimises an
//! `EXISTS` for its first row, believed the filter of the view kept almost every trigger (the estimate of a `CASE ... IS NOT NULL`)
//! and the singleton `schedule_trigger_handover`, which autovacuum never analyzes, 610 rows, and chose the plan with no start-up
//! cost: a nested loop that scanned `schedules` again for each trigger. The view now looks the schedule up by `trigger_id` for each
//! trigger (`LEFT JOIN LATERAL ... LIMIT 1`, the partial unique index `schedules_trigger_uniq`), which gives the planner a probe for
//! each trigger and no join to rescan. The form does not by itself rule out every other plan (PostgreSQL does not take a partial
//! unique index as proof of uniqueness), so these tests are what holds it: the shape of the plan in three states of the
//! statistics, a time that a machine ten times slower than the one that measured it would not exceed, and that the view lists
//! what the previous definition listed, for every kind of drift.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use sagat_application::schedule_triggers::TriggerScheduleStore as _;
use sqlx::PgPool;

use crate::support::trigger_schedules::triggers;

/// The definition the view had before G2049, kept here as the oracle of what it must keep listing.
const PREVIOUS_DEFINITION: &str = "\
SELECT d.* FROM ( \
    SELECT t.id AS trigger_id, t.tenant_id, t.namespace_id, t.workflow_id, t.version_id, t.cron, t.timezone, t.next_fire_at, \
           CASE WHEN s.id IS NULL THEN 'MISSING' \
                WHEN s.state = 'DELETED' THEN 'REVIVED' \
                WHEN r.spec #>> '{cron,0}' IS DISTINCT FROM t.cron \
                  OR r.spec ->> 'timezone' IS DISTINCT FROM t.timezone THEN 'RESPEC' \
                WHEN s.state = 'PAUSED' THEN CASE WHEN h.state = 'LEGACY' THEN 'PAUSED' END \
                WHEN s.state = 'ACTIVE' \
                 AND (s.next_action_at IS NULL OR s.next_actual_at IS NULL) \
                 AND s.remaining_actions IS DISTINCT FROM 0 THEN 'NO_NEXT_TIME' \
           END AS kind, \
           s.id AS schedule_id, s.state_version AS state_version, s.head_revision AS revision \
      FROM public.workflow_triggers t \
     CROSS JOIN public.schedule_trigger_handover h \
      LEFT JOIN public.schedules s ON s.trigger_id = t.id \
      LEFT JOIN public.schedule_revisions r \
             ON r.tenant_id = s.tenant_id AND r.schedule_id = s.id AND r.revision = s.head_revision \
     WHERE t.kind = 'cron') d \
 WHERE d.kind IS NOT NULL";

const TABLES: [&str; 4] = [
    "workflow_triggers",
    "schedules",
    "schedule_revisions",
    "schedule_trigger_handover",
];

/// Keeps autovacuum from analyzing a table behind the test's back: the statistics are what the test says they are.
async fn freeze_the_statistics(pool: &PgPool) {
    for table in TABLES {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} SET (autovacuum_enabled = false)"
        )))
        .execute(pool)
        .await
        .expect("no autovacuum");
    }
}

async fn analyze(pool: &PgPool, tables: &[&str]) {
    for table in tables {
        sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE {table}")))
            .execute(pool)
            .await
            .expect("analyze");
    }
}

/// Writes `triggers` cron triggers as the owner, by bulk statements with the foreign keys and the user triggers of the three
/// tables off for the connection that writes (the per-row guards and counters make a bulk write minutes long; nothing writes
/// after the seeding). The 200 tenants and the namespace are invented. `schedules` says what each trigger gets, by its class
/// (the row number of the trigger, modulo seventeen):
///
/// | class | schedule | kind of drift |
/// | --- | --- | --- |
/// | 0 | active, planned | none |
/// | 1 | none | MISSING |
/// | 2 | deleted, its revision equal to the trigger | REVIVED |
/// | 3 | revision with another expression | RESPEC |
/// | 4 | revision with another zone | RESPEC |
/// | 5 | no revision row | RESPEC |
/// | 6 | paused | PAUSED (while LEGACY) |
/// | 7 | active, neither time, no limit | NO_NEXT_TIME |
/// | 8 | active, neither time, no action left | none (exhausted) |
/// | 9 | deleted, no revision row | REVIVED (it comes before RESPEC) |
/// | 10 | paused, another expression | RESPEC (it comes before PAUSED) |
/// | 11 | active, only the nominal time missing | NO_NEXT_TIME (the `OR` of the two times) |
/// | 12 | active, only the actual time missing | NO_NEXT_TIME |
/// | 13 | active, neither time, five actions left | NO_NEXT_TIME |
/// | 14 | active, both times, no action left | none |
/// | 15 | head revision 2 equal to the trigger, revision 1 another expression | none (the lookup reads the HEAD revision) |
/// | 16 | head revision 2 another expression, revision 1 equal | RESPEC |
///
/// With `converged` every trigger gets the schedule of class 0. Unless it is `converged`, fifty triggers that are not cron
/// triggers (`event`, with no schedule and no business here) are written too: the view must not list them.
async fn seed(pool: &PgPool, triggers: i64, converged: bool) {
    for table in ["workflow_triggers", "schedules", "schedule_revisions"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} DISABLE TRIGGER USER"
        )))
        .execute(pool)
        .await
        .expect("disable the triggers of the table");
    }
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query("SET session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("replica role");
    sqlx::query(
        "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
         SELECT gen_random_uuid(), md5((n % 200)::text)::uuid, md5('namespace')::uuid, md5('workflow')::uuid, \
                md5('version')::uuid, 'cron', (n % 60)::text || ' * * * *', 'UTC' \
           FROM generate_series(1, $1) AS n",
    )
    .bind(triggers)
    .execute(&mut *conn)
    .await
    .expect("the triggers");
    sqlx::query(
        "INSERT INTO schedules (tenant_id, namespace_id, id, name, origin, trigger_id, state, head_revision, state_version, \
                                authorized_by, remaining_actions, next_action_at, next_actual_at, created_at, updated_at, deleted_at) \
         SELECT s.tenant_id, s.namespace_id, gen_random_uuid(), 'trigger-' || replace(s.id::text, '-', ''), 'TRIGGER', s.id, \
                CASE WHEN s.c IN (2, 9) THEN 'DELETED' WHEN s.c IN (6, 10) THEN 'PAUSED' ELSE 'ACTIVE' END, \
                CASE WHEN s.c IN (15, 16) THEN 2 ELSE 1 END, 1 + s.c, 'system:engine', \
                CASE WHEN s.c IN (8, 14) THEN 0 WHEN s.c = 13 THEN 5 END, \
                CASE WHEN s.c IN (7, 8, 11, 13) THEN NULL ELSE now() + interval '1 hour' END, \
                CASE WHEN s.c IN (7, 8, 12, 13) THEN NULL ELSE now() + interval '1 hour' END, \
                now(), now(), CASE WHEN s.c IN (2, 9) THEN now() END \
           FROM (SELECT t.*, CASE WHEN $1 THEN 0 ELSE (row_number() OVER (ORDER BY t.id) % 17)::int END AS c \
                   FROM workflow_triggers t WHERE t.kind = 'cron') s \
          WHERE s.c <> 1",
    )
    .bind(converged)
    .execute(&mut *conn)
    .await
    .expect("the schedules");
    // the class of a schedule is its state version less one
    sqlx::query(
        "INSERT INTO schedule_revisions (tenant_id, schedule_id, revision, spec, policies, action, input_digest, actor, request_id, created_at) \
         SELECT s.tenant_id, s.id, 1, \
                jsonb_build_object('cron', jsonb_build_array(CASE WHEN s.state_version IN (4, 11, 16) THEN '5 5 5 5 5' ELSE t.cron END), \
                                   'timezone', CASE WHEN s.state_version = 5 THEN 'Europe/Paris' ELSE t.timezone END), \
                '{}'::jsonb, '{}'::jsonb, NULL, 'system:engine', NULL, now() \
           FROM schedules s JOIN workflow_triggers t ON t.id = s.trigger_id \
          WHERE s.state_version NOT IN (6, 10)",
    )
    .execute(&mut *conn)
    .await
    .expect("the revisions");
    // the second revision of the two classes whose head is the second: class 15 (state version 16) equal to the trigger,
    // class 16 (state version 17) another expression, each against a first revision that says the opposite
    sqlx::query(
        "INSERT INTO schedule_revisions (tenant_id, schedule_id, revision, spec, policies, action, input_digest, actor, request_id, created_at) \
         SELECT s.tenant_id, s.id, 2, \
                jsonb_build_object('cron', jsonb_build_array(CASE WHEN s.state_version = 17 THEN '5 5 5 5 5' ELSE t.cron END), \
                                   'timezone', t.timezone), \
                '{}'::jsonb, '{}'::jsonb, NULL, 'system:engine', NULL, now() \
           FROM schedules s JOIN workflow_triggers t ON t.id = s.trigger_id \
          WHERE s.state_version IN (16, 17)",
    )
    .execute(&mut *conn)
    .await
    .expect("the second revisions");
    if !converged {
        sqlx::query(
            "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, match_source, match_type) \
             SELECT gen_random_uuid(), md5((n % 200)::text)::uuid, md5('namespace')::uuid, md5('workflow')::uuid, \
                    md5('version')::uuid, 'event', 'test', 'test.' || n \
               FROM generate_series(1, 50) AS n",
        )
        .execute(&mut *conn)
        .await
        .expect("the triggers that are not cron");
    }
    sqlx::query("RESET session_replication_role")
        .execute(&mut *conn)
        .await
        .expect("reset");
}

/// The plan of the check, as text, for the statistics the tables have now.
async fn plan_of_the_check(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>(
        "EXPLAIN (COSTS OFF) SELECT EXISTS (SELECT 1 FROM schedule_trigger_drift)",
    )
    .fetch_all(pool)
    .await
    .expect("the plan")
    .join("\n")
}

/// What the handover asks under its lock, and how long it took.
async fn the_check(pool: &PgPool) -> (bool, Duration) {
    let started = Instant::now();
    let unconverged: bool = sqlx::query_scalar("SELECT schedule_trigger_unconverged()")
        .fetch_one(pool)
        .await
        .expect("the check");
    (unconverged, started.elapsed())
}

/// The check is an index lookup for each trigger and answers well inside the lock timeout and the statement timeout, for
/// whatever the planner knows about the tables. `statistics` names the state the test has put them in.
async fn the_check_holds(owner: &PgPool, runtime: &PgPool, statistics: &str) {
    let plan = plan_of_the_check(runtime).await;
    assert!(
        !plan.contains("Seq Scan on schedules") && !plan.contains("Seq Scan on schedule_revisions"),
        "{statistics}: the plan scans the schedules again for the triggers:\n{plan}"
    );
    assert!(
        plan.contains("schedules_trigger_uniq"),
        "{statistics}: the plan does not look the schedule up by its trigger:\n{plan}"
    );
    for (who, pool) in [("owner", owner), ("runtime role", runtime)] {
        let (unconverged, took) = the_check(pool).await;
        assert!(
            !unconverged,
            "{statistics}: every trigger has a schedule ({who})"
        );
        // the old plan took 2.85 s here; this one 20 ms: a machine ten times slower still answers in a fifth of this
        assert!(
            took < Duration::from_millis(1_000),
            "{statistics}: the check took {took:?} ({who}), the plan was:\n{plan}"
        );
    }
}

/// `stale` analyzes the big tables while they are nearly empty, before the fleet is written: the planner then knows them as
/// they were, where otherwise it knows nothing of them at all (written in bulk, nobody analyzed them). The singleton is not
/// analyzed in either case, as in a database that has never had a reason to.
async fn the_check_does_not_depend_on_the_statistics(size: i64, stale: bool) {
    let t = triggers().await;
    let (owner, runtime) = (&t.w.db.pool, &t.w.execution_pool);
    freeze_the_statistics(owner).await;
    if stale {
        analyze(owner, &TABLES[..3]).await;
    }
    seed(owner, size, true).await;

    // what a fleet written in bulk looks like to the planner: stale numbers, or none (the singleton `schedule_trigger_handover`
    // is never analyzed: one row, and no more writes than autovacuum waits for)
    let first = if stale {
        "analyzed before the fleet was written"
    } else {
        "never analyzed"
    };
    the_check_holds(owner, runtime, &format!("{size} triggers, {first}")).await;

    // what the benchmark did: the big tables analyzed, the singleton not
    analyze(
        owner,
        &["workflow_triggers", "schedules", "schedule_revisions"],
    )
    .await;
    the_check_holds(
        owner,
        runtime,
        &format!("{size} triggers, the big tables analyzed"),
    )
    .await;

    // everything analyzed: the good case, which must not have got worse
    analyze(owner, &["schedule_trigger_handover"]).await;
    the_check_holds(
        owner,
        runtime,
        &format!("{size} triggers, everything analyzed"),
    )
    .await;
}

#[tokio::test]
async fn the_check_under_the_lock_is_an_index_lookup_for_each_trigger_at_10000_whatever_the_statistics_say()
 {
    the_check_does_not_depend_on_the_statistics(10_000, false).await;
}

#[tokio::test]
async fn the_check_under_the_lock_is_an_index_lookup_for_each_trigger_at_5000_with_stale_statistics()
 {
    the_check_does_not_depend_on_the_statistics(5_000, true).await;
}

/// Every row of the view, as text, in the order of the trigger.
async fn rows_of(pool: &PgPool, from: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(v)::text FROM {from} v ORDER BY v.trigger_id"
    )))
    .fetch_all(pool)
    .await
    .expect("rows")
}

async fn kinds_of(pool: &PgPool, from: &str) -> BTreeMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>(sqlx::AssertSqlSafe(format!(
        "SELECT v.kind, count(*) FROM {from} v GROUP BY v.kind"
    )))
    .fetch_all(pool)
    .await
    .expect("kinds")
    .into_iter()
    .collect()
}

#[tokio::test]
async fn the_view_lists_what_its_previous_definition_listed_for_every_kind_of_difference() {
    let t = triggers().await;
    let owner = &t.w.db.pool;
    seed(owner, 1_700, false).await;
    analyze(owner, &TABLES).await;
    let oracle = format!("({PREVIOUS_DEFINITION})");

    // while the cron triggers are still the legacy pass's
    let (view, before) = (
        rows_of(owner, "schedule_trigger_drift").await,
        rows_of(owner, &oracle).await,
    );
    assert!(!before.is_empty());
    assert_eq!(view, before, "LEGACY: the same rows, columns and kinds");
    let kinds = kinds_of(owner, "schedule_trigger_drift").await;
    assert_eq!(
        kinds,
        BTreeMap::from([
            ("MISSING".to_owned(), 100),
            ("REVIVED".to_owned(), 200),
            ("RESPEC".to_owned(), 500),
            ("PAUSED".to_owned(), 100),
            ("NO_NEXT_TIME".to_owned(), 400),
        ]),
        "every kind is there; a planned schedule, an exhausted one, one with both times and no action left, one whose head \
         revision is equal (the first is not) and an event trigger are not"
    );
    let counted = t
        .store
        .convergence()
        .await
        .expect("convergence")
        .unconverged;
    assert_eq!(counted, 1_300, "the count the gauge reads is the view's");
    let unconverged: bool = sqlx::query_scalar("SELECT schedule_trigger_unconverged()")
        .fetch_one(owner)
        .await
        .expect("the check");
    assert!(unconverged, "and the check says there is something");

    // after the handover a pause is the operator's and is not listed (the change is rolled back: it has no reverse)
    let mut tx = owner.begin().await.expect("a transaction");
    sqlx::query("SELECT set_config('sagat.handover_change', 'handover', true)")
        .execute(&mut *tx)
        .await
        .expect("the guard's switch");
    sqlx::query(
        "UPDATE schedule_trigger_handover SET state = 'HANDED_OVER', changed_by = 'test', changed_at = clock_timestamp(), \
                changed_db_role = 'test'",
    )
    .execute(&mut *tx)
    .await
    .expect("hand over, for this transaction");
    let handed_view: Vec<String> = sqlx::query_scalar(
        "SELECT to_jsonb(v)::text FROM schedule_trigger_drift v ORDER BY v.trigger_id",
    )
    .fetch_all(&mut *tx)
    .await
    .expect("rows");
    let handed_before: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(v)::text FROM {oracle} v ORDER BY v.trigger_id"
    )))
    .fetch_all(&mut *tx)
    .await
    .expect("rows");
    tx.rollback().await.expect("rolled back");
    assert_eq!(handed_view, handed_before, "HANDED_OVER: the same rows");
    assert_eq!(
        handed_view.len(),
        1_200,
        "and the hundred paused schedules of class 6 are not among them"
    );
}
