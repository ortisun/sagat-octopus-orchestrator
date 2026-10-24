//! Public child-context refusals for dormant flat OrdinaryLeafV1 Finish.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. The positive source in
//! `certified_flow` belongs only to its original flat Run. These tests create a separate ordinary
//! collection child and a continuation child through public workflow/run paths, and deliberately
//! create no certificate for either. Owner-seeded capacity rows are labelled registry fixtures,
//! never child-source certification.

use super::*;
use reqwest::{Method, StatusCode};
use sagat_domain::execution::RunId;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildContextSnapshot {
    source: String,
    execution: String,
}

#[derive(Clone, Copy)]
struct PublicChild {
    parent: RunId,
    child: RunId,
}

#[derive(Clone)]
struct ChildFinishFacts {
    identity: LeafCollectionPathIdentity,
    run_revision: u64,
    scheduler: SchedulerAuthority,
}

async fn child_context_snapshot(f: &Flow, run: RunId) -> ChildContextSnapshot {
    let source: String = sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(\
         'scopes',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.scope_id),'[]'::jsonb) \
          FROM iteration_scopes s WHERE s.tenant_id=$1 AND s.run_id=$2),\
         'iterations',(SELECT COALESCE(jsonb_agg(to_jsonb(i) ORDER BY i.scope_id,i.iteration_index),\
          '[]'::jsonb) FROM collection_iterations i WHERE i.tenant_id=$1 AND i.run_id=$2),\
         'checkpoints',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.scope_id,c.iteration_index),\
          '[]'::jsonb) FROM iteration_checkpoints c WHERE c.tenant_id=$1 AND c.run_id=$2),\
         'certificates',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.owner_node,c.iteration_index),\
          '[]'::jsonb) FROM leaf_collection_success_certifications c \
          WHERE c.tenant_id=$1 AND c.run_id=$2))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact child source snapshot");
    let execution: String = sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(\
         'run',(SELECT to_jsonb(r) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2),\
         'events',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY e.seq),'[]'::jsonb) \
          FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2),\
         'steps',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.instance_path_indices,s.node),\
          '[]'::jsonb) FROM steps s WHERE s.tenant_id=$1 AND s.run_id=$2),\
         'segments',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.chain_id,s.segment_index),\
          '[]'::jsonb) FROM chain_segments s WHERE s.tenant_id=$1 AND s.run_id=$2),\
         'chains',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.chain_id),'[]'::jsonb) \
          FROM execution_chains c WHERE c.tenant_id=$1 AND \
          (c.current_run_id=$2 OR EXISTS(SELECT 1 FROM chain_segments s \
             WHERE s.tenant_id=$1 AND s.chain_id=c.chain_id AND s.run_id=$2))),\
         'links',(SELECT COALESCE(jsonb_agg(to_jsonb(l) ORDER BY l.parent_run_id,l.parent_instance_id),\
          '[]'::jsonb) FROM child_chain_links l WHERE l.tenant_id=$1 \
          AND (l.parent_run_id=$2 OR EXISTS(SELECT 1 FROM chain_segments s \
             WHERE s.tenant_id=$1 AND s.chain_id=l.child_chain_id AND s.run_id=$2))))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact child execution and topology snapshot");
    ChildContextSnapshot { source, execution }
}

async fn start_public_child_context(
    f: &Flow,
    label: &str,
    chained: bool,
    cancel_policy: Option<&str>,
) -> PublicChild {
    let suffix = Uuid::now_v7();
    let child_name = format!("ctx-child-{label}-{suffix}");
    let child_document = if chained {
        // The current public contract requires a real continueAsNew node for chainPolicy. This
        // execution takes the terminal branch, producing a genuine one-segment FINAL chain.
        json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": child_name.clone()},
            "entrypoint": "work",
            "nodes": [
                {"id": "work", "type": "activity", "activity": "line.reserve", "next": "again"},
                {"id": "again", "type": "condition", "expression": "input.continue",
                 "ifTrue": "rollover", "ifFalse": "done"},
                {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
                {"id": "done", "type": "end"}
            ],
            "chainPolicy": {"maxSegments": 2, "timeoutMs": 600_000}
        })
    } else {
        crate::support::scope_flow::collection_document(
            &child_name,
            vec![
                json!({
                    "id": "work", "type": "activity", "activity": "line.reserve",
                    "next": "returned"
                }),
                json!({"id": "returned", "type": "return", "result": "steps.work.output"}),
            ],
            1,
        )
    };
    crate::support::scope_flow::publish(&f.api, &child_name, child_document)
        .await
        .expect("public ordinary collection child publishes");

    let parent_name = format!("ctx-parent-{label}-{suffix}");
    let mut child_node = json!({
        "id": "kid", "type": "child", "workflowId": child_name.clone(), "next": "done"
    });
    if let Some(cancel_policy) = cancel_policy {
        child_node["cancelPolicy"] = json!(cancel_policy);
    }
    let parent_document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": parent_name.clone()},
        "entrypoint": "kid",
        "nodes": [child_node, {"id": "done", "type": "end"}]
    });
    let parent_workflow =
        crate::support::scope_flow::publish(&f.api, &parent_name, parent_document)
            .await
            .expect("public parent publishes");
    let parent_input = if chained {
        json!({"continue": false, "next": {"continue": false}})
    } else {
        json!({"lines": [{"v": 1}]})
    };
    let started = f
        .api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": parent_workflow, "input": parent_input}))
        .send()
        .await
        .expect("start public parent");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let parent: RunId = crate::support::api::json_of(started).await["runId"]
        .as_str()
        .expect("public parent Run id")
        .parse()
        .expect("valid public parent Run id");
    let child = sagat_application::execution::child_run_id(
        parent,
        &NodeId::parse("kid").expect("public child node"),
    );
    f.drive().await;
    let actual_parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_run_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid())
            .bind(child.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("scheduler created the public child Run");
    assert_eq!(actual_parent, Some(parent.as_uuid()));
    PublicChild { parent, child }
}

async fn finish_facts_for_run(f: &Flow, child: RunId) -> ChildFinishFacts {
    let (namespace, revision): (Uuid, i64) =
        sqlx::query_as("SELECT namespace_id,run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.tenant.as_uuid())
            .bind(child.as_uuid())
            .fetch_one(&f.api.db.pool)
            .await
            .expect("actual child Run identity");
    let shard = Shard::of(child.as_uuid());
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=$1")
            .bind(i16::from(shard.index()))
            .fetch_one(&f.api.db.pool)
            .await
            .expect("actual child scheduler authority");
    let facts = ChildFinishFacts {
        identity: LeafCollectionPathIdentity::new(
            f.tenant,
            NamespaceId::new(namespace),
            child,
            InstancePath::root()
                .child(NodeId::parse("lines").expect("ordinary child owner"), 0)
                .expect("depth-one child path"),
        )
        .expect("qualified ordinary child path"),
        run_revision: u64::try_from(revision).expect("nonnegative child Run revision"),
        scheduler: SchedulerAuthority::for_run(
            owner,
            child,
            u64::try_from(generation).expect("nonnegative child scheduler generation"),
        ),
    };
    let renewed = f
        .api
        .executions
        .renew(
            facts.scheduler.owner(),
            &[facts.scheduler.lease()],
            Duration::from_secs(600),
        )
        .await
        .expect("keep actual child scheduler authority current for context checks");
    assert_eq!(renewed, vec![facts.scheduler.lease()]);
    facts
}

async fn revoke_context_reads_and_probe(pool: &PgPool, f: &Flow) {
    sqlx::query(
        "REVOKE SELECT ON leaf_collection_success_certifications,\
         capacity_group_finish_receipts FROM sagat_app",
    )
    .execute(&f.api.db.pool)
    .await
    .expect("test-only context-before-source restrictions");
    let (runtime_role, certificate_select, receipt_select): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user,\
             has_table_privilege(current_user,'leaf_collection_success_certifications','SELECT'),\
             has_table_privilege(current_user,'capacity_group_finish_receipts','SELECT')",
    )
    .fetch_one(pool)
    .await
    .expect("effective context read privileges");
    assert_eq!(runtime_role, "sagat_app");
    assert_eq!((certificate_select, receipt_select), (false, false));

    for (table, statement) in [
        (
            "leaf_collection_success_certifications",
            "SELECT * FROM leaf_collection_success_certifications LIMIT 0",
        ),
        (
            "capacity_group_finish_receipts",
            "SELECT * FROM capacity_group_finish_receipts LIMIT 0",
        ),
    ] {
        let error = sqlx::query(statement)
            .fetch_optional(pool)
            .await
            .expect_err("runtime-role context read must be denied");
        assert_eq!(
            error
                .as_database_error()
                .and_then(|database| database.code())
                .as_deref(),
            Some("42501"),
            "unexpected {table} denial: {error}"
        );
    }
}

async fn assert_child_context_precedes_source(
    f: &Flow,
    pool: &PgPool,
    facts: &ChildFinishFacts,
    seeded: SeededGroup,
    label: &str,
) {
    let command = ScopedLeafFinishCommand {
        operation_key: Uuid::new_v4(),
        group_id: seeded.group,
        member_id: seeded.selected,
        identity: facts.identity.clone(),
        expected: ScopedLeafFinishExpectations {
            group_revision: 10,
            group_epoch: 4,
            run_revision: facts.run_revision,
        },
        scheduler: facts.scheduler.clone(),
    };
    let source_before = child_context_snapshot(f, facts.identity.run_id()).await;
    let capacity_before = capacity_snapshot(f, seeded.group).await;

    let mut stale = command.clone();
    stale.scheduler = SchedulerAuthority::for_run(
        format!("stale-context-{label}"),
        facts.identity.run_id(),
        facts.scheduler.lease().generation,
    );
    assert_eq!(
        PgScopedLeafFinishStore::new(pool.clone())
            .finish_scoped_leaf(&stale)
            .await,
        Err(ScopedLeafFinishError::SchedulerFenceLost),
        "current scheduler authority precedes {label} context"
    );
    assert_eq!(
        PgScopedLeafFinishStore::new(pool.clone())
            .finish_scoped_leaf(&command)
            .await,
        Err(ScopedLeafFinishError::UnsupportedRunContext),
        "{label} context precedes both denied source and receipt reads"
    );
    assert_eq!(
        child_context_snapshot(f, facts.identity.run_id()).await,
        source_before,
        "{label} refusal changes no child source or execution state"
    );
    assert_eq!(
        capacity_snapshot(f, seeded.group).await,
        capacity_before,
        "{label} refusal changes no capacity state"
    );
    assert_eq!(finish_counts(f, seeded.group).await, (0, 0, 0));
}

#[tokio::test]
async fn native_scoped_finish_refuses_actual_public_child_before_and_after_detachment() {
    let (mut f, flat_sources, pool) = certified_flow("scoped-finish-context-child", 1).await;
    // Keep the acquired placement while enabling real public child creation and settlement.
    f.scheduler = f
        .scheduler
        .with_children(DefinitionCache::new(Arc::new(JsonIrCodec), 8));
    let family = start_public_child_context(&f, "detached", false, Some("detach")).await;
    let facts = finish_facts_for_run(&f, family.child).await;
    let source_counts: (i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM leaf_collection_success_certifications \
          WHERE tenant_id=$1 AND run_id=$2),\
         (SELECT count(*) FROM leaf_collection_success_certifications \
          WHERE tenant_id=$1 AND run_id=$3)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(family.child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("flat versus child certificate inventory");
    assert_eq!(source_counts, (1, 0));
    assert_eq!(flat_sources.len(), 1);

    let live_context: (Option<Uuid>, bool, bool, i64) = sqlx::query_as(
        "SELECT parent_run_id,parent_settled,finished_at IS NULL,\
         (SELECT count(*) FROM chain_segments s WHERE s.tenant_id=r.tenant_id \
          AND s.namespace_id=r.namespace_id AND s.run_id=r.id) \
         FROM runs r WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(family.child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual live non-chain child context");
    assert_eq!(
        live_context,
        (Some(family.parent.as_uuid()), false, true, 0)
    );

    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            // Registry identity only: no child certificate or nested closure is claimed.
            facts.identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    revoke_context_reads_and_probe(&pool, &f).await;
    assert_child_context_precedes_source(&f, &pool, &facts, seeded, "live child").await;

    let cancelled = f
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{}/operations", family.parent),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"action": "CANCEL", "reason": "detach the public child"}))
        .send()
        .await
        .expect("cancel public parent");
    assert_eq!(cancelled.status(), StatusCode::ACCEPTED);
    f.drive().await;
    let detached_context: (Option<Uuid>, bool, bool, String, i64) = sqlx::query_as(
        "SELECT child.parent_run_id,child.parent_settled,child.finished_at IS NULL,parent.status,\
         (SELECT count(*) FROM chain_segments s WHERE s.tenant_id=child.tenant_id \
          AND s.namespace_id=child.namespace_id AND s.run_id=child.id) \
         FROM runs child JOIN runs parent ON parent.tenant_id=child.tenant_id \
          AND parent.id=child.parent_run_id \
         WHERE child.tenant_id=$1 AND child.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(family.child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual detached child context");
    assert_eq!(
        detached_context,
        (
            Some(family.parent.as_uuid()),
            true,
            true,
            "CANCELLED".into(),
            0,
        )
    );
    assert_child_context_precedes_source(&f, &pool, &facts, seeded, "detached child").await;
}

#[tokio::test]
async fn native_scoped_finish_refuses_actual_settled_final_child_chain_segment() {
    let (mut f, flat_sources, pool) = certified_flow("scoped-finish-context-final-chain", 1).await;
    // Keep the acquired placement while enabling real public child creation and settlement.
    f.scheduler = f
        .scheduler
        .with_children(DefinitionCache::new(Arc::new(JsonIrCodec), 8));
    let family = start_public_child_context(&f, "final-chain", true, None).await;
    let initial_chain: (Option<Uuid>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT r.parent_run_id,s.completion_kind,c.final_outcome FROM runs r \
         JOIN chain_segments s ON s.tenant_id=r.tenant_id AND s.namespace_id=r.namespace_id \
          AND s.run_id=r.id \
         JOIN execution_chains c ON c.tenant_id=s.tenant_id AND c.chain_id=s.chain_id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(family.child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual public continuation-child chain membership");
    assert_eq!(initial_chain, (Some(family.parent.as_uuid()), None, None));

    let tasks = f.poll().await;
    let child_task = tasks
        .iter()
        .find(|task| task.run_id == family.child.to_string())
        .expect("public continuation-child activity task");
    f.complete(child_task, json!({"accepted": true})).await;
    f.drive().await;

    let final_context: (
        Option<Uuid>,
        bool,
        String,
        String,
        String,
        Option<String>,
        bool,
    ) = sqlx::query_as(
        "SELECT r.parent_run_id,r.finished_at IS NOT NULL,r.status,s.completion_kind,\
             c.final_outcome,l.final_outcome,l.settled FROM runs r \
             JOIN chain_segments s ON s.tenant_id=r.tenant_id AND s.namespace_id=r.namespace_id \
              AND s.run_id=r.id \
             JOIN execution_chains c ON c.tenant_id=s.tenant_id AND c.chain_id=s.chain_id \
             JOIN child_chain_links l ON l.tenant_id=c.tenant_id AND l.namespace_id=c.namespace_id \
              AND l.child_chain_id=c.chain_id \
             WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(family.child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual settled FINAL child chain context");
    assert_eq!(
        final_context,
        (
            Some(family.parent.as_uuid()),
            true,
            "SUCCEEDED".into(),
            "FINAL".into(),
            "SUCCEEDED".into(),
            Some("SUCCEEDED".into()),
            true,
        )
    );

    let facts = finish_facts_for_run(&f, family.child).await;
    let source_counts: (i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM leaf_collection_success_certifications \
          WHERE tenant_id=$1 AND run_id=$2),\
         (SELECT count(*) FROM leaf_collection_success_certifications \
          WHERE tenant_id=$1 AND run_id=$3)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(family.child.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("flat versus FINAL child certificate inventory");
    assert_eq!(source_counts, (1, 0));
    assert_eq!(flat_sources.len(), 1);

    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            // Registry identity only: FINAL chain history is not canonical leaf certification.
            facts.identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    revoke_context_reads_and_probe(&pool, &f).await;
    assert_child_context_precedes_source(
        &f,
        &pool,
        &facts,
        seeded,
        "settled FINAL child chain segment",
    )
    .await;
}

#[tokio::test]
async fn native_scoped_finish_refuses_actual_parentless_historical_chain_segment() {
    let (f, flat_sources, pool) = certified_flow("scoped-finish-context-historical-chain", 1).await;
    let name = format!("ctx-history-{}", Uuid::now_v7());
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name.clone()},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "line.reserve", "next": "again"},
            {"id": "again", "type": "condition", "expression": "input.continue",
             "ifTrue": "rollover", "ifFalse": "done"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 2, "timeoutMs": 600_000}
    });
    let workflow = crate::support::scope_flow::publish(&f.api, &name, document)
        .await
        .expect("public parentless continuation workflow publishes");
    let started = f
        .api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({
            "workflowId": workflow,
            "input": {"continue": true, "next": {"continue": false}}
        }))
        .send()
        .await
        .expect("start public parentless continuation chain");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let original: RunId = crate::support::api::json_of(started).await["runId"]
        .as_str()
        .expect("original public chain Run id")
        .parse()
        .expect("valid original public chain Run id");
    f.drive().await;
    let tasks = f.poll().await;
    let original_task = tasks
        .iter()
        .find(|task| task.run_id == original.to_string())
        .expect("original public chain activity task");
    f.complete(original_task, json!({"accepted": true})).await;
    f.drive().await;

    let historical: (
        Option<Uuid>,
        bool,
        String,
        String,
        Uuid,
        Option<Uuid>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT r.parent_run_id,r.finished_at IS NOT NULL,r.status,s.completion_kind,\
         c.current_run_id,s.successor_run_id,c.final_outcome FROM runs r \
         JOIN chain_segments s ON s.tenant_id=r.tenant_id AND s.namespace_id=r.namespace_id \
          AND s.run_id=r.id \
         JOIN execution_chains c ON c.tenant_id=s.tenant_id AND c.namespace_id=s.namespace_id \
          AND c.chain_id=s.chain_id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(original.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual public parentless historical chain segment");
    let successor = historical.4;
    assert_ne!(successor, original.as_uuid());
    assert_eq!(
        historical,
        (
            None,
            true,
            "SUCCEEDED".into(),
            "CONTINUED".into(),
            successor,
            Some(successor),
            None,
        )
    );

    let facts = finish_facts_for_run(&f, original).await;
    let source_counts: (i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM leaf_collection_success_certifications \
          WHERE tenant_id=$1 AND run_id=$2),\
         (SELECT count(*) FROM leaf_collection_success_certifications \
          WHERE tenant_id=$1 AND run_id=$3)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .bind(original.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("flat versus historical chain certificate inventory");
    assert_eq!(source_counts, (1, 0));
    assert_eq!(flat_sources.len(), 1);

    let seeded = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            // Registry identity only: a continued segment is not nested source certification.
            facts.identity.clone(),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    revoke_context_reads_and_probe(&pool, &f).await;
    assert_child_context_precedes_source(
        &f,
        &pool,
        &facts,
        seeded,
        "parentless historical chain segment",
    )
    .await;
}
