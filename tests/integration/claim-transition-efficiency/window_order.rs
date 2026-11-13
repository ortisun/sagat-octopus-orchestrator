//! The order a poll tries its candidates in (design 038, FR-038-004, OD-2026-11-12-3): a lone poll
//! tries them oldest first; polls in flight together start at different points of the same window.
//! No candidate is removed from any poll's window.

use crate::support::engine::{World, activity, document, world};
use serde_json::json;

async fn one_activity(w: &World, name: &str) -> sagat_domain::definition::WorkflowId {
    w.publish(document(name, "a", vec![activity("a")])).await.0
}

/// `worker_claim_candidates_total{stage}` so far, zero before its first increment.
fn claim_stage(telemetry: &sagat_telemetry::Metrics, name: &str) -> u64 {
    telemetry
        .render()
        .lines()
        .find(|line| {
            line.starts_with("worker_claim_candidates_total{")
                && line.contains(&format!("stage=\"{name}\""))
        })
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// One worker alone is leased its tasks in the order they became ready.
#[tokio::test]
async fn a_lone_poll_is_served_oldest_first() {
    let w = world().await;
    let workflow = one_activity(&w, "lone-order").await;
    let session = w.session(&["act.a"]).await;
    let mut started = Vec::new();
    for _ in 0..4 {
        started.push(w.start(workflow, &json!({})).await);
    }
    let mut leased = Vec::new();
    for _ in 0..4 {
        let tasks = w.poll(&session, 1).await;
        assert_eq!(tasks.len(), 1);
        leased.push(tasks[0].run_id);
    }
    assert_eq!(leased, started, "oldest first, one at a time");
}

/// Worker A reads four candidates and is held while it leases the first. Worker B, polling
/// meanwhile, starts past what A asked for and tries only runs A is not going to try first:
/// two probes for two leases. Tried in the same order as A, B would probe A's held run first.
#[test]
fn a_poll_in_flight_with_another_starts_past_it() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (telemetry, recorder) =
        sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(in_flight_together(&telemetry));
    });
}

async fn in_flight_together(telemetry: &sagat_telemetry::Metrics) {
    let w = world().await;
    let workflow = one_activity(&w, "together-order").await;
    let a = w.session(&["act.a"]).await;
    let b = w.session(&["act.a"]).await;
    sqlx::query("CREATE TABLE held_worker (worker text NOT NULL)")
        .execute(&w.db.pool)
        .await
        .expect("barrier target");
    sqlx::query("INSERT INTO held_worker VALUES ($1)")
        .bind(a.worker_id.to_string())
        .execute(&w.db.pool)
        .await
        .expect("A is the one held");
    sqlx::query("GRANT SELECT ON held_worker TO PUBLIC")
        .execute(&w.db.pool)
        .await
        .expect("readable by the runtime role");
    sqlx::query(
        "CREATE FUNCTION hold_a_lease() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
           IF OLD.status = 'READY' AND NEW.status = 'LEASED' \
              AND NEW.leased_by = (SELECT worker FROM held_worker) THEN \
             PERFORM pg_advisory_xact_lock_shared(910380008); END IF; RETURN NEW; END $$",
    )
    .execute(&w.db.pool)
    .await
    .expect("barrier function");
    sqlx::query("CREATE TRIGGER hold_a_lease BEFORE UPDATE ON steps FOR EACH ROW EXECUTE FUNCTION hold_a_lease()")
        .execute(&w.db.pool)
        .await
        .expect("barrier trigger");
    let mut barrier = w.db.pool.begin().await.expect("barrier");
    sqlx::query("SELECT pg_advisory_xact_lock(910380008)")
        .execute(&mut *barrier)
        .await
        .expect("hold the barrier");
    let mut started = Vec::new();
    for _ in 0..4 {
        started.push(w.start(workflow, &json!({})).await);
    }
    let poll_a = w.workers.poll(&a.principal, a.worker_id, &a.token, 2, None);
    tokio::pin!(poll_a);
    let held =
        crate::support::eventually("A is held while it leases its first candidate", || async {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND NOT granted \
               AND objid = 910380008 AND database = (SELECT oid FROM pg_database \
               WHERE datname = current_database()))",
            )
            .fetch_one(&w.db.pool)
            .await
            .expect("observe the barrier");
            waiting.then_some(())
        });
    tokio::select! {
        result = &mut poll_a => panic!("A passed the barrier: {result:?}"),
        () = held => {}
    }
    let probed = claim_stage(telemetry, "probed");
    let taken_by_b = w.poll(&b, 2).await;
    let probed_by_b = claim_stage(telemetry, "probed") - probed;
    assert_eq!(taken_by_b.len(), 2, "B is served while A is held");
    assert_eq!(
        probed_by_b, 2,
        "B starts past the two tasks A asked for: it never tries the run A holds"
    );
    barrier.commit().await.expect("release A");
    let taken_by_a = poll_a.await.expect("A's poll");
    assert_eq!(taken_by_a.len(), 2, "A is served all it asked for");
    let mut all: Vec<_> = taken_by_a
        .iter()
        .chain(&taken_by_b)
        .map(|task| task.run_id)
        .collect();
    all.sort_unstable();
    started.sort_unstable();
    assert_eq!(all, started, "every ready run is leased once");
}

/// K polls of one namespace in flight together on one window (design 038, T038-4, SC-038-001).
/// Four workers ask for one task each from four ready runs, and each is held as it leases, so all
/// four are in flight at once. Each starts at its own point of the window and tries one candidate:
/// four probes and four runs opened for four leases. Tried in one order they would make ten probes
/// (the k-th poll skipping the k-1 runs held before it).
#[test]
fn k_polls_in_flight_together_probe_once_per_lease() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (telemetry, recorder) =
        sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(k_in_flight(&telemetry, 4));
    });
}

async fn k_in_flight(telemetry: &sagat_telemetry::Metrics, k: usize) {
    let w = world().await;
    let workflow = one_activity(&w, "k-in-flight").await;
    let mut sessions = Vec::new();
    for _ in 0..k {
        sessions.push(w.session(&["act.a"]).await);
    }
    sqlx::query("CREATE TABLE held_workers (worker text NOT NULL)")
        .execute(&w.db.pool)
        .await
        .expect("barrier targets");
    for session in &sessions {
        sqlx::query("INSERT INTO held_workers VALUES ($1)")
            .bind(session.worker_id.to_string())
            .execute(&w.db.pool)
            .await
            .expect("held");
    }
    sqlx::query("GRANT SELECT ON held_workers TO PUBLIC")
        .execute(&w.db.pool)
        .await
        .expect("readable by the runtime role");
    sqlx::query(
        "CREATE FUNCTION hold_leases() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
           IF OLD.status = 'READY' AND NEW.status = 'LEASED' \
              AND NEW.leased_by IN (SELECT worker FROM held_workers) THEN \
             PERFORM pg_advisory_xact_lock_shared(910380004); END IF; RETURN NEW; END $$",
    )
    .execute(&w.db.pool)
    .await
    .expect("barrier function");
    sqlx::query("CREATE TRIGGER hold_leases BEFORE UPDATE ON steps FOR EACH ROW EXECUTE FUNCTION hold_leases()")
        .execute(&w.db.pool)
        .await
        .expect("barrier trigger");
    let mut barrier = w.db.pool.begin().await.expect("barrier");
    sqlx::query("SELECT pg_advisory_xact_lock(910380004)")
        .execute(&mut *barrier)
        .await
        .expect("hold the barrier");
    let mut started = Vec::new();
    for _ in 0..k {
        started.push(w.start(workflow, &json!({})).await);
    }
    let before = (
        claim_stage(telemetry, "probed"),
        claim_stage(telemetry, "opened"),
        claim_stage(telemetry, "leased"),
    );
    let mut polls = Vec::new();
    for (held, session) in sessions.iter().enumerate() {
        let mut poll = Box::pin(w.workers.poll(
            &session.principal,
            session.worker_id,
            &session.token,
            1,
            None,
        ));
        // Started one after the other, each held before the next begins: all k are in flight.
        let waiting = crate::support::eventually("the poll is held as it leases", || async {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted \
                   AND objid = 910380004 AND database = (SELECT oid FROM pg_database \
                   WHERE datname = current_database())",
            )
            .fetch_one(&w.db.pool)
            .await
            .expect("observe the barrier");
            (waiting == i64::try_from(held + 1).expect("small")).then_some(())
        });
        tokio::select! {
            result = &mut poll => panic!("poll {held} passed the barrier: {result:?}"),
            () = waiting => {}
        }
        polls.push(poll);
    }
    barrier.commit().await.expect("release them");
    let mut leased = Vec::new();
    for poll in polls {
        let tasks = poll.await.expect("poll");
        assert_eq!(tasks.len(), 1, "each poll is served what it asked for");
        leased.push(tasks[0].run_id);
    }
    leased.sort_unstable();
    started.sort_unstable();
    assert_eq!(leased, started, "every run leased once");
    let k = u64::try_from(k).expect("small");
    assert_eq!(
        (
            claim_stage(telemetry, "probed") - before.0,
            claim_stage(telemetry, "opened") - before.1,
            claim_stage(telemetry, "leased") - before.2,
        ),
        (k, k, k),
        "one probe and one opened run per lease"
    );
}
