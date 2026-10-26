//! The measurement driver must exercise runtime authority and sample both transition families.
use sagat_domain::execution::{ExecutionInstanceId, RunStatus};
use sagat_domain::workflow::NodeId;
use sagat_scheduler::CollectionAdvance;
use serde_json::json;

use super::collections::{
    ACTIVITY, collection_document, open_tenant_capacity, probe_transition, run_case,
};
use super::common::platform;

#[test]
fn an_even_probe_stride_samples_admission_and_checkpoint() {
    let sampled: Vec<_> = (2..40)
        .filter(|sequence| probe_transition(*sequence, 10))
        .collect();
    assert!(
        sampled.iter().any(|sequence| sequence % 2 == 0),
        "checkpoint probes"
    );
    assert!(
        sampled.iter().any(|sequence| sequence % 2 == 1),
        "admission probes"
    );
    assert!(!probe_transition(0, 10));
    assert!(!probe_transition(1, 10));
}

#[tokio::test]
async fn measured_peak_and_queue_include_every_admitted_member() {
    let result = run_case(16, 16).await;
    assert!(result.complete);
    assert!(result.probes.iter().any(|probe| probe.label == "admit"));
    assert!(
        result
            .probes
            .iter()
            .any(|probe| probe.label == "checkpoint")
    );
    assert_eq!(
        result.peak_active, 16,
        "the observation must capture the actual configured peak"
    );
    let delays = result
        .samples
        .0
        .get("worker: ready-to-lease queue delay")
        .expect("queue measurements");
    assert_eq!(
        delays.len(),
        16,
        "one ready-to-lease observation per member"
    );
}

#[tokio::test]
async fn measured_collection_writes_use_the_runtime_database_role() {
    let p = platform().await;
    open_tenant_capacity(&p, 1).await;
    let (workflow, _) = p
        .world
        .publish(collection_document("measured-runtime-role", 1, 1))
        .await;
    let run = p.world.start(workflow, &json!({"items":[1]})).await;
    // Setup/observation remains owner-only. Every measured transition, claim and report must be
    // written by the real runtime role, including the whole-run final checkpoint/finish fallback.
    sqlx::raw_sql(
        "CREATE FUNCTION require_measured_runtime_role() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF current_user <> 'sagat_app' THEN \
           RAISE EXCEPTION 'measurement wrote with role %', current_user; \
         END IF; RETURN NEW; END $$; \
         CREATE TRIGGER require_measured_runtime_role BEFORE INSERT ON run_events \
           FOR EACH ROW EXECUTE FUNCTION require_measured_runtime_role();",
    )
    .execute(&p.world.db.pool)
    .await
    .expect("guard actual journal writers");
    let tenant = p.world.admin.tenant_id();
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    let entered = p.scheduler.advance_collection(tenant, run, &owner).await;
    assert!(
        matches!(entered, Ok(CollectionAdvance::Entered)),
        "{entered:?}"
    );
    let session = p.session(&[ACTIVITY]).await;
    for _ in 0..8 {
        p.scheduler
            .advance_collection(tenant, run, &owner)
            .await
            .expect("runtime scheduler write");
        for task in p.poll(&session, 1).await {
            p.complete(&session, &task, &json!({"v":1})).await;
        }
        if p.world.run_status(run).await == RunStatus::Succeeded {
            p.world.assert_consistent(run).await;
            return;
        }
    }
    panic!("the runtime-role workload did not finish");
}
