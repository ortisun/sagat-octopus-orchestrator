//! Activity quota locks are ordered across a multi-activity claim and fenced against policy edits.

use crate::support::engine::{activity, ctx, document, world};
use sagat_application::execution::RunTx;
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::ActivityName;
use serde_json::json;
use std::time::Duration;

async fn wait_for_policy_waiter(pool: &sqlx::PgPool, key: &str, mode: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS ( \
                     SELECT 1 FROM pg_locks \
                      WHERE locktype = 'advisory' AND objsubid = 1 \
                        AND NOT granted AND mode = $2 \
                        AND database = (SELECT oid FROM pg_database \
                                         WHERE datname = current_database()) \
                        AND ((classid::bigint << 32) | objid::bigint) = \
                            hashtextextended($1, 0) \
                 )",
            )
            .bind(key)
            .bind(mode)
            .fetch_one(pool)
            .await
            .expect("inspect policy lock");
            if waiting {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("policy change reached its advisory lock");
}

#[tokio::test]
async fn opposite_node_activity_orders_do_not_abort_a_concurrent_claim() {
    let w = world().await;
    for (name, first, second) in [
        ("order-ab", "act.a", "act.b"),
        ("order-ba", "act.b", "act.a"),
    ] {
        let (wf, _) = w
            .publish(document(
                name,
                "fork",
                vec![
                    json!({"id":"fork","type":"parallel","branches":["one","two"],"join":"all"}),
                    json!({"id":"one","type":"activity","activity":first}),
                    json!({"id":"two","type":"activity","activity":second}),
                ],
            ))
            .await;
        w.start(wf, &json!({})).await;
    }
    // Only a reachable limit is counted under an exclusive lock: without one the claims share
    // their fences and no order of activities could make them wait for each other.
    let context = ctx();
    for name in ["act.a", "act.b"] {
        w.quotas
            .set_activity_limit(
                &w.admin,
                w.namespace.id(),
                ActivityName::parse(name).expect("activity"),
                100,
                &context,
            )
            .await
            .expect("bounded activity");
    }
    let a = w.session(&["act.a", "act.b"]).await;
    let b = w.session(&["act.a", "act.b"]).await;
    // Freeze the first lease persistence after quota admission. With the old node-id order,
    // each transaction owns a different activity count lock here and subsequently waits for the other.
    sqlx::query("CREATE FUNCTION gate_activity_lease() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.status='READY' AND NEW.status='LEASED' THEN PERFORM pg_advisory_xact_lock_shared(910204024); END IF; RETURN NEW; END $$")
        .execute(&w.db.pool).await.expect("gate function");
    sqlx::query("CREATE TRIGGER gate_activity_lease BEFORE UPDATE ON steps FOR EACH ROW EXECUTE FUNCTION gate_activity_lease()")
        .execute(&w.db.pool).await.expect("gate trigger");
    let mut gate = w.db.pool.begin().await.expect("gate");
    sqlx::query("SELECT pg_advisory_xact_lock(910204024)")
        .execute(&mut *gate)
        .await
        .expect("hold gate");
    let release = async {
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                let waiting:i64=sqlx::query_scalar("SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())")
                    .fetch_one(&w.db.pool).await.expect("waiting locks");
                if waiting>=2 {break;}
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("both claimers reached their locks");
        gate.commit().await.expect("release gate");
    };
    let (a, b, ()) = tokio::join!(
        w.workers.poll(&a.principal, a.worker_id, &a.token, 2, None),
        w.workers.poll(&b.principal, b.worker_id, &b.token, 2, None),
        release,
    );
    let a = a.expect("first claim must not deadlock");
    let b = b.expect("second claim must not deadlock");
    assert_eq!(a.len() + b.len(), 4, "every branch gets a committed lease");
    let unique: std::collections::BTreeSet<_> = a
        .iter()
        .chain(&b)
        .map(|t| (t.run_id, t.instance.clone()))
        .collect();
    assert_eq!(unique.len(), 4);
}

#[tokio::test]
async fn a_ceiling_change_waits_for_admission_then_governs_the_next_decision() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("ceiling-fence", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let activity = ActivityName::parse("act.a").expect("activity");
    let mut admitted = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(
        admitted
            .activity_admitted(&activity)
            .await
            .expect("admission")
    );
    let change = w.quotas.grant_ceiling(
        w.admin.tenant_id(),
        QuotaResource::ActivityConcurrency,
        Some(0),
    );
    tokio::pin!(change);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut change)
            .await
            .is_err(),
        "a completed limit change cannot leave an older admission uncommitted"
    );
    // A queued exclusive editor cannot deadlock this transaction when it visits its next
    // activity: its already-held shared ancestor fences are reentrant.
    let next_activity = ActivityName::parse("act.b").expect("activity");
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            admitted.activity_admitted(&next_activity)
        )
        .await
        .expect("existing reader can finish")
        .expect("next admission")
    );
    admitted.commit().await.expect("old decision finished");
    change.await.expect("ceiling applied");
    let mut after = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(
        !after
            .activity_admitted(&activity)
            .await
            .expect("new decision sees zero")
    );
    after.commit().await.expect("finish");
    assert_eq!(
        w.quotas
            .list_activity_limits(&w.admin, w.namespace.id(), &ctx())
            .await
            .expect("list")
            .len(),
        0
    );
}

#[tokio::test]
async fn namespace_and_tenant_defaults_fence_admission_and_take_effect_after_commit() {
    for namespace_level in [true, false] {
        let w = world().await;
        let (wf, _) = w
            .publish(document("default-fence", "a", vec![activity("a")]))
            .await;
        let run = w.start(wf, &json!({})).await;
        let activity = ActivityName::parse("act.a").expect("activity");
        let mut admitted = w
            .store
            .begin(w.admin.tenant_id(), run, Lock::Wait)
            .await
            .expect("begin")
            .expect("run");
        assert!(
            admitted
                .activity_admitted(&activity)
                .await
                .expect("admission")
        );
        let namespace = namespace_level.then_some(w.namespace.id());
        let context = ctx();
        let change = w.quotas.set(
            &w.admin,
            namespace,
            QuotaResource::ActivityConcurrency,
            0,
            &context,
        );
        tokio::pin!(change);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut change)
                .await
                .is_err(),
            "default edit must wait for the admission transaction"
        );
        admitted.commit().await.expect("previous decision ended");
        let changed = change.await.expect("default applied");
        assert_eq!(changed.limit, 0);
        let mut after = w
            .store
            .begin(w.admin.tenant_id(), run, Lock::Wait)
            .await
            .expect("begin")
            .expect("run");
        assert!(
            !after
                .activity_admitted(&activity)
                .await
                .expect("default governs next decision")
        );
        after.commit().await.expect("finish");
    }
}

#[tokio::test]
async fn different_activities_keep_concurrent_admission_with_shared_policy_fences() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("independent-activities", "a", vec![activity("a")]))
        .await;
    let run_a = w.start(wf, &json!({})).await;
    let run_b = w.start(wf, &json!({})).await;
    let mut a = w
        .store
        .begin(w.admin.tenant_id(), run_a, Lock::Wait)
        .await
        .expect("begin")
        .expect("a");
    let mut b = w
        .store
        .begin(w.admin.tenant_id(), run_b, Lock::Wait)
        .await
        .expect("begin")
        .expect("b");
    assert!(
        a.activity_admitted(&ActivityName::parse("act.a").expect("activity"))
            .await
            .expect("a admitted")
    );
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            b.activity_admitted(&ActivityName::parse("act.b").expect("activity"))
        )
        .await
        .expect("different activity is independent")
        .expect("b admitted")
    );
    b.commit().await.expect("b finished");
    a.commit().await.expect("a finished");
}

#[tokio::test]
async fn an_override_waiting_behind_a_ceiling_change_reports_the_current_effective_limit() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("override-fence", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let mut admitted = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(
        admitted
            .activity_admitted(&ActivityName::parse("act.a").expect("activity"))
            .await
            .expect("admitted")
    );
    let lower = w.quotas.grant_ceiling(
        w.admin.tenant_id(),
        QuotaResource::ActivityConcurrency,
        Some(0),
    );
    tokio::pin!(lower);
    // Drive each change to its exact PostgreSQL wait. A short timeout only proves that a future is
    // incomplete; under load it may not have finished the override's ceiling prevalidation yet.
    let tenant_policy = format!("activity-policy:{}", w.admin.tenant_id());
    tokio::select! {
        result = &mut lower => panic!("ceiling editor passed an active admission: {result:?}"),
        () = wait_for_policy_waiter(&w.db.pool, &tenant_policy, "ExclusiveLock") => {}
    }
    let context = ctx();
    let change = w.quotas.set_activity_limit(
        &w.admin,
        w.namespace.id(),
        ActivityName::parse("act.b").expect("activity"),
        2,
        &context,
    );
    tokio::pin!(change);
    tokio::select! {
        result = &mut change => panic!("override passed a queued ceiling editor: {result:?}"),
        () = wait_for_policy_waiter(&w.db.pool, &tenant_policy, "ShareLock") => {}
    }
    admitted.commit().await.expect("release old admission");
    let (lower, view) = tokio::join!(lower, change);
    lower.expect("lowered");
    let view = view.expect("override written");
    assert_eq!((view.limit, view.ceiling), (0, 0));
    let mut after = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(
        !after
            .activity_admitted(&view.activity)
            .await
            .expect("ceiling clamps override")
    );
    after.commit().await.expect("finish");
}

/// Waits until a backend is queued on the advisory lock of `key` in `mode`, then returns.
async fn admission_waits_on(pool: &sqlx::PgPool, key: &str, mode: &str) {
    wait_for_policy_waiter(pool, key, mode).await;
}

fn activity_key(w: &crate::support::engine::World, prefix: &str, activity: &str) -> String {
    format!(
        "{prefix}:{}:{}:{activity}",
        w.admin.tenant_id(),
        w.namespace.id()
    )
}

#[tokio::test]
async fn claims_of_one_activity_without_a_reachable_limit_do_not_wait_for_each_other() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("unbounded-activity", "a", vec![activity("a")]))
        .await;
    let run_a = w.start(wf, &json!({})).await;
    let run_b = w.start(wf, &json!({})).await;
    let name = ActivityName::parse("act.a").expect("activity");
    let mut a = w
        .store
        .begin(w.admin.tenant_id(), run_a, Lock::Wait)
        .await
        .expect("begin")
        .expect("a");
    let mut b = w
        .store
        .begin(w.admin.tenant_id(), run_b, Lock::Wait)
        .await
        .expect("begin")
        .expect("b");
    assert!(a.activity_admitted(&name).await.expect("a admitted"));
    // `a` stays open: with the default limit nothing is counted, so nothing is serialised.
    assert!(
        tokio::time::timeout(Duration::from_secs(5), b.activity_admitted(&name))
            .await
            .expect("the default limit is not a mutex over the activity's claims")
            .expect("b admitted")
    );
    b.commit().await.expect("b finished");
    a.commit().await.expect("a finished");
}

#[tokio::test]
async fn claims_of_one_activity_with_a_reachable_limit_count_one_at_a_time() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("bounded-activity", "a", vec![activity("a")]))
        .await;
    let run_a = w.start(wf, &json!({})).await;
    let run_b = w.start(wf, &json!({})).await;
    let name = ActivityName::parse("act.a").expect("activity");
    let context = ctx();
    w.quotas
        .set_activity_limit(&w.admin, w.namespace.id(), name.clone(), 5, &context)
        .await
        .expect("override");
    let mut a = w
        .store
        .begin(w.admin.tenant_id(), run_a, Lock::Wait)
        .await
        .expect("begin")
        .expect("a");
    let mut b = w
        .store
        .begin(w.admin.tenant_id(), run_b, Lock::Wait)
        .await
        .expect("begin")
        .expect("b");
    assert!(a.activity_admitted(&name).await.expect("a admitted"));
    {
        let second = b.activity_admitted(&name);
        tokio::pin!(second);
        let count_key = activity_key(&w, "activity-count", "act.a");
        tokio::select! {
            result = &mut second => panic!("a second count passed an uncommitted first: {result:?}"),
            () = admission_waits_on(&w.db.pool, &count_key, "ExclusiveLock") => {}
        }
        a.commit().await.expect("a finished");
        assert!(second.await.expect("b admitted after a"));
    }
    b.commit().await.expect("b finished");
}

#[tokio::test]
async fn an_override_waits_for_an_uncounted_admission_then_governs_the_next_decision() {
    let w = world().await;
    let (wf, _) = w
        .publish(document(
            "override-after-unbounded",
            "a",
            vec![activity("a")],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    let name = ActivityName::parse("act.a").expect("activity");
    let mut admitted = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(admitted.activity_admitted(&name).await.expect("admitted"));
    let context = ctx();
    let change = w
        .quotas
        .set_activity_limit(&w.admin, w.namespace.id(), name.clone(), 0, &context);
    tokio::pin!(change);
    let fence = activity_key(&w, "activity-quota", "act.a");
    tokio::select! {
        result = &mut change => panic!("an override passed an admission that read the old limit: {result:?}"),
        () = admission_waits_on(&w.db.pool, &fence, "ExclusiveLock") => {}
    }
    admitted.commit().await.expect("old decision finished");
    assert_eq!(change.await.expect("override written").limit, 0);
    let mut after = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(!after.activity_admitted(&name).await.expect("zero governs"));
    after.commit().await.expect("finish");
}

#[tokio::test]
async fn worker_authority_checks_of_one_namespace_do_not_wait_for_each_other() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("authority-readers", "a", vec![activity("a")]))
        .await;
    let run_a = w.start(wf, &json!({})).await;
    let run_b = w.start(wf, &json!({})).await;
    let worker = w.session(&["act.a"]).await;
    let subject = worker.principal.subject().as_str().to_owned();
    let mut a = w
        .store
        .begin(w.admin.tenant_id(), run_a, Lock::Wait)
        .await
        .expect("begin")
        .expect("a");
    let mut b = w
        .store
        .begin(w.admin.tenant_id(), run_b, Lock::Wait)
        .await
        .expect("begin")
        .expect("b");
    assert!(
        a.current_worker_authorized(&subject)
            .await
            .expect("a checked")
    );
    // `a` stays open, as a report does until it commits: the check reads, so readers share.
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            b.current_worker_authorized(&subject)
        )
        .await
        .expect("one report's authority check is not a mutex over its namespace")
        .expect("b checked")
    );
    b.commit().await.expect("b finished");
    a.commit().await.expect("a finished");
}

#[tokio::test]
async fn a_worker_authority_check_waits_for_a_holder_of_the_namespace_secrets_lock() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("authority-fence", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let worker = w.session(&["act.a"]).await;
    let subject = worker.principal.subject().as_str().to_owned();
    let key = format!("quota:secrets:{}:{}", w.admin.tenant_id(), w.namespace.id());
    // What a catalog mutation holds: the check must still queue behind it, in the same order.
    let mut catalog = w.db.pool.begin().await.expect("catalog mutation");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&key)
        .execute(&mut *catalog)
        .await
        .expect("hold the secrets lock");
    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    {
        let check = tx.current_worker_authorized(&subject);
        tokio::pin!(check);
        tokio::select! {
            result = &mut check => panic!("the check passed a catalog mutation in flight: {result:?}"),
            () = admission_waits_on(&w.db.pool, &key, "ShareLock") => {}
        }
        // The order of the catalog mutations: this lock first, the rows of the authority after.
        // While the check waits here it holds no lock on its membership.
        let mut probe = w.db.pool.begin().await.expect("probe");
        sqlx::query(
            "SELECT 1 FROM memberships WHERE tenant_id = $1 AND subject = $2 FOR UPDATE NOWAIT",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(&subject)
        .fetch_one(&mut *probe)
        .await
        .expect("the membership row is not locked before the secrets lock is granted");
        probe.rollback().await.expect("probe released");
        catalog.commit().await.expect("catalog mutation finished");
        assert!(check.await.expect("checked after the mutation"));
    }
    tx.commit().await.expect("finish");
}

#[tokio::test]
async fn a_revocation_waits_for_a_report_that_checked_and_stops_the_next_one() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("authority-revoked", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let worker = w.session(&["act.a"]).await;
    let subject = worker.principal.subject().as_str().to_owned();
    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(
        tx.current_worker_authorized(&subject)
            .await
            .expect("checked")
    );
    let pool = w.db.pool.clone();
    let tenant = w.admin.tenant_id().as_uuid();
    let revoked = subject.clone();
    let revocation = tokio::spawn(async move {
        sqlx::query("UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2")
            .bind(tenant)
            .bind(revoked)
            .execute(&pool)
            .await
    });
    // The membership row the check read is locked until the report commits: sharing the secrets
    // lock between reports did not take that away.
    crate::support::eventually("the revocation waits for the report", || async {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname = current_database() \
               AND wait_event_type = 'Lock' AND query LIKE 'UPDATE memberships SET role%')",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("observe the revocation");
        waiting.then_some(())
    })
    .await;
    assert!(
        !revocation.is_finished(),
        "a revocation cannot pass a report in flight"
    );
    tx.commit().await.expect("report finished");
    assert_eq!(
        revocation
            .await
            .expect("revocation joins")
            .expect("revoked")
            .rows_affected(),
        1
    );
    let mut after = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("run");
    assert!(
        !after
            .current_worker_authorized(&subject)
            .await
            .expect("checked again"),
        "a revocation that returned stops the next report"
    );
    after.commit().await.expect("finish");
}
