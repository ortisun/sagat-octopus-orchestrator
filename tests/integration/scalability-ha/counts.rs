//! The quota of active runs is read from counters the database keeps with the runs, so a start does
//! not cost what the waiting runs cost (FR-010-003): the counters must always be the count, the quota
//! must still be exact, and the runtime role must be able to keep them without being able to write them.

use sagat_application::error::AppError;
use sagat_application::runs::RunOperation;
use sagat_domain::quota::QuotaResource;
use serde_json::json;

use crate::passes::{scheduler, sleepers};
use crate::support::engine::{World, chain, ctx, world};
use sagat_scheduler::SchedulerSettings;

/// `(namespace counted from the runs, namespace from the counters)` for every namespace that has either.
async fn both(w: &World) -> Vec<(uuid::Uuid, i64, i64)> {
    sqlx::query_as(
        "SELECT n.namespace_id, coalesce(r.n, 0)::bigint, coalesce(c.n, 0)::bigint FROM \
            (SELECT namespace_id FROM runs UNION SELECT namespace_id FROM run_active_counts) n \
          LEFT JOIN (SELECT namespace_id, count(*) AS n FROM runs WHERE finished_at IS NULL GROUP BY 1) r \
                 USING (namespace_id) \
          LEFT JOIN (SELECT namespace_id, sum(active) AS n FROM run_active_counts GROUP BY 1) c \
                 USING (namespace_id)",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("counts")
}

async fn assert_counted(w: &World, what: &str) {
    for (namespace, counted, kept) in both(w).await {
        assert_eq!(
            counted, kept,
            "{what}: namespace {namespace} has {counted} active runs and the counters say {kept}"
        );
    }
}

#[tokio::test]
async fn the_counters_are_the_count_through_starts_cancels_deadlines_and_completions() {
    let w = world().await;
    assert_counted(&w, "empty").await;

    // Thirty suspended runs of the first tenant, ten of the other's.
    let runs = sleepers(&w, "counted", 30).await;
    let other = w.publish_as_other_tenant(chain("counted-other")).await;
    for _ in 0..10 {
        w.start_as_other_tenant(other, &json!({}))
            .await
            .expect("start");
    }
    assert_counted(&w, "after the starts").await;
    let active: i64 = sqlx::query_scalar("SELECT sum(active)::bigint FROM run_active_counts")
        .fetch_one(&w.db.pool)
        .await
        .expect("sum");
    assert_eq!(active, 40);

    // Some are cancelled (a waiting run ends at once), one has its deadline pass, the rest fire and finish.
    for run in &runs[..5] {
        w.operate_run(*run, RunOperation::Cancel).await;
    }
    assert_counted(&w, "after the cancels").await;
    w.age_run_deadline(runs[5]).await;
    let alone = scheduler(&w, SchedulerSettings::default(), None);
    while alone.tick().await.did_work() {}
    assert_counted(&w, "after the deadline and the timers").await;
    let session = w.session(&["act.after", "act.a", "act.b"]).await;
    loop {
        let tasks = w.poll(&session, 16).await;
        if tasks.is_empty() {
            break;
        }
        for task in &tasks {
            w.complete(&session, task, &json!({}))
                .await
                .expect("complete");
        }
        assert_counted(&w, "while the workers finish runs").await;
    }
    let left: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts WHERE tenant_id = $1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("sum");
    let really: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE tenant_id = $1 AND finished_at IS NULL",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(left, really);
    // The other tenant's ten were never touched.
    let theirs: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(active), 0)::bigint FROM run_active_counts WHERE tenant_id = $1",
    )
    .bind(w.other_admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("sum");
    assert_eq!(theirs, 10);
}

#[tokio::test]
async fn deleting_an_unfinished_run_and_opening_a_finished_one_keep_the_count() {
    // Nothing in the platform does either; the database keeps the count whatever does.
    let w = world().await;
    let runs = sleepers(&w, "counted-edge", 3).await;
    w.operate_run(runs[0], RunOperation::Cancel).await;
    assert_counted(&w, "one cancelled").await;
    // A correction of the time a run finished at changes nothing.
    sqlx::query("UPDATE runs SET finished_at = finished_at + interval '1 second' WHERE id = $1")
        .bind(runs[0].as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("correct the time");
    assert_counted(&w, "a finish time corrected").await;
    // The counters never go below zero, whatever is done to them.
    sqlx::query("UPDATE run_active_counts SET active = 0")
        .execute(&w.db.pool)
        .await
        .expect("empty the counters");
    sqlx::query("UPDATE runs SET status = 'CANCELLED', finished_at = now() WHERE id = $1")
        .bind(runs[1].as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("finish a run");
    let lowest: i64 =
        sqlx::query_scalar("SELECT coalesce(min(active), 0)::bigint FROM run_active_counts")
            .fetch_one(&w.db.pool)
            .await
            .expect("min");
    assert!(lowest >= 0, "a counter went to {lowest}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_quota_is_still_exact_when_starts_race_for_the_last_places() {
    let w = world().await;
    let (wf, _) = w.publish(chain("raced")).await;
    // Ten places in the namespace (the operator's ceiling is the default, 10 000).
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            10,
            &ctx(),
        )
        .await
        .expect("limit");
    let world = &w;
    let starts = futures_util::future::join_all((0..40).map(|i| async move {
        world
            .start_with(
                wf,
                &json!({ "i": i }),
                &format!("raced-key-{i:04}-xx"),
                None,
            )
            .await
    }))
    .await;
    let admitted = starts.iter().filter(|r| r.is_ok()).count();
    let refused = starts
        .iter()
        .filter(|r| {
            matches!(
                r,
                Err(AppError::QuotaExceeded(_)) | Err(AppError::NotAllowed(_))
            )
        })
        .count();
    assert_eq!(admitted, 10, "exactly the places there are: {starts:?}");
    assert_eq!(admitted + refused, 40, "{starts:?}");
    assert_counted(&w, "after the race").await;
    // A place that frees up is taken again, and only one.
    w.operate_run(
        *starts.iter().find_map(|r| r.as_ref().ok()).expect("one"),
        RunOperation::Cancel,
    )
    .await;
    let again = futures_util::future::join_all((0..5).map(|i| async move {
        world
            .start_with(
                wf,
                &json!({ "again": i }),
                &format!("again-key-{i:04}-xx"),
                None,
            )
            .await
    }))
    .await;
    assert_eq!(again.iter().filter(|r| r.is_ok()).count(), 1, "{again:?}");
    assert_counted(&w, "after the freed place").await;
}

#[tokio::test]
async fn the_runtime_role_keeps_the_counters_by_writing_runs_and_cannot_write_them_itself() {
    use sagat_application::clock::SystemClock;
    use sagat_application::execution::{DefinitionCache, Engine};
    use sagat_application::runs::{RunService, StartRun};
    use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
    use sagat_infra_postgres::{PgRunStore, PoolConfig, connect};

    use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD};

    let w = world().await;
    let (wf, _) = w.publish(chain("least-privilege")).await;
    let config = PoolConfig::new(w.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD));
    let pool = connect(&config).await.expect("connect as the runtime role");
    let cache = DefinitionCache::new(std::sync::Arc::new(JsonIrCodec), 8);
    let store = PgRunStore::new(pool.clone(), cache.clone());
    let service = RunService::new(store, SystemClock, Engine::new(JsonEvaluator), cache);
    let input = b"{}".to_vec();
    for i in 0..4 {
        service
            .start(
                &w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: wf,
                    version: None,
                    input: &input,
                    correlation_id: None,
                    deadline: None,
                    key: &format!("least-privilege-{i:04}-xx"),
                },
                &ctx(),
            )
            .await
            .expect("start as the runtime role");
    }
    assert_counted(&w, "after four starts by the runtime role").await;
    let kept: i64 = sqlx::query_scalar("SELECT sum(active)::bigint FROM run_active_counts")
        .fetch_one(&pool)
        .await
        .expect("the runtime role reads the counters");
    assert_eq!(kept, 4);
    for statement in [
        "UPDATE run_active_counts SET active = 0",
        "DELETE FROM run_active_counts",
        "INSERT INTO run_active_counts VALUES (gen_random_uuid(), gen_random_uuid(), 0, 99)",
    ] {
        match sqlx::query(statement).execute(&pool).await {
            Err(sqlx::Error::Database(e)) => {
                assert_eq!(e.code().as_deref(), Some("42501"), "{statement}")
            }
            other => panic!("{statement}: expected a permission error, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_temporary_table_cannot_stand_in_for_the_counters_or_borrow_the_function() {
    use sagat_application::clock::SystemClock;
    use sagat_application::execution::{DefinitionCache, Engine};
    use sagat_application::runs::{RunService, StartRun};
    use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
    use sagat_infra_postgres::{PgRunStore, PoolConfig, connect};

    use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD};

    let w = world().await;
    let (wf, _) = w.publish(chain("temp-shadow")).await;
    let mut config = PoolConfig::new(w.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD));
    config.max_connections = 2;
    let pool = connect(&config).await.expect("connect as the runtime role");

    // The function runs with its owner's rights, so what it names must be what the migration meant.
    let (definer, settings): (bool, Vec<String>) = sqlx::query_as(
        "SELECT prosecdef, coalesce(proconfig, '{}') FROM pg_proc WHERE proname = 'run_active_counts_track'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("the function");
    assert!(definer);
    assert_eq!(
        settings,
        vec!["search_path=pg_catalog, public, pg_temp".to_owned()],
        "pg_temp is named, and last"
    );

    // 1. The role cannot attach the function to a table of its own: no EXECUTE for it.
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query("CREATE TEMP TABLE borrowed (LIKE runs INCLUDING DEFAULTS)")
        .execute(&mut *conn)
        .await
        .expect("a temporary table of its own");
    match sqlx::query(
        "CREATE TRIGGER borrow AFTER INSERT ON pg_temp.borrowed FOR EACH ROW EXECUTE FUNCTION run_active_counts_track()",
    )
    .execute(&mut *conn)
    .await
    {
        Err(sqlx::Error::Database(e)) => assert_eq!(e.code().as_deref(), Some("42501")),
        other => panic!("expected a permission error, got {other:?}"),
    }
    drop(conn);

    // 2. A temporary table with the counters' name, in the session that fires the trigger, is not the
    //    table the function writes (its check would refuse any row, so a start would fail if it were).
    let (a, b) = (
        pool.acquire().await.expect("a"),
        pool.acquire().await.expect("b"),
    );
    for mut conn in [a, b] {
        sqlx::query(
            "CREATE TEMP TABLE run_active_counts (tenant_id uuid, namespace_id uuid, slot smallint, \
             active bigint CHECK (active < 0), PRIMARY KEY (tenant_id, namespace_id, slot))",
        )
        .execute(&mut *conn)
        .await
        .expect("a temporary table with the counters' name");
    }
    let cache = DefinitionCache::new(std::sync::Arc::new(JsonIrCodec), 8);
    let service = RunService::new(
        PgRunStore::new(pool.clone(), cache.clone()),
        SystemClock,
        Engine::new(JsonEvaluator),
        cache,
    );
    let input = b"{}".to_vec();
    for i in 0..3 {
        service
            .start(
                &w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: wf,
                    version: None,
                    input: &input,
                    correlation_id: None,
                    deadline: None,
                    key: &format!("temp-shadow-{i:04}-xxxx"),
                },
                &ctx(),
            )
            .await
            .expect("the start reaches the real counters, not the temporary table");
    }
    assert_counted(
        &w,
        "after three starts with a table of the same name in the session",
    )
    .await;
    let kept: i64 =
        sqlx::query_scalar("SELECT coalesce(sum(active), 0)::bigint FROM public.run_active_counts")
            .fetch_one(&w.db.pool)
            .await
            .expect("the counters");
    assert_eq!(kept, 3);
}
