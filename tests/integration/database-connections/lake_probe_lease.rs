//! The claim and the lease of the probe of an Iceberg connection (feature 035, D035-150 slices S2b and S3), over the worker gRPC.
//!
//! Three runners stand in for the three kinds of session a deployment has: one of the lake profile that offers the probe beside its write,
//! one of the lake profile that offers the write alone (a runner of the version before the probe), and one of the SQL profile. What is held:
//! the probe is handed to the first and to no other; its lease is the document the contract pins and nothing else; its connection and its
//! two secrets are handed over as a write's are; the probe has a budget of its own, so that it neither waits for a write nor makes one
//! wait; and what a runner reports as the result of a probe is not accepted yet (the typed result is the next slice), so that no text of a
//! runner's choosing reaches the journal.
//!
//! It runs inside the lifecycle scenario of `main.rs`, with the OpenBao of that scenario.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::append;
use super::lake_probe_start::{platform, reference};
use crate::support::worker::{Platform, Worker};
use reqwest::Method;
use sagat_application::{
    secret_provider::SecretProvider,
    worker::lake_probe_input::{ProbeFacts, lake_probe_input},
};
use sagat_domain::{
    database::{ICEBERG_PROBE_ACTIVITY, ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY},
    identity::{NamespaceId, Role},
};
use serde_json::json;
use std::sync::Arc;

async fn bind(p: &Platform, ns: &str, sub: &str, activities: &[&str], profile: &str) {
    p.grant(p.tenant, sub, Role::DatabaseRunner).await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/worker-bindings/{sub}"),
            Some(&json!({"activities": activities, "profile": profile})),
        )
        .await;
    assert_eq!(status, 200, "{sub}: {body}");
}

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let (p, ns, runs) = platform(&provider, true).await;
    let namespace: NamespaceId = ns.parse().expect("a namespace id");
    let admin = sagat_domain::identity::Principal::new(
        p.tenant,
        crate::support::subject(crate::support::worker::HUMAN),
        Role::Admin,
    );
    let ctx = crate::support::engine::ctx();

    bind(
        &p,
        &ns,
        "probe-runner",
        &[ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY],
        "lake",
    )
    .await;
    bind(&p, &ns, "write-runner", &[ICEBERG_WRITE_ACTIVITY], "lake").await;
    bind(
        &p,
        &ns,
        "sql-runner",
        &[MANAGED_DATABASE_ACTIVITY],
        "database",
    )
    .await;
    let mut probing = p.connect("probe-runner", "acme").await;
    probing
        .register_version(&[ICEBERG_WRITE_ACTIVITY, ICEBERG_PROBE_ACTIVITY], "1.6")
        .await;
    let mut writing = p.connect("write-runner", "acme").await;
    writing
        .register_version(&[ICEBERG_WRITE_ACTIVITY], "1.6")
        .await;
    let mut sql = p.connect("sql-runner", "acme").await;
    sql.register_version(&[MANAGED_DATABASE_ACTIVITY], "1.6")
        .await;

    // --- The probe is handed to the runner that offers it, and to no other.
    let first = runs
        .start_lake_probe(
            &admin,
            namespace,
            &reference("probe-warehouse", None),
            "lake-probe-lease-0001",
            &ctx,
        )
        .await
        .expect("the probe starts");
    assert!(
        writing.poll(8).await.is_empty(),
        "a lake runner that offers the write alone is not handed a probe, though it is of the same profile"
    );
    assert!(
        sql.poll(8).await.is_empty(),
        "a SQL runner is not handed a probe"
    );
    let lease = probing.one_task().await;
    assert_eq!(lease.activity_name, ICEBERG_PROBE_ACTIVITY);
    assert_eq!(lease.run_id, first.info.run_id.to_string());

    // --- Its lease is the document the contract pins, from the connection and the revision it was admitted with, and nothing else.
    let expected = lake_probe_input(&ProbeFacts {
        connection: "probe-warehouse",
        revision: 1,
    });
    assert_eq!(lease.input_json.as_bytes(), expected.as_slice());
    for never in [
        "probe-catalog",
        "probe-storage",
        "secret",
        "catalog.example",
        "s3://",
    ] {
        assert!(
            !lease.input_json.contains(never),
            "{never} is not in the lease of a probe"
        );
    }

    // --- Its connection and its two secrets are handed over as a write's are, read and pinned for the attempt.
    let resolved = probing
        .client
        .resolve_database(probing.req(Worker::resolve_secrets_message(&lease)))
        .await
        .expect("the connection of a probe is resolved")
        .into_inner();
    assert_eq!(resolved.connection_name, "probe-warehouse");
    assert!(
        resolved.iceberg.is_some() && resolved.postgres.is_none() && resolved.credential.is_none()
    );
    let mut roles: Vec<_> = resolved
        .iceberg
        .as_ref()
        .expect("the Iceberg connection")
        .credentials
        .keys()
        .cloned()
        .collect();
    roles.sort();
    assert_eq!(roles, ["catalog", "storage"]);
    let pins: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM step_database_credentials WHERE tenant_id = $1 AND run_id = $2 AND node_id = 'probe'",
    )
    .bind(p.tenant.as_uuid())
    .bind(first.info.run_id.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the pins");
    assert_eq!(
        pins, 2,
        "each secret of the connection is pinned to the attempt under its role"
    );

    // --- The probe has a budget of its own: with one in flight, a second waits, and a write is not made to wait by it.
    let second = runs
        .start_lake_probe(
            &admin,
            namespace,
            &reference("probe-warehouse", None),
            "lake-probe-lease-0002",
            &ctx,
        )
        .await
        .expect("a second probe starts");
    assert!(
        probing.poll(8).await.is_empty(),
        "a second probe waits while one is in flight in the namespace"
    );
    let workflow = p
        .publish(
            "lake-probe-budget",
            &append(
                "lake-probe-budget",
                "probe-warehouse",
                &["/id", "/total"],
                Some(super::lake::rows_schema()),
            ),
        )
        .await;
    let write_run = p
        .start(&workflow, &json!({"rows": [{"id": "r-1", "total": 3}]}))
        .await;
    let write = writing.one_task().await;
    assert_eq!(write.activity_name, ICEBERG_WRITE_ACTIVITY);
    assert_eq!(
        write.run_id, write_run,
        "a probe in flight does not make a write wait"
    );

    // --- What a runner reports is read into typed fields and written again: a report that is not the result of this probe is refused, says
    // nothing of what it held, and leaves the step as it was; nothing of a runner's choosing reaches a table, the journal or the API.
    let canary = format!("CANARY-PROBE-{}", uuid::Uuid::now_v7().simple());
    let connection = json!({"name": "probe-warehouse", "revision": 1});
    let passed = |check: &str| json!({"check": check, "outcome": "PASSED"});
    let skipped = |check: &str| json!({"check": check, "outcome": "SKIPPED"});
    let ok_checks = json!([
        passed("CATALOG_AUTH"),
        passed("WAREHOUSE"),
        passed("STORAGE_ACCESS")
    ]);
    let hostile = [
        (
            "a member the result does not have",
            json!({"kind": "probe", "connection": connection, "checks": ok_checks, "error": format!("Traceback: {canary}")}),
        ),
        (
            "text where a code belongs",
            json!({"kind": "probe", "connection": connection, "checks": [
                {"check": "CATALOG_AUTH", "outcome": "FAILED", "code": format!("{canary}: refused by 10.0.0.7")},
                skipped("WAREHOUSE"), skipped("STORAGE_ACCESS")]}),
        ),
        (
            "the result of another connection",
            json!({"kind": "probe", "connection": {"name": "another-lake", "revision": 1}, "checks": ok_checks}),
        ),
        (
            "the result of another revision",
            json!({"kind": "probe", "connection": {"name": "probe-warehouse", "revision": 2}, "checks": ok_checks}),
        ),
        (
            "a failed check with no code",
            json!({"kind": "probe", "connection": connection, "checks": [
                {"check": "CATALOG_AUTH", "outcome": "FAILED"}, skipped("WAREHOUSE"), skipped("STORAGE_ACCESS")]}),
        ),
        (
            "a check that passed after one that failed",
            json!({"kind": "probe", "connection": connection, "checks": [
                {"check": "CATALOG_AUTH", "outcome": "FAILED", "code": "DATABASE_TLS_FAILED"},
                passed("WAREHOUSE"), skipped("STORAGE_ACCESS")]}),
        ),
        (
            "a receipt of a write",
            json!({"kind": "iceberg", "outcome": "COMMITTED"}),
        ),
        ("anything a runner chooses", json!({"anything": canary})),
    ];
    let mut said_back = Vec::new();
    for (index, (name, body)) in hostile.iter().enumerate() {
        let refused = probing
            .client
            .complete_step(probing.req(super::lake_canaries::completion(
                &lease,
                body,
                &format!("probe-hostile-{index}"),
            )))
            .await
            .expect_err(name);
        said_back.push((name, super::lake_canaries::said(&refused)));
        assert!(
            matches!(
                refused.code(),
                tonic::Code::InvalidArgument
                    | tonic::Code::FailedPrecondition
                    | tonic::Code::PermissionDenied
            ),
            "{name}: {refused:?}"
        );
        assert_eq!(
            p.status(&first.info.run_id.to_string()).await,
            "RUNNING",
            "{name}: the step is as it was"
        );
    }
    for (name, text) in &said_back {
        assert!(
            !text.contains(&canary),
            "{name}: a refusal does not repeat what it was handed"
        );
    }
    crate::support::eventually("the refused results are audited as denials of the run", || async {
        let recorded: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'database.denied' AND resource_id = $2 AND reason = 'lake probe result refused'",
        )
        .bind(p.tenant.as_uuid())
        .bind(first.info.run_id.to_string())
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the audit records");
        (recorded > 0).then_some(recorded)
    })
    .await;
    let tables = super::lake_canaries::tables(&p).await;
    assert!(
        super::lake_canaries::holders(&p, &tables, &canary)
            .await
            .is_empty(),
        "no table holds what a refused result said"
    );
    let completed = |run: uuid::Uuid| {
        let (pool, tenant) = (p.api.db.pool.clone(), p.tenant.as_uuid());
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM run_events WHERE tenant_id = $1 AND run_id = $2 AND event_type = 'step.completed'",
            )
            .bind(tenant)
            .bind(run)
            .fetch_one(&pool)
            .await
            .expect("the events")
        }
    };
    assert_eq!(
        completed(first.info.run_id.as_uuid()).await,
        0,
        "and the step is not completed"
    );

    // --- A result that is the probe's completes it, and what the run keeps is the server's own writing of the typed fields: a failed check
    // is the answer, and the step has succeeded.
    let result = json!({"kind": "probe", "connection": {"revision": 1, "name": "probe-warehouse"}, "checks": [
        {"outcome": "FAILED", "check": "CATALOG_AUTH", "code": "DATABASE_AUTHENTICATION_FAILED"},
        {"outcome": "SKIPPED", "check": "WAREHOUSE"},
        {"outcome": "SKIPPED", "check": "STORAGE_ACCESS"},
    ]});
    let first_ack = probing
        .client
        .complete_step(probing.req(super::lake_canaries::completion(
            &lease,
            &result,
            "probe-result-1",
        )))
        .await
        .expect("the result of the probe completes it")
        .into_inner();
    assert!(!first_ack.already_applied);
    assert_eq!(p.status(&first.info.run_id.to_string()).await, "SUCCEEDED");
    let stored: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM steps s JOIN payloads p ON p.tenant_id = s.tenant_id AND p.digest = s.output_digest WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = 'probe'",
    )
    .bind(p.tenant.as_uuid())
    .bind(first.info.run_id.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the retained output of the probe");
    assert_eq!(
        String::from_utf8(stored).expect("UTF-8"),
        r#"{"kind":"probe","connection":{"name":"probe-warehouse","revision":1},"checks":[{"check":"CATALOG_AUTH","outcome":"FAILED","code":"DATABASE_AUTHENTICATION_FAILED"},{"check":"WAREHOUSE","outcome":"SKIPPED"},{"check":"STORAGE_ACCESS","outcome":"SKIPPED"}]}"#,
        "what the run keeps is the server's writing, not the bytes the runner sent"
    );
    // the same report again is the same completion
    let replay = probing
        .client
        .complete_step(probing.req(super::lake_canaries::completion(
            &lease,
            &result,
            "probe-result-1",
        )))
        .await
        .expect("a replay")
        .into_inner();
    assert!(replay.already_applied);
    assert_eq!(
        completed(first.info.run_id.as_uuid()).await,
        1,
        "a replay is not a second completion"
    );

    // --- When the first probe ends, the second is leased; a failure with a closed code ends a probe as well.
    let next = probing.one_task().await;
    assert_eq!(next.activity_name, ICEBERG_PROBE_ACTIVITY);
    assert_eq!(next.run_id, second.info.run_id.to_string());
    // A probe that never asked for the credentials of the connection has tested nothing: its result is refused until it has resolved them.
    let unresolved = json!({"kind": "probe", "connection": {"name": "probe-warehouse", "revision": 1}, "checks": [
        {"check": "CATALOG_AUTH", "outcome": "PASSED"},
        {"check": "WAREHOUSE", "outcome": "PASSED"},
        {"check": "STORAGE_ACCESS", "outcome": "PASSED"},
    ]});
    let refused = probing
        .client
        .complete_step(probing.req(super::lake_canaries::completion(
            &next,
            &unresolved,
            "probe-unresolved",
        )))
        .await
        .expect_err("a probe that never resolved its credentials");
    assert!(
        matches!(
            refused.code(),
            tonic::Code::InvalidArgument
                | tonic::Code::FailedPrecondition
                | tonic::Code::PermissionDenied
        ),
        "{refused:?}"
    );
    assert_eq!(p.status(&second.info.run_id.to_string()).await, "RUNNING");
    probing
        .client
        .resolve_database(probing.req(Worker::resolve_secrets_message(&next)))
        .await
        .expect("the connection is resolved");
    probing
        .fail_with(&next, "DATABASE_CREDENTIAL_UNAVAILABLE", false)
        .await
        .expect("a closed code fails the step");
    assert_eq!(p.status(&second.info.run_id.to_string()).await, "FAILED");
}
