//! Design 041 (step logs per run): the server's storage of step logs against a real PostgreSQL. Selected on a pull
//! request that changes the server's step-log code, the report path that stores a carry, the worker protocol or an
//! SDK's capture code, not only this directory (`scripts/select-test-targets.py`, constitution 4.5.0, task T041-32).
//! The retention and purge of section 4 are task T041-10's; the ingest of section 2 is task T041-6's.
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod authority;
mod budget;
mod carry;
mod counted;
mod ingest;
mod locks;
mod metrics;
mod quota;
mod retention;
mod rpc;
mod scrubbing;
