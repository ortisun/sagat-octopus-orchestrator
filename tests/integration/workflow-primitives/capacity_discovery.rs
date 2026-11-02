//! Owner-seeded dormant bindings prove discovery only, never creation or execution authority.
use super::fixture;
use sagat_application::{
    capacity_discovery::{CapacityDiscoveryError, CapacityDiscoveryStore},
    maintenance::Window,
};

#[path = "capacity_holder_discovery.rs"]
mod holders;

#[tokio::test]
async fn a_vacant_active_member_is_discovered_with_its_current_head() {
    let f = fixture().await;
    sqlx::query("UPDATE capacity_groups SET possession='VACANT',holder_member_id=NULL,revision=2,epoch=2 WHERE tenant_id=$1 AND group_id=$2")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).execute(&f.w.db.pool).await.expect("owner seeds dormant vacancy");
    let page =
        f.w.store
            .discover_vacant_child_roots(&Window::everything(1), None)
            .await
            .expect("runtime discovery");
    assert_eq!(
        page.candidates.len(),
        1,
        "the persisted active member is a candidate"
    );
    let candidate = &page.candidates[0];
    assert_eq!(candidate.cursor.tenant_id, f.w.admin.tenant_id());
    assert_eq!(candidate.cursor.group_id.as_uuid(), f.group);
    assert_eq!(candidate.cursor.member_id.as_uuid(), f.member);
    assert_eq!(candidate.head_run_id.as_uuid(), f.chain);
    assert_eq!(candidate.chain_id.as_uuid(), f.chain);
    assert_eq!(candidate.namespace_id, f.w.namespace.id());
    assert_eq!((candidate.group_revision, candidate.group_epoch), (2, 2));
    assert_eq!(
        candidate.chain_revision, 0,
        "new chain revision zero is valid"
    );
    assert_eq!(page.scanned_through, Some(candidate.cursor));
    assert!(!page.has_more);
}

#[tokio::test]
async fn invalid_discovery_windows_are_typed_refusals() {
    let f = fixture().await;
    for limit in [0, 257, u32::MAX] {
        assert_eq!(
            f.w.store
                .discover_vacant_child_roots(&Window::everything(limit), None)
                .await,
            Err(CapacityDiscoveryError::InvalidWindow)
        );
    }
    let mut window = Window::everything(1);
    window.exclude = vec![f.w.admin.tenant_id(); 257];
    assert_eq!(
        f.w.store.discover_vacant_child_roots(&window, None).await,
        Err(CapacityDiscoveryError::InvalidWindow)
    );
}

async fn vacant(f: &super::Fixture) {
    sqlx::query("UPDATE capacity_groups SET possession='VACANT',holder_member_id=NULL,revision=2,epoch=2 WHERE tenant_id=$1 AND group_id=$2")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).execute(&f.w.db.pool).await.expect("owner vacancy fixture only");
}

#[tokio::test]
async fn an_empty_filtered_page_advances_before_a_fresh_store_resumes() {
    use sagat_infra_postgres::{BeginSafely as _, PgRunStore};
    use uuid::Uuid;
    let f = fixture().await;
    vacant(&f).await;
    let chain = super::seed_chain(&f.w, "discovery-held-prefix").await;
    let group = Uuid::from_u128(1);
    let member = Uuid::from_u128(2);
    let mut tx =
        f.w.db
            .pool
            .begin_safely()
            .await
            .expect("owner fixture transaction");
    sqlx::query("INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession,holder_member_id) VALUES($1,$2,1,1,1,'HELD',$3)")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(group).bind(member).execute(&mut *tx).await.expect("owner prefix header");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(group).bind(member).bind(f.w.namespace.id().as_uuid()).bind(chain.as_uuid()).execute(&mut *tx).await.expect("owner prefix member");
    tx.commit().await.expect("complete dormant prefix");
    let first =
        f.w.store
            .discover_vacant_child_roots(&Window::everything(1), None)
            .await
            .expect("first page");
    assert!(first.candidates.is_empty());
    assert!(first.has_more);
    assert_eq!(
        first
            .scanned_through
            .expect("filtered input still advances")
            .group_id
            .as_uuid(),
        group
    );
    let fresh = PgRunStore::new(f.w.db.runtime_pool_with(2).await, f.w.cache.clone());
    let second = fresh
        .discover_vacant_child_roots(&Window::everything(1), first.scanned_through)
        .await
        .expect("retained cursor with a new store");
    assert_eq!(second.candidates.len(), 1);
    assert_eq!(second.candidates[0].cursor.group_id.as_uuid(), f.group);
    assert!(!second.has_more);
    let end = fresh
        .discover_vacant_child_roots(&Window::everything(1), second.scanned_through)
        .await
        .expect("end page");
    assert!(end.candidates.is_empty());
    assert!(end.scanned_through.is_none());
    assert!(!end.has_more);
}

#[tokio::test]
async fn shard_and_tenant_filters_consume_their_page_without_granting_authority() {
    use sagat_domain::sharding::{Shard, ShardSet};
    let f = fixture().await;
    vacant(&f).await;
    let mut window = Window::everything(1);
    window.shards = ShardSet::empty();
    let empty =
        f.w.store
            .discover_vacant_child_roots(&window, None)
            .await
            .expect("unowned shard");
    assert!(empty.candidates.is_empty());
    assert!(empty.scanned_through.is_some());
    window.shards.insert(Shard::of(f.chain));
    window.exclude.push(f.w.admin.tenant_id());
    let excluded =
        f.w.store
            .discover_vacant_child_roots(&window, None)
            .await
            .expect("excluded tenant");
    assert!(excluded.candidates.is_empty());
    assert_eq!(excluded.scanned_through, empty.scanned_through);
    window.exclude = vec![f.w.other_admin.tenant_id()];
    assert_eq!(
        f.w.store
            .discover_vacant_child_roots(&window, None)
            .await
            .expect("own shard")
            .candidates
            .len(),
        1
    );
    f.w.quotas
        .set(
            &f.w.admin,
            None,
            sagat_domain::quota::QuotaResource::IterationConcurrency,
            0,
            &crate::support::engine::ctx(),
        )
        .await
        .expect("public quota service permits zero after the binding exists");
    assert_eq!(
        f.w.store
            .discover_vacant_child_roots(&window, None)
            .await
            .expect("zero admission policy does not filter an existing registry hint")
            .candidates
            .len(),
        1
    );
}

#[tokio::test]
async fn runtime_discovery_ignores_a_held_row_lock_and_rewrites_nothing() {
    use sagat_infra_postgres::{BeginSafely as _, PgRunStore};
    use std::time::Duration;
    let f = fixture().await;
    vacant(&f).await;
    let pool = f.w.db.runtime_pool_with(2).await;
    let role: (String, bool) =
        sqlx::query_as("SELECT current_user,rolsuper FROM pg_roles WHERE rolname=current_user")
            .fetch_one(&pool)
            .await
            .expect("actual reader role");
    assert_eq!(role, ("sagat_app".to_owned(), false));
    let store = PgRunStore::new(pool, f.w.cache.clone());
    let state = super::state(&f.w.db.pool, &f).await;
    let execution = super::execution_snapshot(&f).await;
    let counts = super::finish_counts(&f).await;
    let effects: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM audit_records),(SELECT count(*) FROM event_outbox)",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("all audit/outbox counts");
    let mut held =
        f.w.db
            .pool
            .begin_safely()
            .await
            .expect("owner lock barrier");
    sqlx::query(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .fetch_one(&mut *held)
    .await
    .expect("group row held throughout the read");
    let page = crate::support::bounded::within(
        "read-only discovery while G is held",
        Duration::from_secs(2),
        store.discover_vacant_child_roots(&Window::everything(1), None),
    )
    .await
    .expect("plain SELECT needs no row lock");
    assert_eq!(page.candidates.len(), 1);
    held.rollback().await.expect("release barrier");
    assert_eq!(super::state(&f.w.db.pool, &f).await, state);
    assert_eq!(super::execution_snapshot(&f).await, execution);
    assert_eq!(super::finish_counts(&f).await, counts);
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM audit_records),(SELECT count(*) FROM event_outbox)",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit/outbox after read");
    assert_eq!(after, effects);
}

#[tokio::test]
async fn retained_finished_members_are_not_scan_input_and_the_index_reverses() {
    use sagat_infra_postgres::BeginSafely as _;
    let f = fixture().await;
    let index: String = sqlx::query_scalar(
        "SELECT pg_get_indexdef('capacity_members_active_discovery_idx'::regclass)",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("declared partial index");
    assert!(index.contains("(tenant_id, group_id, member_id)"));
    assert!(index.contains("'ACTIVE'"));
    assert!(index.contains("'CHILD_ROOT'"));
    let mut tx =
        f.w.db
            .pool
            .begin_safely()
            .await
            .expect("owner retained tombstone fixture");
    super::retire(&mut tx, &f).await;
    super::finish_member(&mut tx, &f, f.member).await;
    tx.commit()
        .await
        .expect("consistent retained finished fixture");
    let page =
        f.w.store
            .discover_vacant_child_roots(&Window::everything(1), None)
            .await
            .expect("no active input");
    assert!(page.candidates.is_empty());
    assert!(page.scanned_through.is_none());
    assert!(!page.has_more);
    sagat_infra_postgres::MIGRATOR
        .undo(&f.w.db.pool, 20261224000000)
        .await
        .expect("dropping only the read-side index preserves retained metadata");
    sagat_infra_postgres::migrate(&f.w.db.pool)
        .await
        .expect("index reapplied");
    assert!(
        f.w.store
            .discover_vacant_child_roots(&Window::everything(1), None)
            .await
            .expect("retained tombstone after index reapply")
            .candidates
            .is_empty()
    );
}

#[tokio::test]
async fn actual_child_continuation_changes_the_discovered_head_without_changing_membership() {
    let n = super::native_fixture(true).await;
    vacant(&n.f).await;
    let original = super::complete_native_activity(&n).await;
    for _ in 0..8 {
        let round = n.scheduler.tick().await;
        assert_eq!(round.failed, 0);
        if !round.did_work() {
            break;
        }
    }
    let head:uuid::Uuid=sqlx::query_scalar("SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3")
        .bind(n.f.w.admin.tenant_id().as_uuid()).bind(n.f.w.namespace.id().as_uuid()).bind(n.f.chain).fetch_one(&n.f.w.db.pool).await.expect("actual current child head");
    assert_ne!(head, original.as_uuid());
    let page =
        n.f.w
            .store
            .discover_vacant_child_roots(&Window::everything(1), None)
            .await
            .expect("read new head");
    assert_eq!(page.candidates.len(), 1);
    let candidate = &page.candidates[0];
    assert_eq!(candidate.chain_id.as_uuid(), n.f.chain);
    assert_eq!(candidate.head_run_id.as_uuid(), head);
    assert_eq!(candidate.cursor.member_id.as_uuid(), n.f.member);
    n.f.w
        .store
        .verify_run(n.f.w.admin.tenant_id(), original)
        .await
        .expect("original continued journal");
    n.f.w
        .store
        .verify_run(
            n.f.w.admin.tenant_id(),
            sagat_domain::execution::RunId::new(head),
        )
        .await
        .expect("successor journal");
}

#[tokio::test]
async fn equal_registry_ids_in_two_tenants_keep_their_own_namespace_and_head() {
    use sagat_infra_postgres::BeginSafely as _;
    let f = fixture().await;
    vacant(&f).await;
    let workflow =
        f.w.publish_as_other_tenant(crate::support::engine::chain("discovery-other-tenant"))
            .await;
    let run =
        f.w.start_as_other_tenant(workflow, &serde_json::json!({}))
            .await
            .expect("real other-tenant run");
    let version: uuid::Uuid =
        sqlx::query_scalar("SELECT workflow_version_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.w.other_admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("other pinned definition");
    let mut tx =
        f.w.db
            .pool
            .begin_safely()
            .await
            .expect("owner-only dormant registry fixture");
    sqlx::query("INSERT INTO execution_chains(tenant_id,namespace_id,chain_id,current_run_id,definition_version_id,started_by,max_segments,original_deadline_at,created_at,updated_at) VALUES($1,$2,$3,$3,$4,$5,2,$6,$7,$7)")
        .bind(f.w.other_admin.tenant_id().as_uuid()).bind(f.w.other_namespace.id().as_uuid()).bind(run.as_uuid()).bind(version).bind(f.w.other_admin.subject().as_str()).bind(crate::support::now()+time::Duration::hours(1)).bind(crate::support::now()).execute(&mut *tx).await.expect("other tenant chain binding fixture");
    sqlx::query("INSERT INTO chain_segments(tenant_id,namespace_id,chain_id,segment_index,run_id,created_at) VALUES($1,$2,$3,0,$3,$4)")
        .bind(f.w.other_admin.tenant_id().as_uuid()).bind(f.w.other_namespace.id().as_uuid()).bind(run.as_uuid()).bind(crate::support::now()).execute(&mut *tx).await.expect("other tenant segment fixture");
    sqlx::query("INSERT INTO capacity_groups(tenant_id,group_id,revision,epoch,active_members,possession) VALUES($1,$2,2,2,1,'VACANT')")
        .bind(f.w.other_admin.tenant_id().as_uuid()).bind(f.group).execute(&mut *tx).await.expect("same group UUID under another tenant");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
        .bind(f.w.other_admin.tenant_id().as_uuid()).bind(f.group).bind(f.member).bind(f.w.other_namespace.id().as_uuid()).bind(run.as_uuid()).execute(&mut *tx).await.expect("same member UUID under another tenant");
    tx.commit()
        .await
        .expect("complete referential fixture, not group creation authority");
    let page =
        f.w.store
            .discover_vacant_child_roots(&Window::everything(2), None)
            .await
            .expect("both tenants");
    assert_eq!(page.candidates.len(), 2);
    for c in &page.candidates {
        assert_eq!(c.cursor.group_id.as_uuid(), f.group);
        assert_eq!(c.cursor.member_id.as_uuid(), f.member);
        if c.cursor.tenant_id == f.w.admin.tenant_id() {
            assert_eq!(c.namespace_id, f.w.namespace.id());
            assert_eq!(c.head_run_id.as_uuid(), f.chain);
        } else {
            assert_eq!(c.cursor.tenant_id, f.w.other_admin.tenant_id());
            assert_eq!(c.namespace_id, f.w.other_namespace.id());
            assert_eq!(c.head_run_id, run);
        }
    }
    for excluded in [f.w.admin.tenant_id(), f.w.other_admin.tenant_id()] {
        let mut window = Window::everything(2);
        window.exclude.push(excluded);
        let filtered =
            f.w.store
                .discover_vacant_child_roots(&window, None)
                .await
                .expect("exclude exact tenant");
        assert_eq!(filtered.candidates.len(), 1);
        assert_ne!(filtered.candidates[0].cursor.tenant_id, excluded);
        assert_eq!(filtered.scanned_through, page.scanned_through);
    }
}

#[tokio::test]
async fn discovery_span_records_bounded_counts_without_tenant_fields() {
    let f = fixture().await;
    vacant(&f).await;
    let captured = crate::support::logs::Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    let page =
        f.w.store
            .discover_vacant_child_roots(&Window::everything(1), None)
            .await
            .expect("traced read");
    assert_eq!(page.candidates.len(), 1);
    let text = captured.text();
    assert!(
        text.contains("capacity_discovery")
            && text.contains("scanned=1")
            && text.contains("selected=1"),
        "closed query action and counts"
    );
    assert!(!text.contains(&f.w.admin.tenant_id().to_string()));
    assert!(!text.contains(&f.chain.to_string()));
}
