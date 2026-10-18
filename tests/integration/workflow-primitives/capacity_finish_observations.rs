//! Real returned outcomes, isolated metrics and actual OTLP export; no production caller.
use super::commit_proxy::{CommitAckLossProxy, CommitResolution};
use super::*;
use crate::support::collector::Collector;
use sagat_telemetry::{Metrics, SpanId, TraceId};
use std::time::Duration;
use tracing::Instrument;
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

fn total(page: &str, name: &str, outcome: Option<&str>) -> u64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && outcome.is_none_or(|value| line.contains(&format!("outcome=\"{value}\"")))
        })
        .map(|line| {
            line.rsplit(' ')
                .next()
                .expect("sample")
                .parse::<u64>()
                .expect("integer count")
        })
        .sum()
}

#[tokio::test(flavor = "current_thread")]
async fn filtered_finish_span_cannot_overwrite_its_callers_outcome_or_status() {
    let n = super::barriers::runtime_fixture().await;
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    // A real closed native pool returns Storage, which normally sets Finish's error status.
    n.f.w.execution_pool.close().await;
    let collector = Collector::start().await;
    let (_guard, sink) = collector.install(1.0);
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            "sagat_test=info,sagat_infra_postgres=warn",
        ))
        .with(sagat_telemetry::OtlpLayer::new(sink.clone(), 1.0));
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    async {
        let parent=tracing::info_span!(target:"sagat_test","capacity.filtered.caller",trace_id=%trace,sagat.outcome="caller_pending",otel.status_code=tracing::field::Empty);
        async {
            assert!(matches!(PgCapacityFinishStore::new(n.f.w.execution_pool.clone()).finish_current_child_root(&command).await,Err(CapacityFinishError::Storage(_))));
        }.instrument(parent).await;
    }.with_subscriber(subscriber).await;
    sink.shutdown(Duration::from_secs(5)).await;
    assert!(
        collector.named("workflow.capacity.finish").is_empty(),
        "child span deliberately disabled"
    );
    let parents = collector.named("capacity.filtered.caller");
    assert_eq!(parents.len(), 1);
    assert_eq!(parents[0].trace_id, trace.to_string());
    assert_eq!(parents[0].attr("sagat.outcome"), Some("caller_pending"));
    assert_eq!(
        parents[0].status, 0,
        "Finish cannot set caller error status"
    );
    assert_eq!(
        total(
            &metrics.render(),
            "workflow_capacity_finish_attempts_total",
            Some("storage_error")
        ),
        1,
        "filtering does not disable metrics"
    );
    assert_eq!(finish_counts(&n.f).await, (0, 0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn returned_outcomes_export_closed_spans_and_only_confirmed_first_commits_count() {
    let n = super::barriers::runtime_fixture().await;
    let head = complete_native_activity(&n).await;
    let command = native_command(&n, head).await;
    let uncertain = super::barriers::runtime_fixture().await;
    let uncertain_head = complete_native_activity(&uncertain).await;
    let uncertain_command = native_command(&uncertain, uncertain_head).await;
    // The relay deliberately requires reply loss before Z/ReadyForQuery. Retain a test-only
    // deferred COMMIT gate so a fast server cannot finish before the controller shuts the reply.
    let commit_key = i64::try_from(Uuid::new_v4().as_u128() & ((1_u128 << 63) - 1))
        .expect("bounded advisory key");
    let ddl = format!(
        "CREATE FUNCTION capacity_observation_commit_gate() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({commit_key}); RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER capacity_observation_commit_gate AFTER INSERT ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION capacity_observation_commit_gate();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&uncertain.f.w.db.pool)
        .await
        .expect("test-only COMMIT gate");
    let mut commit_gate = uncertain
        .f
        .w
        .db
        .pool
        .begin()
        .await
        .expect("COMMIT gate owner");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(commit_key)
        .execute(&mut *commit_gate)
        .await
        .expect("COMMIT gate held");
    let mut proxy = CommitAckLossProxy::start(&uncertain.f.w.db, CommitResolution::Committed).await;
    let proxy_store = PgCapacityFinishStore::new(proxy.pool.clone());
    let collector = Collector::start().await;
    let (_tracing, sink, logs) = collector.install_with_logs(1.0);
    // The guard remains on this current-thread runtime across awaits. No capacity writer runs
    // on a blocking thread or another test runtime, and no process-global recorder is installed.
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    let parent = SpanId::generate();
    let span =
        tracing::info_span!(target:"sagat_test", "capacity.test",trace_id=%trace,span_id=%parent);
    async {
        let store = PgCapacityFinishStore::new(n.f.w.execution_pool.clone());
        let mut refused = command.clone();
        refused.scheduler = SchedulerAuthority::for_run("sk-live1234567890abcdef", head, 0);
        assert_eq!(store.finish_current_child_root(&refused).await,Err(CapacityFinishError::SchedulerFenceLost));
        let mut gate=n.f.w.db.pool.begin().await.expect("external G gate");
        sqlx::query("SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE").bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.group).execute(&mut *gate).await.expect("G guard");
        assert_eq!(store.finish_current_child_root(&command).await,Err(CapacityFinishError::Busy));
        gate.rollback().await.expect("release G");
        sqlx::raw_sql("CREATE FUNCTION refuse_capacity_observation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test-only capacity observation fault' USING ERRCODE='23514'; END $$; CREATE CONSTRAINT TRIGGER refuse_capacity_observation AFTER INSERT ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION refuse_capacity_observation();").execute(&n.f.w.db.pool).await.expect("confirmed deferred commit fault");
        assert!(matches!(store.finish_current_child_root(&command).await,Err(CapacityFinishError::Storage(sagat_application::error::StoreError::Internal{detail})) if detail.contains("test-only capacity observation fault")));
        assert_eq!(finish_counts(&n.f).await,(0,0,0));
        assert_eq!(total(&metrics.render(),"workflow_capacity_member_finished_total",None),0);
        sqlx::raw_sql("DROP TRIGGER refuse_capacity_observation ON capacity_group_finish_receipts; DROP FUNCTION refuse_capacity_observation();").execute(&n.f.w.db.pool).await.expect("remove deferred fault");
        assert!(!store.finish_current_child_root(&command).await.expect("confirmed first commit").replayed);
        assert!(store.finish_current_child_root(&command).await.expect("receipt replay").replayed);
        assert_eq!(total(&metrics.render(),"workflow_capacity_member_finished_total",None),1);
        proxy.arm();
        let (unknown,()) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(proxy_store.finish_current_child_root(&uncertain_command),async {
                proxy.wait_sent().await;
                proxy.lose_reply().await;
                commit_gate.rollback().await.expect("resolve only after reply is lost");
                assert_eq!(proxy.resolution().await,CommitResolution::Committed);
            })
        }).await.expect("actual native COMMIT acknowledgement loss");
        assert_eq!(unknown,Err(CapacityFinishError::CommitOutcomeUnknown{operation_key:uncertain_command.operation_key}));
        assert_eq!(finish_counts(&uncertain.f).await,(1,1,1),"server committed without acknowledgement");
        assert_eq!(total(&metrics.render(),"workflow_capacity_member_finished_total",None),1,"uncertain commit never becomes a confirmed local transition");
        assert!(PgCapacityFinishStore::new(uncertain.f.w.execution_pool.clone()).finish_current_child_root(&uncertain_command).await.expect("same-command recovery").replayed);
        assert_eq!(total(&metrics.render(),"workflow_capacity_member_finished_total",None),1,"recovery replay never counts a second transition");
    }.instrument(span).await;
    proxy.close().await;
    sink.shutdown(Duration::from_secs(5)).await;
    let page = metrics.render();
    for (outcome, count) in [
        ("applied", 1),
        ("replayed", 2),
        ("busy", 1),
        ("refused", 1),
        ("storage_error", 1),
        ("unknown", 1),
    ] {
        assert_eq!(
            total(
                &page,
                "workflow_capacity_finish_attempts_total",
                Some(outcome)
            ),
            count,
            "{page}"
        );
        assert_eq!(
            total(
                &page,
                "workflow_capacity_finish_duration_seconds_count",
                Some(outcome)
            ),
            count,
            "{page}"
        );
    }
    let spans = collector.named("workflow.capacity.finish");
    assert_eq!(spans.len(), 7, "{spans:?}");
    for (group, outcomes) in [
        (
            n.f.group,
            &["applied", "replayed", "busy", "refused", "storage_error"][..],
        ),
        (uncertain.f.group, &["unknown", "replayed"][..]),
    ] {
        let mut actual = spans
            .iter()
            .filter(|span| span.attr("sagat.capacity.group_id") == Some(group.to_string().as_str()))
            .map(|span| span.attr("sagat.outcome").expect("outcome"))
            .collect::<Vec<_>>();
        let mut expected = outcomes.to_vec();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "per-group actual outcome counts");
    }
    for observed in &spans {
        assert_eq!(observed.trace_id, trace.to_string());
        assert_eq!(observed.parent, Some(parent.to_string()));
        assert!(observed.end >= observed.start);
        let outcome = observed.attr("sagat.outcome").expect("closed outcome");
        assert!(
            [
                "applied",
                "replayed",
                "busy",
                "refused",
                "storage_error",
                "unknown"
            ]
            .contains(&outcome)
        );
        assert_eq!(
            observed.status,
            if ["storage_error", "unknown"].contains(&outcome) {
                2
            } else {
                0
            }
        );
        let f = if observed.attr("sagat.capacity.group_id") == Some(n.f.group.to_string().as_str())
        {
            &n.f
        } else {
            &uncertain.f
        };
        for (key, value) in [
            ("sagat.tenant.id", f.w.admin.tenant_id().to_string()),
            ("sagat.namespace.id", f.w.namespace.id().to_string()),
            ("sagat.capacity.group_id", f.group.to_string()),
            ("sagat.capacity.member_id", f.member.to_string()),
            ("sagat.chain.id", f.chain.to_string()),
            ("sagat.run.id", f.chain.to_string()),
        ] {
            assert_eq!(observed.attr(key), Some(value.as_str()), "{observed:?}");
        }
        assert_eq!(
            observed.attributes.len(),
            7,
            "identities plus finite outcome only: {observed:?}"
        );
    }
    for line in page
        .lines()
        .filter(|line| line.starts_with("workflow_capacity_"))
    {
        if line.starts_with("workflow_capacity_member_finished_total") {
            assert!(
                !line.contains('{'),
                "transition counter has no labels: {line}"
            );
            continue;
        }
        let bucket = line.starts_with("workflow_capacity_finish_duration_seconds_bucket{");
        if let Some((_, labels)) = line.split_once('{') {
            let labels = labels.split_once('}').expect("labels").0;
            assert_eq!(
                labels.split(',').count(),
                if bucket { 2 } else { 1 },
                "exact contracted labels: {line}"
            );
            for label in labels.split(',') {
                let (key, value) = label.split_once('=').expect("label");
                match key {
                    "outcome" => assert!(
                        [
                            "applied",
                            "replayed",
                            "busy",
                            "refused",
                            "storage_error",
                            "unknown"
                        ]
                        .contains(&value.trim_matches('"'))
                    ),
                    "le" => assert!(bucket, "bucket label only: {line}"),
                    _ => panic!("unbounded label {line}"),
                }
            }
        }
    }
    let captured = logs.text();
    assert_eq!(
        captured
            .lines()
            .filter(|line| line.contains("capacity Finish attempt completed"))
            .count(),
        7,
        "one fixed log per returned attempt"
    );
    for unsafe_text in [
        "sk-live1234567890abcdef",
        &command.operation_key.to_string(),
        &uncertain_command.operation_key.to_string(),
    ] {
        assert!(
            !captured.contains(unsafe_text),
            "unnecessary or secret-shaped operation data logged"
        );
        assert!(!page.contains(unsafe_text));
    }
    assert_eq!(finish_counts(&n.f).await, (1, 1, 1));
    assert_eq!(finish_counts(&uncertain.f).await, (1, 1, 1));
}
