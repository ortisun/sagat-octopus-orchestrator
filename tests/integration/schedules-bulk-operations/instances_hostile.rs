//! A process that holds the runtime role's credentials and knows another instance's id (it can read every id) must not be
//! able to touch that instance's row (`T018-080`, review B). Each process keeps a random token to itself; the registry
//! stores its hash, which the runtime role cannot read, and touches a row only for the token that matches. These were
//! first written against the functions as they were (id only) and failed; they run in both roles.
//!
//! What a holder of the runtime role can still do (fill the table with rows of its own, hold the registration lock open)
//! is gap G2000 and is not tested as closed.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::instances::{
    Capability, InstanceKind, InstanceStore, InstanceToken, Registered,
};
use sqlx::Row;
use uuid::Uuid;

use crate::support::instances::{Registry, registration_with, registry};

const ENGINE: InstanceKind = InstanceKind::ExecutionEngine;

fn token(byte: u8) -> InstanceToken {
    InstanceToken::from_bytes([byte; 32])
}

async fn last_seen(r: &Registry, instance: Uuid) -> time::OffsetDateTime {
    sqlx::query_scalar("SELECT last_seen FROM serving_instances WHERE instance_id = $1")
        .bind(instance)
        .fetch_one(&r.db.pool)
        .await
        .expect("last_seen")
}

/// A victim that registered with a token of its own, and an attacker's token that is not it.
async fn victim(r: &Registry) -> (Uuid, InstanceToken, InstanceToken) {
    let (mine, theirs) = (token(1), token(2));
    let instance = Uuid::new_v4();
    assert_eq!(
        r.process
            .register(&registration_with(instance, &mine, ENGINE, &[]))
            .await
            .expect("register"),
        Registered::Serving { epoch: 0 }
    );
    r.age(instance, 20).await;
    (instance, mine, theirs)
}

#[tokio::test]
async fn another_process_cannot_refresh_the_row_of_an_instance_and_a_dead_one_stays_dead() {
    let r = registry().await;
    let (victim, _, theirs) = victim(&r).await;
    let before = last_seen(&r, victim).await;
    let beat = r
        .process
        .heartbeat(victim, &theirs)
        .await
        .expect("a heartbeat for a token that does not match is a row not found");
    assert!(!beat.found, "the row is not theirs");
    assert_eq!(
        last_seen(&r, victim).await,
        before,
        "and it was not refreshed"
    );
    // the same instance id with no token at all in the way of the raw function
    let raw = sqlx::query("SELECT out_found FROM instance_heartbeat($1, NULL)")
        .bind(victim)
        .fetch_one(&r.process_pool)
        .await
        .expect("a call");
    assert!(!raw.get::<bool, _>("out_found"));
    assert_eq!(last_seen(&r, victim).await, before);
}

#[tokio::test]
async fn another_process_cannot_delete_the_row_of_a_live_instance() {
    let r = registry().await;
    let (victim, mine, theirs) = victim(&r).await;
    assert!(
        !r.process
            .deregister(victim, &theirs)
            .await
            .expect("a delete for a token that does not match is a row not found")
    );
    assert!(
        r.row_exists(victim).await,
        "the live instance is still counted"
    );
    assert!(
        r.process.deregister(victim, &mine).await.expect("its own"),
        "the owner can leave"
    );
}

#[tokio::test]
async fn another_process_cannot_rewrite_the_row_of_an_instance_through_the_upsert() {
    let r = registry().await;
    let (victim, mine, theirs) = victim(&r).await;
    // a stale binary claims schedules-v1 over the victim's row, so that an activation verifies falsely
    let attempt = r
        .process
        .register(&registration_with(
            victim,
            &theirs,
            ENGINE,
            &[Capability::SchedulesV1],
        ))
        .await;
    assert!(
        attempt.is_err(),
        "the id belongs to the process that registered it: {attempt:?}"
    );
    let capabilities: Vec<String> =
        sqlx::query_scalar("SELECT capabilities FROM serving_instances WHERE instance_id = $1")
            .bind(victim)
            .fetch_one(&r.db.pool)
            .await
            .expect("capabilities");
    assert!(
        capabilities.is_empty(),
        "the row was rewritten: {capabilities:?}"
    );
    assert!(
        r.require(Capability::SchedulesV1, &[ENGINE]).await.is_err(),
        "the activation sees what the instance really lists"
    );
    // the owner, with its token, registers again as it always could
    assert_eq!(
        r.process
            .register(&registration_with(victim, &mine, ENGINE, &[]))
            .await
            .expect("its own"),
        Registered::Serving { epoch: 0 }
    );
}

#[tokio::test]
async fn an_id_nobody_holds_is_not_found_and_a_heartbeat_creates_nothing() {
    let r = registry().await;
    let stranger = Uuid::new_v4();
    let beat = r
        .process
        .heartbeat(stranger, &token(3))
        .await
        .expect("heartbeat");
    assert!(!beat.found);
    assert!(!r.row_exists(stranger).await);
    assert!(
        !r.process
            .deregister(stranger, &token(3))
            .await
            .expect("leave")
    );
}

#[tokio::test]
async fn the_hash_of_the_token_is_not_readable_by_the_runtime_role_and_is_not_the_token() {
    let r = registry().await;
    let (victim, mine, _) = victim(&r).await;
    // the owner sees a hash, not the token
    let hash: Vec<u8> =
        sqlx::query_scalar("SELECT token_hash FROM serving_instances WHERE instance_id = $1")
            .bind(victim)
            .fetch_one(&r.db.pool)
            .await
            .expect("hash");
    assert_eq!(hash.len(), 32);
    assert_ne!(hash.as_slice(), mine.as_bytes().as_slice());
    // the runtime role reads every other column and not this one
    let runtime = r.db.runtime_pool().await;
    sqlx::query("SELECT instance_id, kind, scheduler_owner, binary_version, capabilities, started_at, last_seen FROM serving_instances")
        .fetch_all(&runtime)
        .await
        .expect("the columns it may read");
    for statement in [
        "SELECT token_hash FROM serving_instances",
        "SELECT * FROM serving_instances",
    ] {
        let refused = sqlx::query(statement).fetch_all(&runtime).await;
        match refused {
            Err(sqlx::Error::Database(e)) => {
                assert_eq!(e.code().as_deref(), Some("42501"), "{statement}");
            }
            other => panic!("{statement}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_token_that_is_too_short_to_be_a_secret_is_refused() {
    let r = registry().await;
    for token in [Vec::<u8>::new(), vec![1u8; 15]] {
        let refused = sqlx::query(
            "SELECT * FROM instance_register($1, $2, 'EXECUTION_ENGINE', NULL, '0.3.0', '{}')",
        )
        .bind(Uuid::new_v4())
        .bind(token.as_slice())
        .fetch_all(&r.process_pool)
        .await;
        match refused {
            Err(sqlx::Error::Database(e)) => assert_eq!(e.code().as_deref(), Some("22023")),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(r.rows().await, 0);
}

#[tokio::test]
async fn a_lost_connection_is_not_a_failure_of_the_registry_and_a_timeout_on_its_own_row_is() {
    use sagat_application::error::StoreError;
    use sqlx::postgres::PgPoolOptions;
    let r = registry().await;
    let (victim, mine, _) = victim(&r).await;
    // a pool of one connection with a short statement timeout, as the process's own
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET statement_timeout = 300")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&r.db.url_for(
            crate::support::RUNTIME_ROLE,
            crate::support::RUNTIME_ROLE_PASSWORD,
        ))
        .await
        .expect("a pool");
    let store = sagat_infra_postgres::PgInstanceStore::new(pool.clone());

    // someone holds the row of the process: its heartbeat waits and the statement is cancelled
    let mut holder = r.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM serving_instances WHERE instance_id = $1 FOR UPDATE")
        .bind(victim)
        .execute(&mut *holder)
        .await
        .expect("hold the row");
    let timed_out = store
        .heartbeat(victim, &mine)
        .await
        .expect_err("the statement was cancelled");
    assert!(
        !matches!(timed_out, StoreError::DatabaseUnavailable),
        "a timeout on its own row is not a lost connection: {timed_out:?}"
    );
    // the connection of the process is cut in the middle of a call: it fails as a lost connection. The call waits for
    // the row, which is watched in pg_stat_activity, and the backend that waits is terminated
    let patient = PgPoolOptions::new()
        .max_connections(1)
        .connect(&r.db.url_for(
            crate::support::RUNTIME_ROLE,
            crate::support::RUNTIME_ROLE_PASSWORD,
        ))
        .await
        .expect("a pool without a bound");
    let patient_store = sagat_infra_postgres::PgInstanceStore::new(patient);
    let call = tokio::spawn(async move { patient_store.heartbeat(victim, &mine).await });
    r.wait_for_lock_waiters(1, "the heartbeat waits for the row")
        .await;
    let cut: i64 = sqlx::query_scalar(
        "SELECT count(pg_terminate_backend(pid)) FROM pg_stat_activity \
         WHERE datname = current_database() AND wait_event_type = 'Lock'",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("terminate the backend that waits");
    assert_eq!(cut, 1);
    let lost = call
        .await
        .expect("the task")
        .expect_err("the connection was cut");
    assert_eq!(lost, StoreError::DatabaseUnavailable, "a lost connection");
    drop(holder);
}
