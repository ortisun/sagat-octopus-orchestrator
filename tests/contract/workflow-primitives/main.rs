//! Machine-readable workflow-primitive contract invariants. Runtime contract cases are added here before
//! their implementations and must use the canonical contracts rather than restating server behavior.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

mod approval_ir;
mod chains;
mod errors;
mod identity_limits;
mod loops;
mod operations;
mod revision;
mod sdk_collection_ir;
mod worker_protocol;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Primitive API failures use `details.reason`; adding an `Error.code` would break exhaustive
/// clients in the five official SDKs.
#[test]
fn workflow_primitives_keep_the_rest_error_code_set_closed() {
    let path = repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let openapi: Value = serde_saphyr::from_str(&source).expect("canonical OpenAPI is YAML");
    let actual = openapi["components"]["schemas"]["Error"]["properties"]["code"]["enum"]
        .as_array()
        .expect("Error.code is an enum")
        .iter()
        .map(|code| code.as_str().expect("an error code is text"))
        .collect::<BTreeSet<_>>();
    let expected = [
        "BAD_REQUEST",
        "UNAUTHENTICATED",
        "FORBIDDEN",
        "NOT_FOUND",
        "CONFLICT",
        "VALIDATION_FAILED",
        "RATE_LIMITED",
        "QUOTA_EXCEEDED",
        "UNAVAILABLE",
        "INTERNAL",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected, "new meanings belong in details.reason");
}
