//! A fitness test of the size of the run state's values (feature 022, decision D022-102; plan of feature 022, "Stack and
//! future sizes of a run creator"): a debug build copies a step's state and a run's state through many frames of the
//! reducer and the store, so every byte they grow is paid by each frame that holds one; a definition's node likewise. The
//! gates feature 022 added to all three grew them past the stack of a 2 MiB test thread on CI; they are boxed, and this
//! test holds each value below the bound of the plan's table, so a change that inlines a large field again fails here,
//! not in a CI process.

use std::mem::size_of;

use sagat_domain::execution::{Gates, RunState, StepState};
use sagat_domain::workflow::Node;

/// The bounds of the plan's table, in bytes.
const STEP_STATE_BOUND: usize = 160;
const RUN_STATE_BOUND: usize = 384;
const NODE_BOUND: usize = 576;

#[test]
fn the_run_state_values_stay_below_their_bounds() {
    let over: Vec<String> = [
        ("StepState", size_of::<StepState>(), STEP_STATE_BOUND),
        ("RunState", size_of::<RunState>(), RUN_STATE_BOUND),
        ("Node", size_of::<Node>(), NODE_BOUND),
        // the gates themselves are one pointer: an ungated run or step pays nothing more
        ("Gates", size_of::<Gates>(), size_of::<usize>()),
    ]
    .into_iter()
    .filter(|(_, size, bound)| size > bound)
    .map(|(value, size, bound)| format!("{value} is {size} bytes, over its bound of {bound}"))
    .collect();
    assert!(
        over.is_empty(),
        "{over:?}: box what grew, or change the bound in the plan first with its reason"
    );
}
