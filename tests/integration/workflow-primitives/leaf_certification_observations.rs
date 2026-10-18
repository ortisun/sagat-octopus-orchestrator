//! Actual native outcomes, isolated metric counts, OTLP export and redaction.

use super::{
    commit_loss::install_commit_probe,
    commit_proxy::{CommitAckLossProxy, CommitResolution},
    refusals::{closed, closed_with_output, counts},
    *,
};
use crate::support::collector::Collector;
use sagat_telemetry::{Metrics, SpanId, TraceId};
use tracing::{Instrument, instrument::WithSubscriber};
use tracing_subscriber::layer::SubscriberExt;

const SPAN: &str = "workflow.collection.leaf.certify";
const ATTEMPTS: &str = "workflow_collection_leaf_certification_attempts_total";
const CONFIRMED: &str = "workflow_collection_leaf_certified_total";
const DURATION: &str = "workflow_collection_leaf_certification_duration_seconds_count";

fn total(page: &str, name: &str, outcome: Option<&str>) -> u64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && outcome.is_none_or(|o| line.contains(&format!("outcome=\"{o}\"")))
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
async fn filtered_leaf_span_preserves_callers_outcome_and_records_storage_error_metric() {
    let (f, _, c) = closed("leaf-filtered-observation").await;
    let pool = f.api.db.runtime_pool_with(1).await;
    pool.close().await;
    let store =
        PgLeafCollectionSuccessStore::new(pool, DefinitionCache::new(Arc::new(JsonIrCodec), 8));
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
        let parent = tracing::info_span!(target:"sagat_test", "leaf.filtered.caller",trace_id=%trace,sagat.outcome="caller_pending",otel.status_code=tracing::field::Empty);
        async {assert!(matches!(store.certify_leaf_collection_success(&c).await, Err(LeafCollectionSuccessError::Storage(_))));}.instrument(parent).await;
    }.with_subscriber(subscriber).await;
    sink.shutdown(Duration::from_secs(5)).await;
    assert!(collector.named(SPAN).is_empty());
    let parents = collector.named("leaf.filtered.caller");
    assert_eq!(parents.len(), 1);
    assert_eq!(parents[0].trace_id, trace.to_string());
    assert_eq!(parents[0].attr("sagat.outcome"), Some("caller_pending"));
    assert_eq!(parents[0].status, 0);
    assert!(parents[0].status_message.is_empty());
    assert_eq!(total(&metrics.render(), ATTEMPTS, Some("storage_error")), 1);
    assert_eq!(total(&metrics.render(), CONFIRMED, None), 0);
    assert_eq!(counts(&f).await, (0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn actual_leaf_outcomes_export_closed_metadata_and_count_only_acknowledged_first_commits() {
    let (f, store, c) = closed_with_output(
        "leaf-observations",
        json!({"private":"LEAF_OUTPUT_PAYLOAD_CANARY"}),
    )
    .await;
    let (uncertain, direct, unknown_command) = closed("leaf-observations-unknown").await;
    let key = i64::try_from(
        u64::from_be_bytes(
            Uuid::new_v4().as_bytes()[..8]
                .try_into()
                .expect("eight bytes"),
        ) >> 1,
    )
    .expect("positive key");
    install_commit_probe(&uncertain, key, CommitResolution::Committed).await;
    let mut gate = uncertain
        .api
        .db
        .pool
        .begin()
        .await
        .expect("test COMMIT gate");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *gate)
        .await
        .expect("hold COMMIT gate");
    let mut proxy = CommitAckLossProxy::start(&uncertain.api.db, CommitResolution::Committed).await;
    let proxied = PgLeafCollectionSuccessStore::new(
        proxy.pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    // Prepare synthetic DDL outside capture: SQLx's debug query logger includes DDL literals.
    // The failing native COMMIT itself remains inside the observation/canary boundary.
    sqlx::raw_sql("CREATE FUNCTION refuse_leaf_observation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'LEAF_OBSERVATION_DRIVER_CANARY' USING ERRCODE='23514'; END $$; CREATE CONSTRAINT TRIGGER refuse_leaf_observation AFTER INSERT ON leaf_collection_success_certifications DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION refuse_leaf_observation();")
        .execute(&f.api.db.pool).await.expect("deferred actual COMMIT abort");
    let collector = Collector::start().await;
    let (_tracing, sink) = collector.install(1.0);
    let logs = crate::support::logs::Captured::default();
    let subscriber = tracing_subscriber::registry()
        .with(sagat_telemetry::OtlpLayer::new(sink.clone(), 1.0))
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(logs.clone())
                .with_ansi(false),
        );
    let _json_logs = tracing::subscriber::set_default(subscriber);
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    let parent = SpanId::generate();
    let span =
        tracing::info_span!(target:"sagat_test", "leaf.test", trace_id=%trace,span_id=%parent);
    async {
        let mut invalid = c.clone();
        invalid.expected_run_revision = 0;
        assert_eq!(store.certify_leaf_collection_success(&invalid).await, Err(LeafCollectionSuccessError::InvalidRevision));
        let mut fenced = c.clone();
        fenced.scheduler = SchedulerAuthority::for_run("sk-live-leaf-canary123456", f.run, 0);
        assert_eq!(store.certify_leaf_collection_success(&fenced).await, Err(LeafCollectionSuccessError::SchedulerFenceLost));
        assert!(matches!(store.certify_leaf_collection_success(&c).await, Err(LeafCollectionSuccessError::Storage(_))));
        assert_eq!(counts(&f).await, (0, 0));
        assert_eq!(total(&metrics.render(), CONFIRMED, None), 0);
        sqlx::raw_sql("DROP TRIGGER refuse_leaf_observation ON leaf_collection_success_certifications; DROP FUNCTION refuse_leaf_observation();")
            .execute(&f.api.db.pool).await.expect("remove actual fault");
        assert!(!store.certify_leaf_collection_success(&c).await.expect("acknowledged first commit").replayed);
        assert!(store.certify_leaf_collection_success(&c).await.expect("canonical replay").replayed);
        assert_eq!(total(&metrics.render(), CONFIRMED, None), 1);
        proxy.arm();
        let (unknown, ()) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(proxied.certify_leaf_collection_success(&unknown_command), async {
                proxy.wait_sent().await;
                proxy.lose_reply().await;
                gate.rollback().await.expect("release only after reply lost");
                assert_eq!(proxy.resolution().await, CommitResolution::Committed);
            })
        }).await.expect("actual Unknown outcome deadline");
        assert_eq!(unknown, Err(LeafCollectionSuccessError::CommitOutcomeUnknown{identity:unknown_command.identity.clone()}));
        assert_eq!(counts(&uncertain).await, (1, 1));
        assert_eq!(total(&metrics.render(), CONFIRMED, None), 1, "Unknown cannot become confirmed from a server-side inference");
        assert!(direct.certify_leaf_collection_success(&unknown_command).await.expect("same identity reconciliation").replayed);
        assert_eq!(total(&metrics.render(), CONFIRMED, None), 1, "replay cannot count another first commit");
    }.instrument(span).await;
    proxy.close().await;
    sink.shutdown(Duration::from_secs(5)).await;
    let page = metrics.render();
    for (outcome, count) in [
        ("certified", 1),
        ("replayed", 2),
        ("refused", 2),
        ("storage_error", 1),
        ("unknown", 1),
    ] {
        assert_eq!(total(&page, ATTEMPTS, Some(outcome)), count, "{page}");
        assert_eq!(total(&page, DURATION, Some(outcome)), count, "{page}");
    }
    let spans = collector.named(SPAN);
    assert_eq!(spans.len(), 7);
    for (flow, outcomes) in [
        (
            &f,
            vec![
                "certified",
                "replayed",
                "refused",
                "refused",
                "storage_error",
            ],
        ),
        (&uncertain, vec!["replayed", "unknown"]),
    ] {
        let run = flow.run.to_string();
        let mut actual: Vec<_> = spans
            .iter()
            .filter(|s| s.attr("sagat.run.id") == Some(run.as_str()))
            .map(|s| s.attr("sagat.outcome").expect("outcome"))
            .collect();
        let mut expected = outcomes;
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected);
    }
    for observed in &spans {
        assert_eq!(observed.trace_id, trace.to_string());
        assert_eq!(observed.parent, Some(parent.to_string()));
        assert!(observed.end >= observed.start);
        assert!(
            observed.status_message.is_empty(),
            "native spans export no error description"
        );
        assert!(observed.events.is_empty());
        assert!(observed.links.is_empty());
        let outcome = observed.attr("sagat.outcome").expect("closed outcome");
        assert_eq!(
            observed.status,
            if ["storage_error", "unknown"].contains(&outcome) {
                2
            } else {
                0
            }
        );
        let command = if observed.attr("sagat.run.id") == Some(f.run.to_string().as_str()) {
            &c
        } else {
            &unknown_command
        };
        for (key, value) in [
            ("sagat.tenant.id", command.identity.tenant_id().to_string()),
            (
                "sagat.namespace.id",
                command.identity.namespace_id().to_string(),
            ),
            ("sagat.run.id", command.identity.run_id().to_string()),
            ("sagat.collection.owner_node", "lines".into()),
            ("sagat.collection.index", "0".into()),
        ] {
            assert_eq!(observed.attr(key), Some(value.as_str()));
        }
        assert_eq!(
            observed.attributes.len(),
            6,
            "qualified identity and finite outcome only"
        );
    }
    for line in page
        .lines()
        .filter(|l| l.starts_with("workflow_collection_leaf_"))
    {
        if line.starts_with(CONFIRMED) {
            assert!(!line.contains('{'));
            continue;
        }
        if let Some((_, labels)) = line.split_once('{') {
            let labels = labels.split_once('}').expect("labels").0;
            let bucket =
                line.starts_with("workflow_collection_leaf_certification_duration_seconds_bucket{");
            assert_eq!(labels.split(',').count(), if bucket { 2 } else { 1 });
            for label in labels.split(',') {
                let (key, value) = label.split_once('=').expect("label");
                match key {
                    "outcome" => assert!(
                        [
                            "certified",
                            "replayed",
                            "refused",
                            "storage_error",
                            "unknown"
                        ]
                        .contains(&value.trim_matches('"'))
                    ),
                    "le" => assert!(bucket),
                    _ => panic!("unbounded metric label"),
                }
            }
        }
    }
    let captured = logs.text();
    let events: Vec<Value> = captured
        .lines()
        .map(|line| serde_json::from_str(line).expect("actual structured event"))
        .collect();
    let completed: Vec<_> = events
        .iter()
        .filter(|event| event["target"] == "sagat_infra_postgres::leaf_certification")
        .collect();
    assert_eq!(completed.len(), 7);
    for event in completed {
        assert_eq!(event["level"], "DEBUG");
        let fields = event["fields"].as_object().expect("event fields");
        assert_eq!(fields.len(), 2, "fixed message and closed outcome only");
        assert_eq!(fields["message"], "leaf certification attempt completed");
        assert!(
            [
                "certified",
                "replayed",
                "refused",
                "storage_error",
                "unknown"
            ]
            .contains(&fields["outcome"].as_str().expect("closed outcome"))
        );
    }
    let exported = format!("{spans:?}");
    for canary in [
        "sk-live-leaf-canary123456",
        "LEAF_OBSERVATION_DRIVER_CANARY",
        "LEAF_OUTPUT_PAYLOAD_CANARY",
    ] {
        assert!(
            !captured.contains(canary),
            "marker leaked from actual native operations: {canary}"
        );
        assert!(!page.contains(canary));
        assert!(!exported.contains(canary));
    }
    assert_eq!(counts(&f).await, (1, 1));
    assert_eq!(counts(&uncertain).await, (1, 1));
}
