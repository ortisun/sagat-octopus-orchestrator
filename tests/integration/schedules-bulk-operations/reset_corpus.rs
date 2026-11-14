//! The corpus of the reset (`T018-055`, `SC-018-011`, `FR-018-035`..`FR-018-037`): runs of every shape the engine
//! has, ended for real by a real worker, reset to every place `reset_points` offers.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_application::worker::ClaimedTask;
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{RunId, RunStatus, StepStatus};
use serde_json::{Value, json};

use crate::business_ids::key;
use crate::support::engine::{
    World, activity, child_node, ctx, document, effect_activities, effect_node, parallel_node,
    saga_node, timer_node, wait_node, with, world,
};

fn unique(name: &str) -> String {
    format!("rc-{name}-{}", uuid::Uuid::now_v7())
}

/// What a worker does while a run is driven to its end.
#[derive(Default, Clone)]
struct Script {
    /// The activities some worker offers.
    activities: Vec<String>,
    /// Activities that fail for good the first time they are leased (the source run only).
    fail: Vec<&'static str>,
    /// `(wait node, event type)`: the signal that wakes the wait, sent once it is waiting.
    signals: Vec<(&'static str, &'static str)>,
    /// `(wait node, event type, correlation key)`: the event that wakes the wait, published once it is waiting.
    events: Vec<(&'static str, &'static str, &'static str)>,
    /// Timer nodes whose due time is brought forward once they wait.
    timers: Vec<&'static str>,
    /// The output of an activity is the number of activities this drive has completed (a loop carries it).
    counting: bool,
}

fn terminal(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled | RunStatus::Compensated
    )
}

/// What a test of the corpus drives: the world, a scheduler that holds the shards, and workers over a store that enables
/// the scoped worker protocol (a task of a collection or a loop belongs to an instance below the root).
struct Rig {
    w: World,
    scheduler: crate::support::engine::Sched,
    workers: crate::support::engine::Workers,
}

async fn rig() -> Rig {
    let w = world().await;
    let store = w.store.clone().with_scoped_worker_protocol();
    let workers = sagat_application::worker::WorkerService::new(
        store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_application::worker::WorkerSettings::default(),
    );
    let scheduler = scheduler_of(&w, store).await;
    Rig {
        w,
        scheduler,
        workers,
    }
}

/// A worker that speaks protocol 1.5, which a task of a collection or a loop needs (the instance it belongs to).
async fn scoped_session(
    w: &World,
    workers: &crate::support::engine::Workers,
    activities: &[&str],
) -> crate::support::engine::Session {
    w.bind_everything(&w.worker).await;
    let caps = sagat_application::worker::Capabilities {
        activities: activities.iter().map(|a| (*a).to_owned()).collect(),
        languages: vec!["rust".into()],
        pools: vec![],
        max_inflight: 100,
        protocol_version: "1.5".into(),
    };
    let registered = workers
        .register(
            &w.worker,
            "payments",
            &format!("reset-corpus-{}", uuid::Uuid::now_v7()),
            &caps,
        )
        .await
        .expect("register");
    crate::support::engine::Session {
        principal: w.worker.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    }
}

/// A scheduler over the store of the world that holds the shards, so that it looks for scoped work (a collection, a loop)
/// as the engine does, and starts the child runs.
async fn scheduler_of(
    w: &World,
    store: sagat_infra_postgres::PgRunStore,
) -> crate::support::engine::Sched {
    let placement =
        sagat_scheduler::Placement::new(format!("reset-corpus-{}", uuid::Uuid::now_v7()));
    sagat_scheduler::Keeper::new(
        store.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: std::time::Duration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("the shards are taken");
    sagat_scheduler::Scheduler::new(
        store,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings {
            discovery_rest: std::time::Duration::ZERO,
            ..sagat_scheduler::SchedulerSettings::default()
        },
    )
    .with_placement(placement)
    .with_children(w.cache.clone())
}

/// How a drive ended, and what the worker was given on the way: `(run, node, the task's input)` of each task.
struct Driven {
    status: RunStatus,
    tasks: Vec<(RunId, String, Value)>,
}

/// Drives `run` (and the children it starts) to its end with a worker, a scheduler and the script.
async fn drive(rig: &Rig, run: RunId, script: &Script) -> Driven {
    let (w, scheduler) = (&rig.w, &rig.scheduler);
    let mut given: Vec<(RunId, String, Value)> = Vec::new();
    let names: Vec<&str> = script.activities.iter().map(String::as_str).collect();
    let session = scoped_session(w, &rig.workers, &names).await;
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut signalled: BTreeSet<&str> = BTreeSet::new();
    let mut completed = 0_u32;
    for _ in 0..200 {
        scheduler.tick().await;
        for timer in &script.timers {
            if w.step_status(run, timer).await == Some(StepStatus::Waiting) {
                w.age_due(run, timer).await;
            }
        }
        for (node, event) in &script.signals {
            if !signalled.contains(node)
                && w.step_status(run, node).await == Some(StepStatus::Waiting)
            {
                w.runs
                    .signal(&w.admin, run, event, br#"{"ok":true}"#, &key(), &ctx())
                    .await
                    .expect("the signal");
                signalled.insert(node);
            }
        }
        for (node, event_type, correlation) in &script.events {
            if !signalled.contains(node)
                && w.step_status(run, node).await == Some(StepStatus::Waiting)
            {
                w.events
                    .publish(
                        &w.admin,
                        sagat_application::events::PublishRequest {
                            namespace: "payments".to_owned(),
                            event_id: Some(
                                sagat_domain::events::EventId::parse(&format!(
                                    "ev-{}",
                                    uuid::Uuid::now_v7().simple()
                                ))
                                .expect("an event id"),
                            ),
                            event_type: sagat_domain::workflow::EventType::parse(event_type)
                                .expect("an event type"),
                            subject: None,
                            correlation_key: Some(
                                sagat_domain::execution::CorrelationKey::parse(correlation)
                                    .expect("a key"),
                            ),
                            occurred_at: None,
                            data: b"{}".to_vec(),
                            dedupe_ttl: None,
                        },
                        &ctx(),
                    )
                    .await
                    .expect("the event is published");
                w.dispatcher.tick().await;
                signalled.insert(node);
            }
        }
        let tasks: Vec<ClaimedTask> = rig
            .workers
            .poll(
                &session.principal,
                session.worker_id,
                &session.token,
                8,
                None,
            )
            .await
            .expect("poll");
        let quiet = tasks.is_empty();
        for task in tasks {
            let activity = task.activity.as_str().to_owned();
            given.push((
                task.run_id,
                task.instance.node().as_str().to_owned(),
                serde_json::from_slice(&task.input).expect("the input of a task is JSON"),
            ));
            if script.fail.contains(&activity.as_str()) && failed.insert(activity.clone()) {
                rig.workers
                    .fail(
                        &session.principal,
                        &crate::support::engine::task_ref(&task),
                        "PAYMENT_DECLINED",
                        false,
                        &format!("failed-{}", task.task_id()),
                    )
                    .await
                    .expect("the failure");
                continue;
            }
            completed += 1;
            let output = if script.counting {
                json!(completed)
            } else {
                // what each execution gives is its own, so that a step a reset kept is told from one it ran again
                json!({ "done": task.instance.node().as_str(), "by": task.task_id().to_string() })
            };
            rig.workers
                .complete(
                    &session.principal,
                    &crate::support::engine::task_ref(&task),
                    &serde_json::to_vec(&output).expect("json"),
                    &format!("done-{}", task.task_id()),
                )
                .await
                .expect("the step completes");
        }
        let status = w.run_status(run).await;
        if quiet && terminal(status) {
            return Driven {
                status,
                tasks: given,
            };
        }
    }
    let steps: Vec<(String, String)> =
        sqlx::query_as("SELECT node, status FROM steps WHERE run_id = $1 ORDER BY node")
            .bind(run.as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .expect("steps");
    panic!(
        "the run never ended: {:?}\n{:?}\n{steps:?}\n{:?}",
        w.run_status(run).await,
        w.journal(run).await,
        scheduler.tick().await
    );
}

struct Shape {
    workflow: WorkflowId,
    input: Value,
    script: Script,
}

async fn collection_capacity(w: &World) {
    sqlx::query(
        "INSERT INTO tenant_collection_policies (tenant_id, max_active_iterations, updated_by) \
         VALUES ($1, 4, 'reset-corpus') ON CONFLICT DO NOTHING",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("iteration capacity");
}

fn structured(name: &str, entrypoint: &str, bodies: Value, nodes: Vec<Value>) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": entrypoint,
        "bodies": bodies,
        "nodes": nodes,
    })
}

async fn shape(w: &World, which: &'static str) -> Shape {
    let name = unique(which);
    let pre = || with(activity("pre"), "next", json!("mid"));
    let acts =
        |names: &[&str]| -> Vec<String> { names.iter().map(|n| format!("act.{n}")).collect() };
    match which {
        "parallel" => Shape {
            workflow: w
                .publish(document(
                    &name,
                    "pre",
                    vec![
                        pre(),
                        with(
                            parallel_node("mid", &["l", "r"], "all"),
                            "next",
                            json!("post"),
                        ),
                        activity("l"),
                        activity("r"),
                        activity("post"),
                    ],
                ))
                .await
                .0,
            input: json!({}),
            script: Script {
                activities: acts(&["pre", "l", "r", "post"]),
                ..Script::default()
            },
        },
        "saga" | "saga-compensated" => {
            let mut activities = acts(&["pre", "post"]);
            activities.extend(effect_activities(&["reserve", "submit"]));
            Shape {
                workflow: w
                    .publish(document(
                        &name,
                        "pre",
                        vec![
                            pre(),
                            saga_node("mid", "reserve", Some("post")),
                            effect_node("reserve", Some("submit"), false),
                            effect_node("submit", None, false),
                            activity("post"),
                        ],
                    ))
                    .await
                    .0,
                input: json!({}),
                script: Script {
                    activities,
                    fail: if which == "saga" {
                        vec![]
                    } else {
                        vec!["act.submit"]
                    },
                    ..Script::default()
                },
            }
        }
        "collection" | "collection-empty" => {
            collection_capacity(w).await;
            Shape {
                workflow: w
                    .publish(structured(
                        &name,
                        "mid",
                        json!({"line": {"entrypoint": "work", "nodes": [
                            {"id": "work", "type": "activity", "activity": "act.work", "next": "returned"},
                            {"id": "returned", "type": "return", "result": "steps.work.output"}
                        ]}}),
                        vec![
                            json!({"id": "mid", "type": "forEach", "items": "input.lines", "body": "line",
                                   "result": "bodyResult", "maxItems": 8, "maxConcurrency": 1, "next": "done"}),
                            json!({"id": "done", "type": "end"}),
                        ],
                    ))
                    .await
                    .0,
                input: if which == "collection" {
                    json!({"lines": [1, 2]})
                } else {
                    json!({"lines": []})
                },
                script: Script {
                    activities: acts(&["work"]),
                    ..Script::default()
                },
            }
        }
        "loop" | "loop-empty" => {
            collection_capacity(w).await;
            // the condition of a loop that does not run is false at the first look
            let condition = if which == "loop" {
                "carry < 2"
            } else {
                "carry < 0"
            };
            Shape {
                workflow: w
                    .publish(structured(
                        &name,
                        "mid",
                        json!({"iteration": {"entrypoint": "increment", "nodes": [
                            {"id": "increment", "type": "activity", "activity": "act.increment", "next": "return-value"},
                            {"id": "return-value", "type": "return", "result": "steps.increment.output"}
                        ]}}),
                        vec![
                            json!({"id": "mid", "type": "while", "condition": condition, "body": "iteration",
                                   "initial": "0", "carry": "bodyResult", "maxIterations": 3,
                                   "timeoutMs": 60000, "next": "done"}),
                            json!({"id": "done", "type": "end"}),
                        ],
                    ))
                    .await
                    .0,
                input: json!({}),
                script: Script {
                    activities: acts(&["increment"]),
                    counting: true,
                    ..Script::default()
                },
            }
        }
        "child" => {
            let kid = unique("kid");
            w.publish(document(
                &kid,
                "kid",
                vec![json!({ "id": "kid", "type": "activity", "activity": "act.kid" })],
            ))
            .await;
            Shape {
                workflow: w
                    .publish(document(
                        &name,
                        "pre",
                        vec![
                            pre(),
                            child_node("mid", &kid, Some("post")),
                            activity("post"),
                        ],
                    ))
                    .await
                    .0,
                input: json!({}),
                script: Script {
                    activities: acts(&["pre", "kid", "post"]),
                    ..Script::default()
                },
            }
        }
        "timer" => Shape {
            workflow: w
                .publish(document(
                    &name,
                    "pre",
                    vec![
                        pre(),
                        with(timer_node("mid", 600_000), "next", json!("post")),
                        activity("post"),
                    ],
                ))
                .await
                .0,
            input: json!({}),
            script: Script {
                activities: acts(&["pre", "post"]),
                timers: vec!["mid"],
                ..Script::default()
            },
        },
        "event" => Shape {
            workflow: w
                .publish(document(
                    &name,
                    "pre",
                    vec![
                        pre(),
                        with(
                            wait_node("mid", "payment.confirmed", "input.k", 600_000, None),
                            "next",
                            json!("post"),
                        ),
                        activity("post"),
                    ],
                ))
                .await
                .0,
            input: json!({"k": "s"}),
            script: Script {
                activities: acts(&["pre", "post"]),
                events: vec![("mid", "payment.confirmed", "s")],
                ..Script::default()
            },
        },
        "signal" => Shape {
            workflow: w
                .publish(document(
                    &name,
                    "pre",
                    vec![
                        pre(),
                        with(
                            wait_node("mid", "payment.confirmed", "input.k", 600_000, None),
                            "next",
                            json!("post"),
                        ),
                        activity("post"),
                    ],
                ))
                .await
                .0,
            input: json!({"k": "s"}),
            script: Script {
                activities: acts(&["pre", "post"]),
                signals: vec![("mid", "payment.confirmed")],
                ..Script::default()
            },
        },
        other => panic!("no shape {other}"),
    }
}

/// A row of a journal as a reset must keep it: everything the source stored but the two marks the reset adds.
type Kept = (
    i64,
    String,
    Option<String>,
    Option<i32>,
    Value,
    Option<Vec<u8>>,
    String,
    Option<String>,
    time::OffsetDateTime,
    Option<Vec<String>>,
    Option<Vec<i32>>,
);

async fn kept(w: &World, run: RunId) -> Vec<Kept> {
    sqlx::query_as(
        "SELECT seq, event_type, node, attempt, detail - 'restored' - 'reset_of', payload_digest, actor, trace_id, \
                occurred_at, instance_path_nodes, instance_path_indices \
         FROM run_events WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("the journal")
}

/// `(node, sequence)` of the events that end a root node, in journal order.
async fn completions(w: &World, run: RunId) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT node, seq FROM run_events WHERE run_id = $1 \
           AND event_type IN ('step.completed','child.finished','collection.finished','loop.finished') \
           AND coalesce(cardinality(instance_path_nodes), 0) = 0 ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("the completions")
}

/// `node -> (status, digest of the output)` of the steps of the root of a run.
async fn root_steps(
    w: &World,
    run: RunId,
) -> std::collections::BTreeMap<String, (String, Option<Vec<u8>>)> {
    sqlx::query_as::<_, (String, String, Option<Vec<u8>>)>(
        "SELECT node, status, output_digest FROM steps WHERE run_id = $1 \
           AND coalesce(cardinality(instance_path_nodes), 0) = 0",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("the steps")
    .into_iter()
    .map(|(node, status, output)| (node, (status, output)))
    .collect()
}

fn block_name(block: &sagat_domain::execution::ResetBlock) -> String {
    use sagat_domain::execution::ResetBlock::{
        ActiveScope, ChangedLater, ExcludedEvent, NotRunning, OpenWindow, OpenWork, ScopedEvents,
    };
    match block {
        ExcludedEvent(event) => format!("excluded:{event}"),
        OpenWindow(event) => format!("window:{event}"),
        NotRunning(status) => format!("not-running:{status:?}"),
        OpenWork(node) => format!("open-work:{}", node.as_str()),
        ActiveScope => "active-scope".to_owned(),
        ScopedEvents => "scoped-events".to_owned(),
        ChangedLater { event_type, node } => {
            format!("changed-later:{event_type}:{}", node.as_str())
        }
    }
}

/// What D1022 says of a shape, written by hand from the rule and read off a journal that was looked at: the points
/// the code must offer, and why each candidate it does not offer is not one.
struct Want {
    source: RunStatus,
    /// `(kind, node)` of each eligible point, in order.
    points: &'static [(&'static str, Option<&'static str>)],
    /// `(node it completes, why it is blocked)` of each candidate that is not eligible, in order.
    blocked: &'static [(&'static str, &'static str)],
}

fn refused_not_eligible<T: std::fmt::Debug>(
    answer: Result<T, sagat_application::error::AppError>,
    what: &str,
) {
    match answer {
        Err(sagat_application::error::AppError::ResetRefused {
            reason: "RESET_POINT_NOT_ELIGIBLE",
            ..
        }) => {}
        other => panic!("{what}: RESET_POINT_NOT_ELIGIBLE was expected, got {other:?}"),
    }
}

fn small(value: u64) -> i64 {
    i64::try_from(value).expect("small")
}

async fn corpus(which: &'static str, want: Want) {
    use sagat_application::runs::RunStore as _;
    let rig = rig().await;
    let w = &rig.w;
    let shape = shape(w, which).await;
    let source = w.start(shape.workflow, &shape.input).await;
    let source_drive = drive(&rig, source, &shape.script).await;
    assert_eq!(
        source_drive.status, want.source,
        "{which}: the source ends as the shape says"
    );
    w.assert_consistent(source).await;
    let source_rows = kept(w, source).await;
    let source_completions = completions(w, source).await;
    let source_steps = root_steps(w, source).await;
    let length = i64::try_from(source_rows.len()).expect("small");

    // the points: what the rule says, kind and node, by hand
    let points = w
        .runs
        .reset_points(&w.admin, source, &ctx())
        .await
        .expect("the points");
    let offered: Vec<(&str, Option<&str>)> = points
        .iter()
        .map(|point| (point.kind.as_str(), point.node.as_deref()))
        .collect();
    assert_eq!(offered, want.points, "{which}: the points offered");
    // and why each other candidate is not one
    let journal = w
        .store
        .reset_source(w.admin.tenant_id(), source)
        .await
        .expect("the source")
        .expect("a source");
    let events: Vec<_> = journal.events.iter().map(|row| row.event.clone()).collect();
    let classified = sagat_domain::execution::reset_points(source, &events).expect("it replays");
    let blocked: Vec<(String, String)> = classified
        .blocked
        .iter()
        .map(|(point, block)| {
            (
                point
                    .completes
                    .as_ref()
                    .map_or_else(String::new, |n| n.as_str().to_owned()),
                block_name(block),
            )
        })
        .collect();
    let wanted: Vec<(String, String)> = want
        .blocked
        .iter()
        .map(|(node, why)| ((*node).to_owned(), (*why).to_owned()))
        .collect();
    assert_eq!(
        blocked, wanted,
        "{which}: the candidates that are not points, and why"
    );

    // every other place of the journal, and one past its end, is refused with the typed reason, and no run is made
    let eligible: BTreeSet<i64> = points.iter().map(|p| small(p.sequence)).collect();
    for sequence in 1..=length + 1 {
        if eligible.contains(&sequence) {
            continue;
        }
        let sequence = u64::try_from(sequence).expect("small");
        refused_not_eligible(
            w.runs
                .reset(
                    &w.admin,
                    source,
                    crate::reset::request(sequence, true),
                    &key(),
                    &ctx(),
                )
                .await,
            &format!("{which}: a reset to {sequence}"),
        );
        refused_not_eligible(
            w.runs
                .reset_preview(
                    &w.admin,
                    source,
                    sequence,
                    sagat_domain::execution::ResetReapply::None,
                    &ctx(),
                )
                .await,
            &format!("{which}: a preview of {sequence}"),
        );
    }
    let made: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE rerun_of = $1")
        .bind(source.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(made, 0, "{which}: a refused reset makes no run");

    // every eligible point
    let mut script = shape.script.clone();
    script.fail.clear();
    for point in &points {
        let at = point.sequence;
        let kept_rows = usize::try_from(at).expect("small");
        let made = w
            .runs
            .reset(
                &w.admin,
                source,
                crate::reset::request(at, true),
                &key(),
                &ctx(),
            )
            .await
            .unwrap_or_else(|error| panic!("{which}: a reset to {at}: {error:?}"));
        let new = made.run.as_ref().expect("the reset made a run").info.run_id;
        let label = format!("{which} at {at}");

        // the projection is what a replay of the journal gives, before the run goes on
        w.assert_consistent(new).await;

        // rows 1..=N are the source's, verbatim, each marked restored; the row after them is the reset
        let new_rows = kept(w, new).await;
        assert!(new_rows.len() > kept_rows, "{label}: the run goes on");
        assert_eq!(
            new_rows[..kept_rows],
            source_rows[..kept_rows],
            "{label}: the prefix is the source's"
        );
        let marks: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT detail->>'restored' FROM run_events WHERE run_id = $1 ORDER BY seq",
        )
        .bind(new.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("marks");
        assert!(
            marks[..kept_rows]
                .iter()
                .all(|mark| mark.as_deref() == Some("true")),
            "{label}: every row of the prefix is marked restored"
        );
        assert_eq!(
            new_rows[kept_rows].1, "run.reset",
            "{label}: the reset follows the prefix"
        );
        assert!(
            marks[kept_rows..].iter().all(Option::is_none),
            "{label}: what the run does itself is not restored"
        );

        // the outputs: what the source's steps held at N, the new run holds; what it had not yet done, it holds nothing of
        let new_steps = root_steps(w, new).await;
        let done_at_n: BTreeSet<&str> = source_completions
            .iter()
            .filter(|(_, seq)| *seq <= small(at))
            .map(|(node, _)| node.as_str())
            .collect();
        for node in &done_at_n {
            assert_eq!(
                new_steps.get(*node),
                source_steps.get(*node),
                "{label}: {node} is as it was"
            );
        }
        // an activity the source completed before the point gave an output, and the new run holds that one
        for row in &source_rows[..kept_rows] {
            let root = row.9.as_ref().is_none_or(Vec::is_empty);
            if row.1 == "step.completed" && root {
                let node = row.2.as_deref().expect("a step has a node");
                assert!(
                    source_steps
                        .get(node)
                        .is_some_and(|(_, output)| output.is_some()),
                    "{label}: the source gave {node} an output"
                );
            }
        }
        for (node, _) in &source_completions {
            if done_at_n.contains(node.as_str()) {
                continue;
            }
            let held = new_steps.get(node);
            assert!(
                held.is_none_or(|(status, output)| output.is_none()
                    && !matches!(status.as_str(), "SUCCEEDED" | "FAILED")),
                "{label}: {node} is not done yet in the new run: {held:?}"
            );
        }

        // it goes on to the end with a worker, and what is left runs again
        let driven = drive(&rig, new, &script).await;
        assert_eq!(
            driven.status,
            RunStatus::Succeeded,
            "{label}: the run finishes"
        );
        w.assert_consistent(new).await;
        // what a step is given of the steps the reset kept is what the source's own run gave it
        for (run, node, input) in &driven.tasks {
            if *run != new {
                continue;
            }
            let Some((_, _, before)) = source_drive
                .tasks
                .iter()
                .find(|(source_run, source_node, _)| *source_run == source && source_node == node)
            else {
                continue;
            };
            for kept_node in &done_at_n {
                if before["steps"].get(*kept_node).is_some() {
                    assert_eq!(
                        input["steps"].get(*kept_node),
                        before["steps"].get(*kept_node),
                        "{label}: the input of {node} holds what {kept_node} gave in the source"
                    );
                }
            }
        }
        // a step that runs again gives an output of its own (what a worker gives is made apart each time)
        if !script.counting {
            let new_steps = root_steps(w, new).await;
            for (node, (_, output)) in &source_steps {
                if done_at_n.contains(node.as_str()) || node.contains('~') {
                    continue;
                }
                if let Some((_, again)) = new_steps.get(node)
                    && output.is_some()
                    && source_rows
                        .iter()
                        .any(|row| row.1 == "step.completed" && row.2.as_deref() == Some(node))
                {
                    assert_ne!(
                        again, output,
                        "{label}: {node} gave its output again, it did not run"
                    );
                }
            }
        }
        let after = completions(w, new).await;
        for (node, seq) in &after {
            assert!(
                !done_at_n.contains(node.as_str()) || *seq <= small(at),
                "{label}: {node} ran again though the reset kept it"
            );
        }
        let ran_again: BTreeSet<&str> = after
            .iter()
            .filter(|(_, seq)| *seq > small(at))
            .map(|(node, _)| node.as_str())
            .collect();
        for (node, _) in &source_completions {
            if !done_at_n.contains(node.as_str()) && !node.contains('~') {
                assert!(
                    ran_again.contains(node.as_str()),
                    "{label}: {node} runs again after the point"
                );
            }
        }
        let leased_again: Vec<String> = sqlx::query_scalar(
            "SELECT node FROM run_events WHERE run_id = $1 AND seq > $2 AND event_type = 'step.leased' \
               AND coalesce(cardinality(instance_path_nodes), 0) = 0",
        )
        .bind(new.as_uuid())
        .bind(small(at))
        .fetch_all(&w.db.pool)
        .await
        .expect("leases");
        assert!(
            leased_again
                .iter()
                .all(|node| !done_at_n.contains(node.as_str())),
            "{label}: a step the reset kept was leased again: {leased_again:?}"
        );
        // a child that the prefix holds is not started again; one it does not hold is
        let started_before = new_rows[..kept_rows]
            .iter()
            .filter(|row| row.1 == "child.started")
            .count();
        let children: i64 =
            sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id = $1")
                .bind(new.as_uuid())
                .fetch_one(&w.db.pool)
                .await
                .expect("children");
        let source_children = source_rows
            .iter()
            .filter(|row| row.1 == "child.started")
            .count();
        assert_eq!(
            usize::try_from(children).expect("small"),
            source_children - started_before,
            "{label}: the children the new run started"
        );
    }

    // the source is what it was, and every reset is in the register, once
    assert_eq!(
        kept(w, source).await,
        source_rows,
        "{which}: the source is untouched"
    );
    w.assert_consistent(source).await;
    let registered: Vec<i64> = sqlx::query_scalar(
        "SELECT source_sequence FROM run_resets WHERE source_run_id = $1 ORDER BY source_sequence",
    )
    .bind(source.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("the register");
    assert_eq!(
        registered,
        eligible.iter().copied().collect::<Vec<_>>(),
        "{which}: one entry per reset"
    );
}

#[tokio::test]
async fn a_parallel_run_is_reset_at_its_start_and_around_the_fork_but_not_inside_a_branch() {
    corpus(
        "parallel",
        Want {
            source: RunStatus::Succeeded,
            points: &[
                ("RUN_STARTED", None),
                ("STEP_COMPLETED", Some("pre")),
                ("STEP_COMPLETED", Some("post")),
            ],
            // the first branch to end leaves the fork open; the second leaves the run waiting for the join
            blocked: &[("l", "open-work:mid"), ("r", "not-running:Waiting")],
        },
    )
    .await;
}

#[tokio::test]
async fn a_saga_that_completed_is_reset_around_it_and_not_inside_it() {
    corpus(
        "saga",
        Want {
            source: RunStatus::Succeeded,
            points: &[
                ("RUN_STARTED", None),
                ("STEP_COMPLETED", Some("pre")),
                ("STEP_COMPLETED", Some("post")),
            ],
            // inside the saga the run waits for its body
            blocked: &[
                ("reserve", "not-running:Waiting"),
                ("submit", "not-running:Waiting"),
            ],
        },
    )
    .await;
}

#[tokio::test]
async fn a_saga_that_was_compensated_is_reset_before_it_and_never_inside_or_after_its_compensation()
{
    corpus(
        "saga-compensated",
        Want {
            source: RunStatus::Compensated,
            points: &[("RUN_STARTED", None), ("STEP_COMPLETED", Some("pre"))],
            // a step of the saga body while the saga waits; the compensation, once the saga aborted
            blocked: &[
                ("reserve", "not-running:Waiting"),
                ("reserve~c", "excluded:saga.aborting"),
            ],
        },
    )
    .await;
}

#[tokio::test]
async fn a_collection_run_is_reset_only_at_its_start_because_the_events_of_an_iteration_are_not_restored_yet()
 {
    corpus(
        "collection",
        Want {
            source: RunStatus::Succeeded,
            points: &[("RUN_STARTED", None)],
            // the completion of the collection holds the events of its iterations (G1980 (1))
            blocked: &[("mid", "scoped-events")],
        },
    )
    .await;
}

#[tokio::test]
async fn a_collection_of_no_item_is_reset_only_at_its_start_as_one_of_two_is() {
    // found by this corpus: the point at the finish of a collection that holds no item was offered, and the run made
    // from it did not replay to its projection (D2156); it holds the entry of its scope, as every collection does
    corpus(
        "collection-empty",
        Want {
            source: RunStatus::Succeeded,
            points: &[("RUN_STARTED", None)],
            blocked: &[("mid", "scoped-events")],
        },
    )
    .await;
}

#[tokio::test]
async fn a_loop_run_is_reset_only_at_its_start_because_a_loop_that_succeeds_ends_at_no_event_of_the_list()
 {
    corpus(
        "loop",
        Want {
            source: RunStatus::Succeeded,
            // G1979 (1): the completion of a loop is not in the list of D1022, and its iterations are scoped (G1980 (1))
            points: &[("RUN_STARTED", None)],
            blocked: &[],
        },
    )
    .await;
}

#[tokio::test]
async fn a_run_with_a_child_is_reset_around_the_child_and_the_child_is_started_again_only_before_it()
 {
    corpus(
        "child",
        Want {
            source: RunStatus::Succeeded,
            points: &[
                ("RUN_STARTED", None),
                ("STEP_COMPLETED", Some("pre")),
                ("CHILD_FINISHED", Some("mid")),
                ("STEP_COMPLETED", Some("post")),
            ],
            blocked: &[],
        },
    )
    .await;
}

#[tokio::test]
async fn a_run_that_waited_for_a_timer_is_reset_around_the_timer_whose_firing_is_not_a_point() {
    corpus(
        "timer",
        Want {
            source: RunStatus::Succeeded,
            // `timer.fired` completes a root node and is not in the list of D1022 (G1979 (1))
            points: &[
                ("RUN_STARTED", None),
                ("STEP_COMPLETED", Some("pre")),
                ("STEP_COMPLETED", Some("post")),
            ],
            blocked: &[],
        },
    )
    .await;
}

#[tokio::test]
async fn a_run_that_waited_for_an_event_is_reset_around_the_wait_as_one_that_waited_for_a_signal_is()
 {
    corpus(
        "event",
        Want {
            source: RunStatus::Succeeded,
            points: &[
                ("RUN_STARTED", None),
                ("STEP_COMPLETED", Some("pre")),
                ("STEP_COMPLETED", Some("post")),
            ],
            blocked: &[],
        },
    )
    .await;
}

#[tokio::test]
async fn a_loop_that_does_not_run_is_reset_only_at_its_start_as_one_that_runs_is() {
    corpus(
        "loop-empty",
        Want {
            source: RunStatus::Succeeded,
            points: &[("RUN_STARTED", None)],
            blocked: &[],
        },
    )
    .await;
}

#[tokio::test]
async fn an_approval_run_is_reset_before_the_approval_and_asks_for_the_decision_again() {
    use crate::support::approval_flow::{ApprovalFlow, INITIATOR, OPERATOR, approval_nodes};
    use reqwest::Method;

    let flow = ApprovalFlow::boot(&["segment.work"]).await;
    let mut nodes = vec![json!({
        "id": "work", "type": "activity", "activity": "segment.work", "next": "review"
    })];
    nodes.extend(approval_nodes(600_000));
    let name = unique("approval");
    let workflow = flow
        .publish(
            &name,
            json!({
                "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
                "metadata": {"name": name}, "entrypoint": "work", "nodes": nodes
            }),
        )
        .await
        .expect("the workflow publishes");
    let source = flow.start(&workflow, json!({})).await;
    flow.work_on(source, json!({"done": "work"})).await;
    let approval = flow.the_approval(source).await;
    let (status, decided) = flow
        .decide(&approval, OPERATOR, "approve-1", "APPROVE")
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{decided}");
    flow.drive().await;
    assert_eq!(flow.run_status(source).await, "SUCCEEDED");
    flow.verify(source).await;
    let source_journal = flow.journal(source).await;

    // what the code does: the decision of an approval completes a root node and is no point (G1979 (1), owner question
    // open), so the places are the start and the end of the activity before it
    let listed = flow
        .api
        .get_as(&format!("/runs/{source}/reset-points"), INITIATOR, "acme")
        .await;
    assert_eq!(listed.status(), reqwest::StatusCode::OK);
    let listed = crate::support::api::json_of(listed).await;
    let points: Vec<(u64, String, Option<String>)> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            (
                item["sequence"].as_u64().expect("sequence"),
                item["kind"].as_str().expect("kind").to_owned(),
                item["node"].as_str().map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(
        points
            .iter()
            .map(|(_, kind, node)| (kind.as_str(), node.as_deref()))
            .collect::<Vec<_>>(),
        [("RUN_STARTED", None), ("STEP_COMPLETED", Some("work"))],
        "{points:?}\n{source_journal:?}"
    );

    for (sequence, kind, _) in &points {
        let reset = flow
            .api
            .as_user(
                Method::POST,
                &format!("/runs/{source}/resets"),
                INITIATOR,
                "acme",
            )
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", uuid::Uuid::now_v7().to_string())
            .json(&json!({
                "sequence": sequence,
                "reason": "the approval was given on a wrong price list",
                "acknowledgeReexecution": true
            }))
            .send()
            .await
            .expect("send");
        let status = reset.status();
        let made = crate::support::api::json_of(reset).await;
        assert!(status.is_success(), "{kind}: {status} {made}");
        let new: RunId = made["newRunId"]
            .as_str()
            .unwrap_or_else(|| panic!("{kind}: the answer names the new run: {made}"))
            .parse()
            .expect("a run id");
        flow.verify(new).await;
        // the work runs again where the point is the start, and the approval is asked again in both
        if kind == "RUN_STARTED" {
            flow.work_on(new, json!({"done": "work again"})).await;
        }
        let again = flow.the_approval(new).await;
        assert_ne!(
            again, approval,
            "{kind}: a new approval, not the one of the source"
        );
        let (status, decided) = flow
            .decide(&again, OPERATOR, "approve-again", "APPROVE")
            .await;
        assert_eq!(status, reqwest::StatusCode::OK, "{kind}: {decided}");
        flow.drive().await;
        assert_eq!(flow.run_status(new).await, "SUCCEEDED", "{kind}");
        flow.verify(new).await;
    }
    assert_eq!(
        flow.journal(source).await,
        source_journal,
        "the source is untouched"
    );
}

#[tokio::test]
async fn a_run_that_waited_for_a_signal_is_reset_around_the_wait_whose_satisfaction_is_not_a_point()
{
    corpus(
        "signal",
        Want {
            source: RunStatus::Succeeded,
            points: &[
                ("RUN_STARTED", None),
                ("STEP_COMPLETED", Some("pre")),
                ("STEP_COMPLETED", Some("post")),
            ],
            blocked: &[],
        },
    )
    .await;
}
