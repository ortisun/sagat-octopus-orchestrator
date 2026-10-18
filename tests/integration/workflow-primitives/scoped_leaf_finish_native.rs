//! First bounded real-PostgreSQL tests for dormant flat OrdinaryLeafV1 Finish.
//!
//! This file is installed as a child of `leaf_certification_native.rs`:
//! `#[path = "scoped_leaf_finish_native.rs"] mod scoped_finish;`

use super::*;
use sagat_application::scoped_leaf_finish::{
    ScopedLeafFinishCommand, ScopedLeafFinishError, ScopedLeafFinishExpectations,
    ScopedLeafFinishResult, ScopedLeafFinishStore,
};
use sagat_domain::execution::{CapacityGroupId, CapacityMemberId, CapacityPossession};
use sagat_infra_postgres::PgScopedLeafFinishStore;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::time::Duration;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceExecutionSnapshot {
    selected_source: String,
    certificates: String,
    run_and_journal: String,
    steps: String,
    topology: String,
}

#[derive(Clone)]
struct SeedMember {
    id: CapacityMemberId,
    identity: LeafCollectionPathIdentity,
    active: bool,
}

#[derive(Clone, Copy)]
struct SeededGroup {
    group: CapacityGroupId,
    selected: CapacityMemberId,
}

async fn certified_flow(
    name: &str,
    line_count: u32,
) -> (Flow, Vec<CertifyLeafCollectionSuccess>, PgPool) {
    let lines: Vec<Value> = (0..line_count)
        .map(|index| json!({"v": index + 1}))
        .collect();
    let f = Flow::start(name, document(name), Value::Array(lines)).await;

    // This capability exists only long enough for the real native producer to certify source.
    sqlx::query("GRANT INSERT ON leaf_collection_success_certifications TO sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("temporary native source writer");
    f.drive().await;
    // Both collections in the real document use maxConcurrency=1, so close each admitted source
    // in sequence instead of treating dormant siblings as simultaneously leasable.
    for index in 0..line_count {
        let tasks = f.poll().await;
        assert_eq!(tasks.len(), 1);
        let task = &tasks[0];
        let instance = task.instance.as_ref().expect("ordinary scoped task");
        assert_eq!(instance.instance_path.len(), 1);
        assert_eq!(instance.instance_path[0].scope_node_id, "lines");
        assert_eq!(instance.instance_path[0].iteration_index, index);
        f.complete(task, json!({"accepted": true, "index": index}))
            .await;
        f.drive().await;
    }
    f.verify().await;

    let source_store = PgLeafCollectionSuccessStore::new(
        f.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    let base = command(&f).await;
    let mut certified = Vec::new();
    for index in 0..line_count {
        let mut source = base.clone();
        source.identity = identity_at(&base.identity, "lines", index);
        let result = source_store
            .certify_leaf_collection_success(&source)
            .await
            .expect("genuine native OrdinaryLeafV1 certificate");
        assert!(!result.replayed);
        assert_eq!(result.record.identity, source.identity);
        certified.push(source);
    }
    let lease = base.scheduler.lease();
    let renewed = f
        .api
        .executions
        .renew(base.scheduler.owner(), &[lease], Duration::from_secs(600))
        .await
        .expect("keep current authority across the bounded native matrix");
    assert_eq!(renewed, vec![lease]);
    sqlx::query("REVOKE INSERT ON leaf_collection_success_certifications FROM sagat_app")
        .execute(&f.api.db.pool)
        .await
        .expect("remove native source writer before Finish");

    // Preserve the released ChildRoot 20-column grant and add only the four scoped extensions in
    // this disposable database. No production migration or startup privilege is widened.
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
        .expect("native adapter role");
    assert_eq!(user, "sagat_app");
    let privileges: (bool, bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT \
         has_table_privilege(current_user,'leaf_collection_success_certifications','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','tenant_id','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','member_kind','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','original_run_id','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','root_owner_node','INSERT'), \
         has_column_privilege(current_user,'capacity_group_finish_receipts','iteration_index','INSERT')",
    )
    .fetch_one(&pool)
    .await
    .expect("effective native privileges");
    assert_eq!(privileges, (false, true, true, true, true, true));

    (f, certified, pool)
}

fn identity_at(
    base: &LeafCollectionPathIdentity,
    owner: &str,
    index: u32,
) -> LeafCollectionPathIdentity {
    LeafCollectionPathIdentity::new(
        base.tenant_id(),
        base.namespace_id(),
        base.run_id(),
        InstancePath::root()
            .child(NodeId::parse(owner).expect("ordinary owner"), index)
            .expect("depth-one ordinary path"),
    )
    .expect("qualified ordinary leaf identity")
}

fn active_member(id: CapacityMemberId, identity: LeafCollectionPathIdentity) -> SeedMember {
    SeedMember {
        id,
        identity,
        active: true,
    }
}

async fn seed_group(
    f: &Flow,
    members: Vec<SeedMember>,
    revision: i64,
    epoch: i64,
    holder: Option<usize>,
) -> SeededGroup {
    assert!(!members.is_empty());
    let group = CapacityGroupId::generate();
    let selected = members[0].id;
    let active_members = i64::try_from(members.iter().filter(|member| member.active).count())
        .expect("bounded fixture members");
    assert!(active_members > 0);
    if let Some(index) = holder {
        assert!(members[index].active, "fixture holder must be active");
    }

    let mut tx = f.api.db.pool.begin().await.expect("owner capacity fixture");
    sqlx::query(
        "INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,\
         possession,holder_member_id) VALUES($1,$2,$3,$4,$5,'VACANT',NULL)",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .bind(revision)
    .bind(epoch)
    .bind(active_members)
    .execute(&mut *tx)
    .await
    .expect("owner-seeded capacity header");
    for member in &members {
        let [segment] = member.identity.path().segments() else {
            panic!("test member is exactly depth one");
        };
        sqlx::query(
            "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
             member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state,finished_at) \
             VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,$5,$6,$7,$8,\
             CASE WHEN $8='FINISHED' THEN clock_timestamp() END)",
        )
        .bind(member.identity.tenant_id().as_uuid())
        .bind(group.as_uuid())
        .bind(member.id.as_uuid())
        .bind(member.identity.namespace_id().as_uuid())
        .bind(member.identity.run_id().as_uuid())
        .bind(segment.scope_node().as_str())
        .bind(i32::try_from(segment.iteration_index()).expect("bounded ordinary index"))
        .bind(if member.active { "ACTIVE" } else { "FINISHED" })
        .execute(&mut *tx)
        .await
        .expect("owner-seeded scoped capacity identity");
    }
    if let Some(index) = holder {
        sqlx::query(
            "UPDATE capacity_groups SET possession='HELD',holder_member_id=$3 \
             WHERE tenant_id=$1 AND group_id=$2",
        )
        .bind(f.tenant.as_uuid())
        .bind(group.as_uuid())
        .bind(members[index].id.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("owner-seeded holder fact");
    }
    tx.commit().await.expect("capacity fixture commits");
    SeededGroup { group, selected }
}

fn finish_command(
    source: &CertifyLeafCollectionSuccess,
    seeded: SeededGroup,
    group_revision: u64,
    group_epoch: u64,
) -> ScopedLeafFinishCommand {
    ScopedLeafFinishCommand {
        operation_key: Uuid::new_v4(),
        group_id: seeded.group,
        member_id: seeded.selected,
        identity: source.identity.clone(),
        expected: ScopedLeafFinishExpectations {
            group_revision,
            group_epoch,
            run_revision: source.expected_run_revision,
        },
        scheduler: source.scheduler.clone(),
    }
}

async fn source_execution_snapshot(f: &Flow) -> SourceExecutionSnapshot {
    // Test-only complete owner snapshots support both genuinely certified indices. The older
    // single-iteration refusal helper uses scalar subqueries and cannot represent this source.
    let (selected_source, run_and_journal): (String, String) = sqlx::query_as(
        "SELECT md5(jsonb_build_object(\
         'scope',(SELECT to_jsonb(s) FROM iteration_scopes s \
          WHERE tenant_id=$1 AND run_id=$2 AND owner_node='lines'),\
         'iterations',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.iteration_index) \
          FROM collection_iterations i JOIN iteration_scopes s USING(tenant_id,run_id,scope_id) \
          WHERE i.tenant_id=$1 AND i.run_id=$2 AND s.owner_node='lines'),\
         'checkpoints',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.iteration_index) \
          FROM iteration_checkpoints c JOIN iteration_scopes s USING(tenant_id,run_id,scope_id) \
          WHERE c.tenant_id=$1 AND c.run_id=$2 AND s.owner_node='lines'),\
         'owner',(SELECT to_jsonb(t) FROM steps t WHERE tenant_id=$1 AND run_id=$2 \
          AND node='lines' AND instance_path_nodes='{}' AND instance_path_indices='{}'),\
         'sagas',(SELECT jsonb_agg(to_jsonb(s) ORDER BY to_jsonb(s)::text) FROM scope_sagas s \
          WHERE tenant_id=$1 AND run_id=$2 AND owner_node='lines' \
          AND owner_path_nodes='{}' AND owner_path_indices='{}'),\
         'saga_entries',(SELECT jsonb_agg(to_jsonb(e) ORDER BY to_jsonb(e)::text) \
          FROM scope_saga_entries e WHERE tenant_id=$1 AND run_id=$2 AND owner_node='lines' \
          AND owner_path_nodes='{}' AND owner_path_indices='{}'),\
         'steps',(SELECT jsonb_agg(to_jsonb(t) ORDER BY instance_path_indices,node) FROM steps t \
          WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=ARRAY['lines']::text[]))::text),\
         md5(jsonb_build_object('run',(SELECT to_jsonb(r) FROM runs r WHERE tenant_id=$1 AND id=$2),\
         'journal',(SELECT jsonb_build_object('count',count(*),'max',max(seq)) \
          FROM run_events WHERE tenant_id=$1 AND run_id=$2))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("complete ordered native source and execution snapshots");
    let certificates: String = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY owner_node,iteration_index) \
         FROM leaf_collection_success_certifications c WHERE tenant_id=$1 AND run_id=$2),\
         'null'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact retained certificates");
    let steps: String = sqlx::query_scalar(
        "SELECT md5(COALESCE((SELECT jsonb_agg(to_jsonb(s) ORDER BY node,instance_path_nodes,\
         instance_path_indices) FROM steps s WHERE tenant_id=$1 AND run_id=$2),\
         'null'::jsonb)::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact Run steps");
    let topology: String = sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(\
         'chains',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY chain_id),'[]'::jsonb) \
          FROM execution_chains c WHERE tenant_id=$1 AND \
          (chain_id=$2 OR current_run_id=$2)),\
         'segments',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY chain_id,segment_index),\
          '[]'::jsonb) FROM chain_segments s WHERE tenant_id=$1 AND run_id=$2))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact Run topology");
    SourceExecutionSnapshot {
        selected_source,
        certificates,
        run_and_journal,
        steps,
        topology,
    }
}

async fn capacity_snapshot(f: &Flow, group: CapacityGroupId) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(\
         'group',(SELECT to_jsonb(g) FROM capacity_groups g WHERE tenant_id=$1 AND group_id=$2),\
         'members',(SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY member_id),'[]'::jsonb) \
          FROM capacity_group_members m WHERE tenant_id=$1 AND group_id=$2),\
         'receipts',(SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY operation_key),'[]'::jsonb) \
          FROM capacity_group_finish_receipts r WHERE tenant_id=$1 AND group_id=$2),\
         'audits',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY id),'[]'::jsonb) \
          FROM audit_records a WHERE tenant_id=$1 AND action='capacity.member.finished' \
          AND resource_id=$2::text),\
         'hints',(SELECT COALESCE(jsonb_agg(to_jsonb(o) ORDER BY id),'[]'::jsonb) \
          FROM event_outbox o WHERE tenant_id=$1 AND aggregate_id=$2 \
          AND event_type='capacity.scoped_member.finished'))",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact capacity transaction snapshot")
}

async fn nonselected_members(
    f: &Flow,
    group: CapacityGroupId,
    selected: CapacityMemberId,
) -> Value {
    sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY member_id),'[]'::jsonb) \
         FROM capacity_group_members m WHERE tenant_id=$1 AND group_id=$2 AND member_id<>$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .bind(selected.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact nonselected members")
}

async fn finish_counts(f: &Flow, group: CapacityGroupId) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM capacity_group_finish_receipts WHERE tenant_id=$1 AND group_id=$2),\
         (SELECT count(*) FROM audit_records WHERE tenant_id=$1 \
          AND action='capacity.member.finished' AND resource_id=$2::text),\
         (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 \
          AND event_type='capacity.scoped_member.finished')",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("scoped Finish side-effect counts")
}

fn possession_storage(possession: CapacityPossession) -> (&'static str, Option<Uuid>) {
    match possession {
        CapacityPossession::Held(holder) => ("HELD", Some(holder.as_uuid())),
        CapacityPossession::Vacant => ("VACANT", None),
        CapacityPossession::Retired => ("RETIRED", None),
    }
}

async fn assert_linked_effect(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    result: &ScopedLeafFinishResult,
) {
    let receipt = sqlx::query(
        "SELECT fingerprint,member_kind,namespace_id,member_id,chain_id,head_run_id,\
         accepted_chain_revision,original_run_id,root_owner_node,iteration_index,\
         expected_group_revision,expected_group_epoch,accepted_run_revision,resulting_revision,\
         resulting_epoch,resulting_active_members,resulting_possession,resulting_holder_member_id,\
         finished_at,audit_id,outbox_id FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("one exact scoped receipt");
    let [segment] = command.identity.path().segments() else {
        panic!("validated scoped command is depth one");
    };
    assert_eq!(
        receipt.get::<Vec<u8>, _>("fingerprint"),
        command.fingerprint().expect("valid command")
    );
    assert_eq!(receipt.get::<String, _>("member_kind"), "SCOPED_PATH");
    assert_eq!(
        receipt.get::<Uuid, _>("namespace_id"),
        command.identity.namespace_id().as_uuid()
    );
    assert_eq!(
        receipt.get::<Uuid, _>("member_id"),
        command.member_id.as_uuid()
    );
    assert_eq!(receipt.get::<Option<Uuid>, _>("chain_id"), None);
    assert_eq!(receipt.get::<Option<Uuid>, _>("head_run_id"), None);
    assert_eq!(
        receipt.get::<Option<i64>, _>("accepted_chain_revision"),
        None
    );
    assert_eq!(
        receipt.get::<Uuid, _>("original_run_id"),
        command.identity.run_id().as_uuid()
    );
    assert_eq!(
        receipt.get::<String, _>("root_owner_node"),
        segment.scope_node().as_str()
    );
    assert_eq!(
        receipt.get::<i32, _>("iteration_index"),
        i32::try_from(segment.iteration_index()).expect("bounded ordinary index")
    );
    assert_eq!(
        receipt.get::<i64, _>("expected_group_revision"),
        i64::try_from(command.expected.group_revision).expect("signed expectation")
    );
    assert_eq!(
        receipt.get::<i64, _>("expected_group_epoch"),
        i64::try_from(command.expected.group_epoch).expect("signed expectation")
    );
    assert_eq!(
        receipt.get::<i64, _>("accepted_run_revision"),
        i64::try_from(command.expected.run_revision).expect("signed expectation")
    );
    assert_eq!(
        receipt.get::<i64, _>("resulting_revision"),
        i64::try_from(result.revision).expect("signed result")
    );
    assert_eq!(
        receipt.get::<i64, _>("resulting_epoch"),
        i64::try_from(result.epoch).expect("signed result")
    );
    assert_eq!(
        receipt.get::<i64, _>("resulting_active_members"),
        i64::from(result.active_members)
    );
    let (possession, holder) = possession_storage(result.possession);
    assert_eq!(receipt.get::<String, _>("resulting_possession"), possession);
    assert_eq!(
        receipt.get::<Option<Uuid>, _>("resulting_holder_member_id"),
        holder
    );
    assert_eq!(
        receipt.get::<OffsetDateTime, _>("finished_at"),
        result.finished_at
    );
    assert_eq!(
        receipt.get::<Uuid, _>("audit_id"),
        result.audit_id.as_uuid()
    );
    assert_eq!(receipt.get::<Uuid, _>("outbox_id"), result.outbox_id);

    let selected: (String, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT state,finished_at FROM capacity_group_members \
         WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.member_id.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("selected member shares the exact guarded result time");
    assert_eq!(selected, ("FINISHED".into(), Some(result.finished_at)));

    let trace: Option<String> = sqlx::query_scalar(
        "SELECT trace_id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(command.identity.namespace_id().as_uuid())
    .bind(command.identity.run_id().as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("guarded Run trace oracle");
    let audit: (String, String, String, String, String, Option<String>, OffsetDateTime) = sqlx::query_as(
        "SELECT actor,action,resource_type,resource_id,reason,trace_id,created_at FROM audit_records \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(command.identity.tenant_id().as_uuid())
    .bind(result.audit_id.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("linked scoped Finish audit");
    assert_eq!(
        audit,
        (
            "system:engine".into(),
            "capacity.member.finished".into(),
            "capacity_group".into(),
            command.group_id.to_string(),
            "ordinary leaf path capacity member finished".into(),
            trace.clone(),
            result.finished_at,
        )
    );
    let outbox: (Uuid, Uuid, String, Value) = sqlx::query_as(
        "SELECT tenant_id,aggregate_id,event_type,payload FROM event_outbox WHERE id=$1",
    )
    .bind(result.outbox_id)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("linked scoped Finish hint");
    assert_eq!(outbox.0, command.identity.tenant_id().as_uuid());
    assert_eq!(outbox.1, command.group_id.as_uuid());
    assert_eq!(outbox.2, "capacity.scoped_member.finished");
    assert_eq!(
        outbox.3["tenantId"],
        command.identity.tenant_id().to_string()
    );
    assert_eq!(
        outbox.3["namespaceId"],
        command.identity.namespace_id().to_string()
    );
    assert_eq!(outbox.3["groupId"], command.group_id.to_string());
    assert_eq!(outbox.3["memberId"], command.member_id.to_string());
    assert_eq!(
        outbox.3["originalRunId"],
        command.identity.run_id().to_string()
    );
    assert_eq!(outbox.3["rootOwnerNode"], segment.scope_node().as_str());
    assert_eq!(outbox.3["iterationIndex"], segment.iteration_index());
    assert_eq!(outbox.3["resultingRevision"], result.revision);
    assert_eq!(outbox.3["epoch"], result.epoch);
    assert_eq!(
        outbox.3["retired"],
        result.possession == CapacityPossession::Retired
    );
    if let Some(value) = outbox.3.get("traceId") {
        assert_eq!(value, &trace.map_or(Value::Null, Value::String));
    }
}

async fn assert_header(
    f: &Flow,
    group: CapacityGroupId,
    expected: (i64, i64, i64, &str, Option<CapacityMemberId>),
) {
    let actual: (i64, i64, i64, String, Option<Uuid>) = sqlx::query_as(
        "SELECT revision,epoch,active_members,possession,holder_member_id \
         FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("capacity header");
    assert_eq!(
        actual,
        (
            expected.0,
            expected.1,
            expected.2,
            expected.3.into(),
            expected.4.map(|member| member.as_uuid()),
        )
    );
}

#[tokio::test]
async fn native_scoped_finish_applies_once_and_replays_original_after_real_later_changes() {
    let (f, sources, pool) = certified_flow("scoped-finish-native-replay", 2).await;
    let selected = CapacityMemberId::generate();
    let later_holder = CapacityMemberId::generate();
    let retained_neighbor = CapacityMemberId::generate();
    let seeded = seed_group(
        &f,
        vec![
            active_member(selected, sources[0].identity.clone()),
            active_member(later_holder, sources[1].identity.clone()),
            // Explicit storage identity only; this test makes no source-closure claim for it.
            active_member(
                retained_neighbor,
                identity_at(&sources[0].identity, "fixture_neighbor", 7),
            ),
        ],
        10,
        4,
        Some(1),
    )
    .await;
    let command = finish_command(&sources[0], seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let neighbors_before = nonselected_members(&f, seeded.group, seeded.selected).await;
    let store = PgScopedLeafFinishStore::new(pool);

    let applied = store
        .finish_scoped_leaf(&command)
        .await
        .expect("first native scoped Finish");
    assert!(!applied.replayed);
    assert_eq!(
        (
            applied.group_id,
            applied.member_id,
            applied.original_run_id,
            applied.revision,
            applied.epoch,
            applied.active_members,
            applied.possession,
        ),
        (
            seeded.group,
            selected,
            f.run,
            11,
            4,
            2,
            CapacityPossession::Held(later_holder),
        )
    );
    assert_header(&f, seeded.group, (11, 4, 2, "HELD", Some(later_holder))).await;
    assert_eq!(
        nonselected_members(&f, seeded.group, seeded.selected).await,
        neighbors_before,
        "first Finish changes no nonselected member"
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    assert_linked_effect(&f, &command, &applied).await;

    // A second genuine certified member changes the same header through the public Finish port.
    let next_seed = SeededGroup {
        group: seeded.group,
        selected: later_holder,
    };
    let next = finish_command(&sources[1], next_seed, 11, 4);
    let second = store
        .finish_scoped_leaf(&next)
        .await
        .expect("later genuine member changes the guarded header");
    assert_eq!(
        (
            second.revision,
            second.epoch,
            second.active_members,
            second.possession,
        ),
        (12, 5, 1, CapacityPossession::Vacant)
    );

    // The original Run then legitimately fails in later work; neither fact rewrites source proof.
    let later = f.poll().await;
    assert_eq!(later.len(), 1);
    assert!(later.iter().all(|task| {
        task.instance
            .as_ref()
            .is_some_and(|instance| instance.instance_path[0].scope_node_id == "later")
    }));
    f.fail(&later[0], "DECLINED", false).await;
    f.drive().await;
    assert_eq!(f.run_status().await, "FAILED");

    let before_replay_source = source_execution_snapshot(&f).await;
    let before_replay_capacity = capacity_snapshot(&f, seeded.group).await;
    let mut replay = store
        .finish_scoped_leaf(&command)
        .await
        .expect("matching historical receipt survives later Run/header changes");
    assert!(replay.replayed);
    replay.replayed = false;
    assert_eq!(
        replay, applied,
        "replay returns the immutable original result"
    );
    assert_eq!(source_execution_snapshot(&f).await, before_replay_source);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        before_replay_capacity
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (2, 2, 2));
    let published = crate::hint_contract::assert_outbox_matches_the_contract(&f.api.db.pool).await;
    crate::hint_contract::assert_published(&published, &["capacity.scoped_member.finished"]);
    f.verify().await;
}

#[tokio::test]
async fn native_scoped_finish_preserves_exact_a1_holder_vacant_and_last_transitions() {
    let (f, sources, pool) = certified_flow("scoped-finish-native-a1", 1).await;
    let store = PgScopedLeafFinishStore::new(pool);
    let all_source_before = source_execution_snapshot(&f).await;

    struct Case {
        name: &'static str,
        active: usize,
        holder: Option<usize>,
        expected_epoch: u64,
        expected_members: u32,
        expected_possession: CapacityPossession,
    }
    for (case_index, case) in [
        Case {
            name: "selected-holder",
            active: 2,
            holder: Some(0),
            expected_epoch: 5,
            expected_members: 1,
            expected_possession: CapacityPossession::Vacant,
        },
        Case {
            name: "nonholder",
            active: 2,
            holder: Some(1),
            expected_epoch: 4,
            expected_members: 1,
            expected_possession: CapacityPossession::Held(CapacityMemberId::new(Uuid::nil())),
        },
        Case {
            name: "vacant",
            active: 2,
            holder: None,
            expected_epoch: 4,
            expected_members: 1,
            expected_possession: CapacityPossession::Vacant,
        },
        Case {
            name: "last",
            active: 1,
            holder: Some(0),
            expected_epoch: 5,
            expected_members: 0,
            expected_possession: CapacityPossession::Retired,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let selected = CapacityMemberId::generate();
        let neighbor = CapacityMemberId::generate();
        let mut members = vec![active_member(selected, sources[0].identity.clone())];
        if case.active == 2 {
            // The neighbor is an explicit storage fixture with another ordinary binding. Its
            // presence is an A1 registry fact and is not asserted to have certified source.
            members.push(active_member(
                neighbor,
                identity_at(
                    &sources[0].identity,
                    &format!("fixture_a1_{case_index}"),
                    u32::try_from(case_index + 1).expect("bounded case index"),
                ),
            ));
        }
        let seeded = seed_group(&f, members, 10, 4, case.holder).await;
        let command = finish_command(&sources[0], seeded, 10, 4);
        let neighbors_before = nonselected_members(&f, seeded.group, seeded.selected).await;
        let applied = store
            .finish_scoped_leaf(&command)
            .await
            .unwrap_or_else(|error| panic!("{}: {error:?}", case.name));
        let expected_possession = if case.name == "nonholder" {
            CapacityPossession::Held(neighbor)
        } else {
            case.expected_possession
        };
        assert_eq!(
            (
                applied.revision,
                applied.epoch,
                applied.active_members,
                applied.possession,
            ),
            (
                11,
                case.expected_epoch,
                case.expected_members,
                expected_possession,
            ),
            "{}",
            case.name
        );
        let (stored_possession, stored_holder) = possession_storage(expected_possession);
        assert_header(
            &f,
            seeded.group,
            (
                11,
                i64::try_from(case.expected_epoch).expect("signed epoch"),
                i64::from(case.expected_members),
                stored_possession,
                stored_holder.map(CapacityMemberId::new),
            ),
        )
        .await;
        assert_eq!(
            nonselected_members(&f, seeded.group, seeded.selected).await,
            neighbors_before,
            "{} changes no nonselected member",
            case.name
        );
        assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
        assert_linked_effect(&f, &command, &applied).await;
    }
    assert_eq!(source_execution_snapshot(&f).await, all_source_before);
    let published = crate::hint_contract::assert_outbox_matches_the_contract(&f.api.db.pool).await;
    crate::hint_contract::assert_published(&published, &["capacity.scoped_member.finished"]);
    f.verify().await;
}

async fn seed_flat_chain_membership(f: &Flow) {
    // A valid owner-created chain membership is only a context-refusal fixture. It does not turn
    // the earlier flat native certificate into chain/descendant closure evidence.
    let mut tx = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("context topology fixture");
    sqlx::query(
        "INSERT INTO execution_chains(tenant_id,namespace_id,chain_id,current_run_id,\
         definition_version_id,started_by,max_segments,original_deadline_at,created_at,updated_at) \
         SELECT tenant_id,namespace_id,id,id,workflow_version_id,'system:test',1,\
         clock_timestamp()+interval '1 day',clock_timestamp(),clock_timestamp() \
         FROM runs WHERE tenant_id=$1 AND id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("valid chain header around original Run");
    sqlx::query(
        "INSERT INTO chain_segments(tenant_id,namespace_id,chain_id,segment_index,run_id,created_at) \
         SELECT tenant_id,namespace_id,id,0,id,clock_timestamp() FROM runs \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("valid original Run chain membership");
    tx.commit().await.expect("context topology fixture commits");
}

#[tokio::test]
async fn native_scoped_finish_refuses_source_authority_context_cas_and_inactive_in_order() {
    let (f, sources, pool) = certified_flow("scoped-finish-native-refusals", 1).await;
    let store = PgScopedLeafFinishStore::new(pool);
    let source_before = source_execution_snapshot(&f).await;

    let absent_identity = identity_at(&sources[0].identity, "lines", 1);
    let absent_member = CapacityMemberId::generate();
    let absent = seed_group(
        &f,
        vec![active_member(absent_member, absent_identity.clone())],
        10,
        4,
        Some(0),
    )
    .await;
    let mut absent_command = finish_command(&sources[0], absent, 11, 4);
    absent_command.identity = absent_identity;
    let absent_before = capacity_snapshot(&f, absent.group).await;
    assert_eq!(
        store.finish_scoped_leaf(&absent_command).await,
        Err(ScopedLeafFinishError::SourceEvidenceMissing),
        "source absence precedes deliberately stale group CAS"
    );
    assert_eq!(capacity_snapshot(&f, absent.group).await, absent_before);
    assert_eq!(finish_counts(&f, absent.group).await, (0, 0, 0));

    let fenced = seed_group(
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
    let mut fenced_command = finish_command(&sources[0], fenced, 11, 4);
    fenced_command.scheduler = SchedulerAuthority::for_run(
        "stale-scoped-finish-owner",
        f.run,
        sources[0].scheduler.lease().generation,
    );
    let fenced_before = capacity_snapshot(&f, fenced.group).await;
    assert_eq!(
        store.finish_scoped_leaf(&fenced_command).await,
        Err(ScopedLeafFinishError::SchedulerFenceLost),
        "current authority precedes source and stale CAS"
    );
    assert_eq!(capacity_snapshot(&f, fenced.group).await, fenced_before);

    let cas = seed_group(
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
    let cas_before = capacity_snapshot(&f, cas.group).await;
    let mut stale_group = finish_command(&sources[0], cas, 11, 5);
    stale_group.expected.run_revision += 1;
    assert_eq!(
        store.finish_scoped_leaf(&stale_group).await,
        Err(ScopedLeafFinishError::StaleGroupRevision { current: 10 })
    );
    let mut stale_epoch = finish_command(&sources[0], cas, 10, 5);
    stale_epoch.expected.run_revision += 1;
    assert_eq!(
        store.finish_scoped_leaf(&stale_epoch).await,
        Err(ScopedLeafFinishError::StaleGroupEpoch { current: 4 })
    );
    let mut stale_run = finish_command(&sources[0], cas, 10, 4);
    stale_run.expected.run_revision += 1;
    assert_eq!(
        store.finish_scoped_leaf(&stale_run).await,
        Err(ScopedLeafFinishError::StaleRunRevision {
            current: sources[0].expected_run_revision,
        })
    );
    assert_eq!(capacity_snapshot(&f, cas.group).await, cas_before);
    assert_eq!(finish_counts(&f, cas.group).await, (0, 0, 0));

    let inactive_selected = CapacityMemberId::generate();
    let inactive_neighbor = CapacityMemberId::generate();
    let inactive = seed_group(
        &f,
        vec![
            SeedMember {
                id: inactive_selected,
                identity: sources[0].identity.clone(),
                active: false,
            },
            active_member(
                inactive_neighbor,
                identity_at(&sources[0].identity, "fixture_inactive_holder", 3),
            ),
        ],
        10,
        4,
        Some(1),
    )
    .await;
    let inactive_command = finish_command(&sources[0], inactive, 10, 4);
    let inactive_before = capacity_snapshot(&f, inactive.group).await;
    // A FINISHED selected fact must not mask any earlier creation CAS refusal.
    let mut inactive_stale_group = finish_command(&sources[0], inactive, 11, 5);
    inactive_stale_group.expected.run_revision += 1;
    assert_eq!(
        store.finish_scoped_leaf(&inactive_stale_group).await,
        Err(ScopedLeafFinishError::StaleGroupRevision { current: 10 })
    );
    let mut inactive_stale_epoch = finish_command(&sources[0], inactive, 10, 5);
    inactive_stale_epoch.expected.run_revision += 1;
    assert_eq!(
        store.finish_scoped_leaf(&inactive_stale_epoch).await,
        Err(ScopedLeafFinishError::StaleGroupEpoch { current: 4 })
    );
    let mut inactive_stale_run = finish_command(&sources[0], inactive, 10, 4);
    inactive_stale_run.expected.run_revision += 1;
    assert_eq!(
        store.finish_scoped_leaf(&inactive_stale_run).await,
        Err(ScopedLeafFinishError::StaleRunRevision {
            current: sources[0].expected_run_revision,
        })
    );
    assert_eq!(capacity_snapshot(&f, inactive.group).await, inactive_before);
    assert_eq!(
        store.finish_scoped_leaf(&inactive_command).await,
        Err(ScopedLeafFinishError::InactiveMember)
    );
    assert_eq!(capacity_snapshot(&f, inactive.group).await, inactive_before);
    assert_eq!(finish_counts(&f, inactive.group).await, (0, 0, 0));

    let context = seed_group(
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
    let context_command = finish_command(&sources[0], context, 10, 4);
    seed_flat_chain_membership(&f).await;
    let context_source_before = source_execution_snapshot(&f).await;
    let context_before = capacity_snapshot(&f, context.group).await;
    assert_eq!(
        store.finish_scoped_leaf(&context_command).await,
        Err(ScopedLeafFinishError::UnsupportedRunContext)
    );
    assert_eq!(source_execution_snapshot(&f).await, context_source_before);
    assert_eq!(capacity_snapshot(&f, context.group).await, context_before);
    assert_eq!(finish_counts(&f, context.group).await, (0, 0, 0));

    assert_eq!(
        SourceExecutionSnapshot {
            topology: source_before.topology.clone(),
            ..source_execution_snapshot(&f).await
        },
        source_before,
        "only the explicit context fixture changed topology"
    );
}

#[tokio::test]
async fn native_scoped_finish_refuses_signed_counter_overflow_atomically() {
    let (f, sources, pool) = certified_flow("scoped-finish-native-overflow", 1).await;
    let selected = CapacityMemberId::generate();
    let seeded = seed_group(
        &f,
        vec![active_member(selected, sources[0].identity.clone())],
        i64::MAX,
        1,
        Some(0),
    )
    .await;
    let command = finish_command(
        &sources[0],
        seeded,
        u64::try_from(i64::MAX).expect("positive signed maximum"),
        1,
    );
    let source_before = source_execution_snapshot(&f).await;
    let capacity_before = capacity_snapshot(&f, seeded.group).await;
    let store = PgScopedLeafFinishStore::new(pool);
    assert_eq!(
        store.finish_scoped_leaf(&command).await,
        Err(ScopedLeafFinishError::CounterExhausted)
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (0, 0, 0));
    assert_header(&f, seeded.group, (i64::MAX, 1, 1, "HELD", Some(selected))).await;
    f.verify().await;
}

#[path = "scoped_leaf_finish_source_controls.rs"]
mod source_controls;

#[path = "scoped_leaf_finish_qualification.rs"]
mod qualification;
#[path = "scoped_leaf_finish_receipts.rs"]
mod receipts;

#[path = "scoped_leaf_finish_entry_barriers.rs"]
mod entry_barriers;

#[path = "scoped_leaf_finish_shard_expiry.rs"]
mod shard_expiry;

#[path = "scoped_leaf_finish_writer_order.rs"]
mod writer_order;

#[path = "scoped_leaf_finish_atomic_failures.rs"]
mod atomic_failures;

#[path = "scoped_leaf_finish_contexts.rs"]
mod contexts;

#[path = "scoped_leaf_finish_commit_loss.rs"]
mod commit_loss;

#[path = "scoped_leaf_finish_process.rs"]
mod process;

#[path = "scoped_leaf_finish_observations.rs"]
mod observations;
