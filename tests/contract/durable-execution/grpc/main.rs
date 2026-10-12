//! The worker protocol over the wire (`worker.proto`): every RPC, its authentication, its error
//! codes and the machine-readable `x-sagat-error` metadata, against the real server, PostgreSQL
//! and identity provider (T002-032).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod limits;
mod protocol;

mod blobs;
