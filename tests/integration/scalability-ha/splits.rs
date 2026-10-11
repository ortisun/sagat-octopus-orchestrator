//! Every pass of the scheduler, with the shards divided between two replicas: each replica does the
//! candidates of the shards it holds and no others, and between them they do all (FR-010-001). The
//! shard of a candidate is its run's, except where the action is on another run: a child that ended
//! is told to its parent (the parent's shard) and a child that must stop is stopped under its own.

use sagat_application::execution::child_run_id;
use sagat_application::runs::RunOperation;
use sagat_domain::execution::RunId;
use sagat_domain::sharding::Shard;
use sagat_scheduler::{Report, SchedulerSettings};
use serde_json::json;

use crate::passes::{TTL, keeper, scheduler};
use crate::support::engine::{
    World, activity, chain, child_node, document, flaky, wait_node, with, world,
};

fn shard(run: RunId) -> Shard {
    Shard::of(run.as_uuid())
}

/// Two replicas share the shards; `acted_on(run)` is the run whose shard decides who does the candidate.
/// `count` is the field of the report the pass fills. Each replica does exactly its own, and the two
/// together all of them.
async fn divided(
    w: &World,
    candidates: &[RunId],
    acted_on: impl Fn(RunId) -> RunId,
    count: impl Fn(&Report) -> u32,
    pass: &str,
) {
    let (a, b) = (keeper(w, "alpha", TTL), keeper(w, "beta", TTL));
    for _ in 0..3 {
        a.round().await.expect("alpha");
        b.round().await.expect("beta");
    }
    let (sa, sb) = (a.placement().owned(), b.placement().owned());
    assert_eq!((sa.len(), sb.len()), (32, 32), "{pass}");
    let mine: Vec<RunId> = candidates
        .iter()
        .copied()
        .filter(|r| sa.contains(shard(acted_on(*r))))
        .collect();
    assert!(
        !mine.is_empty() && mine.len() < candidates.len(),
        "{pass}: the sample must straddle both replicas ({} of {})",
        mine.len(),
        candidates.len()
    );
    let settings = SchedulerSettings::default();
    let alpha = scheduler(w, settings, Some(a.placement().clone()));
    let beta = scheduler(w, settings, Some(b.placement().clone()));
    let first = alpha.tick().await;
    assert_eq!(
        count(&first) as usize,
        mine.len(),
        "{pass}: alpha {first:?}"
    );
    assert_eq!(first.fenced, 0, "{pass}");
    let second = beta.tick().await;
    assert_eq!(
        count(&second) as usize,
        candidates.len() - mine.len(),
        "{pass}: beta {second:?}"
    );
    assert_eq!(second.fenced, 0, "{pass}");
    // Nothing is left, and nothing is done twice.
    assert_eq!(
        count(&alpha.tick().await) + count(&beta.tick().await),
        0,
        "{pass}"
    );
}

fn same(run: RunId) -> RunId {
    run
}

#[tokio::test]
async fn the_deadlines_of_event_waits_are_divided_by_the_shard_of_the_run() {
    let w = world().await;
    let doc = document(
        "split-waits",
        "w",
        vec![
            with(
                wait_node(
                    "w",
                    "payment.confirmed",
                    "input.paymentId",
                    300_000,
                    Some("review"),
                ),
                "next",
                json!("notify"),
            ),
            activity("notify"),
            activity("review"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let mut runs = Vec::new();
    for i in 0..40 {
        let run = w.start(wf, &json!({ "paymentId": format!("p-{i}") })).await;
        w.age_due(run, "w").await;
        runs.push(run);
    }
    divided(&w, &runs, same, |r| r.waits_timed_out, "wait deadlines").await;
}

/// Forty runs of an activity with retries, each with its task leased.
async fn leased(
    w: &World,
    name: &str,
) -> (
    crate::support::engine::Session,
    Vec<sagat_application::worker::ClaimedTask>,
) {
    let (wf, _) = w.publish(document(name, "a", vec![flaky("a", 3)])).await;
    for i in 0..40 {
        w.start(wf, &json!({ "i": i })).await;
    }
    let session = w.session(&["act.a"]).await;
    let mut tasks = Vec::new();
    while tasks.len() < 40 {
        let got = w.poll(&session, 16).await;
        assert!(!got.is_empty(), "every run has a task");
        tasks.extend(got);
    }
    (session, tasks)
}

#[tokio::test]
async fn retries_are_divided_by_the_shard_of_the_run() {
    let w = world().await;
    let (session, tasks) = leased(&w, "split-retries").await;
    // The attempts fail and their back-off elapses.
    for task in &tasks {
        w.fail(&session, task, true).await.expect("fail");
        w.age_retry(task.run_id, task.node.as_str()).await;
    }
    let runs: Vec<RunId> = tasks.iter().map(|t| t.run_id).collect();
    divided(&w, &runs, same, |r| r.retries_released, "retries").await;
}

#[tokio::test]
async fn lost_leases_are_divided_by_the_shard_of_the_run() {
    let w = world().await;
    let (_session, tasks) = leased(&w, "split-leases").await;
    // The workers are gone: the leases run out.
    for task in &tasks {
        w.age_lease(task.run_id, task.node.as_str()).await;
    }
    let runs: Vec<RunId> = tasks.iter().map(|t| t.run_id).collect();
    divided(&w, &runs, same, |r| r.leases_expired, "lost leases").await;
}

#[tokio::test]
async fn deadlines_and_hints_are_divided_by_the_shard_of_the_run() {
    let w = world().await;
    let (wf, _) = w.publish(chain("split-deadlines")).await;
    let mut late = Vec::new();
    let mut quiet = Vec::new();
    for i in 0..40 {
        let deadline = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
        let run = w
            .start_with(
                wf,
                &json!({}),
                &format!("split-late-{i:04}-xxxx"),
                Some(deadline),
            )
            .await
            .expect("start");
        w.age_run_deadline(run).await;
        late.push(run);
        quiet.push(w.start(wf, &json!({ "i": i })).await);
    }
    for run in &quiet {
        w.age_hint(*run, "a").await;
    }
    // Run the deadlines; the quiet runs are hinted by the same passes.
    let (a, b) = (keeper(&w, "alpha", TTL), keeper(&w, "beta", TTL));
    for _ in 0..3 {
        a.round().await.expect("alpha");
        b.round().await.expect("beta");
    }
    let (sa, _) = (a.placement().owned(), b.placement().owned());
    let alpha = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(a.placement().clone()),
    );
    let beta = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(b.placement().clone()),
    );
    let (ra, rb) = (alpha.tick().await, beta.tick().await);
    let own_late = late.iter().filter(|r| sa.contains(shard(**r))).count();
    let own_quiet = quiet.iter().filter(|r| sa.contains(shard(**r))).count();
    assert!(own_late > 0 && own_late < late.len() && own_quiet > 0 && own_quiet < quiet.len());
    assert_eq!(ra.deadlines_enforced as usize, own_late, "{ra:?}");
    assert_eq!(
        rb.deadlines_enforced as usize,
        late.len() - own_late,
        "{rb:?}"
    );
    assert_eq!(ra.hints_renewed as usize, own_quiet, "{ra:?}");
    assert_eq!(rb.hints_renewed as usize, quiet.len() - own_quiet, "{rb:?}");
    assert_eq!((ra.fenced, rb.fenced), (0, 0));
}

/// A parent that starts the child `kid` and then continues with `after`.
async fn mothers(w: &World, n: usize) -> Vec<RunId> {
    let kid = document(
        "split-kid",
        "kid",
        vec![json!({ "id": "kid", "type": "activity", "activity": "act.kid" })],
    );
    w.publish(kid).await;
    let mother = document(
        "split-mother",
        "c",
        vec![
            with(child_node("c", "split-kid", None), "next", json!("after")),
            activity("after"),
        ],
    );
    let (wf, _) = w.publish(mother).await;
    let mut parents = Vec::new();
    for i in 0..n {
        parents.push(w.start(wf, &json!({ "i": i })).await);
    }
    // The children are created by a scheduler that works on every shard (the version before the shards).
    let everyone = scheduler(w, SchedulerSettings::default(), None);
    while everyone.tick().await.did_work() {}
    parents
}

fn child(parent: RunId) -> RunId {
    child_run_id(
        parent,
        &sagat_domain::workflow::NodeId::parse("c").expect("node"),
    )
}

#[tokio::test]
async fn a_child_that_ended_is_told_to_its_parent_by_the_replica_that_holds_the_parents_shard() {
    let w = world().await;
    let parents = mothers(&w, 40).await;
    let session = w.session(&["act.kid", "act.after"]).await;
    let mut done = 0;
    while done < parents.len() {
        for task in w.poll(&session, 16).await {
            if task.node.as_str() == "kid" {
                w.complete(&session, &task, &json!({}))
                    .await
                    .expect("complete");
                done += 1;
            }
        }
    }
    // The shard that decides who tells the parent is the parent's, not the child's.
    divided(
        &w,
        &parents,
        same,
        |r| r.children_finished,
        "ended children",
    )
    .await;
}

#[tokio::test]
async fn the_children_of_a_stopped_parent_are_stopped_by_the_replica_that_holds_their_own_shard() {
    let w = world().await;
    let parents = mothers(&w, 40).await;
    for parent in &parents {
        w.operate_run(*parent, RunOperation::Cancel).await;
    }
    let children: Vec<RunId> = parents.iter().map(|p| child(*p)).collect();
    // Here the action is on the child: it is the child's shard that decides.
    divided(
        &w,
        &children,
        same,
        |r| r.children_cancelled,
        "children to cancel",
    )
    .await;
}
