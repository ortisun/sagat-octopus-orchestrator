//! Real PostgreSQL evidence for reopening admitted collection work.

use sagat_application::execution::{Actor, Effects, RunTx, instance_step_id};
use sagat_application::primitives::{
    CollectionAdmission, CollectionEntry, CollectionLimits, CollectionScopeId, CollectionStore,
    CollectionTransition, SchedulerAuthority,
};
use sagat_application::runs::{Lock, RunStore};
use sagat_application::shards::{Lease, ShardStore};
use sagat_domain::execution::{
    AddressedRunEvent, ExecutionInstanceId, InstancePath, RunEvent, RunId, WaitingOn,
};
use sagat_domain::identity::Subject;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sagat_infra_postgres::PgCollectionStore;
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use std::time::Duration;
use uuid::Uuid;

use crate::support::engine::{chain, world};

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

async fn retain_payload(pool: &sqlx::PgPool, tenant: Uuid, bytes: &[u8]) -> Digest {
    let value = digest(bytes);
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(value.as_bytes().as_slice())
    .bind(bytes)
    .execute(pool)
    .await
    .expect("retain payload");
    value
}

async fn collection_run(w: &crate::support::engine::World, owner: &ExecutionInstanceId) -> RunId {
    let (workflow, version) = w.publish(chain("collection-restore")).await;
    let run = RunId::generate();
    let tenant = w.admin.tenant_id().as_uuid();
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id, max_active_iterations, updated_by) VALUES ($1, 1, 'collection-restore')",
    )
    .bind(tenant)
    .execute(&w.db.pool)
    .await
    .expect("collection capacity policy");
    let input = retain_payload(&w.db.pool, tenant, b"{}").await;
    let now = crate::support::now();
    sqlx::query(
        "INSERT INTO runs \
         (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
          input_digest, started_by, trace_id, created_at, updated_at, root_run_id) \
         VALUES ($1, $2, $3, $4, $5, 'RUNNING', 1, $6, 'collection-restore', \
                 '0af7651916cd43dd8448eb211c80319c', $7, $7, $1)",
    )
    .bind(run.as_uuid())
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input.as_bytes().as_slice())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("collection run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         VALUES ($1, $2, 1, 'run.started', jsonb_build_object('entrypoint', $3::text), \
                 'collection-restore', $4)",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(owner.node().as_str())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("run start journal");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, 'PENDING', $6, '{}', '{}')",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(owner.node().as_str())
    .bind(instance_step_id(version, run, owner))
    .bind(w.namespace.id().as_uuid())
    .bind(now)
    .execute(&w.db.pool)
    .await
    .expect("pending collection owner");
    run
}

fn transition(
    w: &crate::support::engine::World,
    run: RunId,
    version: u64,
    owner: ExecutionInstanceId,
    lease: Lease,
) -> CollectionTransition {
    CollectionTransition::new(
        w.admin.tenant_id(),
        w.namespace.id(),
        run,
        version,
        owner,
        SchedulerAuthority::new("collection-restore", lease),
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        crate::support::now(),
        None,
    )
}

#[tokio::test]
async fn admitted_collection_reopens_with_exact_scope_and_verifies_from_addressed_journal() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("b").expect("owner node"));
    let run = collection_run(&w, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let lease = w
        .store
        .acquire(
            "collection-restore",
            &[Shard::of(run.as_uuid())],
            Duration::from_secs(60),
        )
        .await
        .expect("acquire collection shard")
        .into_iter()
        .next()
        .expect("collection shard is available");
    let store = PgCollectionStore::new(w.db.pool.clone());
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, br#"[1,2]"#).await;
    let item = digest(b"1");
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 1, owner.clone(), lease),
                scope,
                2,
                snapshot,
                CollectionLimits::new(2, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");
    let retained_item: Vec<u8> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(tenant)
            .bind(item.as_bytes().as_slice())
            .fetch_one(&w.db.pool)
            .await
            .expect("entry retains each captured item atomically");
    assert_eq!(retained_item, b"1");
    let member = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(owner.node().clone(), 0)
            .expect("member path"),
        NodeId::parse("work").expect("activity node"),
    );
    let admitted = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(&w, run, entered.run_version, owner.clone(), lease),
                scope,
                0,
                member.clone(),
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                Uuid::now_v7(),
            )
            .expect("admission"),
        )
        .await
        .expect("admit activity");

    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("open run")
        .expect("stored run");
    let state = tx.parts().1;
    assert_eq!(state.version(), admitted.run_version);
    assert!(matches!(
        state.instance(&owner).and_then(|step| step.waiting_on()),
        Some(WaitingOn::Collection)
    ));
    let collection = state.collection(&owner).expect("restored collection");
    assert_eq!(collection.item_count(), 2);
    assert_eq!(collection.max_items(), 2);
    assert_eq!(collection.max_concurrency(), 1);
    assert_eq!(collection.snapshot_digest(), snapshot);
    assert_eq!(collection.next_admission(), 1);
    assert_eq!(collection.active_count(), 1);
    assert_eq!(collection.entry_instance(0), Some(&member));
    assert_eq!(collection.item_digest(0), Some(item));
    drop(tx);

    w.assert_consistent(run).await;
}

#[tokio::test]
async fn addressed_persistence_keeps_scoped_journal_projection_and_output_identity() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("b").expect("owner node"));
    let run = collection_run(&w, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let lease = w
        .store
        .acquire(
            "collection-restore",
            &[Shard::of(run.as_uuid())],
            Duration::from_secs(60),
        )
        .await
        .expect("acquire collection shard")
        .into_iter()
        .next()
        .expect("collection shard is available");
    let store = PgCollectionStore::new(w.db.pool.clone());
    let scope = CollectionScopeId::generate();
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    let item = retain_payload(&w.db.pool, tenant, b"1").await;
    let entered = store
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 1, owner.clone(), lease),
                scope,
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");
    let member_path = InstancePath::root()
        .child(owner.node().clone(), 0)
        .expect("member path");
    let member = ExecutionInstanceId::in_path(
        member_path.clone(),
        NodeId::parse("work").expect("activity node"),
    );
    let returned = ExecutionInstanceId::in_path(
        member_path.clone(),
        NodeId::parse("returned").expect("return node"),
    );
    let admitted = store
        .admit_collection_activity(
            &CollectionAdmission::new(
                transition(&w, run, entered.run_version, owner, lease),
                scope,
                0,
                member.clone(),
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                Uuid::now_v7(),
            )
            .expect("admission"),
        )
        .await
        .expect("admit activity");
    let member_id = instance_step_id(
        w.store
            .get(w.admin.tenant_id(), run)
            .await
            .expect("run record")
            .expect("stored run")
            .info
            .version_id,
        run,
        &member,
    );
    let now = crate::support::now();
    let mut prepared = w.db.pool.begin().await.expect("prepare scoped attempt");
    sqlx::query(
        "UPDATE steps SET status = 'RUNNING', attempt = 1, generation = 1, \
                lease_token_hash = $7, lease_expires_at = $6 + interval '1 minute', \
                leased_by = 'scoped-worker', updated_at = $6 \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
           AND instance_path_nodes = ARRAY['b'] AND instance_path_indices = ARRAY[0] \
           AND step_id = $4 AND namespace_id = $5",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(member.node().as_str())
    .bind(member_id)
    .bind(w.namespace.id().as_uuid())
    .bind(now)
    .bind([7_u8; 32].as_slice())
    .execute(&mut *prepared)
    .await
    .expect("running scoped projection");
    for (offset, event_type) in [(1_u64, "step.leased"), (2, "step.started")] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id, run_id, seq, event_type, node, attempt, detail, actor, occurred_at, \
              instance_id, instance_path_nodes, instance_path_indices) \
             VALUES ($1, $2, $3, $4, $5, 1, '{\"attempt\":1,\"generation\":1}'::jsonb, \
                     'collection-restore', $6, $7, ARRAY['b'], ARRAY[0])",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(i64::try_from(admitted.run_version + offset).expect("sequence"))
        .bind(event_type)
        .bind(member.node().as_str())
        .bind(now)
        .bind(member_id)
        .execute(&mut *prepared)
        .await
        .expect("scoped attempt journal");
    }
    let running_version = admitted.run_version + 2;
    sqlx::query("UPDATE runs SET run_version = $3 WHERE tenant_id = $1 AND id = $2")
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(i64::try_from(running_version).expect("run version"))
        .execute(&mut *prepared)
        .await
        .expect("advance run version");
    prepared.commit().await.expect("commit scoped attempt");

    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("reopen running member")
        .expect("stored run");
    let events = [
        AddressedRunEvent::in_path(
            member_path.clone(),
            RunEvent::StepCompleted {
                node: member.node().clone(),
                attempt: 1,
                generation: 1,
            },
        )
        .expect("address completion"),
        AddressedRunEvent::in_path(
            member_path,
            RunEvent::StepRouted {
                node: member.node().clone(),
                activated: vec![returned.node().clone()],
            },
        )
        .expect("address routing"),
    ];
    for event in &events {
        tx.parts()
            .1
            .apply_addressed(event)
            .expect("apply scoped lifecycle");
    }
    let output = br#"{"ok":true}"#.to_vec();
    tx.persist_addressed(
        &events,
        &[(member.clone(), output.clone())],
        Effects::by(Actor::System("collection-restore"), None),
    )
    .await
    .expect("persist addressed lifecycle");
    tx.commit().await.expect("commit addressed lifecycle");

    let rows: Vec<(String, Vec<String>, Vec<i32>)> = sqlx::query_as(
        "SELECT event_type, instance_path_nodes, instance_path_indices FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 AND seq > $3 ORDER BY seq",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(i64::try_from(running_version).expect("run version"))
    .fetch_all(&w.db.pool)
    .await
    .expect("addressed journal rows");
    assert_eq!(
        rows,
        [
            ("step.completed".into(), vec!["b".into()], vec![0]),
            ("step.routed".into(), vec!["b".into()], vec![0]),
        ]
    );
    let projected: (String, bool, Vec<u8>) = sqlx::query_as(
        "SELECT s.status, s.routed, p.content FROM steps s JOIN payloads p \
           ON p.tenant_id = s.tenant_id AND p.digest = s.output_digest \
         WHERE s.tenant_id = $1 AND s.run_id = $2 AND s.node = $3 \
           AND s.instance_path_nodes = ARRAY['b'] AND s.instance_path_indices = ARRAY[0]",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(member.node().as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("scoped output projection");
    assert_eq!(projected, ("SUCCEEDED".into(), true, output));
    let returned_status: String = sqlx::query_scalar(
        "SELECT status FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
           AND instance_path_nodes = ARRAY['b'] AND instance_path_indices = ARRAY[0]",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(returned.node().as_str())
    .fetch_one(&w.db.pool)
    .await
    .expect("scoped return projection");
    assert_eq!(returned_status, "PENDING");
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn verification_detects_a_scope_projection_that_differs_from_the_journal() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("b").expect("owner node"));
    let run = collection_run(&w, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let lease = w
        .store
        .acquire(
            "collection-restore",
            &[Shard::of(run.as_uuid())],
            Duration::from_secs(60),
        )
        .await
        .expect("acquire collection shard")
        .into_iter()
        .next()
        .expect("collection shard is available");
    let snapshot = retain_payload(&w.db.pool, tenant, b"[1]").await;
    PgCollectionStore::new(w.db.pool.clone())
        .enter_collection(
            &CollectionEntry::new(
                transition(&w, run, 1, owner, lease),
                CollectionScopeId::generate(),
                1,
                snapshot,
                CollectionLimits::new(1, 1, 100_000, 8).expect("limits"),
            )
            .expect("entry"),
        )
        .await
        .expect("enter collection");
    sqlx::query(
        "UPDATE iteration_scopes SET max_concurrency = 2 \
         WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("corrupt scope projection");

    let error = w
        .store
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect_err("journal and projection must differ");
    assert!(
        error.contains("projection differs from the journal"),
        "{error}"
    );

    let stored: i32 = sqlx::query(
        "SELECT max_concurrency FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("stored scope")
    .get("max_concurrency");
    assert_eq!(stored, 2);
}

#[tokio::test]
async fn journal_attempt_spans_use_the_complete_instance_path() {
    let w = world().await;
    let owner = ExecutionInstanceId::root(NodeId::parse("b").expect("owner node"));
    let run = collection_run(&w, &owner).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let root_step = Uuid::now_v7();
    let scoped_step = Uuid::now_v7();
    let now = crate::support::now();
    for (step_id, path_nodes, path_indices) in [
        (root_step, Vec::<String>::new(), Vec::<i32>::new()),
        (scoped_step, vec!["b".to_owned()], vec![0]),
    ] {
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
              updated_at, instance_path_nodes, instance_path_indices) \
             VALUES ($1, $2, 'work', $3, $4, 'READY', 'line.process', $5, $5, $6, $7)",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(step_id)
        .bind(w.namespace.id().as_uuid())
        .bind(now)
        .bind(path_nodes)
        .bind(path_indices)
        .execute(&w.db.pool)
        .await
        .expect("logical work instance");
    }
    for (sequence, step_id, path_nodes, path_indices) in [
        (2_i64, root_step, Vec::<String>::new(), Vec::<i32>::new()),
        (3_i64, scoped_step, vec!["b".to_owned()], vec![0]),
    ] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id, run_id, seq, event_type, node, attempt, detail, actor, occurred_at, \
              instance_id, instance_path_nodes, instance_path_indices) \
             VALUES ($1, $2, $3, 'step.leased', 'work', 1, \
                     '{\"attempt\":1,\"generation\":1}'::jsonb, 'collection-restore', $4, \
                     $5, $6, $7)",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .bind(sequence)
        .bind(now)
        .bind(step_id)
        .bind(path_nodes)
        .bind(path_indices)
        .execute(&w.db.pool)
        .await
        .expect("attempt journal event");
    }

    let entries = w
        .store
        .journal(w.admin.tenant_id(), run, Some(1), 10)
        .await
        .expect("journal query")
        .expect("stored run");
    assert_eq!(entries.len(), 2);
    let root_span = sagat_telemetry::attempt_span_id(root_step, 1).to_string();
    let scoped_span = sagat_telemetry::attempt_span_id(scoped_step, 1).to_string();
    assert_eq!(entries[0].span_id.as_deref(), Some(root_span.as_str()));
    assert_eq!(entries[1].span_id.as_deref(), Some(scoped_span.as_str()));
    assert_ne!(entries[0].span_id, entries[1].span_id);
}
