//! Read-only holder hints. Owner metadata fixtures prove no creator, readiness or execution.
use super::*;
use sagat_infra_postgres::PgRunStore;
use uuid::Uuid;

#[tokio::test]
async fn selected_holder_is_observed_with_its_current_head_and_revisions() {
    let f = fixture().await;
    let runtime = f.w.db.runtime_pool_with(8).await;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .expect("runtime role");
    assert_eq!(role, "sagat_app");
    let store = PgRunStore::new(runtime, f.w.cache.clone());
    let page = store
        .discover_child_root_holders(&Window::everything(1), None)
        .await
        .expect("bounded holder observations");
    assert_eq!(page.candidates.len(), 1);
    let c = &page.candidates[0];
    assert_eq!(c.cursor.tenant_id, f.w.admin.tenant_id());
    assert_eq!(c.cursor.group_id.as_uuid(), f.group);
    assert_eq!(c.cursor.member_id.as_uuid(), f.member);
    assert_eq!(c.namespace_id, f.w.namespace.id());
    assert_eq!(c.chain_id.as_uuid(), f.chain);
    assert_eq!(c.head_run_id.as_uuid(), f.chain);
    let revision: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(f.chain)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("actual run revision");
    assert_eq!(
        c.run_revision,
        u64::try_from(revision).expect("nonnegative run revision")
    );
    assert_eq!(
        (c.group_revision, c.group_epoch, c.chain_revision),
        (1, 1, 0)
    );
    assert_eq!(page.scanned_through, Some(c.cursor));
    assert!(!page.has_more);
}

#[tokio::test]
async fn nonholder_input_advances_the_cursor_and_a_new_store_resumes() {
    let f = fixture().await;
    let peer = super::super::seed_chain(&f.w, "holder-discovery-peer").await;
    let member = Uuid::nil(); // sorts strictly before the v4 holder: a skipped first input page
    let mut owner = f.w.db.pool.begin().await.expect("owner member fixture");
    sqlx::query("INSERT INTO capacity_group_members(tenant_id,group_id,member_id,namespace_id,member_kind,chain_id,state) VALUES($1,$2,$3,$4,'CHILD_ROOT',$5,'ACTIVE')")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).bind(member)
        .bind(f.w.namespace.id().as_uuid()).bind(peer.as_uuid())
        .execute(&mut *owner).await.expect("second member metadata only");
    sqlx::query(
        "UPDATE capacity_groups SET revision=2,active_members=2 WHERE tenant_id=$1 AND group_id=$2",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .execute(&mut *owner)
    .await
    .expect("retain original holder");
    owner.commit().await.expect("fixture commits");
    let runtime = f.w.db.runtime_pool_with(8).await;
    let store = PgRunStore::new(runtime.clone(), f.w.cache.clone());
    let first = store
        .discover_child_root_holders(&Window::everything(1), None)
        .await
        .expect("one scanned nonholder");
    assert!(first.candidates.is_empty());
    assert!(first.has_more);
    let checkpoint = first.scanned_through.expect("progress over skipped input");
    assert_eq!(checkpoint.member_id.as_uuid(), member);
    let restarted = PgRunStore::new(runtime, f.w.cache.clone());
    let next = restarted
        .discover_child_root_holders(&Window::everything(1), Some(checkpoint))
        .await
        .expect("restart retains checkpoint only");
    assert_eq!(next.candidates.len(), 1);
    assert_eq!(next.candidates[0].cursor.member_id.as_uuid(), f.member);
    assert!(!next.has_more);
}

#[tokio::test]
async fn discovery_does_not_wait_for_g_or_change_execution_and_capacity_rows() {
    let f = fixture().await;
    let runtime = f.w.db.runtime_pool_with(8).await;
    let store = PgRunStore::new(runtime, f.w.cache.clone());
    let before = super::super::execution_snapshot(&f).await;
    let versions = super::super::vacate::untouched(&f).await;
    let header = super::super::state(&f.w.db.pool, &f).await;
    let effects: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM audit_records),(SELECT count(*) FROM event_outbox),(SELECT count(*) FROM capacity_group_finish_receipts),(SELECT count(*) FROM capacity_group_transfer_receipts)")
        .fetch_one(&f.w.db.pool).await.expect("all audit/outbox/receipt baselines");
    let mut gate = f.w.db.pool.begin().await.expect("held G barrier");
    // Hold the canonical chain/run/G prefix, then every referenced tuple, unchanged.
    sqlx::query(
        "SELECT chain_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.chain)
    .execute(&mut *gate)
    .await
    .expect("chain held");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(f.w.admin.tenant_id().as_uuid())
        .bind(f.chain)
        .execute(&mut *gate)
        .await
        .expect("run held");
    sqlx::query(
        "SELECT group_id FROM capacity_groups WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.group)
    .execute(&mut *gate)
    .await
    .expect("G held unchanged");
    sqlx::query("SELECT member_id FROM capacity_group_members WHERE tenant_id=$1 AND group_id=$2 FOR UPDATE")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).execute(&mut *gate).await.expect("member held");
    sqlx::query("SELECT run_id FROM chain_segments WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE")
        .bind(f.w.admin.tenant_id().as_uuid())
        .bind(f.chain)
        .execute(&mut *gate)
        .await
        .expect("segment held");
    let page = crate::support::bounded::within(
        "holder observation while all joined tuples are held",
        std::time::Duration::from_secs(2),
        store.discover_child_root_holders(&Window::everything(1), None),
    )
    .await
    .expect("holder query");
    assert_eq!(page.candidates.len(), 1);
    assert_eq!(super::super::execution_snapshot(&f).await, before);
    assert_eq!(super::super::vacate::untouched(&f).await, versions);
    assert_eq!(super::super::state(&f.w.db.pool, &f).await, header);
    let after: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM audit_records),(SELECT count(*) FROM event_outbox),(SELECT count(*) FROM capacity_group_finish_receipts),(SELECT count(*) FROM capacity_group_transfer_receipts)")
        .fetch_one(&f.w.db.pool).await.expect("audit/outbox/receipts after observation");
    assert_eq!(after, effects);
    gate.rollback().await.expect("barrier ends");
}

#[tokio::test]
async fn tenant_exclusion_filters_after_the_input_page() {
    use sagat_domain::sharding::{Shard, ShardSet};
    let f = fixture().await;
    let store = PgRunStore::new(f.w.db.runtime_pool_with(8).await, f.w.cache.clone());
    let mut window = Window::everything(1);
    window.exclude.push(f.w.admin.tenant_id());
    let page = store
        .discover_child_root_holders(&window, None)
        .await
        .expect("excluded tenant");
    assert!(page.candidates.is_empty());
    assert_eq!(
        page.scanned_through
            .expect("excluded membership still consumed")
            .member_id
            .as_uuid(),
        f.member
    );
    assert!(!page.has_more);
    window.exclude.clear();
    window.shards = ShardSet::empty();
    let unowned = store
        .discover_child_root_holders(&window, None)
        .await
        .expect("unowned shard");
    assert!(unowned.candidates.is_empty());
    assert_eq!(unowned.scanned_through, page.scanned_through);
    window.shards.insert(Shard::of(f.chain));
    let owned = store
        .discover_child_root_holders(&window, None)
        .await
        .expect("exact own head shard");
    assert_eq!(owned.candidates.len(), 1);
}

#[tokio::test]
async fn invalid_holder_windows_are_typed_refusals() {
    let f = fixture().await;
    let store = PgRunStore::new(f.w.db.runtime_pool_with(8).await, f.w.cache.clone());
    for limit in [0, 257, u32::MAX] {
        assert_eq!(
            store
                .discover_child_root_holders(&Window::everything(limit), None)
                .await,
            Err(CapacityDiscoveryError::InvalidWindow)
        );
    }
    let mut window = Window::everything(1);
    window.exclude = vec![f.w.admin.tenant_id(); 257];
    assert_eq!(
        store.discover_child_root_holders(&window, None).await,
        Err(CapacityDiscoveryError::InvalidWindow)
    );
}

#[tokio::test]
async fn paused_holder_remains_an_observation_with_its_new_revision() {
    use sagat_application::runs::RunOperation;
    use sagat_domain::execution::RunId;
    let f = fixture().await;
    f.w.operate(RunId::new(f.chain), RunOperation::Pause)
        .await
        .expect("public pause");
    let (paused, revision): (bool, i64) =
        sqlx::query_as("SELECT paused,run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(f.w.admin.tenant_id().as_uuid())
            .bind(f.chain)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("paused head");
    assert!(paused);
    let store = PgRunStore::new(f.w.db.runtime_pool_with(8).await, f.w.cache.clone());
    let page = store
        .discover_child_root_holders(&Window::everything(1), None)
        .await
        .expect("readiness is not discovery");
    assert_eq!(page.candidates.len(), 1);
    assert_eq!(
        page.candidates[0].run_revision,
        u64::try_from(revision).expect("nonnegative revision")
    );
}

#[tokio::test]
async fn vacancy_is_filtered_and_finished_history_is_not_input() {
    let f = fixture().await;
    let store = PgRunStore::new(f.w.db.runtime_pool_with(8).await, f.w.cache.clone());
    super::vacant(&f).await; // dormant owner metadata only
    let vacant = store
        .discover_child_root_holders(&Window::everything(1), None)
        .await
        .expect("vacant is not held");
    assert!(vacant.candidates.is_empty());
    assert!(vacant.scanned_through.is_some());
    let mut owner = f.w.db.pool.begin().await.expect("owner retirement fixture");
    sqlx::query("UPDATE capacity_group_members SET state='FINISHED',finished_at=clock_timestamp() WHERE tenant_id=$1 AND group_id=$2")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).execute(&mut *owner).await.expect("retained member");
    sqlx::query("UPDATE capacity_groups SET possession='RETIRED',active_members=0,revision=3,epoch=3 WHERE tenant_id=$1 AND group_id=$2")
        .bind(f.w.admin.tenant_id().as_uuid()).bind(f.group).execute(&mut *owner).await.expect("retired metadata");
    owner.commit().await.expect("retired fixture commits");
    let retired = store
        .discover_child_root_holders(&Window::everything(1), None)
        .await
        .expect("history excluded before joins");
    assert!(retired.candidates.is_empty());
    assert!(retired.scanned_through.is_none());
    assert!(!retired.has_more);
}

#[tokio::test]
async fn holder_span_has_closed_kind_and_bounded_counts_without_resource_ids() {
    let f = fixture().await;
    let store = PgRunStore::new(f.w.db.runtime_pool_with(8).await, f.w.cache.clone());
    let captured = crate::support::logs::Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    let mut excluded = Window::everything(1);
    excluded.exclude.push(f.w.admin.tenant_id());
    let filtered = store
        .discover_child_root_holders(&excluded, None)
        .await
        .expect("traced filtered read");
    assert!(filtered.candidates.is_empty());
    let filtered_text = captured.text();
    let filtered_close: Vec<_> = filtered_text
        .lines()
        .filter(|line| {
            line.contains("capacity_holder_discovery")
                && line.contains("close")
                && line.contains("time.busy=")
                && line.contains("time.idle=")
        })
        .collect();
    assert_eq!(
        filtered_close.len(),
        1,
        "one actual close event: {filtered_text}"
    );
    assert!(filtered_close[0].contains("scanned=1") && filtered_close[0].contains("selected=0"));
    let page = store
        .discover_child_root_holders(&Window::everything(1), None)
        .await
        .expect("traced holder read");
    assert_eq!(page.candidates.len(), 1);
    let text = captured.text();
    let closed: Vec<_> = text
        .lines()
        .filter(|line| {
            line.contains("capacity_holder_discovery")
                && line.contains("close")
                && line.contains("time.busy=")
                && line.contains("time.idle=")
        })
        .collect();
    assert_eq!(closed.len(), 2, "both calls must close spans: {text}");
    assert!(closed[1].contains("scanned=1") && closed[1].contains("selected=1"));
    for id in [
        f.w.admin.tenant_id().to_string(),
        f.w.namespace.id().to_string(),
        f.chain.to_string(),
        f.group.to_string(),
        f.member.to_string(),
    ] {
        assert!(
            !text.contains(&id),
            "holder reader logs no high-cardinality identity"
        );
    }
}

#[tokio::test]
async fn actual_continuation_changes_the_observed_head_and_its_requested_shard() {
    crate::support::bounded::within(
        "holder observation after actual continuation",
        crate::support::bounded::LIFECYCLE_LIMIT,
        async {
            use sagat_domain::sharding::{Shard, ShardSet};
            let n = super::super::worker_leases::guarded_fixture_with_continuation(true).await;
            let original = super::super::complete_native_activity(&n).await;
            let round = n.scheduler.tick().await;
            assert_eq!(round.continuations_advanced, 1, "{round:?}");
            assert_eq!(round.failed, 0);
            let store = PgRunStore::new(n.f.w.execution_pool.clone(), n.f.w.cache.clone());
            let all = store
                .discover_child_root_holders(&Window::everything(1), None)
                .await
                .expect("current holder");
            assert_eq!(all.candidates.len(), 1);
            let c = &all.candidates[0];
            assert_ne!(c.head_run_id, original);
            assert_eq!(c.chain_id, original);
            let mut window = Window::everything(1);
            window.shards = ShardSet::empty();
            window.shards.insert(Shard::of(c.head_run_id.as_uuid()));
            let selected = store
                .discover_child_root_holders(&window, None)
                .await
                .expect("current-head shard");
            assert_eq!(selected.candidates, all.candidates);
            // Real ids can share one of64 shards. Deterministic distinct-id selection is additionally
            // required by the selector's unit truth table when the shared GREEN filter is introduced.
            if Shard::of(c.chain_id.as_uuid()) != Shard::of(c.head_run_id.as_uuid()) {
                window.shards = ShardSet::empty();
                window.shards.insert(Shard::of(c.chain_id.as_uuid()));
                assert!(
                    store
                        .discover_child_root_holders(&window, None)
                        .await
                        .expect("old shard only")
                        .candidates
                        .is_empty()
                );
            }
        },
    )
    .await;
}
