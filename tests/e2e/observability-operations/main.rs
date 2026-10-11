//! The four user stories of feature 008 through the public interfaces: the trace of a request, the
//! run it started and the attempts of its steps; the timeline of a run; the alerts for runs that are
//! stuck; and the audited actions of an operator. Real PostgreSQL, the REST and gRPC servers, a fake
//! collector for the spans.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod helpers;
mod trace_links;
mod us02;
