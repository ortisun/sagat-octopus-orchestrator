//! Feature 022: a world whose store runs the readiness gate of flow control (T022-096), shared by the suites that
//! exercise or measure it.

use std::sync::Arc;

use sagat_application::definitions::DefinitionService;
use sagat_application::flow_control::FlowControlSwitches;
use sagat_application::instances::{Activation, Capability, InstanceKind, InstanceStore};
use sagat_application::keyed_digest::KeyRing;
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::Subject;
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{PgInstanceStore, PgRunStore};
use serde_json::Value;
use uuid::Uuid;

use super::engine::{World, ctx};
use super::instances::{nothing, registration};

const BOTH_ON: FlowControlSwitches = FlowControlSwitches {
    concurrency_keys: true,
    rate_limits: true,
};
const KINDS: [InstanceKind; 2] = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];

/// A world whose store runs the readiness gate: both switches on, a digest key ring whose version 1 is active, both
/// capabilities activated for a fleet of processes that list them, and step gates admitted (tests only).
pub async fn gated_world() -> World {
    let ring = KeyRing::parse(&format!("1:{}", "5a".repeat(32))).expect("a key ring");
    let w = super::engine::world_with_store(
        sagat_application::worker::WorkerSettings::default(),
        sagat_application::execution::BreakerPolicy::default(),
        |store: PgRunStore| {
            store
                .with_flow_control(BOTH_ON)
                .with_flow_control_keys(Arc::new(ring))
                .admitting_step_gates_for_tests()
        },
    )
    .await;
    let activated: (String, i64) = sqlx::query_as(
        "SELECT out_outcome, out_rows FROM flow_control_key_activate(1::smallint, 'operator')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("the digest key version is activated");
    assert_eq!(activated.0, "ACTIVATED", "{activated:?}");
    let registry = PgInstanceStore::new(w.db.pool.clone());
    let actor = Subject::parse("operator:flow-control").expect("subject");
    let both = [
        Capability::FlowControlConcurrencyV1,
        Capability::FlowControlRateV1,
    ];
    for kind in KINDS {
        registry
            .register(&registration(Uuid::new_v4(), kind, &both))
            .await
            .expect("a process that runs both gates");
    }
    for capability in both {
        registry
            .activate(
                Activation {
                    capability,
                    kinds: &KINDS,
                    actor: &actor,
                },
                nothing,
            )
            .await
            .expect("activated");
    }
    w
}

/// Publishes `doc` with the switches on (a control plane of the fleet).
pub async fn publish(w: &World, doc: Value) -> WorkflowId {
    let defs = DefinitionService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        JsonIrCodec,
    )
    .with_flow_control(BOTH_ON);
    let name = doc["metadata"]["name"].as_str().expect("name").to_owned();
    let workflow = defs
        .create_workflow(
            &w.admin,
            "payments",
            &name,
            &serde_json::to_vec(&doc).expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created")
        .workflow
        .id();
    defs.publish(
        &w.admin,
        workflow,
        1,
        None,
        &format!("publish-{}", Uuid::now_v7()),
        &ctx(),
    )
    .await
    .expect("published");
    workflow
}
