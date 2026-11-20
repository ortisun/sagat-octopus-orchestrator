//! The one admission point and the fleet activation of the two capabilities (T022-093, T022-094; FR-022-053,
//! FR-022-059; SC-022-027): every run creator passes `PgRunStore::create`, which admits a gated definition only when every
//! capability it needs is switched on, activated for the fleet and served by the process (D022-057: the concurrency
//! gate, with the deployment's digest keys; never the rate gate in this build); publication and starts wait for the
//! operator's activation; the activation waits for every live process to run the gate; and a continuation successor
//! never admits a run past the gate.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{DefinitionService, IrError};
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::flow_control::{
    FlowControlCapability, FlowControlRefusal, FlowControlSwitches,
};
use sagat_application::instances::{
    Activated, Activation, Capability, ChangeRefused, InstanceKind, InstanceStore, Registered,
    Removed,
};
use sagat_application::runs::{RunService, StartRun};
use sagat_domain::identity::Subject;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgInstanceStore, PgRunStore};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, ctx, world};
use crate::support::instances::{TEST_TOKEN, nothing, registration};

const BOTH_ON: FlowControlSwitches = FlowControlSwitches {
    concurrency_keys: true,
    rate_limits: true,
};
const KINDS: [InstanceKind; 2] = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];

fn gated(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "fetch",
        "rateLimit": {"key": "input.providerId", "limit": 10, "per": "second"},
        "nodes": [{"id": "fetch", "type": "activity", "activity": "provider.fetch"}]
    })
}

fn operator() -> Subject {
    Subject::parse("operator:flow-control").expect("subject")
}

/// The services a control plane would build with the switches on, over the world's store.
fn definitions(w: &World) -> DefinitionService<PgRunStore, SystemClock, JsonIrCodec> {
    DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec).with_flow_control(BOTH_ON)
}

fn runs(store: PgRunStore) -> RunService<PgRunStore, SystemClock, JsonEvaluator> {
    RunService::new(
        store,
        SystemClock,
        Engine::new(JsonEvaluator),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    )
}

async fn create(w: &World, doc: &Value) -> sagat_domain::definition::WorkflowId {
    let name = doc["metadata"]["name"].as_str().expect("name");
    definitions(w)
        .create_workflow(
            &w.admin,
            "payments",
            name,
            &serde_json::to_vec(doc).expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created")
        .workflow
        .id()
}

async fn publish(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
) -> Result<sagat_application::definitions::Published, AppError> {
    definitions(w)
        .publish(
            &w.admin,
            workflow,
            1,
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
}

async fn start(
    service: &RunService<PgRunStore, SystemClock, JsonEvaluator>,
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
) -> Result<sagat_domain::execution::RunId, AppError> {
    let input = serde_json::to_vec(&json!({"providerId": "p-1"})).expect("json");
    service
        .start(
            &w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: &input,
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::now_v7()),
            },
            &ctx(),
        )
        .await
        .map(|started| started.info.run_id)
}

async fn runs_of(w: &World, workflow: sagat_domain::definition::WorkflowId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM runs WHERE workflow_id = $1")
        .bind(workflow.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

/// SC-022-027: with one live engine that does not run the gate, the operator's activation is refused; once every live
/// control plane and engine lists the capability it is recorded.
#[tokio::test]
async fn the_activation_is_refused_while_a_live_process_lacks_the_capability() {
    let w = world().await;
    let registry = PgInstanceStore::new(w.db.pool.clone());
    let lacking = Uuid::new_v4();
    assert!(matches!(
        registry
            .register(&registration(lacking, InstanceKind::ExecutionEngine, &[]))
            .await
            .expect("an engine of this build registers"),
        Registered::Serving { .. }
    ));
    let actor = operator();
    let refused = registry
        .activate(
            Activation {
                capability: Capability::FlowControlRateV1,
                kinds: &KINDS,
                actor: &actor,
            },
            nothing,
        )
        .await;
    match refused {
        Err(ChangeRefused::InstancesLackCapability { missing, examples }) => {
            assert_eq!(missing, 1);
            assert_eq!(examples, vec![lacking]);
        }
        other => panic!("expected INSTANCES_LACK_CAPABILITY, got {other:?}"),
    }
    registry
        .deregister(lacking, &TEST_TOKEN)
        .await
        .expect("the old engine leaves");
    for kind in KINDS {
        registry
            .register(&registration(
                Uuid::new_v4(),
                kind,
                &[Capability::FlowControlRateV1],
            ))
            .await
            .expect("a process that runs the gate");
    }
    assert!(matches!(
        registry
            .activate(
                Activation {
                    capability: Capability::FlowControlRateV1,
                    kinds: &KINDS,
                    actor: &actor,
                },
                nothing,
            )
            .await,
        Ok(Activated::Recorded { .. })
    ));
}

/// FR-022-053, FR-022-059, D022-057: before the activation, publication is refused `FLOW_CONTROL_ACTIVATION_PENDING`;
/// after it, a start reaches the one admission point, which refuses a rate-limited definition
/// `BINARY_CAPABILITY_MISSING` (no process of this build serves the rate gate, even with the digest keys); when the
/// requirement is removed, the start is refused `FLOW_CONTROL_ACTIVATION_PENDING`; with the switch off,
/// `CAPABILITY_DISABLED`. No run of the gated definition ever exists.
#[tokio::test]
async fn publication_and_starts_wait_for_the_activation_and_this_build_never_admits_a_rate_limited_run()
 {
    let w = world().await;
    let workflow = create(&w, &gated("provider-sync")).await;
    match publish(&w, workflow).await {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(
                *error,
                IrError::FlowControlActivationPending {
                    capability: "rateLimits"
                }
            ),
            "{error:?}"
        ),
        other => panic!("expected FLOW_CONTROL_ACTIVATION_PENDING, got {other:?}"),
    }

    let registry = PgInstanceStore::new(w.db.pool.clone());
    for kind in KINDS {
        registry
            .register(&registration(
                Uuid::new_v4(),
                kind,
                &[Capability::FlowControlRateV1],
            ))
            .await
            .expect("processes that would run the gate");
    }
    let actor = operator();
    registry
        .activate(
            Activation {
                capability: Capability::FlowControlRateV1,
                kinds: &KINDS,
                actor: &actor,
            },
            nothing,
        )
        .await
        .expect("activated");
    publish(&w, workflow)
        .await
        .expect("published once activated");

    let on = runs(
        w.store
            .clone()
            .with_flow_control(BOTH_ON)
            .with_flow_control_keys(Arc::new(
                sagat_application::keyed_digest::KeyRing::parse(crate::support::flow_control::RING)
                    .expect("a ring"),
            )),
    );
    assert_eq!(
        start(&on, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::BinaryCapabilityMissing
        )),
        "this build serves no rate gate, so it admits no rate-limited run"
    );
    assert_eq!(
        start(&runs(w.store.clone()), &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::CapabilityDisabled {
                capability: FlowControlCapability::RateLimits
            }
        )),
        "a deployment that switched the capability off starts none"
    );
    assert!(matches!(
        registry
            .remove(Capability::FlowControlRateV1, &actor, nothing)
            .await,
        Ok(Removed::Removed { .. })
    ));
    assert_eq!(
        start(&on, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::ActivationPending {
                capability: FlowControlCapability::RateLimits
            }
        ))
    );
    assert_eq!(runs_of(&w, workflow).await, 0, "no run of it exists");
}

/// D022-057 (G1198): a concurrency-gated definition is admitted by the one admission point, with no test door, exactly
/// when the process's switch is on, the fleet activated the capability, and the process holds the deployment's digest
/// keys; a process with the switch and the activation but without the keys refuses it `BINARY_CAPABILITY_MISSING`, and
/// one without the switch `CAPABILITY_DISABLED`. The admitted run holds its permit.
#[tokio::test]
async fn a_process_with_the_switch_the_activation_and_the_keys_admits_a_concurrency_gated_run() {
    let w = world().await;
    crate::support::flow_control::activate(&w.db.pool).await;
    let workflow = create(
        &w,
        &json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": "provider-serial"},
            "entrypoint": "fetch",
            "concurrency": {"group": "providers", "key": "input.providerId", "limit": 1},
            "nodes": [{"id": "fetch", "type": "activity", "activity": "provider.fetch"}]
        }),
    )
    .await;
    publish(&w, workflow).await.expect("published");
    let ring = || {
        Arc::new(
            sagat_application::keyed_digest::KeyRing::parse(crate::support::flow_control::RING)
                .expect("a ring"),
        )
    };
    let switched_off = runs(w.store.clone().with_flow_control_keys(ring()));
    assert_eq!(
        start(&switched_off, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::CapabilityDisabled {
                capability: FlowControlCapability::ConcurrencyKeys
            }
        ))
    );
    let without_keys = runs(w.store.clone().with_flow_control(BOTH_ON));
    assert_eq!(
        start(&without_keys, &w, workflow).await,
        Err(AppError::FlowControl(
            FlowControlRefusal::BinaryCapabilityMissing
        )),
        "a process without the digest keys serves no gate"
    );
    assert_eq!(runs_of(&w, workflow).await, 0, "nothing was admitted");
    let serving = runs(
        w.store
            .clone()
            .with_flow_control(BOTH_ON)
            .with_flow_control_keys(ring()),
    );
    let run = start(&serving, &w, workflow)
        .await
        .expect("admitted by the one admission point");
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM concurrency_holders WHERE run_id = $1 AND holder_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("holders");
    assert_eq!(held, 1, "the run holds its permit");
    w.assert_consistent(run).await;
}

/// FR-022-059, T022-093: a continuation successor is the one other writer of runs. A definition with a run-level gate is
/// refused beside a continuation at publication; a stored one that has it anyway (here, a version changed behind the
/// store's back) is refused at the hand-off, and the same hand-off goes through once the document is what it was.
#[tokio::test]
async fn a_gated_definition_that_reaches_a_continuation_successor_is_refused() {
    let w = world().await;
    let name = format!(
        "gated-successor-{}",
        &Uuid::now_v7().simple().to_string()[..8]
    );
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    });
    let (workflow, version) = w.publish(document.clone()).await;
    let chain = w
        .start(workflow, &json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    let session = w.session(&["segment.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete the activity");

    let mut tampered = document.clone();
    tampered["concurrency"] = json!({"limit": 1});
    let swap = |doc: Value| {
        let pool = w.db.pool.clone();
        async move {
            // each in a transaction of its own: the deferred accounting of the update must not hold the table
            sqlx::query(
                "ALTER TABLE workflow_versions DISABLE TRIGGER workflow_versions_immutable",
            )
            .execute(&pool)
            .await
            .expect("the owner lifts the guard");
            sqlx::query("UPDATE workflow_versions SET document = $2 WHERE id = $1")
                .bind(version.as_uuid())
                .bind(serde_json::to_vec(&doc).expect("json"))
                .execute(&pool)
                .await
                .expect("a version changed behind the store's back");
            sqlx::query(
                "ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_immutable",
            )
            .execute(&pool)
            .await
            .expect("the guard again");
        }
    };
    swap(tampered).await;

    let placement = Placement::new(format!("gated-successor-{name}"));
    Keeper::new(
        w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    // a fresh cache: the store reads the version as it is stored now
    let fresh = || {
        PgRunStore::new(
            w.execution_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        )
    };
    let scheduler = Scheduler::new(
        fresh(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement.clone());
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 0, "{report:?}");
    let segments: i32 = sqlx::query_scalar(
        "SELECT segment_count FROM execution_chains WHERE tenant_id = $1 AND chain_id = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the chain");
    assert_eq!(segments, 1, "no successor was admitted");

    // a gate on a step is refused the same way: the successor's steps would run ungated (security review m1)
    let mut node_gated = document.clone();
    node_gated["nodes"][0]["concurrency"] = json!({"limit": 1});
    swap(node_gated).await;
    let scheduler = Scheduler::new(
        fresh(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement.clone());
    assert_eq!(
        scheduler.tick().await.continuations_advanced,
        0,
        "a node-level gate is refused at the hand-off too"
    );

    swap(document).await;
    let scheduler = Scheduler::new(
        fresh(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let report = scheduler.tick().await;
    assert_eq!(
        report.continuations_advanced, 1,
        "the refusal was the gate's: {report:?}"
    );
}

/// Architecture review M3: with no live process at all the registry would record the requirement and then no process
/// of this build would serve; the activation's own step refuses unless a live process of each kind runs the gate.
#[tokio::test]
async fn the_activation_of_an_empty_fleet_is_refused_by_its_own_step() {
    let w = world().await;
    let registry = PgInstanceStore::new(w.db.pool.clone());
    let actor = operator();
    let activation = Activation {
        capability: Capability::FlowControlConcurrencyV1,
        kinds: &KINDS,
        actor: &actor,
    };
    let refused = registry
        .activate(
            activation,
            sagat_infra_postgres::live_capable_step(Capability::FlowControlConcurrencyV1, &KINDS),
        )
        .await;
    assert!(
        matches!(refused, Err(ChangeRefused::Store(_))),
        "nothing would serve after it: {refused:?}"
    );
    let recorded: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM platform_requirements)")
        .fetch_one(&w.db.pool)
        .await
        .expect("read");
    assert!(!recorded, "nothing was recorded");
    for kind in KINDS {
        registry
            .register(&registration(
                Uuid::new_v4(),
                kind,
                &[Capability::FlowControlConcurrencyV1],
            ))
            .await
            .expect("a process that runs the gate");
    }
    assert!(matches!(
        registry
            .activate(
                Activation {
                    capability: Capability::FlowControlConcurrencyV1,
                    kinds: &KINDS,
                    actor: &actor,
                },
                sagat_infra_postgres::live_capable_step(
                    Capability::FlowControlConcurrencyV1,
                    &KINDS
                ),
            )
            .await,
        Ok(Activated::Recorded { .. })
    ));
}
