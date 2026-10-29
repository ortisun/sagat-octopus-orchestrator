//! O701: zero stops fresh admission while accepted work retains its right to finish.
use crate::support::scope_flow::{Flow, collection_document};
use serde_json::json;

#[tokio::test]
async fn an_explicit_zero_policy_finishes_accepted_timers_and_blocks_new_members() {
    let name = "zero-policy-accepted-timer";
    let flow = Flow::start(
        name,
        collection_document(
            name,
            vec![
                json!({"id":"delay","type":"timer","durationMs":60000,"next":"returned"}),
                json!({"id":"returned","type":"return","result":"item"}),
            ],
            1,
        ),
        json!([1, 2]),
    )
    .await;
    flow.drive().await;
    let response = flow
        .api
        .as_user(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit":0}))
        .send()
        .await
        .expect("explicit zero policy");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "O701 allows zero without pausing admitted work"
    );
    let quota: serde_json::Value = response.json().await.expect("quota view");
    assert_eq!(quota["limit"], 0);
    assert_eq!(quota["source"], "tenant");
    assert_eq!(quota["used"], 1);
    super::collection_timers::age(&flow).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "WAITING");
    assert!(flow.poll().await.is_empty());
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT count(*) FILTER (WHERE event_type='collection.iteration_admitted'),count(*) FILTER (WHERE event_type='timer.fired'),count(*) FILTER (WHERE event_type='collection.iteration_checkpointed') FROM run_events WHERE tenant_id=$1 AND run_id=$2")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("committed member progression");
    assert_eq!(
        counts,
        (1, 1, 1),
        "only the admitted member may progress at zero"
    );
    let active: i32 = sqlx::query_scalar(
        "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1",
    )
    .bind(flow.tenant.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("released slot");
    assert_eq!(active, 0);
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='quota.changed' AND reason='iteration_concurrency = 0'")
        .bind(flow.tenant.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("same-transaction quota audit");
    assert_eq!(audits, 1);
    let response = flow
        .api
        .as_user(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit":1}))
        .send()
        .await
        .expect("resume fresh admission");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    flow.drive().await;
    super::collection_timers::age(&flow).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}

#[tokio::test]
async fn zero_policy_schema_retains_its_value_until_a_safe_downgrade() {
    use sagat_infra_postgres::{MIGRATOR, migrate};
    use sqlx::{Connection, PgConnection};
    let flow = super::collection_timers::waiting_timer("zero-policy-schema").await;
    for value in [0_i32, 1, 1_000_000_000] {
        sqlx::query(
            "UPDATE tenant_collection_policies SET max_active_iterations=$2 WHERE tenant_id=$1",
        )
        .bind(flow.tenant.as_uuid())
        .bind(value)
        .execute(&flow.api.db.pool)
        .await
        .expect("valid stored policy boundary");
    }
    for value in [-1_i32, 1_000_000_001] {
        let error = sqlx::query(
            "UPDATE tenant_collection_policies SET max_active_iterations=$2 WHERE tenant_id=$1",
        )
        .bind(flow.tenant.as_uuid())
        .bind(value)
        .execute(&flow.api.db.pool)
        .await
        .expect_err("invalid stored policy boundary");
        assert_eq!(
            error
                .as_database_error()
                .expect("native CHECK")
                .code()
                .as_deref(),
            Some("23514")
        );
        assert_eq!(
            error
                .as_database_error()
                .expect("native CHECK")
                .constraint(),
            Some("tenant_collection_policies_limit")
        );
    }
    sqlx::query("UPDATE tenant_collection_policies SET max_active_iterations=0 WHERE tenant_id=$1")
        .bind(flow.tenant.as_uuid())
        .execute(&flow.api.db.pool)
        .await
        .expect("retained zero");
    let mut connection = PgConnection::connect(&flow.api.db.url)
        .await
        .expect("dedicated migration session");
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        MIGRATOR.undo(&mut connection, 20_261_221_000_000),
    )
    .await
    .expect("bounded refused downgrade")
    .expect_err("retained zero forbids downgrade");
    connection
        .close()
        .await
        .expect("release SQLx migrator's session advisory lock after refusal");
    let retained:(i32,bool)=sqlx::query_as("SELECT max_active_iterations,EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=20261222000000 AND success) FROM tenant_collection_policies WHERE tenant_id=$1")
        .bind(flow.tenant.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("unchanged policy and migration");
    assert_eq!(retained, (0, true));
    sqlx::query("UPDATE tenant_collection_policies SET max_active_iterations=1 WHERE tenant_id=$1")
        .bind(flow.tenant.as_uuid())
        .execute(&flow.api.db.pool)
        .await
        .expect("explicit positive replacement");
    MIGRATOR
        .undo(&flow.api.db.pool, 20_261_221_000_000)
        .await
        .expect("positive policy reverses safely");
    let error = sqlx::query(
        "UPDATE tenant_collection_policies SET max_active_iterations=0 WHERE tenant_id=$1",
    )
    .bind(flow.tenant.as_uuid())
    .execute(&flow.api.db.pool)
    .await
    .expect_err("old positive-only CHECK restored");
    assert_eq!(
        error
            .as_database_error()
            .expect("old CHECK")
            .code()
            .as_deref(),
        Some("23514")
    );
    migrate(&flow.api.db.pool)
        .await
        .expect("reapply additive migration");
    sqlx::query("UPDATE tenant_collection_policies SET max_active_iterations=0 WHERE tenant_id=$1")
        .bind(flow.tenant.as_uuid())
        .execute(&flow.api.db.pool)
        .await
        .expect("zero accepted again after up");
    super::collection_timers::age(&flow).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}

#[tokio::test]
async fn zero_policy_inserts_without_a_row_and_clear_restores_the_distinct_default() {
    use sagat_application::primitives::{CollectionStore, PrimitiveStoreError};
    use sagat_domain::{execution::ExecutionInstanceId, workflow::NodeId};
    use sagat_scheduler::CollectionAdvance;
    let name = "zero-policy-insert";
    let flow=Flow::start(name,collection_document(name,vec![
        json!({"id":"choose","type":"condition","expression":"item > 0","ifTrue":"returned","ifFalse":"returned"}),
        json!({"id":"returned","type":"return","result":"item"}),
    ],1),json!([1])).await;
    let cleared = flow
        .api
        .as_user(
            reqwest::Method::DELETE,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("clear initial positive fixture policy");
    assert_eq!(cleared.status(), reqwest::StatusCode::OK);
    let default: serde_json::Value = cleared.json().await.expect("default quota");
    assert_eq!(
        (default["limit"].clone(), default["source"].clone()),
        (json!(0), json!("default"))
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_collection_policies WHERE tenant_id=$1")
            .bind(flow.tenant.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("no policy row before first zero write");
    assert_eq!(count, 0);
    let configured = flow
        .api
        .as_user(
            reqwest::Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit":0}))
        .send()
        .await
        .expect("INSERT zero policy");
    assert_eq!(configured.status(), reqwest::StatusCode::OK);
    let configured: serde_json::Value = configured.json().await.expect("tenant quota");
    assert_eq!(
        (configured["limit"].clone(), configured["source"].clone()),
        (json!(0), json!("tenant"))
    );
    let stored: i32 = sqlx::query_scalar(
        "SELECT max_active_iterations FROM tenant_collection_policies WHERE tenant_id=$1",
    )
    .bind(flow.tenant.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("explicit zero row exists");
    assert_eq!(stored, 0);
    let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
    assert_eq!(
        flow.scheduler
            .advance_collection(flow.tenant, flow.run, &owner)
            .await
            .expect("capture scope without admitting a member"),
        CollectionAdvance::Entered
    );
    let command = super::collection_control_admission::admission(&flow, 0).await;
    let before = super::collection_finish_atomic::snapshot(
        &flow.api.db.pool,
        flow.tenant.as_uuid(),
        flow.run,
    )
    .await;
    assert_eq!(
        flow.api.executions.admit_collection_member(&command).await,
        Err(PrimitiveStoreError::AtCapacity)
    );
    assert_eq!(
        super::collection_finish_atomic::snapshot(
            &flow.api.db.pool,
            flow.tenant.as_uuid(),
            flow.run
        )
        .await,
        before
    );
    let cleared = flow
        .api
        .as_user(
            reqwest::Method::DELETE,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("clear explicit zero");
    assert_eq!(cleared.status(), reqwest::StatusCode::OK);
    let default: serde_json::Value = cleared.json().await.expect("default again");
    assert_eq!(
        (default["limit"].clone(), default["source"].clone()),
        (json!(0), json!("default"))
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM tenant_collection_policies WHERE tenant_id=$1")
            .bind(flow.tenant.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("clear removes row");
    assert_eq!(count, 0);
    assert_eq!(
        flow.api.executions.admit_collection_member(&command).await,
        Err(PrimitiveStoreError::PolicyMissing)
    );
    flow.verify().await;
}

async fn observe_policy_lock(
    pool: &sqlx::PgPool,
    waiter: (i32, time::OffsetDateTime),
    blocker: (i32, time::OffsetDateTime),
) {
    tokio::time::timeout(std::time::Duration::from_secs(15),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity w JOIN pg_stat_activity b ON b.pid=$3 AND b.backend_start=$4 WHERE w.pid=$1 AND w.backend_start=$2 AND w.datname=current_database() AND w.wait_event_type='Lock' AND $3=ANY(pg_blocking_pids(w.pid)))")
                .bind(waiter.0).bind(waiter.1).bind(blocker.0).bind(blocker.1).fetch_one(pool).await.expect("exact policy relation lock edge");
            if waiting {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("policy writer/downgrade reached the actual held lock");
}

#[tokio::test]
async fn zero_policy_writer_and_downgrade_obey_both_relation_lock_orders() {
    use sagat_infra_postgres::{MIGRATOR, migrate};
    use sqlx::{Connection, PgConnection};
    let _server = crate::support::hold_server();
    for writer_first in [true, false] {
        let flow = super::collection_timers::waiting_timer(&format!(
            "zero-policy-down-order-{writer_first}"
        ))
        .await;
        let runtime = flow.api.db.runtime_pool().await;
        let mut writer = runtime.acquire().await.expect("runtime policy writer");
        let role: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&mut *writer)
            .await
            .expect("writer role");
        assert_eq!(role, "sagat_app");
        let writer_id:(i32,time::OffsetDateTime)=sqlx::query_as("SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()")
            .fetch_one(&mut *writer).await.expect("live runtime writer identity");
        let mut down = PgConnection::connect(&flow.api.db.url)
            .await
            .expect("dedicated migrator");
        let down_id:(i32,time::OffsetDateTime)=sqlx::query_as("SELECT pg_backend_pid(),backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid()")
            .fetch_one(&mut down).await.expect("live migrator identity");
        let tenant = flow.tenant.as_uuid();
        if writer_first {
            let mut writer_tx = writer.begin().await.expect("writer transaction");
            sqlx::query(
                "UPDATE tenant_collection_policies SET max_active_iterations=0 WHERE tenant_id=$1",
            )
            .bind(tenant)
            .execute(&mut *writer_tx)
            .await
            .expect("zero prepared under runtime relation lock");
            let migration = async move {
                let result = MIGRATOR.undo(&mut down, 20_261_221_000_000).await;
                down.close()
                    .await
                    .expect("release advisory lock even after refused migration");
                result
            };
            let release = async {
                observe_policy_lock(&flow.api.db.pool, down_id, writer_id).await;
                writer_tx.commit().await.expect("zero policy commits first");
            };
            let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
                tokio::join!(migration, release)
            })
            .await
            .expect("bounded refused migration and writer release");
            let error = result.expect_err("committed zero prevents down");
            assert!(
                error
                    .to_string()
                    .contains("an explicit zero iteration policy is retained"),
                "expected retention guard, got {error}"
            );
            let retained: i32 = sqlx::query_scalar(
                "SELECT max_active_iterations FROM tenant_collection_policies WHERE tenant_id=$1",
            )
            .bind(tenant)
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("zero not changed by refused down");
            assert_eq!(retained, 0);
            sqlx::query(
                "UPDATE tenant_collection_policies SET max_active_iterations=1 WHERE tenant_id=$1",
            )
            .bind(tenant)
            .execute(&mut *writer)
            .await
            .expect("restore positive fixture policy");
        } else {
            let mut down_tx = down.begin().await.expect("hold actual downgrade prefix");
            sqlx::query("LOCK TABLE tenant_collection_policies IN ACCESS EXCLUSIVE MODE")
                .execute(&mut *down_tx)
                .await
                .expect("migration's actual relation lock");
            let write = tokio::spawn(async move {
                sqlx::query("UPDATE tenant_collection_policies SET max_active_iterations=0 WHERE tenant_id=$1")
                    .bind(tenant).execute(&mut *writer).await
            });
            observe_policy_lock(&flow.api.db.pool, writer_id, down_id).await;
            MIGRATOR
                .undo(&mut *down_tx, 20_261_221_000_000)
                .await
                .expect("actual down while new zero waits");
            down_tx
                .commit()
                .await
                .expect("positive-only schema commits first");
            down.close().await.expect("close successful migrator");
            let error = tokio::time::timeout(std::time::Duration::from_secs(15), write)
                .await
                .expect("bounded writer result")
                .expect("writer task")
                .expect_err("restored CHECK rejects late zero");
            assert_eq!(
                error
                    .as_database_error()
                    .expect("native restored constraint")
                    .code()
                    .as_deref(),
                Some("23514")
            );
            assert_eq!(
                error
                    .as_database_error()
                    .expect("native restored constraint")
                    .constraint(),
                Some("tenant_collection_policies_limit")
            );
            migrate(&flow.api.db.pool)
                .await
                .expect("schema-first upgrade before zero-capable writers resume");
        }
        super::collection_timers::age(&flow).await;
        flow.drive().await;
        assert_eq!(flow.run_status().await, "SUCCEEDED");
        flow.verify().await;
    }
}
