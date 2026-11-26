//! Fault tests of design 044, workflow as code (`docs/designs/044-workflow-as-code.md`): the crash points of a
//! completion (Task 044-9, run by `scripts/test-workflow-as-code-process-faults.sh`) and the deploy, cold-cache and
//! move faults of Success criterion 044-002 (Task 044-28). The modules land with those tasks.
#![allow(clippy::expect_used, clippy::panic)]
