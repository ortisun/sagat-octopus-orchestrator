//! The queue of a gated step against PostgreSQL: the grant routine (T26, T022-097; ADR-0089 decisions 3 and 4) and the
//! timeout routine (T27, T022-098; ADR-0089 decision 5), one each for both gates. Both rest on the readiness gate as it
//! is built (ADR-0094, proposed, O1171 pending). Every run is checked by `verify_run`.
//!
//! Races are held by locks and observed through `pg_stat_activity`, never by sleeps.

use sagat_application::runs::RunOperation;
use sagat_domain::execution::RunId;
use sagat_domain::sharding::ShardSet;
use sagat_scheduler::SchedulerSettings;
use serde_json::{Value, json};

use super::readiness::{granted, hints, queued, rows, serial, step};
use crate::support::engine::{World, activity, document, parallel_node, wait_node, with};
use crate::support::flow_control::{age_gate_deadline, gated_world, publish, publish_as};
use crate::support::scale::{TTL, keeper, scheduler};

/// `concurrency` of group `group` on `input.accountId` with `limit`.
fn permit(node: Value, group: &str, limit: u32) -> Value {
    with(
        node,
        "concurrency",
        json!({"group": group, "key": "input.accountId", "limit": limit}),
    )
}

/// `rateLimit` of group `group` on `input.accountId`, `limit` an hour (a granted slot never ages out here: its lease
/// instant is the pass's, T022-074).
fn slot(node: Value, group: &str, limit: u32) -> Value {
    with(
        node,
        "rateLimit",
        json!({"group": group, "key": "input.accountId", "limit": limit, "per": "hour"}),
    )
}

async fn journal_has(w: &World, run: RunId, event: &str) -> usize {
    w.journal(run).await.iter().filter(|e| *e == event).count()
}

/// Waits until `n` backends of this database wait for a lock (the barrier of a held-lock race).
async fn lock_waiters(w: &World, n: i64) {
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
             AND wait_event_type = 'Lock' AND state = 'active'",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("activity");
        if waiting >= n {
            return;
        }
        tokio::task::yield_now().await;
    }
}

// ------------------------------------------------------------------------------------------------- grant (T26)

/// A released permit lets the next pass grant the oldest waiter, announce it and lease it; the younger one waits, and a
/// key with nothing grantable rests until the next release.
#[tokio::test]
async fn a_released_permit_is_granted_to_the_oldest_waiter_by_the_next_pass() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("grant-oldest", "g", vec![serial(activity("g"))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let older = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let younger = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    queued(&w, older, "g", (true, false)).await;
    queued(&w, younger, "g", (true, false)).await;
    assert_eq!(w.scheduler.tick().await.gates_granted, 0, "nothing is free");

    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    granted(&w, older, "g", (true, false)).await;
    assert_eq!(hints(&w, older, "g").await, 1, "the grant announces it");
    queued(&w, younger, "g", (true, false)).await;
    let rested: Option<time::OffsetDateTime> = sqlx::query_scalar(
        "SELECT next_eligible_at FROM concurrency_semaphores WHERE tenant_id = $1",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("semaphore");
    assert!(rested.is_none(), "nothing more is grantable: the key rests");
    assert_eq!(w.scheduler.tick().await.gates_granted, 0);

    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, older, "the worker gets the granted step");
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    granted(&w, younger, "g", (true, false)).await;
    for run in [holder, older] {
        w.assert_consistent(run).await;
    }
}

/// A saturated second key is granted at its limit and no further, beside the first key's grant.
#[tokio::test]
async fn a_saturated_second_key_is_granted_at_its_limit() {
    let w = gated_world().await;
    let one = publish(
        &w,
        document(
            "key-limit-one",
            "g",
            vec![permit(activity("g"), "acct-one", 1)],
        ),
    )
    .await;
    let two = publish(
        &w,
        document(
            "key-limit-two",
            "g",
            vec![permit(activity("g"), "acct-two", 2)],
        ),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let mut holders = Vec::new();
    for _ in 0..2 {
        holders.push(w.start(two, &json!({"accountId": "b"})).await);
    }
    holders.push(w.start(one, &json!({"accountId": "a"})).await);
    let mut waiters_two = Vec::new();
    for _ in 0..3 {
        waiters_two.push(w.start(two, &json!({"accountId": "b"})).await);
    }
    let waiter_one = w.start(one, &json!({"accountId": "a"})).await;
    for task in w.poll(&session, 8).await {
        assert!(holders.contains(&task.run_id));
        w.complete(&session, &task, &json!({})).await.expect("done");
    }
    let report = w.scheduler.tick().await;
    assert_eq!(
        report.gates_granted, 3,
        "two of the second key, one of the first: {report:?}"
    );
    granted(&w, waiter_one, "g", (true, false)).await;
    granted(&w, waiters_two[0], "g", (true, false)).await;
    granted(&w, waiters_two[1], "g", (true, false)).await;
    queued(&w, waiters_two[2], "g", (true, false)).await;
    let held: i64 = sqlx::query_scalar(
        "SELECT holders::bigint FROM concurrency_semaphores WHERE group_name = 'acct-two'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("semaphore");
    assert_eq!(held, 2, "never more than the limit");
}

/// ADR-0089 decision 4: the head of a key refused only by its other gate keeps its room, so a younger waiter of the
/// same key is not granted ahead of it; once its other gate frees, the head is granted first.
#[tokio::test]
async fn a_dual_gated_head_is_not_starved_by_a_younger_waiter() {
    let w = gated_world().await;
    let dual = publish(
        &w,
        document(
            "dual-head",
            "d",
            vec![slot(permit(activity("d"), "acct-k", 2), "acct-r", 1)],
        ),
    )
    .await;
    let single = publish(
        &w,
        document(
            "single-younger",
            "s",
            vec![permit(activity("s"), "acct-k", 2)],
        ),
    )
    .await;
    let session = w.session(&["act.s"]).await;
    let account = json!({"accountId": "acct-1"});
    // the slot's holder: granted, never leased
    let slot_holder = w.start(dual, &account).await;
    granted(&w, slot_holder, "d", (true, true)).await;
    let permit_holder = w.start(single, &account).await;
    granted(&w, permit_holder, "s", (true, false)).await;
    let head = w.start(dual, &account).await;
    queued(&w, head, "d", (true, true)).await;
    let younger = w.start(single, &account).await;
    queued(&w, younger, "s", (true, false)).await;

    // a permit frees: the head has room on its permit but not on its slot, and the younger waiter is held behind it
    let task = w.one_task_of(&session, "act.s").await;
    assert_eq!(task.run_id, permit_holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    let report = w.scheduler.tick().await;
    assert_eq!(
        report.gates_granted, 0,
        "room is reserved for the head: {report:?}"
    );
    queued(&w, head, "d", (true, true)).await;
    queued(&w, younger, "s", (true, false)).await;

    // the slot frees (its unleased holder is cancelled and returns it): the head is granted, then the younger one
    w.operate_run(slot_holder, RunOperation::Cancel).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 2, "{report:?}");
    granted(&w, head, "d", (true, true)).await;
    granted(&w, younger, "s", (true, false)).await;
    let (head_at, younger_at): (time::OffsetDateTime, time::OffsetDateTime) = sqlx::query_as(
        "SELECT (SELECT acquired_at FROM concurrency_holders WHERE run_id = $1), \
                (SELECT acquired_at FROM concurrency_holders WHERE run_id = $2)",
    )
    .bind(head.as_uuid())
    .bind(younger.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("holders");
    assert!(head_at <= younger_at, "the head first");
    w.assert_consistent(slot_holder).await;
}

/// ADR-0089 decision 3: keys are served fairly between tenants and bounded per pass; with more keys than one pass's
/// budget, every key of either tenant is granted within two passes.
#[tokio::test]
async fn many_keys_and_tenants_beyond_one_pass_are_each_granted_within_two_passes() {
    let w = gated_world().await;
    let doc = document("many-keys", "g", vec![serial(activity("g"))]);
    let mine = publish(&w, doc.clone()).await;
    let theirs = publish_as(&w, &w.other_admin, doc).await;
    let session = w.session(&["act.g"]).await;
    let mut waiters = Vec::new();
    for (tenant, keys) in [(0, 4), (1, 2)] {
        for key in 0..keys {
            let input = json!({"accountId": format!("acct-{key}")});
            let mut pair = Vec::new();
            for _ in 0..2 {
                pair.push(if tenant == 0 {
                    w.start(mine, &input).await
                } else {
                    w.start_as_other_tenant(theirs, &input)
                        .await
                        .expect("started")
                });
            }
            let (holder, waiter) = (pair[0], pair[1]);
            waiters.push((tenant, holder, waiter));
        }
    }
    // the holders of the first tenant finish through its worker; the second tenant's holders are cancelled
    for task in w.poll(&session, 16).await {
        w.complete(&session, &task, &json!({})).await.expect("done");
    }
    for (tenant, holder, _) in &waiters {
        if *tenant == 1 {
            w.runs
                .operate(
                    &w.other_admin,
                    *holder,
                    RunOperation::Cancel,
                    "free the key",
                    None,
                    &format!("cancel-{holder}"),
                    &crate::support::engine::ctx(),
                )
                .await
                .expect("cancelled");
        }
    }
    let narrow = scheduler(
        &w,
        SchedulerSettings {
            batch: 4,
            ..SchedulerSettings::default()
        },
        None,
    );
    let first = narrow.tick().await;
    assert_eq!(first.gates_granted, 4, "one pass's budget: {first:?}");
    let mut second_tenant_granted = 0;
    for (tenant, _, waiter) in &waiters {
        if *tenant == 1 && rows_of(&w, *waiter).await.0 == 1 {
            second_tenant_granted += 1;
        }
    }
    assert!(
        second_tenant_granted >= 1,
        "the second tenant is served in the first pass"
    );
    let second = narrow.tick().await;
    assert_eq!(second.gates_granted, 2, "{second:?}");
    for (_, _, waiter) in &waiters {
        assert_eq!(
            rows_of(&w, *waiter).await,
            (1, 0),
            "every key granted within two passes"
        );
    }
}

/// (holders, waiters) of a run's step `g`, whatever its tenant.
async fn rows_of(w: &World, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_holders WHERE run_id = $1 AND node = 'g'), \
                (SELECT count(*) FROM concurrency_waiters WHERE run_id = $1 AND node = 'g')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

/// A replica that lost the key's home shard is refused by its fence and grants nothing; the new owner grants.
#[tokio::test]
async fn a_stale_replica_is_refused_by_the_key_fence() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("fenced-grant", "g", vec![serial(activity("g"))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("alpha");
    let stale = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(alpha.placement().clone()),
    );
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");

    for statement in [
        "UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'",
        "UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'alpha'",
    ] {
        sqlx::query(statement)
            .execute(&w.db.pool)
            .await
            .expect("alpha expired");
    }
    let beta = keeper(&w, "beta", TTL);
    beta.round().await.expect("beta");
    assert_eq!(beta.placement().owned(), ShardSet::all());

    let report = stale.tick().await;
    assert_eq!(report.gates_granted, 0, "{report:?}");
    assert!(report.fenced >= 1, "the fence refused it: {report:?}");
    queued(&w, waiter, "g", (true, false)).await;
    let owner = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(beta.placement().clone()),
    );
    assert_eq!(owner.tick().await.gates_granted, 1);
    granted(&w, waiter, "g", (true, false)).await;
}

// ----------------------------------------------------------------------------------------------- timeout (T27)

/// A waiter past its deadline fails `CONCURRENCY_WAIT_TIMEOUT` through its failure route, with no attempt spent and no
/// row left; a waiter before its deadline is left alone.
#[tokio::test]
async fn a_waiter_past_its_deadline_fails_through_its_route() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "timeout-route",
            "g",
            vec![
                with(serial(activity("g")), "onError", json!("recover")),
                activity("recover"),
            ],
        ),
    )
    .await;
    let _holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let late = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let early = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    age_gate_deadline(&w, late, "g").await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.gate_waits_timed_out, 1, "{report:?}");
    let (status, failure, attempt): (String, Option<String>, i32) = sqlx::query_as(
        "SELECT status, failure_code, attempt FROM steps WHERE run_id = $1 AND node = 'g'",
    )
    .bind(late.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("step");
    assert_eq!(
        (status.as_str(), failure.as_deref(), attempt),
        ("FAILED", Some("CONCURRENCY_WAIT_TIMEOUT"), 0)
    );
    assert_eq!(
        rows(&w, late, "g").await,
        (0, 0, 0, 0),
        "nothing left in a queue"
    );
    assert_eq!(
        step(&w, late, "recover").await.0,
        "READY",
        "the failure route takes it"
    );
    assert_eq!(journal_has(&w, late, "concurrency.wait_timed_out").await, 1);
    w.assert_consistent(late).await;
    queued(&w, early, "g", (true, false)).await;
}

/// FR-022-052's precedence for a step queued for both gates: the gate that refuses it at its deadline times out (rate
/// first when both do) and the other is dequeued.
#[tokio::test]
async fn a_dual_gated_waiter_times_out_on_the_gate_that_refuses_it_rate_first() {
    let w = gated_world().await;
    let dual = publish(
        &w,
        document(
            "dual-timeout",
            "d",
            vec![slot(permit(activity("d"), "acct-k", 1), "acct-r", 1)],
        ),
    )
    .await;
    let permit_only = publish(
        &w,
        document("permit-only", "p", vec![permit(activity("p"), "acct-k", 1)]),
    )
    .await;
    let slot_only = publish(
        &w,
        document("slot-only", "r", vec![slot(activity("r"), "acct-r", 1)]),
    )
    .await;
    for (case, holder_of, expected_timed_out, expected_dequeued, code) in [
        (
            "both refuse",
            Some(dual),
            "rate_limit.wait_timed_out",
            "concurrency.dequeued",
            "RATE_LIMIT_WAIT_TIMEOUT",
        ),
        (
            "only the permit refuses",
            Some(permit_only),
            "concurrency.wait_timed_out",
            "rate_limit.dequeued",
            "CONCURRENCY_WAIT_TIMEOUT",
        ),
        (
            "only the slot refuses",
            Some(slot_only),
            "rate_limit.wait_timed_out",
            "concurrency.dequeued",
            "RATE_LIMIT_WAIT_TIMEOUT",
        ),
    ] {
        let account = json!({"accountId": format!("acct-{}", case.len())});
        let holder = w.start(holder_of.expect("a holder"), &account).await;
        let waiter = w.start(dual, &account).await;
        queued(&w, waiter, "d", (true, true)).await;
        age_gate_deadline(&w, waiter, "d").await;
        let report = w.scheduler.tick().await;
        assert_eq!(report.gate_waits_timed_out, 1, "{case}: {report:?}");
        assert_eq!(
            journal_has(&w, waiter, expected_timed_out).await,
            1,
            "{case}"
        );
        assert_eq!(
            journal_has(&w, waiter, expected_dequeued).await,
            1,
            "{case}"
        );
        let failure: Option<String> =
            sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'd'")
                .bind(waiter.as_uuid())
                .fetch_one(&w.db.pool)
                .await
                .expect("step");
        assert_eq!(failure.as_deref(), Some(code), "{case}");
        assert_eq!(rows(&w, waiter, "d").await, (0, 0, 0, 0), "{case}");
        w.assert_consistent(waiter).await;
        w.operate_run(holder, RunOperation::Cancel).await;
    }
}

/// Races of the timeout against a grant (ADR-0089 decision 5), 20 rounds: both reach the waiter's run lock, and the
/// first commit decides. A due waiter is never granted, whichever of the two takes the run first (a grant checks the
/// deadline under the keys' locks); a waiter granted before its deadline is never timed out. Exactly one outcome each.
#[tokio::test]
async fn a_timeout_and_a_grant_of_one_waiter_are_decided_by_the_first_commit() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("timeout-grant-race", "g", vec![serial(activity("g"))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    for round in 0..20 {
        let account = json!({"accountId": format!("race-{round}")});
        let holder = w.start(workflow, &account).await;
        let waiter = w.start(workflow, &account).await;
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, holder);
        w.complete(&session, &task, &json!({}))
            .await
            .expect("the key frees");
        let hold_key = || async {
            let mut key = w.db.pool.begin().await.expect("tx");
            sqlx::query(
                "SELECT 1 FROM concurrency_semaphores WHERE tenant_id = $1 AND waiters > 0 FOR UPDATE",
            )
            .bind(w.admin.tenant_id().as_uuid())
            .execute(&mut *key)
            .await
            .expect("the key held");
            key
        };
        let other = || scheduler(&w, SchedulerSettings::default(), None);
        match round % 3 {
            // the grant takes the run first and waits for the key; the timeout waits for the run
            0 => {
                age_gate_deadline(&w, waiter, "g").await;
                let key = hold_key().await;
                let (grants, timeouts) = (other(), other());
                let grant = tokio::spawn(async move { grants.grant_gates().await });
                lock_waiters(&w, 1).await;
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                lock_waiters(&w, 2).await;
                key.rollback().await.expect("released");
                let (g, t) = (grant.await.expect("grant"), timeout.await.expect("timeout"));
                assert_eq!(g.gates_granted, 0, "a due waiter is never granted: {g:?}");
                assert_eq!(t.gate_waits_timed_out, 1, "{t:?}");
            }
            // the timeout takes the run and waits for the key; the grant skips the locked run
            1 => {
                age_gate_deadline(&w, waiter, "g").await;
                let key = hold_key().await;
                let timeouts = other();
                let timeout = tokio::spawn(async move { timeouts.time_out_gates().await });
                lock_waiters(&w, 1).await;
                let report = w.scheduler.grant_gates().await;
                assert_eq!(
                    report.gates_granted, 0,
                    "the run is the timeout's: {report:?}"
                );
                key.rollback().await.expect("released");
                let t = timeout.await.expect("timeout");
                assert_eq!(t.gate_waits_timed_out, 1, "{t:?}");
            }
            // the grant commits before the deadline: nothing is due afterwards
            _ => {
                let report = w.scheduler.grant_gates().await;
                assert_eq!(report.gates_granted, 1, "{report:?}");
                age_gate_deadline(&w, waiter, "g").await;
                assert_eq!(w.scheduler.time_out_gates().await.gate_waits_timed_out, 0);
            }
        }
        let (acquired, timed_out) = (
            journal_has(&w, waiter, "concurrency.acquired").await,
            journal_has(&w, waiter, "concurrency.wait_timed_out").await,
        );
        assert_eq!(
            acquired + timed_out,
            1,
            "round {round}: exactly one outcome"
        );
        assert_eq!(timed_out == 1, round % 3 != 2, "round {round}");
        let (_, waiters, _, _) = rows(&w, waiter, "g").await;
        assert_eq!(waiters, 0, "round {round}: no waiter row is left");
        let report = w.scheduler.tick().await;
        assert_eq!(
            report.gates_granted + report.gate_waits_timed_out,
            0,
            "decided once: {report:?}"
        );
        w.assert_consistent(waiter).await;
        if acquired == 1 {
            let task = w.one_task(&session).await;
            w.complete(&session, &task, &json!({})).await.expect("done");
        }
    }
}

/// A signal and the timeout of another step of the same run race for the run's lock: whichever commits first, both
/// effects land once and the journal stays consistent.
#[tokio::test]
async fn a_timeout_and_a_signal_on_one_run_both_land_once_in_either_order() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "timeout-signal-race",
            "p",
            vec![
                parallel_node("p", &["w", "g"], "all"),
                wait_node("w", "account.ready", "input.accountId", 3_600_000, None),
                with(
                    permit(activity("g"), "acct-sig", 1),
                    "onError",
                    json!("recover"),
                ),
                activity("recover"),
            ],
        ),
    )
    .await;
    let blocker = publish(
        &w,
        document(
            "timeout-signal-holder",
            "g",
            vec![permit(activity("g"), "acct-sig", 1)],
        ),
    )
    .await;
    for round in 0..4 {
        let account = json!({"accountId": format!("signal-{round}")});
        let _holder = w.start(blocker, &account).await;
        let run = w.start(workflow, &account).await;
        queued(&w, run, "g", (true, false)).await;
        age_gate_deadline(&w, run, "g").await;
        let mut held = w.db.pool.begin().await.expect("tx");
        sqlx::query("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE")
            .bind(run.as_uuid())
            .execute(&mut *held)
            .await
            .expect("the run held");
        let timeout_first = round % 2 == 0;
        let (second_may_go, second_waits) = tokio::sync::oneshot::channel::<()>();
        let mut second_waits = Some(second_waits);
        let timeout_gate = if timeout_first {
            None
        } else {
            second_waits.take()
        };
        let signal_gate = second_waits.take();
        let timeouts = scheduler(&w, SchedulerSettings::default(), None);
        let timeout = async {
            if let Some(gate) = timeout_gate {
                gate.await.expect("go");
            }
            timeouts.time_out_gates().await
        };
        let signal = async {
            if let Some(gate) = signal_gate {
                gate.await.expect("go");
            }
            w.runs
                .signal(
                    &w.admin,
                    run,
                    "account.ready",
                    b"{}",
                    &format!("signal-{round}"),
                    &crate::support::engine::ctx(),
                )
                .await
        };
        let barrier = async {
            lock_waiters(&w, 1).await;
            second_may_go.send(()).expect("second");
            lock_waiters(&w, 2).await;
            held.rollback().await.expect("released");
        };
        let (t, signalled, ()) = tokio::join!(timeout, signal, barrier);
        signalled.expect("signalled");
        assert_eq!(t.gate_waits_timed_out, 1, "round {round}: {t:?}");
        assert_eq!(journal_has(&w, run, "concurrency.wait_timed_out").await, 1);
        assert_eq!(journal_has(&w, run, "wait.satisfied").await, 1);
        w.assert_consistent(run).await;
    }
}
