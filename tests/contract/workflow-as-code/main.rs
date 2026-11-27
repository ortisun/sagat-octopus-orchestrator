//! Contract tests of design 044, workflow as code (`docs/designs/044-workflow-as-code.md`, Task 044-2 and
//! Task 044-30): the worker protocol's additive minor, the code document of the IR schema, the REST and event
//! contracts, the reset rules of the new event kinds, the history export format and the golden corpus. They read
//! the canonical contracts; none restates server behaviour.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

mod events;
mod goldens;
mod history_export;
mod openapi;
mod protocol;
mod reset_rules;
mod schema;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn contracts() -> PathBuf {
    repository_root().join("specs/001-platform-foundation/contracts")
}

fn read_json(relative: &str) -> serde_json::Value {
    let path = repository_root().join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

/// The compiled worker protocol.
fn worker_proto() -> prost_types::FileDescriptorSet {
    protox::compile(["worker.proto"], [contracts()]).expect("worker.proto compiles")
}
