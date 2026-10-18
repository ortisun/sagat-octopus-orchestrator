//! Returned observations for dormant flat OrdinaryLeafV1 Finish.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. These tests use only the existing
//! dormant native store, genuine public-source certification, owner-seeded capacity membership,
//! the shared capacity observation vocabulary and the existing capacity COMMIT relay.

use super::*;
use crate::{
    capacity_finish::commit_proxy::{CommitAckLossProxy, CommitResolution},
    support::{collector::Collector, scope_flow::RunHold},
};
use sagat_telemetry::{Metrics, SpanId, TraceId};
use tracing::{Instrument, instrument::WithSubscriber};
use tracing_subscriber::layer::SubscriberExt;

const SPAN: &str = "workflow.capacity.finish";
const ATTEMPTS: &str = "workflow_capacity_finish_attempts_total";
const DURATION: &str = "workflow_capacity_finish_duration_seconds_count";
const TRANSITIONS: &str = "workflow_capacity_member_finished_total";
const OWNER_NODE_CANARY: &str = "scoped_owner_canary";
const BODY_NAME_CANARY: &str = "scoped_body_canary";
const BODY_NODE_CANARY: &str = "scoped_source_body_canary";
const SOURCE_PAYLOAD_CANARY: &str = "SCOPED_SOURCE_PAYLOAD_CANARY";
const SOURCE_OUTPUT_CANARY: &str = "SCOPED_SOURCE_OUTPUT_CANARY";
const SCHEDULER_OWNER_CANARY: &str = "sk-live-scoped-observation-secret-canary";
const RAW_SQL_CANARY: &str = "SCOPED_FINISH_RAW_SQL_CANARY";
const SQL_DETAIL_CANARY: &str = "SCOPED_FINISH_DETAIL_CANARY";
const SQL_HINT_CANARY: &str = "SCOPED_FINISH_HINT_CANARY";
const WAIT: Duration = Duration::from_secs(10);
const RUN_ENTRY: &str =
    "SELECT workflow_version_id,run_version,parent_run_id IS NULL AS parentless";

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
                .expect("metric sample")
                .parse::<u64>()
                .expect("integer metric count")
        })
        .sum()
}

fn advisory_key() -> i64 {
    i64::try_from(
        u64::from_be_bytes(
            Uuid::new_v4().as_bytes()[..8]
                .try_into()
                .expect("eight UUID bytes"),
        ) >> 1,
    )
    .expect("positive advisory key")
}

fn observation_document(name: &str) -> Value {
    let mut document = document(name);
    document["entrypoint"] = json!(OWNER_NODE_CANARY);
    document["nodes"][0]["id"] = json!(OWNER_NODE_CANARY);
    document["nodes"][0]["body"] = json!(BODY_NAME_CANARY);

    let body = document["bodies"]
        .as_object_mut()
        .expect("workflow bodies")
        .remove("line")
        .expect("ordinary source body");
    document["bodies"]
        .as_object_mut()
        .expect("workflow bodies")
        .insert(BODY_NAME_CANARY.into(), body);
    document["bodies"][BODY_NAME_CANARY]["entrypoint"] = json!(BODY_NODE_CANARY);
    document["bodies"][BODY_NAME_CANARY]["nodes"][0]["id"] = json!(BODY_NODE_CANARY);
    document["bodies"][BODY_NAME_CANARY]["nodes"][1]["result"] =
        json!(format!("steps.{BODY_NODE_CANARY}.output"));
    document
}

/// One genuinely executed/certified ordinary source whose retained owner, body and values are
/// distinctive telemetry canaries. Capacity identity remains an explicit dormant owner fixture.
async fn observation_flow(name: &str) -> (Flow, CertifyLeafCollectionSuccess, PgPool) {
    let f = Flow::start(
        name,
        observation_document(name),
        json!([{"v":1,"payload":SOURCE_PAYLOAD_CANARY}]),
    )
    .await;
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("temporary native source writer");
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0]
            .instance
            .as_ref()
            .expect("ordinary scoped task")
            .instance_path[0]
            .scope_node_id,
        OWNER_NODE_CANARY
    );
    f.complete(
        &tasks[0],
        json!({"accepted":true,"payload":SOURCE_OUTPUT_CANARY}),
    )
    .await;
    f.drive().await;
    f.verify().await;

    let source_store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let mut source = command(&f).await;
    source.identity = identity_at(&source.identity, OWNER_NODE_CANARY, 0);
    let certified = source_store
        .certify_leaf_collection_success(&source)
        .await
        .expect("genuine native OrdinaryLeafV1 certificate");
    assert!(!certified.replayed);
    assert_eq!(certified.record.identity, source.identity);

    let lease = source.scheduler.lease();
    assert_eq!(
        f.api
            .executions
            .renew(source.scheduler.owner(), &[lease], Duration::from_secs(600))
            .await
            .expect("keep current native authority"),
        vec![lease]
    );
    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("remove native source writer before Finish");
    sqlx::query(
        "GRANT INSERT (member_kind,original_run_id,root_owner_node,iteration_index) \
         ON capacity_group_finish_receipts TO sagat_app",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("test-only scoped receipt extensions");

    let pool = f.api.db.runtime_pool_with(2).await;
    let user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("explicit native role");
    assert_eq!(user, "sagat_app");
    (f, source, pool)
}

async fn seeded_observation(name: &str) -> (Flow, ScopedLeafFinishCommand, PgPool) {
    let (f, source, pool) = observation_flow(name).await;
    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            source.identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(&source, seeded, 10, 4);
    (f, command, pool)
}

async fn install_unknown_commit_gate(f: &Flow, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION scoped_finish_observation_commit_gate() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({key}); \
         RETURN NEW; END $$; \
         CREATE CONSTRAINT TRIGGER scoped_finish_observation_commit_gate \
         AFTER INSERT ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED \
         FOR EACH ROW EXECUTE FUNCTION scoped_finish_observation_commit_gate();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(&f.api.db.pool)
        .await
        .expect("test-only deferred COMMIT gate");
}

async fn actual_run_waiter(pool: &PgPool) -> i32 {
    loop {
        let waiter: Option<i32> = sqlx::query_scalar(
            "SELECT leader.pid FROM pg_stat_activity leader \
             WHERE leader.datname=current_database() AND leader.usename='sagat_app' \
             AND leader.wait_event_type='Lock' AND position($1 in leader.query)>0 \
             AND EXISTS(SELECT 1 FROM pg_stat_activity holder \
                 WHERE holder.datname=leader.datname \
                 AND holder.pid=ANY(pg_blocking_pids(leader.pid)) \
                 AND position('SELECT 1 FROM runs' in holder.query)>0) \
             LIMIT 1",
        )
        .bind(RUN_ENTRY)
        .fetch_optional(pool)
        .await
        .expect("actual original-Run waiter observation");
        if let Some(waiter) = waiter {
            return waiter;
        }
        tokio::task::yield_now().await;
    }
}

async fn waiter_stopped(pool: &PgPool, pid: i32) {
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
             WHERE pid=$1 AND datname=current_database() AND wait_event_type='Lock')",
        )
        .bind(pid)
        .fetch_one(pool)
        .await
        .expect("cancelled waiter state");
        if !waiting {
            return;
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn filtered_scoped_finish_span_cannot_overwrite_its_callers_outcome_or_status() {
    let (f, sources, pool) = certified_flow("scoped-finish-observation-filtered", 1).await;
    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            sources[0].identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(&sources[0], seeded, 10, 4);
    pool.close().await;
    let store = PgScopedLeafFinishStore::new(pool);
    let collector = Collector::start().await;
    let (_collector_guard, sink) = collector.install(1.0);
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            "sagat_test=info,sagat_infra_postgres=warn",
        ))
        .with(sagat_telemetry::OtlpLayer::new(sink.clone(), 1.0));
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let _metrics_guard = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    async {
        let parent = tracing::info_span!(
            target: "sagat_test",
            "scoped.finish.filtered.caller",
            trace_id = %trace,
            sagat.outcome = "caller_pending",
            otel.status_code = tracing::field::Empty,
        );
        async {
            let error = store
                .finish_scoped_leaf(&command)
                .await
                .expect_err("closed native pool returns Storage");
            assert!(matches!(&error, ScopedLeafFinishError::Storage(_)));
            let display_debug = format!("{error} {error:?}");
            let operation_key = command.operation_key.to_string();
            for canary in [command.scheduler.owner(), "lines", operation_key.as_str()] {
                assert!(
                    !display_debug.contains(canary),
                    "application Display/Debug disclosed closed-pool command data"
                );
            }
        }
        .instrument(parent)
        .await;
    }
    .with_subscriber(subscriber)
    .await;
    sink.shutdown(Duration::from_secs(5)).await;

    assert!(collector.named(SPAN).is_empty(), "child span is filtered");
    let parents = collector.named("scoped.finish.filtered.caller");
    assert_eq!(parents.len(), 1);
    assert_eq!(parents[0].trace_id, trace.to_string());
    assert_eq!(parents[0].attr("sagat.outcome"), Some("caller_pending"));
    assert_eq!(parents[0].status, 0);
    assert!(parents[0].status_message.is_empty());
    assert_eq!(total(&metrics.render(), ATTEMPTS, Some("storage_error")), 1);
    assert_eq!(total(&metrics.render(), TRANSITIONS, None), 0);
    assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
}

#[tokio::test(flavor = "current_thread")]
async fn returned_scoped_finish_outcomes_are_closed_redacted_and_count_only_acknowledged_apply() {
    let (f, command, pool) = seeded_observation("scoped-finish-observations").await;
    let (uncertain, uncertain_command, uncertain_pool) =
        seeded_observation("scoped-finish-observations-unknown").await;
    let key = advisory_key();
    install_unknown_commit_gate(&uncertain, key).await;
    let mut commit_gate = uncertain
        .api
        .db
        .pool
        .begin()
        .await
        .expect("COMMIT gate owner");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *commit_gate)
        .await
        .expect("hold deferred COMMIT gate");
    let mut proxy = CommitAckLossProxy::start(&uncertain.api.db, CommitResolution::Committed).await;
    let proxied_store = PgScopedLeafFinishStore::new(proxy.pool.clone());

    // Install the raw server diagnostic before log capture: only the actual failing native COMMIT
    // belongs to the observation boundary, not SQLx's debug rendering of test-only DDL literals.
    sqlx::raw_sql(
        "CREATE FUNCTION refuse_scoped_finish_observation() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN \
         RAISE EXCEPTION 'SCOPED_FINISH_RAW_SQL_CANARY' \
         USING ERRCODE='23514',DETAIL='SCOPED_FINISH_DETAIL_CANARY',\
         HINT='SCOPED_FINISH_HINT_CANARY'; END $$; \
         CREATE CONSTRAINT TRIGGER refuse_scoped_finish_observation \
         AFTER INSERT ON capacity_group_finish_receipts DEFERRABLE INITIALLY DEFERRED \
         FOR EACH ROW EXECUTE FUNCTION refuse_scoped_finish_observation();",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("confirmed deferred COMMIT fault");

    let collector = Collector::start().await;
    let (collector_guard, sink) = collector.install(1.0);
    let logs = crate::support::logs::Captured::default();
    let subscriber = tracing_subscriber::registry()
        .with(sagat_telemetry::OtlpLayer::new(sink.clone(), 1.0))
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(logs.clone())
                .with_ansi(false),
        );
    let subscriber_guard = tracing::subscriber::set_default(subscriber);
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let metrics_guard = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    let parent = SpanId::generate();
    let parent_span = tracing::info_span!(
        target: "sagat_test",
        "scoped.finish.test",
        trace_id = %trace,
        span_id = %parent,
    );
    let (applied, recovered) = async {
        let store = PgScopedLeafFinishStore::new(pool.clone());
        let mut refused = command.clone();
        refused.scheduler = SchedulerAuthority::for_run(SCHEDULER_OWNER_CANARY, f.run, 0);
        let refusal = store
            .finish_scoped_leaf(&refused)
            .await
            .expect_err("wrong scheduler owner is refused");
        assert_eq!(refusal, ScopedLeafFinishError::SchedulerFenceLost);
        assert!(!format!("{refusal} {refusal:?}").contains(SCHEDULER_OWNER_CANARY));

        let mut group_gate = f.api.db.pool.begin().await.expect("selected-G gate");
        sqlx::query(
            "SELECT group_id FROM capacity_groups \
             WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
        )
        .bind(f.tenant.as_uuid())
        .bind(command.group_id.as_uuid())
        .execute(&mut *group_gate)
        .await
        .expect("hold selected G");
        assert_eq!(
            store.finish_scoped_leaf(&command).await,
            Err(ScopedLeafFinishError::Busy)
        );
        group_gate.rollback().await.expect("release selected G");

        let before_fault = capacity_snapshot(&f, command.group_id).await;
        let storage = store
            .finish_scoped_leaf(&command)
            .await
            .expect_err("actual deferred COMMIT refusal");
        assert!(matches!(&storage, ScopedLeafFinishError::Storage(_)));
        let safe_application_error = format!("{storage} {storage:?}");
        for canary in [RAW_SQL_CANARY, SQL_DETAIL_CANARY, SQL_HINT_CANARY] {
            assert!(!safe_application_error.contains(canary));
        }
        assert_eq!(capacity_snapshot(&f, command.group_id).await, before_fault);
        assert_eq!(finish_counts(&f, command.group_id).await, (0, 0, 0));
        assert_eq!(total(&metrics.render(), TRANSITIONS, None), 0);
        sqlx::raw_sql(
            "DROP TRIGGER refuse_scoped_finish_observation \
             ON capacity_group_finish_receipts; \
             DROP FUNCTION refuse_scoped_finish_observation();",
        )
        .execute(&f.api.db.pool)
        .await
        .expect("remove confirmed COMMIT fault");

        let applied = store
            .finish_scoped_leaf(&command)
            .await
            .expect("acknowledged first application");
        assert!(!applied.replayed);
        let replay = store
            .finish_scoped_leaf(&command)
            .await
            .expect("exact original receipt replay");
        assert!(replay.replayed);
        assert_eq!(total(&metrics.render(), TRANSITIONS, None), 1);

        proxy.arm();
        let (unknown, ()) = tokio::time::timeout(WAIT, async {
            tokio::join!(
                proxied_store.finish_scoped_leaf(&uncertain_command),
                async {
                    proxy.wait_sent().await;
                    proxy.lose_reply().await;
                    commit_gate
                        .rollback()
                        .await
                        .expect("release only after COMMIT reply loss");
                    assert_eq!(proxy.resolution().await, CommitResolution::Committed);
                }
            )
        })
        .await
        .expect("actual native COMMIT acknowledgement-loss deadline");
        assert_eq!(
            unknown,
            Err(ScopedLeafFinishError::CommitOutcomeUnknown {
                operation_key: uncertain_command.operation_key,
            })
        );
        assert_eq!(
            finish_counts(&uncertain, uncertain_command.group_id).await,
            (1, 1, 1),
            "the server committed without acknowledgement"
        );
        assert_eq!(
            total(&metrics.render(), TRANSITIONS, None),
            1,
            "an unknown commit is not a confirmed local transition"
        );
        let recovered = PgScopedLeafFinishStore::new(uncertain_pool.clone())
            .finish_scoped_leaf(&uncertain_command)
            .await
            .expect("same-command fresh-pool reconciliation");
        assert!(recovered.replayed);
        assert_eq!(
            total(&metrics.render(), TRANSITIONS, None),
            1,
            "receipt replay never acknowledges the original COMMIT"
        );
        (applied, recovered)
    }
    .instrument(parent_span)
    .await;
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
        assert_eq!(total(&page, ATTEMPTS, Some(outcome)), count, "{page}");
        assert_eq!(total(&page, DURATION, Some(outcome)), count, "{page}");
    }
    assert_eq!(total(&page, ATTEMPTS, None), 7, "no extra attempts: {page}");
    assert_eq!(
        total(&page, DURATION, None),
        7,
        "no extra durations: {page}"
    );
    assert_eq!(total(&page, TRANSITIONS, None), 1, "{page}");

    let spans = collector.named(SPAN);
    assert_eq!(spans.len(), 7, "{spans:#?}");
    let primary_group = command.group_id.to_string();
    let uncertain_group = uncertain_command.group_id.to_string();
    for (group, outcomes) in [
        (
            command.group_id,
            &["applied", "replayed", "busy", "refused", "storage_error"][..],
        ),
        (uncertain_command.group_id, &["unknown", "replayed"][..]),
    ] {
        let group = group.to_string();
        let mut actual = spans
            .iter()
            .filter(|span| span.attr("sagat.capacity.group_id") == Some(group.as_str()))
            .map(|span| span.attr("sagat.outcome").expect("returned outcome"))
            .collect::<Vec<_>>();
        let mut expected = outcomes.to_vec();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected);
    }
    for observed in &spans {
        assert_eq!(observed.trace_id, trace.to_string());
        assert_eq!(observed.parent, Some(parent.to_string()));
        assert!(observed.end >= observed.start);
        assert!(observed.status_message.is_empty());
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
        let command = if observed.attr("sagat.capacity.group_id") == Some(primary_group.as_str()) {
            &command
        } else {
            assert_eq!(
                observed.attr("sagat.capacity.group_id"),
                Some(uncertain_group.as_str())
            );
            &uncertain_command
        };
        for (key, value) in [
            ("sagat.tenant.id", command.identity.tenant_id().to_string()),
            (
                "sagat.namespace.id",
                command.identity.namespace_id().to_string(),
            ),
            ("sagat.capacity.group_id", command.group_id.to_string()),
            ("sagat.capacity.member_id", command.member_id.to_string()),
            ("sagat.run.id", command.identity.run_id().to_string()),
        ] {
            assert_eq!(observed.attr(key), Some(value.as_str()), "{observed:#?}");
        }
        assert_eq!(
            observed.attributes.len(),
            6,
            "five retained identities plus finite outcome only: {observed:#?}"
        );
        for forbidden in [
            "sagat.chain.id",
            "sagat.capacity.holder_member_id",
            "sagat.collection.owner_node",
            "sagat.collection.index",
            "sagat.operation.key",
        ] {
            assert_eq!(observed.attr(forbidden), None, "{observed:#?}");
        }
    }

    for line in page
        .lines()
        .filter(|line| line.starts_with("workflow_capacity_"))
    {
        let series = line
            .split(['{', ' '])
            .next()
            .expect("capacity metric series");
        assert!(
            [
                ATTEMPTS,
                TRANSITIONS,
                "workflow_capacity_finish_duration_seconds_bucket",
                "workflow_capacity_finish_duration_seconds_sum",
                DURATION,
            ]
            .contains(&series),
            "unexpected capacity metric series: {line}"
        );
        if line.starts_with(TRANSITIONS) {
            assert!(!line.contains('{'), "transition count is unlabeled: {line}");
            continue;
        }
        {
            let (_, labels) = line
                .split_once('{')
                .expect("every non-transition metric has closed outcome labels");
            let labels = labels.split_once('}').expect("metric labels").0;
            let bucket = line.starts_with("workflow_capacity_finish_duration_seconds_bucket{");
            assert_eq!(
                labels.split(',').count(),
                if bucket { 2 } else { 1 },
                "exact fixed labels: {line}"
            );
            for label in labels.split(',') {
                let (key, value) = label.split_once('=').expect("metric label");
                match key {
                    "outcome" => assert!(
                        [
                            "applied",
                            "replayed",
                            "busy",
                            "refused",
                            "storage_error",
                            "unknown",
                        ]
                        .contains(&value.trim_matches('"'))
                    ),
                    "le" => assert!(bucket, "histogram boundary only: {line}"),
                    _ => panic!("unbounded metric label: {line}"),
                }
            }
        }
    }

    let captured = logs.text();
    let events: Vec<Value> = captured
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("structured log event"))
        .collect();
    let completed: Vec<_> = events
        .iter()
        .filter(|event| event["target"] == "sagat_infra_postgres::scoped_leaf_finish")
        .collect();
    assert_eq!(completed.len(), 7, "one fixed log per returned call");
    let logged_outcomes: Vec<_> = completed
        .iter()
        .map(|event| {
            event["fields"]["outcome"]
                .as_str()
                .expect("returned log outcome")
        })
        .collect();
    assert_eq!(
        logged_outcomes,
        [
            "refused",
            "busy",
            "storage_error",
            "applied",
            "replayed",
            "unknown",
            "replayed"
        ],
        "each sequential returned call logs its own exact outcome"
    );
    for event in completed {
        assert_eq!(event["level"], "DEBUG");
        let fields = event["fields"].as_object().expect("log fields");
        assert_eq!(fields.len(), 2, "fixed message and finite outcome only");
        assert_eq!(fields["message"], "capacity Finish attempt completed");
        assert!(
            [
                "applied",
                "replayed",
                "busy",
                "refused",
                "storage_error",
                "unknown",
            ]
            .contains(&fields["outcome"].as_str().expect("closed log outcome"))
        );
    }

    let exported = format!("{:#?}", collector.spans());
    let command_key = command.operation_key.to_string();
    let uncertain_key = uncertain_command.operation_key.to_string();
    let applied_audit = applied.audit_id.as_uuid().to_string();
    let applied_hint = applied.outbox_id.to_string();
    let recovered_audit = recovered.audit_id.as_uuid().to_string();
    let recovered_hint = recovered.outbox_id.to_string();
    for canary in [
        SCHEDULER_OWNER_CANARY,
        RAW_SQL_CANARY,
        SQL_DETAIL_CANARY,
        SQL_HINT_CANARY,
        OWNER_NODE_CANARY,
        BODY_NAME_CANARY,
        BODY_NODE_CANARY,
        SOURCE_PAYLOAD_CANARY,
        SOURCE_OUTPUT_CANARY,
        command_key.as_str(),
        uncertain_key.as_str(),
        applied_audit.as_str(),
        applied_hint.as_str(),
        recovered_audit.as_str(),
        recovered_hint.as_str(),
    ] {
        assert!(
            !captured.contains(canary),
            "canary leaked in logs: {canary}"
        );
        assert!(!page.contains(canary), "canary leaked in metrics: {canary}");
        assert!(
            !exported.contains(canary),
            "canary leaked in spans: {canary}"
        );
    }
    assert_eq!(finish_counts(&f, command.group_id).await, (1, 1, 1));
    assert_eq!(
        finish_counts(&uncertain, uncertain_command.group_id).await,
        (1, 1, 1)
    );

    drop(metrics_guard);
    drop(subscriber_guard);
    drop(collector_guard);
}

#[tokio::test(flavor = "current_thread")]
async fn abandoned_scoped_finish_waiting_on_run_has_no_returned_observation_or_effect() {
    let (f, sources, pool) = certified_flow("scoped-finish-observation-abandoned", 1).await;
    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            sources[0].identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    let command = finish_command(&sources[0], seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let capacity_before = capacity_snapshot(&f, command.group_id).await;
    let run_hold = RunHold::take(&f).await;

    let collector = Collector::start().await;
    let (collector_guard, sink) = collector.install(1.0);
    let logs = crate::support::logs::Captured::default();
    let subscriber = tracing_subscriber::registry()
        .with(sagat_telemetry::OtlpLayer::new(sink.clone(), 1.0))
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(logs.clone())
                .with_ansi(false),
        );
    let subscriber_guard = tracing::subscriber::set_default(subscriber);
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(600));
    let metrics_guard = metrics::set_default_local_recorder(&recorder);
    let trace = TraceId::generate();
    let parent = SpanId::generate();
    let store = PgScopedLeafFinishStore::new(pool.clone());
    let parent_span = tracing::info_span!(
        target: "sagat_test",
        "scoped.finish.abandoned.caller",
        trace_id = %trace,
        span_id = %parent,
    );
    let waiter = async {
        let mut pending = Box::pin(store.finish_scoped_leaf(&command));
        let waiter = tokio::select! {
            result = pending.as_mut() => panic!("native call bypassed held Run: {result:?}"),
            waiter = tokio::time::timeout(WAIT, actual_run_waiter(&f.api.db.pool)) => {
                waiter.expect("actual original-Run wait deadline")
            }
        };
        drop(pending);
        waiter
    }
    .instrument(parent_span)
    .await;
    run_hold.release().await;
    pool.close().await;
    tokio::time::timeout(WAIT, waiter_stopped(&f.api.db.pool, waiter))
        .await
        .expect("dropped native waiter cancellation deadline");

    let mut run_probe = f.api.db.pool.begin().await.expect("post-cancel Run probe");
    sqlx::query(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 \
         FOR UPDATE NOWAIT",
    )
    .bind(f.tenant.as_uuid())
    .bind(command.identity.namespace_id().as_uuid())
    .bind(f.run.as_uuid())
    .execute(&mut *run_probe)
    .await
    .expect("dropped transaction cannot later acquire or retain the Run lock");
    run_probe.rollback().await.expect("Run probe ends");

    sink.shutdown(Duration::from_secs(5)).await;
    let page = metrics.render();
    assert_eq!(total(&page, ATTEMPTS, None), 0, "{page}");
    assert_eq!(total(&page, DURATION, None), 0, "{page}");
    assert_eq!(total(&page, TRANSITIONS, None), 0, "{page}");
    assert_eq!(
        logs.text()
            .lines()
            .filter(|line| line.contains("capacity Finish attempt completed"))
            .count(),
        0,
        "an abandoned call has no returned-attempt log"
    );
    let abandoned = collector.named(SPAN);
    assert_eq!(abandoned.len(), 1, "the attempted child span still closes");
    assert_eq!(abandoned[0].trace_id, trace.to_string());
    assert_eq!(abandoned[0].parent, Some(parent.to_string()));
    assert_eq!(abandoned[0].attr("sagat.outcome"), None);
    assert_eq!(abandoned[0].status, 0);
    assert!(abandoned[0].status_message.is_empty());
    assert_eq!(abandoned[0].attributes.len(), 5);
    for (key, value) in [
        ("sagat.tenant.id", command.identity.tenant_id().to_string()),
        (
            "sagat.namespace.id",
            command.identity.namespace_id().to_string(),
        ),
        ("sagat.capacity.group_id", command.group_id.to_string()),
        ("sagat.capacity.member_id", command.member_id.to_string()),
        ("sagat.run.id", command.identity.run_id().to_string()),
    ] {
        assert_eq!(abandoned[0].attr(key), Some(value.as_str()));
    }
    for forbidden in [
        "sagat.chain.id",
        "sagat.collection.owner_node",
        "sagat.collection.index",
        "sagat.operation.key",
    ] {
        assert_eq!(abandoned[0].attr(forbidden), None);
    }
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        capacity_snapshot(&f, command.group_id).await,
        capacity_before
    );
    assert_eq!(finish_counts(&f, command.group_id).await, (0, 0, 0));

    drop(metrics_guard);
    drop(subscriber_guard);
    drop(collector_guard);

    let fresh_pool = f.api.db.runtime_pool_with(1).await;
    let applied = PgScopedLeafFinishStore::new(fresh_pool)
        .finish_scoped_leaf(&command)
        .await
        .expect("the unchanged command applies through a fresh runtime pool");
    assert!(!applied.replayed);
    assert_linked_effect(&f, &command, &applied).await;
    assert_eq!(finish_counts(&f, command.group_id).await, (1, 1, 1));
    assert_eq!(source_execution_snapshot(&f).await, source_before);
}
