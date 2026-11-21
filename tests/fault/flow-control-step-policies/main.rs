//! Flow control and step policies (feature 022): crashes at each commit point of admission, grant, timeout and lease
//! recording, duplicate grants across a shard takeover, the grant-versus-timeout and grant-versus-cancel races, lost
//! hints and restarts (T022-028, T022-080; ADR-0056, ADR-0088, ADR-0089 evidence).
//!
//! The modules land with the tasks that need them (see `specs/022-flow-control-step-policies/tasks.md`): `concurrency`
//! for a run's permit (T022-028); `rate_limits` for the rate gate (T022-080).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod concurrency;
mod rate_limits;
