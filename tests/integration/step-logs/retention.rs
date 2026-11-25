//! Design 041 section 4 and section 2b, task T041-10: the two steps of the purge, the quota reclaims of the purge pass
//! and the publication of the quota-lease counters, at the store and through the scheduler's housekeeping. Rows are
//! asserted by SQL; the streams and lines are written by SQL (the ingest of task T041-6 writes them in production).
//! The store is the world's, which runs as the runtime role under `SAGAT_TEST_AS_RUNTIME_ROLE`.

use std::time::Duration;

use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::maintenance::{MaintenanceStore, StepLogPurge};
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use sqlx::Connection as _;
use uuid::Uuid;

use crate::counted::{recorded, series};
use crate::support::bounded;
use crate::support::engine::{World, world};

const LIMIT: Duration = Duration::from_secs(60);

/// A worker stream of the world's first tenant whose retention ended `ended_ago` (negative: still to come).
async fn stream(w: &World, ended_ago: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "INSERT INTO step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
           instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
         VALUES ($1, gen_random_uuid(), $2, gen_random_uuid(), gen_random_uuid(), 'a', '{{}}', '{{}}', 1, 0, 14, \
           now() - interval '14 days' - interval '{ended_ago}', now() - interval '{ended_ago}') RETURNING stream_id"
    )))
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("a stream")
}

/// `count` lines of `text` bytes each on `stream`, one insert statement (charged to the owner's quota lease).
async fn lines(w: &World, stream: i64, count: i64, text: i32) {
    sqlx::query(
        "INSERT INTO step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind, message) \
         SELECT $1, $2, s, gen_random_uuid(), 'info', 'line', repeat('x', $4) FROM generate_series(1, $3) s",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(stream)
    .bind(count)
    .bind(text)
    .execute(&w.db.pool)
    .await
    .expect("lines");
}

/// U = S + Σ used of the world's first tenant.
async fn usage(w: &World) -> i64 {
    sqlx::query_scalar("SELECT sagat_quota_step_log_usage($1)")
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the usage")
}

/// (`removed_at` is set, `purged_at` is set, lines left) of a stream.
async fn state(w: &World, stream: i64) -> (bool, bool, i64) {
    sqlx::query_as(
        "SELECT removed_at IS NOT NULL, purged_at IS NOT NULL, \
                (SELECT count(*) FROM step_log_lines l WHERE l.tenant_id = s.tenant_id AND l.stream_id = s.stream_id) \
           FROM step_log_streams s WHERE s.stream_id = $1",
    )
    .bind(stream)
    .fetch_one(&w.db.pool)
    .await
    .expect("the stream")
}

/// The two steps of the purge release exactly what the lines were charged: 3 lines of 1,000 bytes charge
/// 3 × (1,000 + 128) = 3,384 bytes, and after the mark and the delete the tenant's usage is 0 again. A stream whose
/// retention has not ended is neither marked nor touched.
#[tokio::test]
async fn a_purge_releases_the_quota_bytes_and_leaves_a_live_stream_alone() {
    bounded::within("purge", LIMIT, async {
        let w = world().await;
        let expired = stream(&w, "1 minute").await;
        let live = stream(&w, "-1 day").await;
        lines(&w, expired, 3, 1_000).await;
        lines(&w, live, 1, 10).await;
        assert_eq!(usage(&w).await, 3 * 1_128 + 138);
        assert_eq!(w.store.step_log_purge_backlog().await.expect("backlog"), 1);
        assert_eq!(
            w.store
                .mark_expired_step_log_streams(500)
                .await
                .expect("mark"),
            1
        );
        assert_eq!(
            state(&w, expired).await,
            (true, false, 3),
            "marked, lines not yet deleted"
        );
        assert_eq!(
            w.store.step_log_purge_backlog().await.expect("backlog"),
            1,
            "removal begun, not purged"
        );
        assert_eq!(
            w.store.purge_step_log_lines(500).await.expect("purge"),
            StepLogPurge::Purged {
                lines: 3,
                streams: 1
            }
        );
        assert_eq!(state(&w, expired).await, (true, true, 0));
        assert_eq!(state(&w, live).await, (false, false, 1));
        assert_eq!(
            usage(&w).await,
            138,
            "the purge released 3,384 bytes, exactly the lines' charge"
        );
        assert_eq!(w.store.step_log_purge_backlog().await.expect("backlog"), 0);
        assert_eq!(
            w.store.purge_step_log_lines(500).await.expect("purge"),
            StepLogPurge::Purged {
                lines: 0,
                streams: 0
            },
            "nothing left to purge"
        );
    })
    .await;
}

/// A stream row an ingest holds (the hold after T7) is skipped by the mark, which never waits for it, and marked by
/// the next call once the ingest has ended.
#[tokio::test]
async fn a_stream_row_held_by_an_ingest_is_skipped_by_the_mark_and_marked_by_the_next_pass() {
    bounded::within("held stream", LIMIT, async {
        let w = world().await;
        let held = stream(&w, "1 minute").await;
        let other = stream(&w, "1 minute").await;
        let mut ingest = w.db.pool.begin().await.expect("begin");
        sqlx::query("SELECT 1 FROM step_log_streams WHERE stream_id = $1 FOR UPDATE")
            .bind(held)
            .execute(&mut *ingest)
            .await
            .expect("the ingest holds the stream row");
        assert_eq!(
            w.store
                .mark_expired_step_log_streams(500)
                .await
                .expect("mark"),
            1
        );
        assert_eq!(
            state(&w, held).await.0,
            false,
            "the held stream was skipped"
        );
        assert_eq!(state(&w, other).await.0, true);
        ingest.commit().await.expect("the ingest ends");
        assert_eq!(
            w.store
                .mark_expired_step_log_streams(500)
                .await
                .expect("mark"),
            1
        );
        assert_eq!(state(&w, held).await.0, true, "marked by the next pass");
    })
    .await;
}

/// A removed stream of 1,200 lines is purged over three transactions of at most 500 lines; `removed_at` stays the
/// first mark's, and `purged_at` is set by the transaction that deleted the last line.
#[tokio::test]
async fn a_stream_purged_over_several_transactions_keeps_removed_at_from_the_first() {
    bounded::within("several", LIMIT, async {
        let w = world().await;
        let s = stream(&w, "1 minute").await;
        lines(&w, s, 1_200, 1).await;
        assert_eq!(
            w.store
                .mark_expired_step_log_streams(500)
                .await
                .expect("mark"),
            1
        );
        let removed_at = || async {
            sqlx::query_scalar::<_, time::OffsetDateTime>(
                "SELECT removed_at FROM step_log_streams WHERE stream_id = $1",
            )
            .bind(s)
            .fetch_one(&w.db.pool)
            .await
            .expect("removed_at")
        };
        let first = removed_at().await;
        let mut gone = Vec::new();
        for _ in 0..3 {
            gone.push(w.store.purge_step_log_lines(500).await.expect("purge"));
            assert_eq!(removed_at().await, first, "removed_at is the first mark's");
        }
        assert_eq!(
            gone,
            vec![
                StepLogPurge::Purged {
                    lines: 500,
                    streams: 0
                },
                StepLogPurge::Purged {
                    lines: 500,
                    streams: 0
                },
                StepLogPurge::Purged {
                    lines: 200,
                    streams: 1
                },
            ]
        );
        assert_eq!(state(&w, s).await, (true, true, 0));
        assert_eq!(usage(&w).await, 0);
    })
    .await;
}

/// A release that would take the tenant's usage below zero is refused at the purge's commit: the purge answers
/// `Underflow`, nothing is deleted, and the pass counts it in `step_log_purge_failures_total{cause="underflow"}`.
/// The usage is made inconsistent by hand (S lowered by the owner), as only a broken accounting would.
#[test]
fn a_purge_whose_release_would_underflow_is_refused_and_counted() {
    let page = recorded(|_| async {
        let w = world().await;
        let s = stream(&w, "1 minute").await;
        lines(&w, s, 2, 100).await;
        sqlx::query(
            "UPDATE tenant_retained_usage SET step_log_bytes = step_log_bytes - 1000000 WHERE tenant_id = $1",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("a broken accounting");
        assert_eq!(
            w.store
                .mark_expired_step_log_streams(500)
                .await
                .expect("mark"),
            1
        );
        assert_eq!(
            w.store.purge_step_log_lines(500).await.expect("purge"),
            StepLogPurge::Underflow
        );
        assert_eq!(
            state(&w, s).await,
            (true, false, 2),
            "the purge was rolled back"
        );
        w.scheduler.housekeeping().await;
        assert_eq!(state(&w, s).await, (true, false, 2));
    });
    assert_eq!(
        series(
            &page,
            "step_log_purge_failures_total",
            &[("cause", "underflow")]
        ),
        Some(1.0),
        "{page}"
    );
}

/// The housekeeping pass marks, deletes over as many transactions as it needs within its bounds, and counts the lines
/// it purged and the backlog it found at its start.
#[test]
fn the_housekeeping_purges_in_rounds_and_counts_the_lines_and_the_backlog() {
    let page = recorded(|_| async {
        let w = world().await;
        let s = stream(&w, "1 minute").await;
        lines(&w, s, 1_201, 1).await;
        stream(&w, "-1 day").await;
        w.scheduler.housekeeping().await;
        assert_eq!(state(&w, s).await, (true, true, 0));
        assert_eq!(usage(&w).await, 0);
    });
    assert_eq!(
        series(&page, "step_log_purged_lines_total", &[]),
        Some(1_201.0),
        "{page}"
    );
    assert_eq!(
        series(&page, "step_log_purge_backlog", &[]),
        Some(1.0),
        "{page}"
    );
}

/// The quota-lease reclaim of the purge pass (ported from the prototype's `the_idle_reclaim_folds_and_deletes_only_
/// leases_idle_past_the_interval` and `the_lease_of_an_ended_backend_is_reclaimed_at_once`): an idle quota lease and
/// one of an ended backend are folded and deleted, a quota lease charged a moment ago is kept; a tenant whose step-log
/// fence a test transaction holds is skipped and counted, and reclaimed by the next pass.
#[tokio::test]
async fn the_pass_reclaims_idle_and_ended_quota_leases_and_skips_a_fenced_tenant_until_the_next_pass()
 {
    bounded::within("reclaim", LIMIT, async {
        let w = world().await;
        let tenant = w.admin.tenant_id().as_uuid();
        let s = stream(&w, "-1 day").await;
        let mut idle = sqlx::PgConnection::connect(&w.db.url).await.expect("an idle writer");
        let mut live = sqlx::PgConnection::connect(&w.db.url).await.expect("a live writer");
        let mut ended = sqlx::PgConnection::connect(&w.db.url).await.expect("a writer that ends");
        for (conn, first) in [(&mut idle, 1_i64), (&mut live, 10), (&mut ended, 20)] {
            sqlx::query(
                "INSERT INTO step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind, message) \
                 VALUES ($1, $2, $3, gen_random_uuid(), 'info', 'line', 'x')",
            )
            .bind(tenant)
            .bind(s)
            .bind(first)
            .execute(conn)
            .await
            .expect("a charge into a new quota lease");
        }
        let idle_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut idle).await.expect("pid");
        let ended_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut ended).await.expect("pid");
        sqlx::query("UPDATE step_log_quota_leases SET charged_at = clock_timestamp() - interval '11 minutes' WHERE writer = $1")
            .bind(idle_pid)
            .execute(&w.db.pool)
            .await
            .expect("age the idle quota lease");
        ended.close().await.expect("the backend ends");
        bounded::within("the backend is gone", LIMIT, async {
            while sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)")
                .bind(ended_pid)
                .fetch_one(&w.db.pool)
                .await
                .expect("pg_stat_activity")
            {
                tokio::task::yield_now().await;
            }
        })
        .await;
        let before = usage(&w).await;
        // The step-log fence held exclusively, as a change of the limit holds it: the tenant is skipped and counted.
        let mut fence = w.db.pool.begin().await.expect("begin");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('quota-policy:tenant_step_log_bytes:' || $1::text, 0))")
            .bind(tenant)
            .execute(&mut *fence)
            .await
            .expect("hold the fence");
        let skipped = w.store.reclaim_idle_step_log_leases(100).await.expect("the reclaim");
        assert_eq!((skipped.tenants, skipped.leases, skipped.skipped), (0, 0, 1));
        fence.rollback().await.expect("release the fence");
        let done = w.store.reclaim_idle_step_log_leases(100).await.expect("the reclaim");
        assert_eq!((done.tenants, done.leases, done.skipped), (1, 2, 0), "the idle and the ended quota leases");
        let writers: Vec<i32> = sqlx::query_scalar("SELECT writer FROM step_log_quota_leases WHERE tenant_id = $1")
            .bind(tenant)
            .fetch_all(&w.db.pool)
            .await
            .expect("the quota leases");
        let live_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut live).await.expect("pid");
        assert_eq!(writers, vec![live_pid], "only the live writer's quota lease is kept");
        assert_eq!(usage(&w).await, before, "a reclaim moves room, never usage");
        let nothing = w.store.reclaim_idle_step_log_leases(100).await.expect("the reclaim");
        assert_eq!((nothing.tenants, nothing.leases, nothing.skipped), (0, 0, 0));
    })
    .await;
}

/// Three writers' first charges: three refills, each advancing the database's `refill` counter once. The writers are
/// returned open, so that the pass's ended-backend reclaim leaves their quota leases alone while the test reads them.
async fn three_refills(w: &World, stream: i64, first: i64) -> Vec<sqlx::PgConnection> {
    let mut writers = Vec::new();
    for sequence in [first, first + 1, first + 2] {
        let mut writer = sqlx::PgConnection::connect(&w.db.url)
            .await
            .expect("a writer");
        sqlx::query(
            "INSERT INTO step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind, message) \
             VALUES ($1, $2, $3, gen_random_uuid(), 'info', 'line', 'x')",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(stream)
        .bind(sequence)
        .execute(&mut writer)
        .await
        .expect("a refill");
        writers.push(writer);
    }
    writers
}

fn scheduler(
    w: &World,
    placement: Placement,
) -> Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy> {
    Scheduler::new(
        w.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement)
}

/// The replica that holds shard 0 publishes the database's counters: its first reading only remembers them, each later
/// pass adds what they rose by, once (three refills between two passes add 3 to `kind="refill"`, and a third pass adds
/// nothing), and it sets the quota-lease rows gauge (3 rows); once it has lost the shard, it sets that gauge to 0.
#[test]
fn the_holder_of_shard_zero_publishes_each_rise_of_the_counters_once_and_clears_its_gauge_when_it_loses_the_shard()
 {
    let midway = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let seen = std::rc::Rc::clone(&midway);
    let page = recorded(|metrics| async move {
        let w = world().await;
        let s = stream(&w, "-1 day").await;
        let placement = Placement::new(format!("step-logs-{}", Uuid::now_v7()));
        let keeper = Keeper::new(
            w.store.clone(),
            placement.clone(),
            KeeperSettings {
                ttl: Duration::from_secs(3_600),
            },
        );
        keeper
            .round()
            .await
            .expect("the shards, shard 0 among them");
        let holder = scheduler(&w, placement);
        holder.housekeeping().await;
        let _writers = three_refills(&w, s, 1).await;
        holder.housekeeping().await;
        holder.housekeeping().await;
        *seen.borrow_mut() = metrics.render();
        keeper.leave().await;
        holder.housekeeping().await;
    });
    let midway = midway.borrow().clone();
    assert_eq!(
        series(&midway, "step_log_quota_lease_rows", &[]),
        Some(3.0),
        "the three writers' quota leases: {midway}"
    );
    assert_eq!(
        series(&page, "step_log_quota_refills_total", &[("kind", "refill")]),
        Some(3.0),
        "three refills, each counted once: {page}"
    );
    assert_eq!(
        series(&page, "step_log_quota_lease_rows", &[]),
        Some(0.0),
        "the replica that lost shard 0 cleared its gauge: {page}"
    );
}

/// A replica that does not hold shard 0 publishes nothing of the counters, however they rise.
#[test]
fn a_replica_without_shard_zero_publishes_no_counter() {
    let page = recorded(|_| async {
        let w = world().await;
        let s = stream(&w, "-1 day").await;
        let bystander = scheduler(&w, Placement::new(format!("bystander-{}", Uuid::now_v7())));
        bystander.housekeeping().await;
        let _writers = three_refills(&w, s, 1).await;
        bystander.housekeeping().await;
    });
    for kind in ["refill", "reclaim", "idle", "ended"] {
        assert_eq!(
            series(&page, "step_log_quota_refills_total", &[("kind", kind)]),
            None,
            "{page}"
        );
    }
    assert!(!page.contains("step_log_quota_lease_rows"), "{page}");
}
