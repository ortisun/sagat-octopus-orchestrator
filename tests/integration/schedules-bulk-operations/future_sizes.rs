//! A fitness test of the stack a run creator needs (feature 022, D022-070; plan of feature 022, "Stack and future sizes
//! of a run creator"): a debug build keeps each future a frame creates in that frame, so a future that grows inline grows
//! the stack of every caller. A batch item's rerun overflowed the 2 MiB test thread when the gated start made these
//! futures larger; each is now boxed where it is created, and this test holds it below the bound of the plan's table, so
//! a change that inlines it again fails here, not in a CI process. The futures are built and dropped, never polled.

use std::mem::size_of_val;

use sagat_application::batch::DueJob;
use sagat_application::execution::{Actor, Effects, Engine};
use sagat_application::runs::{Lock, RunStore as _, StartRun};
use serde_json::json;
use uuid::Uuid;

use crate::batch_executor::executor;
use crate::firing::fixture;
use crate::support::engine::ctx;

/// The bounds of the plan's table, in bytes.
const STEP_BOUND: usize = 3_072;
const START_BOUND: usize = 9_216;
const ENGINE_START_BOUND: usize = 6_144;

#[tokio::test]
async fn the_futures_of_a_run_creator_stay_below_their_bounds() {
    let f = fixture().await;
    let exec = executor(&f);
    let due = DueJob {
        tenant_id: f.w.admin.tenant_id(),
        id: Uuid::nil(),
    };
    let step = exec.step(&due, None);
    let step_size = size_of_val(&step);
    drop(step);

    let input = serde_json::to_vec(&json!({})).expect("json");
    let request = StartRun {
        namespace: "payments",
        workflow: f.workflow,
        version: None,
        input: &input,
        correlation_id: None,
        deadline: None,
        key: "future-sizes",
    };
    let context = ctx();
    let start = f.w.runs.start(&f.w.admin, &request, &context);
    let start_size = size_of_val(&start);
    drop(start);

    let run = f.w.start(f.workflow, &json!({})).await;
    let mut tx =
        f.w.store
            .begin(f.w.admin.tenant_id(), run, Lock::Wait)
            .await
            .expect("begin")
            .expect("the run");
    let engine = Engine::new(sagat_infra_ir::JsonEvaluator);
    let started = engine.start(
        &mut tx,
        Effects::by(Actor::System("future-sizes"), None),
        run,
        None,
    );
    let engine_start_size = size_of_val(&started);
    drop(started);

    let over: Vec<String> = [
        ("BatchExecutor::step", step_size, STEP_BOUND),
        ("RunService::start", start_size, START_BOUND),
        ("Engine::start", engine_start_size, ENGINE_START_BOUND),
    ]
    .into_iter()
    .filter(|(_, size, bound)| size > bound)
    .map(|(future, size, bound)| format!("{future} is {size} bytes, over its bound of {bound}"))
    .collect();
    assert!(
        over.is_empty(),
        "{over:?}: box what grew where it is created, or change the bound in the plan first with its reason"
    );
}
