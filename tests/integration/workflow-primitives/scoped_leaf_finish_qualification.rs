//! Bounded exact binding, namespace, and cross-kind qualification controls.
//!
//! Install as a child of `scoped_leaf_finish_native.rs`. Owner-written ChildRoot rows below are
//! labelled storage decoder fixtures; they do not claim native ChildRoot finality.

use super::*;

#[derive(Clone, Copy)]
struct PublishedRun {
    namespace: sagat_domain::identity::NamespaceId,
    run: sagat_domain::execution::RunId,
}

async fn publish_started_run(
    f: &Flow,
    namespace_slug: &str,
    namespace: sagat_domain::identity::NamespaceId,
    label: &str,
) -> PublishedRun {
    let name = format!("sf-{label}-{}", Uuid::new_v4());
    let created = f
        .api
        .as_user(reqwest::Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", namespace_slug)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({
            "name": name,
            "ir": {
                "apiVersion": "sagat-octopus-orchestrator.io/v1",
                "kind": "Workflow",
                "metadata": {"name": name},
                "entrypoint": "done",
                "nodes": [{"id": "done", "type": "end"}]
            }
        }))
        .send()
        .await
        .expect("create alternate published Run workflow");
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let workflow = crate::support::api::json_of(created).await["id"]
        .as_str()
        .expect("alternate workflow id")
        .to_owned();
    let published = f
        .api
        .as_user(
            reqwest::Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", namespace_slug)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish alternate workflow");
    assert_eq!(published.status(), reqwest::StatusCode::CREATED);
    let started = f
        .api
        .as_user(reqwest::Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", namespace_slug)
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": {}}))
        .send()
        .await
        .expect("start alternate published Run");
    assert_eq!(started.status(), reqwest::StatusCode::ACCEPTED);
    let run: sagat_domain::execution::RunId = crate::support::api::json_of(started).await["runId"]
        .as_str()
        .expect("alternate run id")
        .parse()
        .expect("alternate run id");
    let actual_namespace: Uuid = sqlx::query_scalar(
        "SELECT namespace_id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual published/started Run has the requested namespace");
    assert_eq!(actual_namespace, namespace.as_uuid());
    PublishedRun { namespace, run }
}

async fn create_second_namespace(
    f: &Flow,
    label: &str,
) -> (String, sagat_domain::identity::NamespaceId) {
    let workspace = f
        .api
        .create_workspace("alice", "acme", &format!("qualification-{label}"))
        .await;
    let slug = format!("qualification-{label}");
    let namespace = f
        .api
        .create_namespace("alice", "acme", &workspace, &slug, "dev")
        .await
        .parse()
        .expect("second namespace id");
    (slug, namespace)
}

async fn attach_owner_chain_fixture(f: &Flow, alternate: PublishedRun) -> i64 {
    // Storage decoder fixture only. It gives a proper retained ChildRoot chain identity to rows
    // below and makes no claim that this alternate Run reached native ChildRoot finality.
    let mut tx = f.api.db.pool.begin().await.expect("owner chain fixture");
    sqlx::query(
        "INSERT INTO execution_chains(tenant_id,namespace_id,chain_id,current_run_id,\
         definition_version_id,started_by,max_segments,original_deadline_at,created_at,updated_at) \
         SELECT tenant_id,namespace_id,id,id,workflow_version_id,'system:test',1,\
         clock_timestamp()+interval '1 day',clock_timestamp(),clock_timestamp() \
         FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("owner-created alternate chain header");
    sqlx::query(
        "INSERT INTO chain_segments(tenant_id,namespace_id,chain_id,segment_index,run_id,created_at) \
         VALUES($1,$2,$3,0,$3,clock_timestamp())",
    )
    .bind(f.tenant.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("owner-created alternate chain segment");
    tx.commit().await.expect("alternate chain fixture commits");
    sqlx::query_scalar(
        "SELECT revision FROM execution_chains \
         WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("alternate chain revision")
}

async fn alternate_run_snapshot(f: &Flow, alternate: PublishedRun) -> String {
    sqlx::query_scalar(
        "SELECT md5(jsonb_build_object(\
         'run',(SELECT to_jsonb(r) FROM runs r WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3),\
         'journal',(SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY seq),'[]'::jsonb) \
          FROM run_events e WHERE tenant_id=$1 AND run_id=$3),\
         'steps',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY instance_path_indices,node),\
          '[]'::jsonb) FROM steps s WHERE tenant_id=$1 AND run_id=$3),\
         'certificates',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY owner_node,iteration_index),\
          '[]'::jsonb) FROM leaf_collection_success_certifications c \
          WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3))::text)",
    )
    .bind(f.tenant.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("exact alternate Run/source snapshot")
}

fn identity_for_run(
    base: &LeafCollectionPathIdentity,
    alternate: PublishedRun,
    owner: &str,
    index: u32,
) -> LeafCollectionPathIdentity {
    LeafCollectionPathIdentity::new(
        base.tenant_id(),
        alternate.namespace,
        alternate.run,
        InstancePath::root()
            .child(NodeId::parse(owner).expect("fixture owner"), index)
            .expect("depth-one fixture path"),
    )
    .expect("qualified alternate Run identity")
}

async fn seed_child_root_group(
    f: &Flow,
    alternate: PublishedRun,
    revision: i64,
    epoch: i64,
) -> SeededGroup {
    let group = CapacityGroupId::generate();
    let selected = CapacityMemberId::generate();
    let mut tx = f
        .api
        .db
        .pool
        .begin()
        .await
        .expect("ChildRoot decoder fixture");
    sqlx::query(
        "INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,\
         possession,holder_member_id) VALUES($1,$2,$3,$4,1,'VACANT',NULL)",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .bind(revision)
    .bind(epoch)
    .execute(&mut *tx)
    .await
    .expect("ChildRoot decoder group");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .bind(selected.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("proper ChildRoot decoder member");
    sqlx::query(
        "UPDATE capacity_groups SET possession='HELD',holder_member_id=$3 \
         WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .bind(selected.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("ChildRoot decoder holder");
    tx.commit()
        .await
        .expect("ChildRoot decoder fixture commits");
    SeededGroup { group, selected }
}

async fn insert_finished_child_root_member(
    f: &Flow,
    group: CapacityGroupId,
    member: CapacityMemberId,
    alternate: PublishedRun,
) {
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,state,finished_at) \
         VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'FINISHED',clock_timestamp())",
    )
    .bind(f.tenant.as_uuid())
    .bind(group.as_uuid())
    .bind(member.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .execute(&f.api.db.pool)
    .await
    .expect("finished ChildRoot decoder member");
}

async fn insert_child_root_receipt_fixture(
    f: &Flow,
    command: &ScopedLeafFinishCommand,
    child: CapacityMemberId,
    alternate: PublishedRun,
    chain_revision: i64,
    fingerprint: [u8; 32],
) {
    let run_revision: i64 = sqlx::query_scalar(
        "SELECT run_version FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("alternate Run revision for ChildRoot decoder receipt");
    sqlx::query(
        "INSERT INTO capacity_group_finish_receipts(\
         tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,member_kind,\
         chain_id,head_run_id,expected_group_revision,expected_group_epoch,\
         accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,\
         resulting_active_members,resulting_possession,resulting_holder_member_id,finished_at,\
         audit_id,outbox_id) \
         VALUES($1,$2,$3,$4,$5,$6,'CHILD_ROOT',$7,$7,5,4,$8,$9,6,4,1,'HELD',$10,\
         clock_timestamp(),$11,$12)",
    )
    .bind(f.tenant.as_uuid())
    .bind(command.group_id.as_uuid())
    .bind(command.operation_key)
    .bind(fingerprint.as_slice())
    .bind(alternate.namespace.as_uuid())
    .bind(child.as_uuid())
    .bind(alternate.run.as_uuid())
    .bind(chain_revision)
    .bind(run_revision)
    .bind(command.member_id.as_uuid())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(&f.api.db.pool)
    .await
    .expect("schema-valid owner-only ChildRoot decoder receipt");
}

async fn assert_refusal_unchanged(
    f: &Flow,
    store: &PgScopedLeafFinishStore,
    command: &ScopedLeafFinishCommand,
    observed_group: CapacityGroupId,
    expected: ScopedLeafFinishError,
) {
    let source_before = source_execution_snapshot(f).await;
    let capacity_before = capacity_snapshot(f, observed_group).await;
    let counts_before = finish_counts(f, observed_group).await;
    assert_eq!(store.finish_scoped_leaf(command).await, Err(expected));
    assert_eq!(source_execution_snapshot(f).await, source_before);
    assert_eq!(capacity_snapshot(f, observed_group).await, capacity_before);
    assert_eq!(finish_counts(f, observed_group).await, counts_before);
}

#[tokio::test]
async fn qualification_distinguishes_exact_absence_from_observable_binding_mismatch() {
    let (f, sources, pool) = certified_flow("scoped-finish-qualification-binding", 1).await;
    let store = PgScopedLeafFinishStore::new(pool);
    let source_before = source_execution_snapshot(&f).await;
    let valid = seed_group(
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
    let base = finish_command(&sources[0], valid, 10, 4);
    // The hidden binding deliberately disagrees, so unfenced discovery would leak a
    // BindingMismatch instead of the required opaque Missing at this first stage.
    let hidden_path = InstancePath::root()
        .child(
            NodeId::parse("unobservable_probe").expect("ordinary owner"),
            7,
        )
        .expect("depth-one opaque probe");

    let mut absent_tenant = base.clone();
    absent_tenant.identity = LeafCollectionPathIdentity::new(
        sagat_domain::identity::TenantId::new(Uuid::new_v4()),
        base.identity.namespace_id(),
        base.identity.run_id(),
        hidden_path.clone(),
    )
    .expect("qualified absent tenant identity");
    assert_refusal_unchanged(
        &f,
        &store,
        &absent_tenant,
        valid.group,
        ScopedLeafFinishError::Missing,
    )
    .await;

    let mut absent_namespace = base.clone();
    absent_namespace.identity = LeafCollectionPathIdentity::new(
        base.identity.tenant_id(),
        sagat_domain::identity::NamespaceId::new(Uuid::new_v4()),
        base.identity.run_id(),
        hidden_path.clone(),
    )
    .expect("qualified absent namespace identity");
    assert_refusal_unchanged(
        &f,
        &store,
        &absent_namespace,
        valid.group,
        ScopedLeafFinishError::Missing,
    )
    .await;

    let mut absent_group = base.clone();
    absent_group.group_id = CapacityGroupId::generate();
    absent_group.identity = LeafCollectionPathIdentity::new(
        base.identity.tenant_id(),
        base.identity.namespace_id(),
        base.identity.run_id(),
        hidden_path.clone(),
    )
    .expect("group discovery must remain opaque before binding");
    assert_refusal_unchanged(
        &f,
        &store,
        &absent_group,
        valid.group,
        ScopedLeafFinishError::Missing,
    )
    .await;

    let mut absent_member = base.clone();
    absent_member.member_id = CapacityMemberId::generate();
    assert_refusal_unchanged(
        &f,
        &store,
        &absent_member,
        valid.group,
        ScopedLeafFinishError::Missing,
    )
    .await;

    let alternate = publish_started_run(
        &f,
        "payments",
        sources[0].identity.namespace_id(),
        "binding-alternate",
    )
    .await;
    attach_owner_chain_fixture(&f, alternate).await;
    let alternate_before = alternate_run_snapshot(&f, alternate).await;

    let wrong_kind = seed_child_root_group(&f, alternate, 10, 4).await;
    let wrong_kind_command = finish_command(&sources[0], wrong_kind, 10, 4);
    assert_refusal_unchanged(
        &f,
        &store,
        &wrong_kind_command,
        wrong_kind.group,
        ScopedLeafFinishError::BindingMismatch,
    )
    .await;
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );

    let wrong_original_run = seed_group(
        &f,
        vec![active_member(
            CapacityMemberId::generate(),
            identity_for_run(&sources[0].identity, alternate, "lines", 0),
        )],
        10,
        4,
        Some(0),
    )
    .await;
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );
    let wrong_original_run_command = finish_command(&sources[0], wrong_original_run, 10, 4);
    assert_refusal_unchanged(
        &f,
        &store,
        &wrong_original_run_command,
        wrong_original_run.group,
        ScopedLeafFinishError::BindingMismatch,
    )
    .await;

    for (label, stored_identity) in [
        (
            "owner",
            identity_at(&sources[0].identity, "fixture_wrong_owner", 0),
        ),
        ("index", identity_at(&sources[0].identity, "lines", 7)),
    ] {
        let mismatched = seed_group(
            &f,
            vec![active_member(CapacityMemberId::generate(), stored_identity)],
            10,
            4,
            Some(0),
        )
        .await;
        let command = finish_command(&sources[0], mismatched, 10, 4);
        assert_refusal_unchanged(
            &f,
            &store,
            &command,
            mismatched.group,
            ScopedLeafFinishError::BindingMismatch,
        )
        .await;
        assert_eq!(
            finish_counts(&f, mismatched.group).await,
            (0, 0, 0),
            "{label}"
        );
    }

    let control = store
        .finish_scoped_leaf(&base)
        .await
        .expect("exact binding positive native control");
    assert!(!control.replayed);
    assert_eq!(control.possession, CapacityPossession::Retired);
    assert_linked_effect(&f, &base, &control).await;
    let replay = store
        .finish_scoped_leaf(&base)
        .await
        .expect("exact binding positive replay control");
    let mut expected_replay = control.clone();
    expected_replay.replayed = true;
    assert_eq!(replay, expected_replay);
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );
    assert_eq!(finish_counts(&f, valid.group).await, (1, 1, 1));
    f.verify().await;
}

#[tokio::test]
async fn qualification_preserves_a_same_tenant_holder_from_another_real_namespace() {
    let (f, sources, pool) = certified_flow("scoped-finish-qualification-namespace", 1).await;
    let (namespace_slug, namespace) = create_second_namespace(&f, "holder").await;
    let alternate = publish_started_run(&f, &namespace_slug, namespace, "namespace-holder").await;
    let alternate_before = alternate_run_snapshot(&f, alternate).await;
    let foreign_certificates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM leaf_collection_success_certifications \
         WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(alternate.namespace.as_uuid())
    .bind(alternate.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("foreign holder source absence oracle");
    assert_eq!(
        foreign_certificates, 0,
        "the holder is registry storage, not source proof"
    );

    let selected = CapacityMemberId::generate();
    let foreign_holder = CapacityMemberId::generate();
    let seeded = seed_group(
        &f,
        vec![
            active_member(selected, sources[0].identity.clone()),
            active_member(
                foreign_holder,
                identity_for_run(&sources[0].identity, alternate, "fixture_holder", 0),
            ),
        ],
        10,
        4,
        Some(1),
    )
    .await;
    let command = finish_command(&sources[0], seeded, 10, 4);
    let source_before = source_execution_snapshot(&f).await;
    let holder_before = nonselected_members(&f, seeded.group, seeded.selected).await;
    let store = PgScopedLeafFinishStore::new(pool);

    // This member exists in another real namespace but has a different Run/path binding.
    // Missing must come from namespace-qualified discovery, not a later Run lookup.
    let mut hidden_holder = command.clone();
    hidden_holder.member_id = foreign_holder;
    assert_refusal_unchanged(
        &f,
        &store,
        &hidden_holder,
        seeded.group,
        ScopedLeafFinishError::Missing,
    )
    .await;
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );

    let applied = store
        .finish_scoped_leaf(&command)
        .await
        .expect("selected source does not require the foreign holder Run/source");
    assert_eq!(
        (
            applied.revision,
            applied.epoch,
            applied.active_members,
            applied.possession,
        ),
        (11, 4, 1, CapacityPossession::Held(foreign_holder))
    );
    assert_header(&f, seeded.group, (11, 4, 1, "HELD", Some(foreign_holder))).await;
    assert_eq!(
        nonselected_members(&f, seeded.group, seeded.selected).await,
        holder_before,
        "the other namespace's holder remains byte-for-byte unchanged"
    );
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_linked_effect(&f, &command, &applied).await;

    let capacity_before_replay = capacity_snapshot(&f, seeded.group).await;
    let replay = store
        .finish_scoped_leaf(&command)
        .await
        .expect("same-namespace selected receipt replays with a foreign-namespace holder");
    let mut expected_replay = applied.clone();
    expected_replay.replayed = true;
    assert_eq!(replay, expected_replay);
    assert_eq!(
        capacity_snapshot(&f, seeded.group).await,
        capacity_before_replay
    );
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 1, 1));
    f.verify().await;
}

#[tokio::test]
async fn qualification_rejects_a_group_wide_cross_kind_operation_key_then_applies_a_fresh_key() {
    let (f, sources, pool) = certified_flow("scoped-finish-qualification-cross-kind", 1).await;
    let alternate = publish_started_run(
        &f,
        "payments",
        sources[0].identity.namespace_id(),
        "cross-kind-child",
    )
    .await;
    let chain_revision = attach_owner_chain_fixture(&f, alternate).await;
    let selected = CapacityMemberId::generate();
    let seeded = seed_group(
        &f,
        vec![active_member(selected, sources[0].identity.clone())],
        6,
        4,
        Some(0),
    )
    .await;
    let child = CapacityMemberId::generate();
    insert_finished_child_root_member(&f, seeded.group, child, alternate).await;
    let command = finish_command(&sources[0], seeded, 6, 4);
    let mut conflicting_fingerprint = command.fingerprint().expect("supported flat command");
    conflicting_fingerprint[0] ^= 0xff;
    insert_child_root_receipt_fixture(
        &f,
        &command,
        child,
        alternate,
        chain_revision,
        conflicting_fingerprint,
    )
    .await;
    let fixture: (
        String,
        Uuid,
        Uuid,
        Option<Uuid>,
        Option<String>,
        Option<i32>,
    ) = sqlx::query_as(
        "SELECT member_kind,member_id,chain_id,original_run_id,root_owner_node,iteration_index \
         FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(f.tenant.as_uuid())
    .bind(seeded.group.as_uuid())
    .bind(command.operation_key)
    .fetch_one(&f.api.db.pool)
    .await
    .expect("proper stored cross-kind decoder fixture");
    assert_eq!(
        fixture,
        (
            "CHILD_ROOT".into(),
            child.as_uuid(),
            alternate.run.as_uuid(),
            None,
            None,
            None,
        )
    );

    let source_before = source_execution_snapshot(&f).await;
    let alternate_before = alternate_run_snapshot(&f, alternate).await;
    let capacity_before = capacity_snapshot(&f, seeded.group).await;
    let store = PgScopedLeafFinishStore::new(pool);
    assert_eq!(
        store.finish_scoped_leaf(&command).await,
        Err(ScopedLeafFinishError::FingerprintConflict),
        "the group-wide key conflicts before decoding another kind's result"
    );
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );
    assert_eq!(capacity_snapshot(&f, seeded.group).await, capacity_before);
    assert_eq!(finish_counts(&f, seeded.group).await, (1, 0, 0));

    let mut control = command.clone();
    control.operation_key = Uuid::new_v4();
    let applied = store
        .finish_scoped_leaf(&control)
        .await
        .expect("fresh group-wide key applies the supported flat selected member");
    assert_eq!(
        (
            applied.revision,
            applied.epoch,
            applied.active_members,
            applied.possession,
        ),
        (7, 5, 0, CapacityPossession::Retired)
    );
    assert_linked_effect(&f, &control, &applied).await;
    let after_apply = capacity_snapshot(&f, seeded.group).await;
    let replay = store
        .finish_scoped_leaf(&control)
        .await
        .expect("fresh-key applied result replays exactly");
    let mut expected_replay = applied.clone();
    expected_replay.replayed = true;
    assert_eq!(replay, expected_replay);
    assert_eq!(capacity_snapshot(&f, seeded.group).await, after_apply);
    assert_eq!(source_execution_snapshot(&f).await, source_before);
    assert_eq!(
        alternate_run_snapshot(&f, alternate).await,
        alternate_before
    );
    assert_eq!(finish_counts(&f, seeded.group).await, (2, 1, 1));
    f.verify().await;
}

#[path = "scoped_leaf_finish_cross_kind_decoder.rs"]
mod cross_kind_decoder;
