//! PostgreSQL reader defenses for the dormant collection `RUN_DATA_LIMIT` drain shape.
//!
//! The member prefix is produced only through the public scheduler and worker paths. Until the
//! native producer is enabled, this suite writes the new drain projection directly and labels it
//! as a storage fixture; it does not manufacture journal or source history.

use sagat_application::error::StoreError;
use sagat_application::execution::{PathExecutionStore, PathExecutionTx, RunTx};
use sagat_application::primitives::{
    CollectionDrainFinish, CollectionScopeId, CollectionStore, CollectionTransition,
    PrimitiveStoreError, SchedulerAuthority,
};
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::execution::{
    AncestorExecutionFence, CollectionDrainReason, CollectionPhase, ExecutionInstanceId,
    InstancePath, RunId,
};
use sagat_domain::identity::Subject;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::NodeId;
use sagat_infra_postgres::PgCollectionStore;
use serde_json::json;
use sqlx::Executor;
use uuid::Uuid;

use crate::byte_ceilings::{answer, collection};
use crate::limits::{allow_iterations, register_scoped, scheduler, scoped_workers};
use crate::support::engine::{World, world};
use crate::support::now;

struct Fixture {
    world: World,
    run: RunId,
    scope_id: Uuid,
    version: u64,
    owner: ExecutionInstanceId,
    member_path: InstancePath,
    scheduler_owner: String,
    scheduler_generation: u64,
}

#[allow(clippy::type_complexity)] // One fixture query verifies the complete public prefix.
async fn genuine_checkpointed_fixture(label: &str) -> Fixture {
    let world = world().await;
    allow_iterations(&world, 2).await;
    let (workflow, _) = world
        .publish(collection(label, "input.items", 1, 1, "bodyResult"))
        .await;
    let run = world.start(workflow, &json!({"items":[1]})).await;
    let scheduler = scheduler(&world, label).await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;

    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(answer(&world, &workers, &session, "1").await, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);

    let tenant = world.admin.tenant_id().as_uuid();
    let row: (
        Uuid,
        i64,
        String,
        i32,
        i32,
        i32,
        String,
        bool,
        String,
        Option<String>,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT scope.scope_id,run.run_version,scope.lifecycle,scope.admitted_count, \
                scope.completed_count,scope.active_count,member.lifecycle,member.slot_held, \
                owner.status,owner.waiting_kind, \
                (SELECT count(*) FROM iteration_checkpoints checkpoint \
                  WHERE checkpoint.tenant_id=run.tenant_id AND checkpoint.run_id=run.id \
                    AND checkpoint.scope_id=scope.scope_id), \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.instance_path_nodes=ARRAY['items']::text[] \
                  AND event.instance_path_indices=ARRAY[0]::integer[] \
                  AND event.node='work' AND event.event_type='step.leased'), \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id AND event.instance_path_nodes=ARRAY['items']::text[] \
                  AND event.instance_path_indices=ARRAY[0]::integer[] \
                  AND event.node='work' AND event.event_type='step.completed') \
           FROM runs run \
           JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
           JOIN collection_iterations member ON member.tenant_id=scope.tenant_id \
             AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id \
             AND member.iteration_index=0 \
           JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
             AND owner.instance_path_nodes=scope.owner_path_nodes \
             AND owner.instance_path_indices=scope.owner_path_indices \
             AND owner.node=scope.owner_node \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("public checkpointed collection prefix");
    assert_eq!(
        (&*row.2, row.3, row.4, row.5, &*row.6, row.7),
        ("OPEN", 1, 1, 0, "CHECKPOINTED", false)
    );
    assert_eq!((&*row.8, row.9.as_deref()), ("WAITING", Some("COLLECTION")));
    assert_eq!((row.10, row.11, row.12), (1, 1, 1));

    let shard = Shard::of(run.as_uuid());
    let (scheduler_owner, scheduler_generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=$1")
            .bind(i16::from(shard.index()))
            .fetch_one(&world.db.pool)
            .await
            .expect("scheduler authority for fixture run");
    Fixture {
        world,
        run,
        scope_id: row.0,
        version: u64::try_from(row.1).expect("positive run version"),
        owner: ExecutionInstanceId::root(NodeId::parse("items").expect("owner node")),
        member_path: InstancePath::root()
            .child(NodeId::parse("items").expect("owner node"), 0)
            .expect("member path"),
        scheduler_owner,
        scheduler_generation: u64::try_from(scheduler_generation)
            .expect("positive scheduler generation"),
    }
}

async fn project_static_draining(fixture: &Fixture) {
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    sqlx::query(
        "UPDATE runs SET status='CANCELLING',stop_reason='FAILED', \
                stop_code='RUN_DATA_LIMIT',finished_at=NULL \
          WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("static cancelling run projection");
    sqlx::query(
        "UPDATE iteration_scopes SET lifecycle='DRAINING',result_digest=NULL, \
                drain_kind='RUN_DATA_LIMIT',drain_iteration=NULL, \
                drain_failure_code='RUN_DATA_LIMIT',drain_sequence=$4, \
                drain_outbox_id=$5,finish_sequence=NULL,finish_outbox_id=NULL \
          WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .bind(fixture.scope_id)
    .bind(i64::try_from(fixture.version).expect("stored run version"))
    .bind(Uuid::now_v7())
    .execute(&fixture.world.db.pool)
    .await
    .expect("static data-limit draining projection");
    sqlx::query(
        "UPDATE steps SET status='WAITING',waiting_kind='COLLECTION',failure_code=NULL \
          WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes='{}' \
            AND instance_path_indices='{}' AND node='items'",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("static draining owner projection");
}

async fn project_static_failed(fixture: &Fixture) {
    project_static_draining(fixture).await;
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    sqlx::query(
        "UPDATE iteration_scopes SET lifecycle='FAILED',finish_sequence=$4,finish_outbox_id=$5 \
          WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .bind(fixture.scope_id)
    .bind(i64::try_from(fixture.version).expect("stored run version"))
    .bind(Uuid::now_v7())
    .execute(&fixture.world.db.pool)
    .await
    .expect("static data-limit failed scope projection");
    sqlx::query(
        "UPDATE steps SET status='FAILED',waiting_kind=NULL,failure_code='RUN_DATA_LIMIT' \
          WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes='{}' \
            AND instance_path_indices='{}' AND node='items'",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("static failed owner projection");
    sqlx::query(
        "UPDATE runs SET status='FAILED',finished_at=clock_timestamp() \
          WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant)
    .bind(fixture.run.as_uuid())
    .execute(&fixture.world.db.pool)
    .await
    .expect("static failed run projection");
}

async fn assert_full_and_descendant_restore(fixture: &Fixture, expected: CollectionPhase) {
    let tenant = fixture.world.admin.tenant_id();
    let mut full = fixture
        .world
        .store
        .begin(tenant, fixture.run, Lock::Wait)
        .await
        .expect("full run reader accepts data-limit projection")
        .expect("run exists");
    let scope = full
        .parts()
        .1
        .collection(&fixture.owner)
        .expect("restored root collection");
    assert_eq!(scope.phase(), expected);
    assert_eq!(
        scope.drain_reason(),
        Some(&CollectionDrainReason::RunDataLimit)
    );
    drop(full);

    let mut path = fixture
        .world
        .store
        .begin_current_path(tenant, fixture.run, fixture.member_path.clone(), Lock::Wait)
        .await
        .expect("descendant reader accepts data-limit ancestor")
        .expect("member path exists");
    match path.snapshot().state.ancestors() {
        [
            AncestorExecutionFence::Collection {
                owner,
                iteration_index,
                phase,
                active,
            },
        ] => {
            assert_eq!(owner, &fixture.owner);
            assert_eq!(*iteration_index, 0);
            assert_eq!(*phase, expected);
            assert!(!active);
        }
        ancestors => panic!("unexpected ancestor fence: {ancestors:?}"),
    }
}

#[tokio::test]
async fn static_data_limit_draining_restores_in_full_and_descendant_readers() {
    let fixture = genuine_checkpointed_fixture("data-limit-reader-draining").await;
    project_static_draining(&fixture).await;
    assert_full_and_descendant_restore(&fixture, CollectionPhase::Draining).await;
}

#[tokio::test]
async fn static_data_limit_failed_restores_in_full_and_descendant_readers() {
    let fixture = genuine_checkpointed_fixture("data-limit-reader-failed").await;
    project_static_failed(&fixture).await;
    assert_full_and_descendant_restore(&fixture, CollectionPhase::Failed).await;
}

async fn drop_scope_shape_constraints(fixture: &Fixture) {
    // This world is private to one test. Commit the DDL before opening a reader transaction: an
    // uncommitted ALTER plus another pooled read would wait on its own AccessExclusiveLock.
    fixture
        .world
        .db
        .pool
        .execute(
            "ALTER TABLE iteration_scopes \
               DROP CONSTRAINT iteration_scopes_drain_reason, \
               DROP CONSTRAINT iteration_scopes_terminal_result",
        )
        .await
        .expect("remove constraints only in isolated corruption fixture");
}

#[derive(Debug, Clone, Copy)]
enum Corruption {
    NullSequence,
    NullCode,
    NonPositiveSequence,
    MissingOutbox,
    NonNullIteration,
    WrongCode,
    ResultOnDrain,
    WrongPhase,
    FailedOwner,
    WrongFailedOwnerCode,
}

impl Corruption {
    const ALL: [Self; 10] = [
        Self::NullSequence,
        Self::NullCode,
        Self::NonPositiveSequence,
        Self::MissingOutbox,
        Self::NonNullIteration,
        Self::WrongCode,
        Self::ResultOnDrain,
        Self::WrongPhase,
        Self::FailedOwner,
        Self::WrongFailedOwnerCode,
    ];

    const fn full_detail(self) -> &'static str {
        match self {
            Self::NullSequence
            | Self::NullCode
            | Self::NonPositiveSequence
            | Self::MissingOutbox
            | Self::NonNullIteration
            | Self::WrongCode => "collection drain projection",
            Self::ResultOnDrain | Self::WrongPhase => "unsupported collection scope projection",
            Self::FailedOwner | Self::WrongFailedOwnerCode => "collection projection",
        }
    }
}

async fn reset_unconstrained_draining(fixture: &Fixture) {
    project_static_draining(fixture).await;
}

async fn apply_corruption(fixture: &Fixture, corruption: Corruption) {
    let tenant = fixture.world.admin.tenant_id().as_uuid();
    let scope_sql = match corruption {
        Corruption::NullSequence => Some(
            "UPDATE iteration_scopes SET drain_sequence=NULL \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::NullCode => Some(
            "UPDATE iteration_scopes SET drain_failure_code=NULL \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::NonPositiveSequence => Some(
            "UPDATE iteration_scopes SET drain_sequence=0 \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::MissingOutbox => Some(
            "UPDATE iteration_scopes SET drain_outbox_id=NULL \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::NonNullIteration => Some(
            "UPDATE iteration_scopes SET drain_iteration=0 \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::WrongCode => Some(
            "UPDATE iteration_scopes SET drain_failure_code='EXPRESSION_ERROR' \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::ResultOnDrain => Some(
            "UPDATE iteration_scopes SET result_digest=snapshot_digest \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::WrongPhase => Some(
            "UPDATE iteration_scopes SET lifecycle='OPEN' \
              WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3",
        ),
        Corruption::FailedOwner | Corruption::WrongFailedOwnerCode => None,
    };
    if let Some(sql) = scope_sql {
        sqlx::query(sql)
            .bind(tenant)
            .bind(fixture.run.as_uuid())
            .bind(fixture.scope_id)
            .execute(&fixture.world.db.pool)
            .await
            .expect("commit one malformed scope field");
    } else {
        let failure_code = if matches!(corruption, Corruption::WrongFailedOwnerCode) {
            "EXPRESSION_ERROR"
        } else {
            "RUN_DATA_LIMIT"
        };
        sqlx::query(
            "UPDATE steps SET status='FAILED',waiting_kind=NULL,failure_code=$3 \
              WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes='{}' \
                AND instance_path_indices='{}' AND node='items'",
        )
        .bind(tenant)
        .bind(fixture.run.as_uuid())
        .bind(failure_code)
        .execute(&fixture.world.db.pool)
        .await
        .expect("commit malformed owner status or failure code");
    }
}

async fn assert_full_corruption(fixture: &Fixture, detail: &'static str) {
    match fixture
        .world
        .store
        .begin(fixture.world.admin.tenant_id(), fixture.run, Lock::Wait)
        .await
    {
        Err(StoreError::Internal { detail: actual }) => {
            assert_eq!(
                actual,
                format!("stored {detail} violates a domain invariant")
            );
        }
        Err(error) => panic!("wrong full-reader error: {error:?}"),
        Ok(_) => panic!("full reader accepted malformed data-limit projection"),
    }
}

async fn assert_descendant_corruption(fixture: &Fixture) {
    match fixture
        .world
        .store
        .begin_current_path(
            fixture.world.admin.tenant_id(),
            fixture.run,
            fixture.member_path.clone(),
            Lock::Wait,
        )
        .await
    {
        Err(StoreError::Internal { detail }) => {
            assert_eq!(
                detail,
                "stored collection data-limit ancestor projection violates a domain invariant"
            )
        }
        Err(error) => panic!("wrong descendant-reader error: {error:?}"),
        Ok(_) => panic!("descendant reader accepted malformed data-limit ancestor"),
    }
}

#[tokio::test]
async fn malformed_static_data_limit_shapes_are_typed_corruption_in_both_readers() {
    let fixture = genuine_checkpointed_fixture("data-limit-reader-corruption").await;
    project_static_draining(&fixture).await;
    drop_scope_shape_constraints(&fixture).await;

    for corruption in Corruption::ALL {
        if matches!(corruption, Corruption::WrongFailedOwnerCode) {
            project_static_failed(&fixture).await;
        } else {
            reset_unconstrained_draining(&fixture).await;
        }
        apply_corruption(&fixture, corruption).await;
        assert_full_corruption(&fixture, corruption.full_detail()).await;
        assert_descendant_corruption(&fixture).await;
    }
}

async fn write_stamp(
    fixture: &Fixture,
) -> (i64, String, String, Option<i64>, String, i64, i64, i64) {
    sqlx::query_as(
        "SELECT run.run_version,run.status,scope.lifecycle,scope.finish_sequence,owner.status, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
                  AND event.run_id=run.id), \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=run.tenant_id \
                  AND message.aggregate_id=run.id), \
                (SELECT count(*) FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                  AND audit.resource_type='run' AND audit.resource_id=run.id::text) \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id AND scope.scope_id=$3 \
           JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
             AND owner.instance_path_nodes=scope.owner_path_nodes \
             AND owner.instance_path_indices=scope.owner_path_indices AND owner.node=scope.owner_node \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(fixture.world.admin.tenant_id().as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(fixture.scope_id)
    .fetch_one(&fixture.world.db.pool)
    .await
    .expect("collection write stamp")
}

#[tokio::test]
async fn native_finish_guard_rejects_data_limit_iteration_before_writes() {
    let fixture = genuine_checkpointed_fixture("data-limit-finish-guard").await;
    project_static_draining(&fixture).await;
    drop_scope_shape_constraints(&fixture).await;
    apply_corruption(&fixture, Corruption::NonNullIteration).await;
    let before = write_stamp(&fixture).await;
    let transition = CollectionTransition::new(
        fixture.world.admin.tenant_id(),
        fixture.world.namespace.id(),
        fixture.run,
        fixture.version,
        fixture.owner.clone(),
        SchedulerAuthority::for_run(
            fixture.scheduler_owner.clone(),
            fixture.run,
            fixture.scheduler_generation,
        ),
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        now(),
        None,
    );
    let finish = CollectionDrainFinish::new(
        transition,
        CollectionScopeId::new(fixture.scope_id),
        Uuid::now_v7(),
    );
    let result = PgCollectionStore::new(fixture.world.execution_pool.clone())
        .finish_collection_drain(&finish)
        .await;
    assert!(
        matches!(
            result,
            Err(PrimitiveStoreError::Storage(StoreError::Internal { ref detail }))
                if detail == "collection drain reason"
        ),
        "unexpected finish result: {result:?}"
    );
    assert_eq!(
        write_stamp(&fixture).await,
        before,
        "guard must precede writes"
    );
}
