//! Schema-only probes for the dormant fixed OrdinaryLeafV1 scoped Finish binding.
//! Owner rows establish storage shape only, never source closure, certificate validity,
//! native Finish authority or a production caller.

use super::*;
use sagat_application::runs::StartRun;

const OWNER: &str = "lines";
const INDEX: i32 = 0;

struct ScopedFixture {
    base: Fixture,
    member: Uuid,
    original_run: Uuid,
    operation_key: Uuid,
}

#[derive(Clone, Copy)]
struct MemberShape<'a> {
    namespace: Uuid,
    kind: &'a str,
    chain: Option<Uuid>,
    original_run: Option<Uuid>,
    owner: Option<&'a str>,
    index: Option<i32>,
}

#[derive(Clone, Copy)]
struct ReceiptShape<'a> {
    member: Uuid,
    namespace: Uuid,
    kind: &'a str,
    chain: Option<Uuid>,
    head_run: Option<Uuid>,
    accepted_chain_revision: Option<i64>,
    accepted_run_revision: i64,
    original_run: Option<Uuid>,
    owner: Option<&'a str>,
    index: Option<i32>,
}

async fn flat_run(w: &World, name: &str) -> RunId {
    let (workflow, _) = w.publish(chain(name)).await;
    w.start(workflow, &serde_json::json!({})).await
}

async fn flat_run_in(w: &World, namespace: &str, name: &str) -> RunId {
    let bytes = serde_json::to_vec(&chain(name)).expect("flat run document");
    let created = w
        .defs
        .create_workflow(
            &w.admin,
            namespace,
            name,
            &bytes,
            &format!("create-{}", Uuid::new_v4()),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("create workflow in exact namespace");
    let workflow = created.workflow.id();
    w.defs
        .publish(
            &w.admin,
            workflow,
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::new_v4()),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("publish in exact namespace");
    let input = serde_json::to_vec(&serde_json::json!({})).expect("flat input");
    w.runs
        .start(
            &w.admin,
            &StartRun {
                namespace,
                workflow,
                version: None,
                input: &input,
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::new_v4()),
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("start flat run in exact namespace")
        .info
        .run_id
}

async fn scoped_fixture() -> ScopedFixture {
    let base = fixture().await;
    let original_run = flat_run(&base.w, "scoped-original-flat").await.as_uuid();
    let member = Uuid::new_v4();
    let operation_key = Uuid::new_v4();
    let tenant = base.w.admin.tenant_id().as_uuid();
    let namespace = base.w.namespace.id().as_uuid();
    let mut tx = base.w.db.pool.begin().await.expect("scoped owner fixture");
    let _: Uuid = sqlx::query_scalar(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(original_run)
    .fetch_one(&mut *tx)
    .await
    .expect("qualified original Run first");
    let _: Uuid = sqlx::query_scalar(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(base.group)
    .fetch_one(&mut *tx)
    .await
    .expect("selected G second");
    sqlx::query("UPDATE capacity_groups SET revision=2 WHERE tenant_id=$1 AND group_id=$2")
        .bind(tenant)
        .bind(base.group)
        .execute(&mut *tx)
        .await
        .expect("raw scoped receipt header");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state,finished_at) \
         VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,$5,$6,$7,'FINISHED',clock_timestamp())",
    )
    .bind(tenant)
    .bind(base.group)
    .bind(member)
    .bind(namespace)
    .bind(original_run)
    .bind(OWNER)
    .bind(INDEX)
    .execute(&mut *tx)
    .await
    .expect("valid retained scoped member");
    sqlx::query(
        "INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,\
         fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,\
         expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,\
         resulting_epoch,resulting_active_members,resulting_possession,\
         resulting_holder_member_id,finished_at,audit_id,outbox_id,member_kind,original_run_id,\
         root_owner_node,iteration_index) \
         VALUES($1,$2,$3,$4,$5,$6,NULL,NULL,1,1,NULL,1,2,1,1,'HELD',$7,\
         clock_timestamp(),$8,$9,'SCOPED_PATH',$10,$11,$12)",
    )
    .bind(tenant)
    .bind(base.group)
    .bind(operation_key)
    .bind(vec![3_u8; 32])
    .bind(namespace)
    .bind(member)
    .bind(base.member)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(original_run)
    .bind(OWNER)
    .bind(INDEX)
    .execute(&mut *tx)
    .await
    .expect("valid retained scoped receipt");
    tx.commit().await.expect("scoped storage fixture commits");
    ScopedFixture {
        base,
        member,
        original_run,
        operation_key,
    }
}

async fn insert_member(
    f: &Fixture,
    member: Uuid,
    shape: MemberShape<'_>,
    state: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state,finished_at) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,\
         CASE WHEN $10='FINISHED' THEN clock_timestamp() END)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .bind(member)
    .bind(shape.namespace)
    .bind(shape.kind)
    .bind(shape.chain)
    .bind(shape.original_run)
    .bind(shape.owner)
    .bind(shape.index)
    .bind(state)
    .execute(&f.w.db.pool)
    .await
    .map(|_| ())
}

async fn seed_valid_scoped_member(
    f: &Fixture,
    member: Uuid,
    original_run: Uuid,
    owner: &str,
    index: i32,
    with_receipt: bool,
) {
    let tenant = f.w.admin.tenant_id().as_uuid();
    let namespace = f.w.namespace.id().as_uuid();
    let mut tx =
        f.w.db
            .pool
            .begin()
            .await
            .expect("ordered scoped probe fixture");
    let _: Uuid = sqlx::query_scalar(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(original_run)
    .fetch_one(&mut *tx)
    .await
    .expect("probe original Run first");
    let _: Uuid = sqlx::query_scalar(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(f.group)
    .fetch_one(&mut *tx)
    .await
    .expect("probe G second");
    sqlx::query(
        "INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,\
         member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state,finished_at) \
         VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,$5,$6,$7,'FINISHED',clock_timestamp())",
    )
    .bind(tenant)
    .bind(f.group)
    .bind(member)
    .bind(namespace)
    .bind(original_run)
    .bind(owner)
    .bind(index)
    .execute(&mut *tx)
    .await
    .expect("valid ordered scoped probe member");
    if with_receipt {
        sqlx::query(
            "INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,\
             fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,\
             expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,\
             resulting_epoch,resulting_active_members,resulting_possession,finished_at,audit_id,\
             outbox_id,member_kind,original_run_id,root_owner_node,iteration_index) \
             VALUES($1,$2,$3,$4,$5,$6,NULL,NULL,2,1,NULL,1,3,1,1,'VACANT',\
             clock_timestamp(),$7,$8,'SCOPED_PATH',$9,$10,$11)",
        )
        .bind(tenant)
        .bind(f.group)
        .bind(Uuid::new_v4())
        .bind(vec![6_u8; 32])
        .bind(namespace)
        .bind(member)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(original_run)
        .bind(owner)
        .bind(index)
        .execute(&mut *tx)
        .await
        .expect("valid ordered scoped endpoint receipt");
    }
    tx.commit().await.expect("ordered scoped probe commits");
}

async fn insert_receipt(
    f: &Fixture,
    shape: ReceiptShape<'_>,
    expected_revision: i64,
) -> Result<(), sqlx::Error> {
    insert_receipt_at(
        f,
        (f.w.admin.tenant_id().as_uuid(), f.group),
        shape,
        expected_revision,
    )
    .await
}

async fn insert_receipt_at(
    f: &Fixture,
    target: (Uuid, Uuid),
    shape: ReceiptShape<'_>,
    expected_revision: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,\
         fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,\
         expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,\
         resulting_epoch,resulting_active_members,resulting_possession,finished_at,audit_id,\
         outbox_id,member_kind,original_run_id,root_owner_node,iteration_index) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,1,$10,$11,$9+1,1,1,'VACANT',\
         clock_timestamp(),$12,$13,$14,$15,$16,$17)",
    )
    .bind(target.0)
    .bind(target.1)
    .bind(Uuid::new_v4())
    .bind(vec![4_u8; 32])
    .bind(shape.namespace)
    .bind(shape.member)
    .bind(shape.chain)
    .bind(shape.head_run)
    .bind(expected_revision)
    .bind(shape.accepted_chain_revision)
    .bind(shape.accepted_run_revision)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(shape.kind)
    .bind(shape.original_run)
    .bind(shape.owner)
    .bind(shape.index)
    .execute(&f.w.db.pool)
    .await
    .map(|_| ())
}

async fn insert_child_receipt(
    f: &Fixture,
    operation_key: Uuid,
    expected_revision: i64,
    resulting_revision: i64,
    resulting_epoch: i64,
    resulting_members: i64,
    possession: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,\
         fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,\
         expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,\
         resulting_epoch,resulting_active_members,resulting_possession,finished_at,audit_id,outbox_id) \
         VALUES($1,$2,$3,$4,$5,$6,$7,$7,$8,1,0,0,$9,$10,$11,$12,clock_timestamp(),$13,$14)",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .bind(operation_key)
    .bind(vec![5_u8; 32])
    .bind(f.w.namespace.id().as_uuid())
    .bind(f.member)
    .bind(f.chain)
    .bind(expected_revision)
    .bind(resulting_revision)
    .bind(resulting_epoch)
    .bind(resulting_members)
    .bind(possession)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(&f.w.db.pool)
    .await
    .map(|_| ())
}

fn assert_named(error: sqlx::Error, code: &str, constraint: &str) {
    error_is(error, code, Some(constraint));
}

#[tokio::test]
async fn scoped_member_and_receipt_bind_to_a_qualified_flat_run_beside_child_root() {
    let f = scoped_fixture().await;
    let tenant = f.base.w.admin.tenant_id().as_uuid();
    type MemberRow = (
        String,
        Option<Uuid>,
        Option<Uuid>,
        Option<String>,
        Option<i32>,
        String,
    );
    let members: Vec<MemberRow> = sqlx::query_as(
        "SELECT member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state \
             FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 ORDER BY member_kind",
    )
    .bind(tenant)
    .bind(f.base.group)
    .fetch_all(&f.base.w.db.pool)
    .await
    .expect("both member variants");
    assert_eq!(
        members,
        vec![
            (
                "CHILD_ROOT".into(),
                Some(f.base.chain),
                None,
                None,
                None,
                "ACTIVE".into()
            ),
            (
                "SCOPED_PATH".into(),
                None,
                Some(f.original_run),
                Some(OWNER.into()),
                Some(INDEX),
                "FINISHED".into()
            ),
        ]
    );
    type ReceiptRow = (
        String,
        Option<Uuid>,
        Option<Uuid>,
        Option<i64>,
        Option<Uuid>,
        Option<String>,
        Option<i32>,
        i64,
    );
    let receipt: ReceiptRow = sqlx::query_as(
        "SELECT member_kind,chain_id,head_run_id,accepted_chain_revision,original_run_id,\
         root_owner_node,iteration_index,accepted_run_revision FROM capacity_group_finish_receipts \
         WHERE tenant_id=$1 AND group_id=$2 AND operation_key=$3",
    )
    .bind(tenant)
    .bind(f.base.group)
    .bind(f.operation_key)
    .fetch_one(&f.base.w.db.pool)
    .await
    .expect("exact scoped receipt");
    assert_eq!(
        receipt,
        (
            "SCOPED_PATH".into(),
            None,
            None,
            None,
            Some(f.original_run),
            Some(OWNER.into()),
            Some(INDEX),
            1
        )
    );
    let source: (bool, bool) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3),\
         EXISTS(SELECT 1 FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3)",
    )
    .bind(tenant).bind(f.base.w.namespace.id().as_uuid()).bind(f.original_run)
    .fetch_one(&f.base.w.db.pool).await.expect("flat original Run shape");
    assert_eq!(source, (true, false));
}

#[tokio::test]
async fn variants_bounds_qualified_run_and_exact_receipt_binding_are_closed() {
    let f = scoped_fixture().await;
    let namespace = f.base.w.namespace.id().as_uuid();
    let child = MemberShape {
        namespace,
        kind: "CHILD_ROOT",
        chain: Some(f.base.chain),
        original_run: None,
        owner: None,
        index: None,
    };
    let scoped = MemberShape {
        namespace,
        kind: "SCOPED_PATH",
        chain: None,
        original_run: Some(f.original_run),
        owner: Some("probe"),
        index: Some(1),
    };
    for (label, shape) in [
        (
            "scoped null original Run",
            MemberShape {
                original_run: None,
                ..scoped
            },
        ),
        (
            "scoped null owner",
            MemberShape {
                owner: None,
                ..scoped
            },
        ),
        (
            "scoped null index",
            MemberShape {
                index: None,
                ..scoped
            },
        ),
        (
            "scoped extra chain",
            MemberShape {
                chain: Some(f.base.chain),
                ..scoped
            },
        ),
        (
            "child null chain",
            MemberShape {
                chain: None,
                ..child
            },
        ),
        (
            "child extra original Run",
            MemberShape {
                original_run: Some(f.original_run),
                ..child
            },
        ),
        (
            "child extra owner",
            MemberShape {
                owner: Some("extra"),
                ..child
            },
        ),
        (
            "child extra index",
            MemberShape {
                index: Some(1),
                ..child
            },
        ),
    ] {
        assert_named(
            insert_member(&f.base, Uuid::new_v4(), shape, "ACTIVE")
                .await
                .expect_err(label),
            "23514",
            "capacity_group_members_kind",
        );
    }
    let owner_65 = format!("a{}", "b".repeat(64));
    for (label, shape) in [
        (
            "negative member index",
            MemberShape {
                index: Some(-1),
                ..scoped
            },
        ),
        (
            "member index above endpoint",
            MemberShape {
                index: Some(10_000),
                ..scoped
            },
        ),
        (
            "member owner above endpoint",
            MemberShape {
                owner: Some(owner_65.as_str()),
                ..scoped
            },
        ),
        (
            "MemberShape empty owner",
            MemberShape {
                owner: Some(""),
                ..scoped
            },
        ),
        (
            "MemberShape leading punctuation",
            MemberShape {
                owner: Some(".owner"),
                ..scoped
            },
        ),
        (
            "MemberShape non-ASCII owner",
            MemberShape {
                owner: Some("é"),
                ..scoped
            },
        ),
        (
            "member owner regex",
            MemberShape {
                owner: Some("bad/owner"),
                ..scoped
            },
        ),
    ] {
        assert_named(
            insert_member(&f.base, Uuid::new_v4(), shape, "ACTIVE")
                .await
                .expect_err(label),
            "23514",
            "capacity_group_members_kind",
        );
    }

    let other_namespace = f.base.w.add_namespace("scoped-other").await;
    let other_namespace_run = flat_run_in(&f.base.w, "scoped-other", "scoped-other-flat")
        .await
        .as_uuid();
    let foreign_workflow = f
        .base
        .w
        .publish_as_other_tenant(chain("scoped-foreign-flat"))
        .await;
    let foreign_run = f
        .base
        .w
        .start_as_other_tenant(foreign_workflow, &serde_json::json!({}))
        .await
        .expect("existing foreign tenant flat Run")
        .as_uuid();
    for (label, shape) in [
        (
            "existing foreign tenant Run",
            MemberShape {
                namespace: f.base.w.other_namespace.id().as_uuid(),
                original_run: Some(foreign_run),
                owner: Some("foreign"),
                index: Some(2),
                ..scoped
            },
        ),
        (
            "existing Run from another namespace",
            MemberShape {
                original_run: Some(other_namespace_run),
                owner: Some("other-ns"),
                index: Some(3),
                ..scoped
            },
        ),
        (
            "missing Run",
            MemberShape {
                original_run: Some(Uuid::new_v4()),
                owner: Some("missing"),
                index: Some(4),
                ..scoped
            },
        ),
    ] {
        assert_named(
            insert_member(&f.base, Uuid::new_v4(), shape, "ACTIVE")
                .await
                .expect_err(label),
            "23503",
            "capacity_group_members_original_run_fk",
        );
    }

    let probe_member = Uuid::new_v4();
    seed_valid_scoped_member(&f.base, probe_member, f.original_run, "probe", 1, false).await;
    assert_named(
        insert_member(&f.base, Uuid::new_v4(), scoped, "FINISHED")
            .await
            .expect_err("duplicate path"),
        "23505",
        "capacity_group_members_scoped_path_unique",
    );
    let child_receipt = ReceiptShape {
        member: f.base.member,
        namespace,
        kind: "CHILD_ROOT",
        chain: Some(f.base.chain),
        head_run: Some(f.base.chain),
        accepted_chain_revision: Some(0),
        accepted_run_revision: 0,
        original_run: None,
        owner: None,
        index: None,
    };
    let scoped_receipt = ReceiptShape {
        member: probe_member,
        namespace,
        kind: "SCOPED_PATH",
        chain: None,
        head_run: None,
        accepted_chain_revision: None,
        accepted_run_revision: 1,
        original_run: Some(f.original_run),
        owner: Some("probe"),
        index: Some(1),
    };
    for (label, shape) in [
        (
            "scoped null original Run",
            ReceiptShape {
                original_run: None,
                ..scoped_receipt
            },
        ),
        (
            "scoped null owner",
            ReceiptShape {
                owner: None,
                ..scoped_receipt
            },
        ),
        (
            "scoped null index",
            ReceiptShape {
                index: None,
                ..scoped_receipt
            },
        ),
        (
            "scoped extra chain",
            ReceiptShape {
                chain: Some(f.base.chain),
                ..scoped_receipt
            },
        ),
        (
            "scoped extra head",
            ReceiptShape {
                head_run: Some(f.base.chain),
                ..scoped_receipt
            },
        ),
        (
            "scoped extra accepted chain",
            ReceiptShape {
                accepted_chain_revision: Some(0),
                ..scoped_receipt
            },
        ),
        (
            "scoped accepted run zero",
            ReceiptShape {
                accepted_run_revision: 0,
                ..scoped_receipt
            },
        ),
        (
            "child null chain",
            ReceiptShape {
                chain: None,
                ..child_receipt
            },
        ),
        (
            "child null head",
            ReceiptShape {
                head_run: None,
                ..child_receipt
            },
        ),
        (
            "child null accepted chain",
            ReceiptShape {
                accepted_chain_revision: None,
                ..child_receipt
            },
        ),
        (
            "child extra original Run",
            ReceiptShape {
                original_run: Some(f.original_run),
                ..child_receipt
            },
        ),
        (
            "child extra owner",
            ReceiptShape {
                owner: Some("extra"),
                ..child_receipt
            },
        ),
        (
            "child extra index",
            ReceiptShape {
                index: Some(1),
                ..child_receipt
            },
        ),
        (
            "ReceiptShape empty owner",
            ReceiptShape {
                owner: Some(""),
                ..scoped_receipt
            },
        ),
        (
            "ReceiptShape leading punctuation",
            ReceiptShape {
                owner: Some(".owner"),
                ..scoped_receipt
            },
        ),
        (
            "ReceiptShape non-ASCII owner",
            ReceiptShape {
                owner: Some("é"),
                ..scoped_receipt
            },
        ),
        (
            "receipt owner regex",
            ReceiptShape {
                owner: Some("bad/owner"),
                ..scoped_receipt
            },
        ),
        (
            "negative receipt index",
            ReceiptShape {
                index: Some(-1),
                ..scoped_receipt
            },
        ),
        (
            "receipt index above endpoint",
            ReceiptShape {
                index: Some(10_000),
                ..scoped_receipt
            },
        ),
        (
            "receipt owner above endpoint",
            ReceiptShape {
                owner: Some(owner_65.as_str()),
                ..scoped_receipt
            },
        ),
    ] {
        assert_named(
            insert_receipt(&f.base, shape, 2).await.expect_err(label),
            "23514",
            "capacity_group_finish_receipts_kind",
        );
    }

    let alternate_run = flat_run(&f.base.w, "scoped-binding-alternate")
        .await
        .as_uuid();
    for (label, shape) in [
        (
            "member",
            ReceiptShape {
                member: Uuid::new_v4(),
                ..scoped_receipt
            },
        ),
        (
            "namespace",
            ReceiptShape {
                namespace: other_namespace.id().as_uuid(),
                ..scoped_receipt
            },
        ),
        (
            "original Run",
            ReceiptShape {
                original_run: Some(alternate_run),
                ..scoped_receipt
            },
        ),
        (
            "owner",
            ReceiptShape {
                owner: Some("other-owner"),
                ..scoped_receipt
            },
        ),
        (
            "index",
            ReceiptShape {
                index: Some(2),
                ..scoped_receipt
            },
        ),
    ] {
        assert_named(
            insert_receipt(&f.base, shape, 2).await.expect_err(label),
            "23503",
            "capacity_group_finish_receipts_scoped_binding_fk",
        );
    }

    // Both destination groups exist. Only the new scoped FK closes cross-group
    // and cross-tenant binding; the nullable legacy chain FK cannot do so.
    let tenant = f.base.w.admin.tenant_id().as_uuid();
    let foreign_tenant = f.base.w.other_admin.tenant_id().as_uuid();
    let other_group = Uuid::new_v4();
    for (label, target) in [
        ("existing different group", (tenant, other_group)),
        (
            "existing different tenant, same group UUID",
            (foreign_tenant, f.base.group),
        ),
    ] {
        sqlx::query(
            "INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession) \
             VALUES($1,$2,1,1,0,'RETIRED')",
        )
        .bind(target.0)
        .bind(target.1)
        .execute(&f.base.w.db.pool)
        .await
        .expect("existing qualified destination group");
        assert_named(
            insert_receipt_at(&f.base, target, scoped_receipt, 2)
                .await
                .expect_err(label),
            "23503",
            "capacity_group_finish_receipts_scoped_binding_fk",
        );
    }

    let minimum_owner_member = Uuid::new_v4();
    seed_valid_scoped_member(&f.base, minimum_owner_member, f.original_run, "a", 0, false).await;
    insert_receipt(
        &f.base,
        ReceiptShape {
            member: minimum_owner_member,
            owner: Some("a"),
            index: Some(0),
            ..scoped_receipt
        },
        4,
    )
    .await
    .expect("one-character owner and lower index endpoint accepted by both tables");

    let owner_64 = format!("a{}", "b".repeat(63));
    let endpoint_member = Uuid::new_v4();
    seed_valid_scoped_member(
        &f.base,
        endpoint_member,
        f.original_run,
        owner_64.as_str(),
        9_999,
        true,
    )
    .await;
}

#[tokio::test]
async fn receipt_operation_revision_and_member_keys_remain_global_across_kinds() {
    let f = scoped_fixture().await;
    assert_named(
        insert_child_receipt(&f.base, f.operation_key, 2, 3, 2, 0, "RETIRED")
            .await
            .expect_err("global operation key"),
        "23505",
        "capacity_group_finish_receipts_pkey",
    );
    assert_named(
        insert_child_receipt(&f.base, Uuid::new_v4(), 1, 2, 1, 1, "VACANT")
            .await
            .expect_err("global result revision"),
        "23505",
        "capacity_group_finish_receipts_result_revision_unique",
    );
    let keys: Vec<(String, Vec<String>)> = sqlx::query_as(
        "SELECT c.conname::text,array_agg(a.attname::text ORDER BY k.ord)::text[] FROM pg_constraint c \
         CROSS JOIN LATERAL unnest(c.conkey) WITH ORDINALITY AS k(attnum,ord) \
         JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.attnum \
         WHERE c.conrelid='capacity_group_finish_receipts'::regclass \
         AND c.conname IN ('capacity_group_finish_receipts_pkey','capacity_group_finish_receipts_result_revision_unique','capacity_group_finish_receipts_member_unique') \
         GROUP BY c.conname ORDER BY c.conname",
    ).fetch_all(&f.base.w.db.pool).await.expect("global receipt key catalog");
    assert_eq!(
        keys,
        vec![
            (
                "capacity_group_finish_receipts_member_unique".into(),
                vec!["tenant_id".into(), "group_id".into(), "member_id".into()]
            ),
            (
                "capacity_group_finish_receipts_pkey".into(),
                vec![
                    "tenant_id".into(),
                    "group_id".into(),
                    "operation_key".into()
                ]
            ),
            (
                "capacity_group_finish_receipts_result_revision_unique".into(),
                vec![
                    "tenant_id".into(),
                    "group_id".into(),
                    "resulting_revision".into()
                ]
            ),
        ]
    );
}

#[tokio::test]
async fn active_scoped_identity_and_finished_retention_use_always_guards() {
    let f = scoped_fixture().await;
    let tenant = f.base.w.admin.tenant_id().as_uuid();
    let namespace = f.base.w.namespace.id().as_uuid();
    let alternate_original = flat_run(&f.base.w, "scoped-identity-alternate")
        .await
        .as_uuid();
    let alternate_chain = seed_chain(&f.base.w, "scoped-kind-alternate")
        .await
        .as_uuid();
    let active_member = Uuid::new_v4();
    let mut tx = f
        .base
        .w
        .db
        .pool
        .begin()
        .await
        .expect("active scoped fixture");
    let _: Uuid = sqlx::query_scalar(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR UPDATE",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(f.original_run)
    .fetch_one(&mut *tx)
    .await
    .expect("active original Run first");
    let _: Uuid = sqlx::query_scalar(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(f.base.group)
    .fetch_one(&mut *tx)
    .await
    .expect("active G second");
    sqlx::query(
        "UPDATE capacity_groups SET revision=3,active_members=2 WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(tenant)
    .bind(f.base.group)
    .execute(&mut *tx)
    .await
    .expect("active header");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,original_run_id,root_owner_node,iteration_index,state) VALUES($1,$2,$3,$4,'SCOPED_PATH',NULL,$5,'active',2,'ACTIVE')")
        .bind(tenant).bind(f.base.group).bind(active_member).bind(namespace).bind(f.original_run)
        .execute(&mut *tx).await.expect("valid active scoped member");
    tx.commit().await.expect("active scoped fixture commits");

    for (sql, label) in [
        (
            "UPDATE capacity_group_members SET original_run_id=$4 WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3",
            "alternate qualified Run",
        ),
        (
            "UPDATE capacity_group_members SET root_owner_node='changed' WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3 AND $4::uuid IS NOT NULL",
            "valid unique owner",
        ),
        (
            "UPDATE capacity_group_members SET iteration_index=3 WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3 AND $4::uuid IS NOT NULL",
            "valid unique index",
        ),
    ] {
        error_is(
            sqlx::query(sql)
                .bind(tenant)
                .bind(f.base.group)
                .bind(active_member)
                .bind(alternate_original)
                .execute(&f.base.w.db.pool)
                .await
                .expect_err(label),
            "23514",
            None,
        );
    }
    error_is(sqlx::query("UPDATE capacity_group_members SET member_kind='CHILD_ROOT',chain_id=$4,original_run_id=NULL,root_owner_node=NULL,iteration_index=NULL WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
        .bind(tenant).bind(f.base.group).bind(active_member).bind(alternate_chain)
        .execute(&f.base.w.db.pool).await.expect_err("valid ChildRoot tuple remains immutable"), "23514", None);
    error_is(sqlx::query("UPDATE capacity_group_members SET finished_at=finished_at+interval '1 microsecond' WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
        .bind(tenant).bind(f.base.group).bind(f.member)
        .execute(&f.base.w.db.pool).await.expect_err("finished timestamp remains immutable"), "23514", None);

    let mut replica = f
        .base
        .w
        .db
        .pool
        .acquire()
        .await
        .expect("replica owner connection");
    sqlx::query("SET session_replication_role='replica'")
        .execute(&mut *replica)
        .await
        .expect("replica mode");
    let rewrite = sqlx::query("UPDATE capacity_group_members SET root_owner_node='replica-change' WHERE tenant_id=$1 AND group_id=$2 AND member_id=$3")
        .bind(tenant).bind(f.base.group).bind(active_member).execute(&mut *replica).await;
    sqlx::query("SET session_replication_role='origin'")
        .execute(&mut *replica)
        .await
        .expect("origin mode");
    error_is(
        rewrite.expect_err("ALWAYS guard fires for ACTIVE row in replica mode"),
        "23514",
        None,
    );
    for statement in [
        "UPDATE capacity_group_finish_receipts SET fingerprint=fingerprint",
        "DELETE FROM capacity_group_finish_receipts",
        "TRUNCATE capacity_group_finish_receipts",
        "DELETE FROM capacity_group_members WHERE member_kind='SCOPED_PATH'",
        "TRUNCATE capacity_group_members CASCADE",
    ] {
        error_is(
            sqlx::query(statement)
                .execute(&f.base.w.db.pool)
                .await
                .expect_err(statement),
            "23001",
            None,
        );
    }
    let guards: Vec<(String, String)> = sqlx::query_as(
        "SELECT tgname::text,tgenabled::text FROM pg_trigger \
         WHERE tgrelid IN ('capacity_group_members'::regclass,'capacity_group_finish_receipts'::regclass) \
         AND tgname IN ('capacity_group_members_guard','capacity_group_members_no_truncate','capacity_group_finish_receipts_append_only','capacity_group_finish_receipts_no_truncate') ORDER BY tgname",
    ).fetch_all(&f.base.w.db.pool).await.expect("ALWAYS guard catalog");
    assert_eq!(
        guards,
        vec![
            (
                "capacity_group_finish_receipts_append_only".into(),
                "A".into()
            ),
            (
                "capacity_group_finish_receipts_no_truncate".into(),
                "A".into()
            ),
            ("capacity_group_members_guard".into(), "A".into()),
            ("capacity_group_members_no_truncate".into(), "A".into()),
        ]
    );
}

#[tokio::test]
async fn runtime_insert_columns_are_exactly_legacy_and_scoped_release_stays_closed() {
    let f = fixture().await;
    let runtime = f.w.db.runtime_pool().await;
    let table_insert: bool = sqlx::query_scalar(
        "SELECT has_table_privilege(current_user,'capacity_group_finish_receipts','INSERT')",
    )
    .fetch_one(&runtime)
    .await
    .expect("table privilege");
    assert!(!table_insert);
    let insert_columns: Vec<String> = sqlx::query_scalar(
        "SELECT attname::text FROM pg_attribute WHERE attrelid='capacity_group_finish_receipts'::regclass \
         AND attnum>0 AND NOT attisdropped \
         AND has_column_privilege(current_user,'capacity_group_finish_receipts',attname::text,'INSERT') ORDER BY attnum",
    ).fetch_all(&runtime).await.expect("effective INSERT column set");
    assert_eq!(
        insert_columns,
        [
            "tenant_id",
            "group_id",
            "operation_key",
            "fingerprint",
            "namespace_id",
            "member_id",
            "chain_id",
            "head_run_id",
            "expected_group_revision",
            "expected_group_epoch",
            "accepted_chain_revision",
            "accepted_run_revision",
            "resulting_revision",
            "resulting_epoch",
            "resulting_active_members",
            "resulting_possession",
            "resulting_holder_member_id",
            "finished_at",
            "audit_id",
            "outbox_id",
        ]
        .map(str::to_owned)
        .to_vec()
    );

    let mut conn = runtime.acquire().await.expect("runtime connection");
    receipt(&mut conn, &f, f.member, f.chain, 1, (2, "RETIRED", None, 0))
        .await
        .expect("exact legacy 20-column INSERT succeeds");
    let stored: (String, Option<Uuid>, Option<String>, Option<i32>) = sqlx::query_as(
        "SELECT member_kind,original_run_id,root_owner_node,iteration_index FROM capacity_group_finish_receipts WHERE tenant_id=$1 AND group_id=$2",
    ).bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).fetch_one(&mut *conn).await.expect("legacy defaults");
    assert_eq!(stored, ("CHILD_ROOT".into(), None, None, None));
    error_is(sqlx::query("INSERT INTO capacity_group_finish_receipts(tenant_id,group_id,operation_key,fingerprint,namespace_id,member_id,chain_id,head_run_id,expected_group_revision,expected_group_epoch,accepted_chain_revision,accepted_run_revision,resulting_revision,resulting_epoch,resulting_active_members,resulting_possession,finished_at,audit_id,outbox_id,member_kind,original_run_id,root_owner_node,iteration_index) VALUES($1,$2,$3,$4,$5,$6,NULL,NULL,2,1,NULL,1,3,1,1,'VACANT',clock_timestamp(),$7,$8,'SCOPED_PATH',$9,'lines',0)")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(Uuid::new_v4()).bind(vec![8_u8;32])
        .bind(f.w.namespace.id().as_uuid()).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(f.chain)
        .execute(&mut *conn).await.expect_err("all scoped INSERT columns denied"), "42501", None);
    error_is(
        sqlx::query("INSERT INTO leaf_collection_success_certifications DEFAULT VALUES")
            .execute(&mut *conn)
            .await
            .expect_err("certificate INSERT remains closed"),
        "42501",
        None,
    );
}

#[path = "capacity_scoped_downgrade.rs"]
mod downgrade;
