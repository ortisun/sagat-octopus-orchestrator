use std::collections::BTreeMap;
use std::time::Duration;

use sagat_application::telemetry::TelemetryStore;
use sagat_scheduler::{ProjectorSettings, TelemetryProjector};
use serde_json::json;

use crate::support::collector::Collector;
use crate::support::worker::{Platform, platform, single_ir};

fn projector(
    p: &Platform,
    collector: Option<&Collector>,
) -> TelemetryProjector<sagat_infra_postgres::PgRunStore> {
    TelemetryProjector::new(
        p.api.executions.clone(),
        collector.map(Collector::client),
        ProjectorSettings::default(),
    )
}

/// Starts a run of a one-step workflow and has a worker complete it.
async fn finished_run(p: &Platform, name: &str) -> String {
    let workflow = p.publish(name, &single_ir(name, None)).await;
    let run = p.start(&workflow, &json!({"n": 1})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let lease = worker.one_task().await;
    worker
        .complete(&lease, &json!({"ok": true}))
        .await
        .expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    run
}

async fn pending(p: &Platform, run: &str) -> (i64, i64) {
    sqlx::query_as("SELECT head_seq, applied_seq FROM telemetry_pending WHERE run_id = $1::uuid")
        .bind(run)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the run's note")
}

fn span_ids(collector: &Collector) -> BTreeMap<String, usize> {
    collector
        .spans()
        .iter()
        .fold(BTreeMap::new(), |mut acc, s| {
            *acc.entry(format!("{}:{}", s.name, s.span_id)).or_default() += 1;
            acc
        })
}

#[tokio::test]
async fn a_collector_that_is_down_delays_the_spans_and_loses_none_of_them() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let collector = Collector::start().await;
    let run = finished_run(&p, "outage").await;
    let (head, applied) = pending(&p, &run).await;
    assert!(
        head > 0 && applied == 0,
        "the journal moved and nothing was projected: {head}/{applied}"
    );

    collector.go_down();
    let projector = projector(&p, Some(&collector));
    for _ in 0..3 {
        let round = projector.run_once().await;
        assert!(round.failed, "{round:?}");
        assert_eq!(
            round.runs, 0,
            "no run is marked projected while its spans are not taken"
        );
    }
    assert_eq!(
        pending(&p, &run).await,
        (head, 0),
        "the progress did not move"
    );
    let backlog = p.api.executions.backlog().await.expect("backlog");
    assert_eq!(backlog.runs, 1);
    assert!(backlog.oldest_seconds >= 0.0);
    assert!(collector.spans().is_empty());

    collector.recover();
    let round = projector.run_once().await;
    assert!(!round.failed, "{round:?}");
    assert_eq!(round.runs, 1);
    assert_eq!(pending(&p, &run).await, (head, head), "now it is caught up");
    let ids = span_ids(&collector);
    assert_eq!(ids.len(), 2, "the attempt and the run: {ids:?}");
    assert!(ids.values().all(|n| *n == 1), "each span once: {ids:?}");
    assert_eq!(p.api.executions.backlog().await.expect("backlog").runs, 0);
}

#[tokio::test]
async fn a_batch_the_collector_refuses_for_good_is_dropped_and_the_run_moves_on() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let collector = Collector::start().await;
    let run = finished_run(&p, "refused").await;
    collector.answer_with(&[400]);
    let round = projector(&p, Some(&collector)).run_once().await;
    assert_eq!(round.rejected, 1, "{round:?}");
    assert!(
        !round.failed,
        "a refusal is not an outage: sending the same batch again changes nothing"
    );
    let (head, applied) = pending(&p, &run).await;
    assert_eq!(head, applied, "the run is not asked for again");
}

#[tokio::test]
async fn a_projector_that_dies_after_exporting_exports_the_same_spans_again_with_the_same_ids() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let collector = Collector::start().await;
    let run = finished_run(&p, "crash").await;
    let (head, _) = pending(&p, &run).await;

    // The first projector exports the batch and dies before it records its progress: emulated by
    // exporting what it would have, then leaving the note as it was.
    let first = projector(&p, Some(&collector));
    assert!(!first.run_once().await.failed);
    sqlx::query("UPDATE telemetry_pending SET applied_seq = 0, behind_since = clock_timestamp() WHERE run_id = $1::uuid")
        .bind(&run)
        .execute(&p.api.db.pool)
        .await
        .expect("the note is as the dead projector left it");

    let second = projector(&p, Some(&collector));
    assert!(!second.run_once().await.failed);
    assert_eq!(pending(&p, &run).await, (head, head));
    let ids = span_ids(&collector);
    assert_eq!(ids.len(), 2, "the same two spans, not four: {ids:?}");
    assert!(
        ids.values().all(|n| *n == 2),
        "each exported twice under one id, which a backend keyed on span ids stores once: {ids:?}"
    );
}

#[tokio::test]
async fn only_one_projector_works_at_a_time_and_a_dead_one_is_replaced() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let collector = Collector::start().await;
    finished_run(&p, "lease").await;

    // Another process holds the lease.
    sqlx::query("INSERT INTO engine_leases (name, holder, expires_at) VALUES ('telemetry-projector', 'someone-else', clock_timestamp() + interval '1 minute') ON CONFLICT (name) DO UPDATE SET holder = 'someone-else', expires_at = EXCLUDED.expires_at")
        .execute(&p.api.db.pool)
        .await
        .expect("a lease");
    let projector = projector(&p, Some(&collector));
    let round = projector.run_once().await;
    assert!(round.passive && round.runs == 0, "{round:?}");
    assert!(collector.spans().is_empty());

    // It dies: the lease runs out and this one takes over.
    sqlx::query("UPDATE engine_leases SET expires_at = clock_timestamp() - interval '1 second' WHERE name = 'telemetry-projector'")
        .execute(&p.api.db.pool)
        .await
        .expect("the lease runs out");
    let round = projector.run_once().await;
    assert!(!round.passive && round.runs == 1, "{round:?}");
    // And keeps it: a second round renews it.
    assert!(!projector.run_once().await.passive);
}

#[tokio::test]
async fn a_run_with_no_trace_is_skipped_once_and_never_asked_for_again() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let collector = Collector::start().await;
    let run = finished_run(&p, "legacy").await;
    sqlx::query("UPDATE runs SET trace_id = NULL WHERE id = $1::uuid")
        .bind(&run)
        .execute(&p.api.db.pool)
        .await
        .expect("a run from before runs had a trace");
    let round = projector(&p, Some(&collector)).run_once().await;
    assert_eq!(
        (round.skipped, round.runs, round.failed),
        (1, 0, false),
        "{round:?}"
    );
    let (head, applied) = pending(&p, &run).await;
    assert_eq!(head, applied);
    assert!(collector.spans().is_empty());
}

#[tokio::test]
async fn without_a_collector_the_measurements_are_still_made_and_the_progress_moves() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let run = finished_run(&p, "no-collector").await;
    let before = sagat_telemetry::Metrics::global().render();
    let round = projector(&p, None).run_once().await;
    assert_eq!(
        (round.runs, round.spans, round.failed),
        (1, 0, false),
        "{round:?}"
    );
    let (head, applied) = pending(&p, &run).await;
    assert_eq!(head, applied);
    let after = sagat_telemetry::Metrics::global().render();
    let count = |page: &str, series: &str| -> f64 {
        page.lines()
            .find_map(|l| {
                l.strip_prefix(series)
                    .and_then(|rest| rest.trim().parse().ok())
            })
            .unwrap_or(0.0)
    };
    for series in [
        "step_attempts_total{outcome=\"succeeded\"}",
        "runs_terminal_total{status=\"SUCCEEDED\"}",
    ] {
        assert!(
            count(&after, series) >= count(&before, series) + 1.0,
            "{series} did not move:\n{after}"
        );
    }
    assert!(
        after.contains("step_attempt_duration_seconds_bucket"),
        "{after}"
    );
    assert!(after.contains("run_duration_seconds_bucket"), "{after}");
}

#[tokio::test]
async fn a_second_round_after_a_projected_run_does_nothing() {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let collector = Collector::start().await;
    finished_run(&p, "idempotent").await;
    let projector = projector(&p, Some(&collector));
    assert_eq!(projector.run_once().await.runs, 1);
    let after_first = span_ids(&collector);
    for _ in 0..3 {
        let round = projector.run_once().await;
        assert_eq!(
            (round.runs, round.spans, round.skipped),
            (0, 0, 0),
            "{round:?}"
        );
    }
    assert_eq!(span_ids(&collector), after_first);
}

#[tokio::test]
async fn the_projector_never_makes_a_transition_wait() {
    // A round that is stuck on a collector which never answers holds no lock on a run: the worker
    // completes its step in the meantime.
    let p = platform().await;
    p.bind_everything("worker-1").await;
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}/v1/traces", silent.local_addr().expect("addr"));
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = silent.accept().await {
            held.push(socket);
        }
    });
    let mut config = sagat_telemetry::OtlpConfig::new(url, "sagat-test");
    config.timeout = Duration::from_secs(20);
    config.retries = 0;
    let stuck = TelemetryProjector::new(
        p.api.executions.clone(),
        Some(sagat_telemetry::OtlpClient::new(config).expect("client")),
        ProjectorSettings::default(),
    );

    let workflow = p.publish("busy", &single_ir("busy", None)).await;
    let first = p.start(&workflow, &json!({})).await;
    let running = tokio::spawn(async move { stuck.run_once().await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    // While the round waits for the collector, runs start and complete at full speed.
    let started = std::time::Instant::now();
    let second = p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let leases = worker.poll(2).await;
    assert_eq!(leases.len(), 2);
    for lease in &leases {
        worker.complete(lease, &json!({})).await.expect("complete");
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "transitions waited for the projector"
    );
    assert_eq!(p.status(&first).await, "SUCCEEDED");
    assert_eq!(p.status(&second).await, "SUCCEEDED");
    running.abort();
}
