//! The instance-capability registry against a real PostgreSQL (`T018-080`, `FR-018-052`, `SC-018-017`): what a process
//! may do (register, beat, leave), what only the operator may do (activate, remove), the closed values, the guards that
//! keep a requirement from changing behind the epoch's back, the privileges of the runtime role, and the data model.
//!
//! The processes run as the runtime role when `SAGAT_TEST_AS_RUNTIME_ROLE=1`; the operator is always the owner. The
//! age of a heartbeat is written (`Registry::age`), never waited for.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::instances::{
    Activated, Activation, Beat, Capability, ChangeRefused, InstanceKind, InstanceStore, LIVENESS,
    Member, Registered, Removed, Requirement, Requirements,
};
use sagat_domain::identity::Subject;
use sqlx::Row;
use uuid::Uuid;

use crate::support::instances::{Registry, TEST_TOKEN, nothing, registration, registry};

const ENGINE: InstanceKind = InstanceKind::ExecutionEngine;
const CONTROL_PLANE: InstanceKind = InstanceKind::ControlPlane;
const SCHEDULES: Capability = Capability::SchedulesV1;

const CHECK_VIOLATION: &str = "23514";
const RESTRICT_VIOLATION: &str = "23001";
const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// The SQLSTATE of a failed statement.
fn code<T>(result: Result<T, sqlx::Error>) -> String {
    match result {
        Ok(_) => panic!("the statement was expected to fail"),
        Err(sqlx::Error::Database(e)) => e.code().map(|c| c.to_string()).unwrap_or_default(),
        Err(other) => panic!("not a database error: {other}"),
    }
}

// --- a process: register, beat, leave -----------------------------------------------------------------------------

#[tokio::test]
async fn a_process_registers_before_serving_and_its_row_says_what_it_is() {
    let r = registry().await;
    let instance = Uuid::new_v4();
    let got = r
        .process
        .register(&sagat_application::instances::Registration {
            instance,
            token: &TEST_TOKEN,
            kind: ENGINE,
            scheduler_owner: Some("engine-a-0123"),
            binary_version: "0.3.0",
            capabilities: &[SCHEDULES],
        })
        .await
        .expect("register");
    assert_eq!(got, Registered::Serving { epoch: 0 });

    let row = sqlx::query(
        "SELECT kind, scheduler_owner, binary_version, capabilities, started_at <= last_seen AS ordered, \
                last_seen > clock_timestamp() - interval '5 seconds' AS fresh \
           FROM serving_instances WHERE instance_id = $1",
    )
    .bind(instance)
    .fetch_one(&r.db.pool)
    .await
    .expect("its row");
    assert_eq!(row.get::<String, _>("kind"), "EXECUTION_ENGINE");
    assert_eq!(
        row.get::<Option<String>, _>("scheduler_owner").as_deref(),
        Some("engine-a-0123")
    );
    assert_eq!(row.get::<String, _>("binary_version"), "0.3.0");
    assert_eq!(
        row.get::<Vec<String>, _>("capabilities"),
        vec!["schedules-v1"]
    );
    assert!(row.get::<bool, _>("ordered") && row.get::<bool, _>("fresh"));
    assert_eq!(r.process.epoch().await.expect("epoch"), 0);
    assert_eq!(
        r.process.requirements().await.expect("requirements"),
        Requirements {
            epoch: 0,
            required: vec![]
        }
    );
}

#[tokio::test]
async fn a_binary_that_lacks_a_required_capability_is_refused_and_leaves_no_row() {
    let r = registry().await;
    assert_eq!(
        r.require(SCHEDULES, &[ENGINE]).await.expect("activate"),
        Activated::Recorded { epoch: 1 }
    );
    let old = Uuid::new_v4();
    let got = r
        .process
        .register(&registration(old, ENGINE, &[]))
        .await
        .expect("register");
    assert_eq!(
        got,
        Registered::CapabilityMissing {
            epoch: 1,
            missing: vec!["schedules-v1".to_owned()]
        }
    );
    assert!(!r.row_exists(old).await, "a refused process is not counted");
    // the same binary with the capability is accepted under the same epoch
    let new = Uuid::new_v4();
    assert_eq!(
        r.process
            .register(&registration(new, ENGINE, &[SCHEDULES]))
            .await
            .expect("register"),
        Registered::Serving { epoch: 1 }
    );
    assert_eq!(r.rows().await, 1);
}

#[tokio::test]
async fn a_requirement_of_another_kind_does_not_concern_a_process() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    let control_plane = Uuid::new_v4();
    assert_eq!(
        r.process
            .register(&registration(control_plane, CONTROL_PLANE, &[]))
            .await
            .expect("register"),
        Registered::Serving { epoch: 1 },
        "a control plane without the capability serves: the requirement is of engines"
    );
}

#[tokio::test]
async fn registering_again_refreshes_the_row_and_never_changes_the_kind() {
    let r = registry().await;
    let instance = r.serving(ENGINE, &[]).await;
    r.age(instance, 20).await;
    let before: (time::OffsetDateTime, time::OffsetDateTime) = sqlx::query_as(
        "SELECT started_at, last_seen FROM serving_instances WHERE instance_id = $1",
    )
    .bind(instance)
    .fetch_one(&r.db.pool)
    .await
    .expect("row");
    assert_eq!(
        r.process
            .register(&registration(instance, ENGINE, &[SCHEDULES]))
            .await
            .expect("again"),
        Registered::Serving { epoch: 0 }
    );
    let after: (time::OffsetDateTime, time::OffsetDateTime, Vec<String>) = sqlx::query_as(
        "SELECT started_at, last_seen, capabilities FROM serving_instances WHERE instance_id = $1",
    )
    .bind(instance)
    .fetch_one(&r.db.pool)
    .await
    .expect("row");
    assert_eq!(after.0, before.0, "it started once");
    assert!(after.1 > before.1, "and was seen again");
    assert_eq!(after.2, vec!["schedules-v1"], "with what it lists now");
    assert_eq!(r.rows().await, 1);

    // another kind under the same id is a different process wearing this one's id: refused, nothing changed
    let changed = sqlx::query(
        "SELECT * FROM instance_register($1, $2, 'CONTROL_PLANE', NULL, '0.3.0', '{}')",
    )
    .bind(instance)
    .bind(TEST_TOKEN.as_bytes().as_slice())
    .fetch_all(&r.process_pool)
    .await;
    assert_eq!(code(changed), RESTRICT_VIOLATION);
    let kind: String =
        sqlx::query_scalar("SELECT kind FROM serving_instances WHERE instance_id = $1")
            .bind(instance)
            .fetch_one(&r.db.pool)
            .await
            .expect("kind");
    assert_eq!(kind, "EXECUTION_ENGINE");
}

#[tokio::test]
async fn a_heartbeat_touches_only_its_own_row_and_reports_the_epoch_and_the_live_counts() {
    let r = registry().await;
    let a = r.serving(ENGINE, &[]).await;
    let b = r.serving(ENGINE, &[]).await;
    let silent = r.serving(CONTROL_PLANE, &[]).await;
    r.age(a, 20).await;
    r.age(b, 20).await;
    r.age(silent, 40).await;
    let seen = |instance: Uuid| {
        let pool = r.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, time::OffsetDateTime>(
                "SELECT last_seen FROM serving_instances WHERE instance_id = $1",
            )
            .bind(instance)
            .fetch_one(&pool)
            .await
            .expect("last_seen")
        }
    };
    let (b_before, silent_before) = (seen(b).await, seen(silent).await);
    let a_before = seen(a).await;

    let beat = r
        .process
        .heartbeat(a, &TEST_TOKEN)
        .await
        .expect("heartbeat");
    assert!(beat.found);
    assert_eq!(beat.epoch, 0);
    assert_eq!(
        beat.live,
        vec![("EXECUTION_ENGINE".to_owned(), 2)],
        "two engines are live (one beat just now, one 20 s ago); the control plane silent for 40 s is not"
    );
    assert!(seen(a).await > a_before, "its own row moved");
    assert_eq!(seen(b).await, b_before, "another instance's row did not");
    assert_eq!(seen(silent).await, silent_before);
}

#[tokio::test]
async fn a_heartbeat_of_an_unknown_instance_touches_nothing_and_says_it_is_not_found() {
    let r = registry().await;
    let known = r.serving(ENGINE, &[]).await;
    let stranger = Uuid::new_v4();
    let beat = r
        .process
        .heartbeat(stranger, &TEST_TOKEN)
        .await
        .expect("heartbeat");
    assert!(!beat.found);
    assert!(
        !r.row_exists(stranger).await,
        "a heartbeat never creates a row"
    );
    assert_eq!(r.rows().await, 1);
    assert!(r.row_exists(known).await);
}

#[tokio::test]
async fn leaving_deletes_only_the_instances_own_row() {
    let r = registry().await;
    let a = r.serving(ENGINE, &[]).await;
    let b = r.serving(ENGINE, &[]).await;
    assert!(r.process.deregister(a, &TEST_TOKEN).await.expect("leave"));
    assert!(
        !r.process.deregister(a, &TEST_TOKEN).await.expect("again"),
        "already gone"
    );
    assert!(
        !r.process
            .deregister(Uuid::new_v4(), &TEST_TOKEN)
            .await
            .expect("stranger")
    );
    assert!(r.row_exists(b).await);
    assert_eq!(r.rows().await, 1);
}

// --- the closed values ----------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_values_a_row_may_hold_are_closed() {
    let r = registry().await;
    let call = |kind: &'static str, owner: Option<String>, version: String, caps: Vec<String>| {
        let pool = r.process_pool.clone();
        async move {
            sqlx::query("SELECT * FROM instance_register($1, $6, $2, $3, $4, $5)")
                .bind(Uuid::new_v4())
                .bind(kind)
                .bind(owner)
                .bind(version)
                .bind(caps)
                .bind(TEST_TOKEN.as_bytes().as_slice())
                .fetch_all(&pool)
                .await
        }
    };
    let ok = || ("ENGINE_X", None, "0.3.0".to_owned(), vec![]);
    let (_, owner, version, caps) = ok();
    assert_eq!(
        code(call("RUNNER", owner.clone(), version.clone(), caps.clone()).await),
        CHECK_VIOLATION,
        "a kind is one of the closed set"
    );
    assert_eq!(
        code(
            call(
                "EXECUTION_ENGINE",
                owner.clone(),
                version.clone(),
                vec!["bogus-v1".to_owned()]
            )
            .await
        ),
        CHECK_VIOLATION,
        "so is a capability"
    );
    assert_eq!(
        code(
            call(
                "EXECUTION_ENGINE",
                owner.clone(),
                "v".repeat(65),
                caps.clone()
            )
            .await
        ),
        CHECK_VIOLATION,
        "a version is at most 64 characters"
    );
    assert_eq!(
        code(
            call(
                "EXECUTION_ENGINE",
                owner.clone(),
                String::new(),
                caps.clone()
            )
            .await
        ),
        CHECK_VIOLATION,
        "and not empty"
    );
    assert_eq!(
        code(
            call(
                "EXECUTION_ENGINE",
                Some("o".repeat(256)),
                version.clone(),
                caps.clone()
            )
            .await
        ),
        CHECK_VIOLATION,
        "an owner name is at most 255 characters"
    );
    call(
        "EXECUTION_ENGINE",
        Some("o".repeat(255)),
        "v".repeat(64),
        caps.clone(),
    )
    .await
    .expect("the limits themselves are accepted");
    // the table bounds the list whoever writes it
    let too_many = sqlx::query(
        "INSERT INTO serving_instances (instance_id, kind, binary_version, capabilities, started_at, last_seen, token_hash) \
         VALUES (gen_random_uuid(), 'EXECUTION_ENGINE', 'x', \
                 ARRAY(SELECT 'schedules-v1' FROM generate_series(1, 33)), now(), now(), sha256('x'::bytea))",
    )
    .execute(&r.db.pool)
    .await;
    assert_eq!(code(too_many), CHECK_VIOLATION);
    let exactly = sqlx::query(
        "INSERT INTO serving_instances (instance_id, kind, binary_version, capabilities, started_at, last_seen, token_hash) \
         VALUES (gen_random_uuid(), 'EXECUTION_ENGINE', 'x', \
                 ARRAY(SELECT 'schedules-v1' FROM generate_series(1, 32)), now(), now(), sha256('x'::bytea))",
    )
    .execute(&r.db.pool)
    .await;
    exactly.expect("thirty-two are allowed");
}

// --- liveness, the day rule and the cap -----------------------------------------------------------------------------

#[tokio::test]
async fn a_row_silent_for_a_day_is_deleted_by_any_heartbeat_and_one_silent_for_less_is_not() {
    let r = registry().await;
    let beating = r.serving(ENGINE, &[]).await;
    let long_gone = r.serving(CONTROL_PLANE, &[]).await;
    let recent = r.serving(CONTROL_PLANE, &[]).await;
    r.age(long_gone, 25 * 3600).await;
    r.age(recent, 23 * 3600).await;
    r.process
        .heartbeat(beating, &TEST_TOKEN)
        .await
        .expect("heartbeat");
    assert!(!r.row_exists(long_gone).await, "silent for 25 hours");
    assert!(
        r.row_exists(recent).await,
        "silent for 23 hours: kept for the operator to read"
    );
    assert!(r.row_exists(beating).await);
}

#[tokio::test]
async fn the_table_holds_a_thousand_live_rows_and_refuses_the_next_one() {
    let r = registry().await;
    let known = r.serving(ENGINE, &[]).await;
    r.seed(999, ENGINE, 1).await;
    assert_eq!(r.rows().await, 1000);
    let newcomer = Uuid::new_v4();
    assert_eq!(
        r.process
            .register(&registration(newcomer, ENGINE, &[]))
            .await
            .expect("register"),
        Registered::Full { epoch: 0 }
    );
    assert!(!r.row_exists(newcomer).await);
    assert_eq!(r.rows().await, 1000);
    // a process that is already in the table registers again at the cap
    assert_eq!(
        r.process
            .register(&registration(known, ENGINE, &[SCHEDULES]))
            .await
            .expect("register"),
        Registered::Serving { epoch: 0 }
    );
    assert_eq!(r.rows().await, 1000);
}

#[tokio::test]
async fn a_table_full_of_silent_rows_makes_room_and_a_crash_loop_cannot_lock_the_fleet_out() {
    let r = registry().await;
    r.seed(1000, ENGINE, 60).await;
    let newcomer = Uuid::new_v4();
    assert_eq!(
        r.process
            .register(&registration(newcomer, ENGINE, &[]))
            .await
            .expect("register"),
        Registered::Serving { epoch: 0 },
        "the rows that are no longer live give way"
    );
    assert_eq!(r.rows().await, 1000, "exactly one was made room for");
    assert!(r.row_exists(newcomer).await);
    // 999 live and one silent: the silent one goes, the live ones never do
    let r2 = registry().await;
    r2.seed(999, ENGINE, 1).await;
    r2.seed(1, ENGINE, 60).await;
    let second = Uuid::new_v4();
    assert_eq!(
        r2.process
            .register(&registration(second, ENGINE, &[]))
            .await
            .expect("register"),
        Registered::Serving { epoch: 0 }
    );
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM serving_instances WHERE last_seen > clock_timestamp() - interval '30 seconds'",
    )
    .fetch_one(&r2.db.pool)
    .await
    .expect("live");
    assert_eq!(live, 1000, "999 live rows kept, the silent one replaced");
}

// --- the operator: activation and removal -------------------------------------------------------------------------

#[tokio::test]
async fn an_activation_verifies_every_live_instance_of_the_required_kinds() {
    let r = registry().await;
    let good = r.serving(ENGINE, &[SCHEDULES]).await;
    let old = r.serving(ENGINE, &[]).await;
    let _other_kind = r.serving(CONTROL_PLANE, &[]).await;
    let refused = r
        .require(SCHEDULES, &[ENGINE])
        .await
        .expect_err("an old engine is live");
    assert_eq!(
        refused,
        ChangeRefused::InstancesLackCapability {
            missing: 1,
            examples: vec![old]
        },
        "the control plane does not count, the engine that lists it does not either"
    );
    assert_eq!(r.epoch_now().await, 0, "nothing was recorded");
    assert_eq!(r.history().await, 0);
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirements")
        .fetch_one(&r.db.pool)
        .await
        .expect("count");
    assert_eq!(recorded, 0);

    r.process
        .deregister(old, &TEST_TOKEN)
        .await
        .expect("the old engine is stopped");
    assert_eq!(
        r.require(SCHEDULES, &[ENGINE]).await.expect("activate"),
        Activated::Recorded { epoch: 1 }
    );
    assert!(r.row_exists(good).await);
}

#[tokio::test]
async fn an_activation_names_how_many_lack_it_and_at_most_sixteen_of_them() {
    let r = registry().await;
    r.seed(20, ENGINE, 1).await;
    let refused = r
        .require(SCHEDULES, &[ENGINE])
        .await
        .expect_err("twenty engines lack it");
    match refused {
        ChangeRefused::InstancesLackCapability { missing, examples } => {
            assert_eq!(missing, 20);
            assert_eq!(examples.len(), 16);
            let distinct: BTreeSet<_> = examples.iter().collect();
            assert_eq!(distinct.len(), 16, "sixteen different instances");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_instance_silent_for_thirty_seconds_no_longer_blocks_an_activation() {
    let r = registry().await;
    let old = r.serving(ENGINE, &[]).await;
    r.age(old, 20).await;
    assert!(
        r.require(SCHEDULES, &[ENGINE]).await.is_err(),
        "seen 20 s ago it is live and lacks the capability"
    );
    r.age(old, 40).await;
    assert_eq!(
        r.require(SCHEDULES, &[ENGINE]).await.expect("activate"),
        Activated::Recorded { epoch: 1 },
        "seen 40 s ago it is not live"
    );
    assert_eq!(LIVENESS, Duration::from_secs(30));
}

#[tokio::test]
async fn an_activation_records_the_requirement_the_epoch_and_the_history() {
    let r = registry().await;
    let actor = Subject::parse("ops-on-call").expect("subject");
    assert_eq!(
        r.operator
            .activate(
                Activation {
                    capability: SCHEDULES,
                    kinds: &[ENGINE],
                    actor: &actor
                },
                nothing
            )
            .await
            .expect("activate"),
        Activated::Recorded { epoch: 1 }
    );
    let row = sqlx::query(
        "SELECT capability, kinds, recorded_by, required_since FROM platform_requirements",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("the requirement");
    assert_eq!(row.get::<String, _>("capability"), "schedules-v1");
    assert_eq!(row.get::<Vec<String>, _>("kinds"), vec!["EXECUTION_ENGINE"]);
    assert_eq!(row.get::<String, _>("recorded_by"), "ops-on-call");
    let history = sqlx::query(
        "SELECT action, capability, kinds, actor, epoch FROM platform_requirement_history",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("the history");
    assert_eq!(history.get::<String, _>("action"), "ACTIVATED");
    assert_eq!(history.get::<String, _>("capability"), "schedules-v1");
    assert_eq!(
        history.get::<Vec<String>, _>("kinds"),
        vec!["EXECUTION_ENGINE"]
    );
    assert_eq!(history.get::<String, _>("actor"), "ops-on-call");
    assert_eq!(history.get::<i64, _>("epoch"), 1);
    assert_eq!(r.epoch_now().await, 1);
    assert_eq!(
        r.process.requirements().await.expect("read"),
        Requirements {
            epoch: 1,
            required: vec![Requirement {
                capability: "schedules-v1".to_owned(),
                kinds: vec!["EXECUTION_ENGINE".to_owned()]
            }]
        }
    );
}

#[tokio::test]
async fn an_activation_names_at_least_one_kind_and_is_refused_before_the_database_is_asked() {
    let r = registry().await;
    let refused = r
        .require(SCHEDULES, &[])
        .await
        .expect_err("a requirement of no kind means nothing");
    assert_eq!(
        refused,
        ChangeRefused::Store(StoreError::StateConflict(
            "an activation names at least one kind"
        ))
    );
    assert_eq!((r.epoch_now().await, r.history().await), (0, 0));
}

#[tokio::test]
async fn the_requirements_are_read_in_the_order_of_their_names() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    r.require(Capability::AccessNarrowingV1, &[CONTROL_PLANE, ENGINE])
        .await
        .expect("activate");
    let read = r.process.requirements().await.expect("read");
    assert_eq!(read.epoch, 2);
    assert_eq!(
        read.required,
        vec![
            Requirement {
                capability: "access-narrowing-v1".to_owned(),
                kinds: vec!["CONTROL_PLANE".to_owned(), "EXECUTION_ENGINE".to_owned()]
            },
            Requirement {
                capability: "schedules-v1".to_owned(),
                kinds: vec!["EXECUTION_ENGINE".to_owned()]
            },
        ],
        "sorted by capability, kinds as recorded (sorted)"
    );
}

#[tokio::test]
async fn activating_again_is_a_no_op_and_for_other_kinds_is_refused() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    // the feature's own step must not run again: it ran in the transaction that recorded the requirement
    let ran = Arc::new(Mutex::new(0));
    let again = {
        let ran = Arc::clone(&ran);
        let actor = Subject::operator();
        r.operator
            .activate(
                Activation {
                    capability: SCHEDULES,
                    kinds: &[ENGINE],
                    actor: &actor,
                },
                move |_| {
                    *ran.lock().expect("lock") += 1;
                    Box::pin(async { Ok(()) })
                },
            )
            .await
            .expect("again")
    };
    assert_eq!(again, Activated::AlreadyRecorded);
    assert_eq!(*ran.lock().expect("lock"), 0);
    assert_eq!((r.epoch_now().await, r.history().await), (1, 1));
    assert_eq!(
        r.require(SCHEDULES, &[ENGINE, CONTROL_PLANE])
            .await
            .expect_err("another set of kinds"),
        ChangeRefused::KindsDiffer
    );
    assert_eq!((r.epoch_now().await, r.history().await), (1, 1));
}

#[tokio::test]
async fn the_features_step_runs_in_the_transaction_of_the_activation_and_its_failure_records_nothing()
 {
    let r = registry().await;
    // the step sees the requirement its own transaction wrote, and nobody else does until it commits
    let seen = Arc::new(Mutex::new(None));
    let outside = Arc::new(Mutex::new(None));
    let actor = Subject::operator();
    let pool = r.db.pool.clone();
    let (seen_in, outside_in) = (Arc::clone(&seen), Arc::clone(&outside));
    r.operator
        .activate(
            Activation {
                capability: SCHEDULES,
                kinds: &[ENGINE],
                actor: &actor,
            },
            move |connection| {
                Box::pin(async move {
                    let inside: i64 =
                        sqlx::query_scalar("SELECT count(*) FROM platform_requirements")
                            .fetch_one(&mut *connection)
                            .await
                            .map_err(|e| StoreError::Internal {
                                detail: e.to_string(),
                            })?;
                    let beside: i64 =
                        sqlx::query_scalar("SELECT count(*) FROM platform_requirements")
                            .fetch_one(&pool)
                            .await
                            .map_err(|e| StoreError::Internal {
                                detail: e.to_string(),
                            })?;
                    *seen_in.lock().expect("lock") = Some(inside);
                    *outside_in.lock().expect("lock") = Some(beside);
                    Ok(())
                })
            },
        )
        .await
        .expect("activate");
    assert_eq!(*seen.lock().expect("lock"), Some(1));
    assert_eq!(*outside.lock().expect("lock"), Some(0));

    // a step that fails takes the requirement, the epoch and the history with it
    let r = registry().await;
    let refused = r
        .operator
        .activate(
            Activation {
                capability: SCHEDULES,
                kinds: &[ENGINE],
                actor: &actor,
            },
            |_| Box::pin(async { Err(StoreError::StateConflict("the handover row is not there")) }),
        )
        .await
        .expect_err("the step failed");
    assert_eq!(
        refused,
        ChangeRefused::Store(StoreError::StateConflict("the handover row is not there"))
    );
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirements")
        .fetch_one(&r.db.pool)
        .await
        .expect("count");
    assert_eq!(
        (recorded, r.epoch_now().await, r.history().await),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn a_removal_moves_the_epoch_again_and_lets_an_old_binary_register() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    let actor = Subject::parse("ops-on-call").expect("subject");
    assert_eq!(
        r.operator
            .remove(SCHEDULES, &actor, nothing)
            .await
            .expect("remove"),
        Removed::Removed { epoch: 2 }
    );
    assert_eq!(
        r.operator
            .remove(SCHEDULES, &actor, nothing)
            .await
            .expect("again"),
        Removed::NotRecorded
    );
    assert_eq!((r.epoch_now().await, r.history().await), (2, 2));
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM platform_requirement_history ORDER BY id")
            .fetch_all(&r.db.pool)
            .await
            .expect("history");
    assert_eq!(actions, vec!["ACTIVATED", "REMOVED"]);
    assert_eq!(
        r.process
            .register(&registration(Uuid::new_v4(), ENGINE, &[]))
            .await
            .expect("register"),
        Registered::Serving { epoch: 2 }
    );
}

#[tokio::test]
async fn the_removal_step_can_refuse_and_then_nothing_is_removed() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    let actor = Subject::operator();
    let refused = r
        .operator
        .remove(SCHEDULES, &actor, |_| {
            Box::pin(async { Err(StoreError::StateConflict("a schedule still depends on it")) })
        })
        .await
        .expect_err("the feature verifies nothing depends on it");
    assert_eq!(
        refused,
        ChangeRefused::Store(StoreError::StateConflict("a schedule still depends on it"))
    );
    assert_eq!((r.epoch_now().await, r.history().await), (1, 1));
    assert_eq!(
        r.process.requirements().await.expect("read").required.len(),
        1
    );
}

#[tokio::test]
async fn a_process_learns_of_a_requirement_at_its_next_beat_and_serves_again_when_it_is_removed() {
    let r = registry().await;
    let member = Member::new(
        r.process.clone(),
        Uuid::new_v4(),
        ENGINE,
        "0.3.0",
        &[],
        None,
    );
    member.register().await.expect("registered");
    assert!(member.gate().is_open());
    assert_eq!(member.beat().await, Ok(Beat::Serving));

    // while it beats it is live and lacks the capability, so the activation is refused
    assert!(r.require(SCHEDULES, &[ENGINE]).await.is_err());
    // silent for a while (a partition), the activation goes through
    r.age(member.instance(), 60).await;
    assert_eq!(
        r.require(SCHEDULES, &[ENGINE]).await.expect("activate"),
        Activated::Recorded { epoch: 1 }
    );
    assert!(member.gate().is_open(), "it has not beaten since");
    assert_eq!(member.beat().await, Ok(Beat::Refused));
    assert!(!member.gate().is_open());
    assert_eq!(member.gate().missing(), vec!["schedules-v1"]);
    assert_eq!(member.gate().epoch(), 1);

    // the operator removes the requirement: the next beat opens the gate again
    let actor = Subject::operator();
    r.operator
        .remove(SCHEDULES, &actor, nothing)
        .await
        .expect("remove");
    assert_eq!(member.beat().await, Ok(Beat::Serving));
    assert!(member.gate().is_open());
    assert_eq!(member.gate().epoch(), 2);

    // a clean stop deletes the row
    member.leave().await;
    assert!(!r.row_exists(member.instance()).await);
}

#[tokio::test]
async fn a_row_the_day_rule_deleted_is_registered_again_with_the_whole_check() {
    let r = registry().await;
    let member = Member::new(
        r.process.clone(),
        Uuid::new_v4(),
        ENGINE,
        "0.3.0",
        &[],
        None,
    );
    member.register().await.expect("registered");
    sqlx::query("DELETE FROM serving_instances WHERE instance_id = $1")
        .bind(member.instance())
        .execute(&r.db.pool)
        .await
        .expect("the row is gone");
    assert_eq!(member.beat().await, Ok(Beat::Serving));
    assert!(r.row_exists(member.instance()).await, "registered again");

    sqlx::query("DELETE FROM serving_instances WHERE instance_id = $1")
        .bind(member.instance())
        .execute(&r.db.pool)
        .await
        .expect("gone again");
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    assert_eq!(
        member.beat().await,
        Ok(Beat::Refused),
        "registering again meets the requirement"
    );
    assert!(!member.gate().is_open());
    assert!(!r.row_exists(member.instance()).await);
}

// --- the guards ------------------------------------------------------------------------------------------------------

/// A transaction of the owner that has set the flag the activation functions set.
async fn flagged(r: &Registry) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = r.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT set_config('sagat.registry_change', 'activation', true)")
        .execute(&mut *tx)
        .await
        .expect("flag");
    tx
}

const INSERT_REQUIREMENT: &str = "INSERT INTO platform_requirements (capability, kinds, required_since, recorded_by) \
     VALUES ('schedules-v1', ARRAY['EXECUTION_ENGINE'], now(), 'hand')";

const INSERT_REQUIREMENT_OTHER: &str = "INSERT INTO platform_requirements (capability, kinds, required_since, recorded_by) \
     VALUES ('payload-encoding-v1', ARRAY['EXECUTION_ENGINE'], now(), 'hand')";

#[tokio::test]
async fn requirements_and_the_epoch_change_only_through_the_activation_functions() {
    let r = registry().await;
    // without the flag nothing writes them, the owner included
    assert_eq!(
        code(sqlx::query(INSERT_REQUIREMENT).execute(&r.db.pool).await),
        RESTRICT_VIOLATION
    );
    assert_eq!(
        code(
            sqlx::query("UPDATE platform_requirement_epoch SET epoch = epoch + 1")
                .execute(&r.db.pool)
                .await
        ),
        RESTRICT_VIOLATION
    );
    // the flag alone does not let a requirement in without the epoch moving in the same transaction
    let mut tx = flagged(&r).await;
    sqlx::query(INSERT_REQUIREMENT)
        .execute(&mut *tx)
        .await
        .expect("insert");
    assert_eq!(
        code(tx.commit().await),
        RESTRICT_VIOLATION,
        "the requirement was recorded and the epoch did not move"
    );
    // nor does the epoch move by more than one
    let mut tx = flagged(&r).await;
    assert_eq!(
        code(
            sqlx::query("UPDATE platform_requirement_epoch SET epoch = epoch + 2")
                .execute(&mut *tx)
                .await
        ),
        RESTRICT_VIOLATION
    );
    drop(tx);
    // together, with the flag, they are accepted: what the functions do
    let mut tx = flagged(&r).await;
    sqlx::query("UPDATE platform_requirement_epoch SET epoch = epoch + 1")
        .execute(&mut *tx)
        .await
        .expect("bump");
    sqlx::query(INSERT_REQUIREMENT)
        .execute(&mut *tx)
        .await
        .expect("insert");
    tx.commit()
        .await
        .expect("a requirement with the epoch moved by one");
    // the flag is what the guard of the requirements looks at, and not the epoch's: with the epoch moved in this
    // transaction (so the deferred check would pass) a requirement written without the flag is still refused
    let mut tx = flagged(&r).await;
    sqlx::query("UPDATE platform_requirement_epoch SET epoch = epoch + 1")
        .execute(&mut *tx)
        .await
        .expect("bump");
    sqlx::query("SELECT set_config('sagat.registry_change', '', true)")
        .execute(&mut *tx)
        .await
        .expect("the flag is off");
    assert_eq!(
        code(
            sqlx::query(INSERT_REQUIREMENT_OTHER)
                .execute(&mut *tx)
                .await
        ),
        RESTRICT_VIOLATION
    );
    drop(tx);
    // a requirement is never changed in place
    let mut tx = flagged(&r).await;
    assert_eq!(
        code(
            sqlx::query("UPDATE platform_requirements SET kinds = ARRAY['CONTROL_PLANE']")
                .execute(&mut *tx)
                .await
        ),
        RESTRICT_VIOLATION
    );
    drop(tx);
    // the closed values hold for whoever writes
    let mut tx = flagged(&r).await;
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO platform_requirements (capability, kinds, required_since, recorded_by) \
                 VALUES ('bogus-v1', ARRAY['EXECUTION_ENGINE'], now(), 'hand')"
            )
            .execute(&mut *tx)
            .await
        ),
        CHECK_VIOLATION
    );
    drop(tx);
    let mut tx = flagged(&r).await;
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO platform_requirements (capability, kinds, required_since, recorded_by) \
                 VALUES ('access-narrowing-v1', ARRAY['RUNNER'], now(), 'hand')"
            )
            .execute(&mut *tx)
            .await
        ),
        CHECK_VIOLATION,
        "a kind outside the closed set"
    );
    drop(tx);
    let mut tx = flagged(&r).await;
    assert_eq!(
        code(
            sqlx::query(
                "INSERT INTO platform_requirements (capability, kinds, required_since, recorded_by) \
                 VALUES ('access-narrowing-v1', ARRAY[]::text[], now(), 'hand')"
            )
            .execute(&mut *tx)
            .await
        ),
        CHECK_VIOLATION,
        "a requirement of no kind"
    );
}

#[tokio::test]
async fn the_epoch_is_one_row_and_the_history_and_the_epoch_cannot_be_rewritten_or_emptied() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    let pool = &r.db.pool;
    for statement in [
        "DELETE FROM platform_requirement_epoch",
        "TRUNCATE platform_requirement_epoch",
        "INSERT INTO platform_requirement_epoch (only_row, epoch) VALUES (false, 5)",
        "UPDATE platform_requirement_history SET actor = 'someone-else'",
        "DELETE FROM platform_requirement_history",
        "TRUNCATE platform_requirement_history",
        "DELETE FROM platform_requirements",
        "TRUNCATE platform_requirements",
    ] {
        let got = sqlx::query(statement).execute(pool).await;
        let sqlstate = match got {
            Err(sqlx::Error::Database(e)) => e.code().map(|c| c.to_string()).unwrap_or_default(),
            other => panic!("{statement}: expected a refusal, got {other:?}"),
        };
        assert!(
            sqlstate == RESTRICT_VIOLATION || sqlstate == CHECK_VIOLATION,
            "{statement}: {sqlstate}"
        );
    }
    // the guards are ENABLE ALWAYS: a session that turned triggers off for itself still meets them
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .expect("replica role");
    assert_eq!(
        code(
            sqlx::query("DELETE FROM platform_requirement_history")
                .execute(&mut *tx)
                .await
        ),
        RESTRICT_VIOLATION
    );
    drop(tx);
    assert_eq!((r.epoch_now().await, r.history().await), (1, 1));
    let triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
          WHERE NOT t.tgisinternal AND t.tgenabled <> 'A' \
            AND c.relname IN ('platform_requirements', 'platform_requirement_epoch', 'platform_requirement_history')",
    )
    .fetch_one(pool)
    .await
    .expect("triggers");
    assert_eq!(
        triggers, 0,
        "every guard of the three tables is ENABLE ALWAYS"
    );
}

// --- privileges -----------------------------------------------------------------------------------------------------

const PROCESS_FUNCTIONS: [&str; 3] = [
    "instance_register(uuid, bytea, text, text, text, text[])",
    "instance_heartbeat(uuid, bytea)",
    "instance_deregister(uuid, bytea)",
];
const OPERATOR_FUNCTIONS: [&str; 3] = [
    "platform_requirement_lock()",
    "platform_requirement_activate(text, text[], text)",
    "platform_requirement_remove(text, text)",
];

#[tokio::test]
async fn the_runtime_role_reads_the_registry_and_calls_three_functions_only() {
    let r = registry().await;
    let owner = &r.db.pool;
    // the instances are readable column by column, all but the hash of the token (a column privilege is not a table one)
    for column in [
        "instance_id",
        "kind",
        "scheduler_owner",
        "binary_version",
        "capabilities",
        "started_at",
        "last_seen",
    ] {
        let has: bool = sqlx::query_scalar(
            "SELECT has_column_privilege('sagat_app', 'serving_instances', $1, 'SELECT')",
        )
        .bind(column)
        .fetch_one(owner)
        .await
        .expect("privilege");
        assert!(has, "SELECT on serving_instances.{column}");
    }
    let hash: bool = sqlx::query_scalar(
        "SELECT has_column_privilege('sagat_app', 'serving_instances', 'token_hash', 'SELECT')",
    )
    .fetch_one(owner)
    .await
    .expect("privilege");
    assert!(!hash, "the runtime role cannot read the hash of a token");
    for table in [
        "serving_instances",
        "platform_requirements",
        "platform_requirement_epoch",
    ] {
        for (privilege, expected) in [
            ("SELECT", table != "serving_instances"),
            ("INSERT", false),
            ("UPDATE", false),
            ("DELETE", false),
            ("TRUNCATE", false),
        ] {
            let has: bool = sqlx::query_scalar("SELECT has_table_privilege('sagat_app', $1, $2)")
                .bind(table)
                .bind(privilege)
                .fetch_one(owner)
                .await
                .expect("privilege");
            assert_eq!(has, expected, "{privilege} on {table}");
        }
    }
    let history: bool = sqlx::query_scalar(
        "SELECT has_table_privilege('sagat_app', 'platform_requirement_history', 'SELECT')",
    )
    .fetch_one(owner)
    .await
    .expect("privilege");
    assert!(!history, "the history is the operator's to read");
    for function in PROCESS_FUNCTIONS {
        let has: bool =
            sqlx::query_scalar("SELECT has_function_privilege('sagat_app', $1, 'EXECUTE')")
                .bind(function)
                .fetch_one(owner)
                .await
                .expect("privilege");
        assert!(has, "{function}");
    }
    for function in OPERATOR_FUNCTIONS {
        let has: bool =
            sqlx::query_scalar("SELECT has_function_privilege('sagat_app', $1, 'EXECUTE')")
                .bind(function)
                .fetch_one(owner)
                .await
                .expect("privilege");
        assert!(!has, "{function} is the operator's");
    }
    // and nobody else holds them: a role with no grant of its own. Roles belong to the server, not to the test's
    // database, so a server that ran this test before (or runs it in parallel) already has it; it never gets a grant.
    sqlx::query(
        "DO $$ BEGIN CREATE ROLE registry_outsider NOLOGIN; \
         EXCEPTION WHEN duplicate_object THEN NULL; END $$",
    )
    .execute(owner)
    .await
    .expect("role");
    for function in PROCESS_FUNCTIONS.into_iter().chain(OPERATOR_FUNCTIONS) {
        let has: bool =
            sqlx::query_scalar("SELECT has_function_privilege('registry_outsider', $1, 'EXECUTE')")
                .bind(function)
                .fetch_one(owner)
                .await
                .expect("privilege");
        assert!(!has, "{function} is not PUBLIC's");
    }

    // what the role is refused when it tries
    let runtime = r.db.runtime_pool().await;
    for statement in [
        "INSERT INTO serving_instances (instance_id, kind, binary_version, capabilities, started_at, last_seen, token_hash) \
         VALUES (gen_random_uuid(), 'EXECUTION_ENGINE', 'x', '{}', now(), now(), sha256('x'::bytea))",
        "UPDATE serving_instances SET last_seen = now()",
        "DELETE FROM serving_instances",
        "SELECT epoch FROM platform_requirement_epoch FOR SHARE",
        "SELECT * FROM platform_requirement_history",
        "SELECT platform_requirement_lock()",
        "SELECT * FROM platform_requirement_activate('schedules-v1', ARRAY['EXECUTION_ENGINE'], 'me')",
        "SELECT * FROM platform_requirement_remove('schedules-v1', 'me')",
    ] {
        assert_eq!(
            code(sqlx::query(statement).fetch_all(&runtime).await),
            INSUFFICIENT_PRIVILEGE,
            "{statement}"
        );
    }
    // it reads the three tables
    for read in [
        "SELECT instance_id, kind, scheduler_owner, binary_version, capabilities, started_at, last_seen FROM serving_instances",
        "SELECT * FROM platform_requirements",
        "SELECT * FROM platform_requirement_epoch",
    ] {
        sqlx::query(read)
            .fetch_all(&runtime)
            .await
            .expect("the runtime role reads the registry");
    }
}

#[tokio::test]
async fn the_functions_are_security_definer_with_a_fixed_search_path_and_the_liveness_they_name() {
    let r = registry().await;
    let rows = sqlx::query(
        "SELECT proname, prosecdef, proconfig, prosrc FROM pg_proc \
          WHERE proname IN ('instance_register', 'instance_heartbeat', 'instance_deregister', \
                            'platform_requirement_lock', 'platform_requirement_activate', 'platform_requirement_remove')",
    )
    .fetch_all(&r.db.pool)
    .await
    .expect("functions");
    assert_eq!(rows.len(), 6);
    for row in &rows {
        let name: String = row.get("proname");
        assert!(row.get::<bool, _>("prosecdef"), "{name} runs as its owner");
        let config: Vec<String> = row
            .get::<Option<Vec<String>>, _>("proconfig")
            .unwrap_or_default();
        assert!(
            config
                .iter()
                .any(|c| c == "search_path=pg_catalog, pg_temp"),
            "{name}: {config:?}"
        );
        let source: String = row.get("prosrc");
        if matches!(
            name.as_str(),
            "instance_heartbeat" | "platform_requirement_activate" | "instance_register"
        ) {
            assert!(
                source.contains("interval '30 seconds'"),
                "{name} judges liveness with the window the code names ({:?})",
                LIVENESS
            );
        }
    }
}

// --- the closed values, in code, in the schema and in the data model ----------------------------------------------------

fn quoted(definition: &str) -> BTreeSet<String> {
    definition
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn the_capabilities_and_kinds_in_code_are_the_closed_values_of_the_schema_and_of_the_data_model()
 {
    let r = registry().await;
    let in_code: BTreeSet<String> = Capability::ALL
        .iter()
        .map(|c| c.as_str().to_owned())
        .collect();
    let kinds_in_code: BTreeSet<String> = InstanceKind::ALL
        .iter()
        .map(|k| k.as_str().to_owned())
        .collect();
    let definition = |constraint: &'static str| {
        let pool = r.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = $1",
            )
            .bind(constraint)
            .fetch_one(&pool)
            .await
            .expect("constraint")
        }
    };
    assert_eq!(
        quoted(&definition("serving_instances_capabilities_closed").await)
            .into_iter()
            .filter(|v| v.ends_with("-v1") || v.contains("-v"))
            .collect::<BTreeSet<_>>(),
        in_code
    );
    assert_eq!(
        quoted(&definition("platform_requirements_capability_valid").await),
        in_code
    );
    assert_eq!(
        quoted(&definition("platform_requirement_history_capability_valid").await),
        in_code
    );
    assert_eq!(
        quoted(&definition("serving_instances_kind_valid").await),
        kinds_in_code
    );
    assert!(
        quoted(&definition("platform_requirements_kinds_valid").await).is_superset(&kinds_in_code)
    );

    // the data model's rule 5 names the same values (and no other)
    let data_model = include_str!("../../../specs/018-schedules-bulk-operations/data-model.md");
    let rule = data_model
        .split("5. **Capability values**")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("rule 5 of the registry in the data model");
    let named: BTreeSet<String> = rule
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|token| token.ends_with("-v1"))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        named, in_code,
        "the data model and the code list the same capabilities"
    );
}

#[tokio::test]
async fn the_history_says_which_database_role_recorded_it_as_well_as_who_the_command_said() {
    let r = registry().await;
    r.require(SCHEDULES, &[ENGINE]).await.expect("activate");
    let session: String = sqlx::query_scalar("SELECT session_user::text")
        .fetch_one(&r.db.pool)
        .await
        .expect("the session user");
    let recorded: String =
        sqlx::query_scalar("SELECT database_user FROM platform_requirement_history")
            .fetch_one(&r.db.pool)
            .await
            .expect("the history");
    assert_eq!(recorded, session);
    let actor: String = sqlx::query_scalar("SELECT actor FROM platform_requirement_history")
        .fetch_one(&r.db.pool)
        .await
        .expect("the history");
    assert_eq!(
        actor, "system:operator",
        "what the command said is kept apart from it"
    );
}

fn bounds(lock_ms: u64, statement_ms: u64, step_ms: u64) -> sagat_infra_postgres::OperatorBounds {
    sagat_infra_postgres::OperatorBounds {
        lock_timeout: Duration::from_millis(lock_ms),
        statement_timeout: Duration::from_millis(statement_ms),
        step: Duration::from_millis(step_ms),
    }
}

fn operator_with(
    r: &Registry,
    bounds: sagat_infra_postgres::OperatorBounds,
) -> sagat_infra_postgres::PgInstanceStore {
    sagat_infra_postgres::PgInstanceStore::new(r.db.pool.clone()).with_bounds(bounds)
}

#[tokio::test]
async fn the_operators_transaction_runs_under_its_own_lock_and_statement_timeouts() {
    let r = registry().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let operator = operator_with(&r, bounds(1500, 2500, 60_000));
    let actor = Subject::operator();
    let into = Arc::clone(&seen);
    operator
        .activate(
            Activation {
                capability: SCHEDULES,
                kinds: &[ENGINE],
                actor: &actor,
            },
            move |connection| {
                Box::pin(async move {
                    for setting in ["lock_timeout", "statement_timeout"] {
                        let value: String = sqlx::query_scalar("SELECT current_setting($1)")
                            .bind(setting)
                            .fetch_one(&mut *connection)
                            .await
                            .map_err(|e| StoreError::Internal {
                                detail: e.to_string(),
                            })?;
                        into.lock().expect("lock").push(value);
                    }
                    Ok(())
                })
            },
        )
        .await
        .expect("activate");
    assert_eq!(*seen.lock().expect("lock"), vec!["1500ms", "2500ms"]);
    // and the settings end with the transaction
    let after: String = sqlx::query_scalar("SELECT current_setting('lock_timeout')")
        .fetch_one(&r.db.pool)
        .await
        .expect("setting");
    assert_eq!(
        after, "0",
        "a setting of the transaction does not leak to the pool"
    );
}

#[tokio::test]
async fn a_feature_step_that_takes_too_long_is_cut_off_and_nothing_is_recorded() {
    let r = registry().await;
    let operator = operator_with(&r, bounds(1000, 5000, 150));
    let actor = Subject::operator();
    let started = std::time::Instant::now();
    let refused = operator
        .activate(
            Activation {
                capability: SCHEDULES,
                kinds: &[ENGINE],
                actor: &actor,
            },
            |_| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok(())
                })
            },
        )
        .await
        .expect_err("the step took longer than it may");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    match refused {
        ChangeRefused::Store(StoreError::Unavailable { detail }) => {
            assert!(detail.contains("took longer"), "{detail}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!((r.epoch_now().await, r.history().await), (0, 0));
    // the epoch is free again: a process registers without waiting
    let registered = tokio::time::timeout(
        Duration::from_secs(5),
        r.process
            .register(&registration(Uuid::new_v4(), ENGINE, &[])),
    )
    .await
    .expect("it did not wait for an operator that was cut off")
    .expect("register");
    assert_eq!(registered, Registered::Serving { epoch: 0 });
}

#[tokio::test]
async fn a_statement_of_the_step_that_outlasts_the_statement_timeout_is_cancelled() {
    let r = registry().await;
    let operator = operator_with(&r, bounds(1000, 150, 60_000));
    let actor = Subject::operator();
    let refused = operator
        .activate(
            Activation {
                capability: SCHEDULES,
                kinds: &[ENGINE],
                actor: &actor,
            },
            |connection| {
                Box::pin(async move {
                    sqlx::query("SELECT pg_sleep(10)")
                        .execute(&mut *connection)
                        .await
                        .map_err(|e| StoreError::Unavailable {
                            detail: e.to_string(),
                        })?;
                    Ok(())
                })
            },
        )
        .await
        .expect_err("the statement was cancelled");
    assert!(
        matches!(&refused, ChangeRefused::Store(StoreError::Unavailable { detail }) if detail.contains("statement timeout")),
        "{refused:?}"
    );
    assert_eq!((r.epoch_now().await, r.history().await), (0, 0));
}

#[tokio::test]
async fn an_operator_that_waits_for_the_epoch_longer_than_its_lock_timeout_gives_up() {
    let r = registry().await;
    // a transaction holds the epoch share lock, as a request path that fences on it would
    let mut holder = r.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT epoch FROM platform_requirement_epoch FOR SHARE")
        .execute(&mut *holder)
        .await
        .expect("hold the epoch");
    let operator = operator_with(&r, bounds(200, 5000, 60_000));
    let actor = Subject::operator();
    let started = std::time::Instant::now();
    let refused = operator
        .activate(
            Activation {
                capability: SCHEDULES,
                kinds: &[ENGINE],
                actor: &actor,
            },
            nothing,
        )
        .await
        .expect_err("it does not wait for ever");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(matches!(refused, ChangeRefused::Store(_)), "{refused:?}");
    drop(holder);
    assert_eq!(r.epoch_now().await, 0);
}
