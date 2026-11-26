//! A lease token and a session token are a worker's bearer authority (delivery acknowledgement, Task 006-043;
//! `specs/006-sdk-multilanguage/stream-completion.md`, "Wire"): every message with a field whose name ends in `_token`
//! skips the generated `Debug`, and its replacement prints counts and fences only. The canary of each is set, the message
//! printed (alone and nested in the message that carries it), and the token never appears. A message of the contract that
//! gains such a field and is not covered here fails `every_message_with_a_token_field_is_covered`.
#![allow(clippy::expect_used)]

use std::collections::BTreeSet;

use sagat_api_grpc::pb;

const LEASE: &str = "canary-lease-token-5e0a";
const SESSION: &str = "canary-session-token-81c3";

/// The messages this test prints, by their name in the contract.
const COVERED: [&str; 11] = [
    "RegisterWorkerResponse",
    "PollTasksRequest",
    "TaskLease",
    "HeartbeatRequest",
    "CompleteStepRequest",
    "FailStepRequest",
    "ResolveSecretsRequest",
    "WriteBlobHeader",
    "ReadBlobRequest",
    "AppendStepLogsRequest",
    "TaskAcknowledgement",
];

fn lease() -> pb::TaskLease {
    pb::TaskLease {
        task_id: "t".into(),
        lease_token: LEASE.into(),
        attempt: 1,
        generation: 2,
        acknowledge_before_start: true,
        ..Default::default()
    }
}

#[test]
fn no_token_prints() {
    let printed = [
        format!(
            "{:?}",
            pb::RegisterWorkerResponse {
                session_token: SESSION.into(),
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            pb::PollTasksRequest {
                session_token: SESSION.into(),
                ..Default::default()
            }
        ),
        format!("{:?}", lease()),
        format!(
            "{:?}",
            pb::PollTasksResponse {
                tasks: vec![lease()],
                retry_after_ms: 0
            }
        ),
        format!(
            "{:?}",
            pb::HeartbeatRequest {
                lease_token: LEASE.into(),
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            pb::CompleteStepRequest {
                lease_token: LEASE.into(),
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            pb::FailStepRequest {
                lease_token: LEASE.into(),
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            pb::ResolveSecretsRequest {
                lease_token: LEASE.into(),
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            pb::WriteBlobRequest {
                part: Some(pb::write_blob_request::Part::Header(pb::WriteBlobHeader {
                    lease_token: LEASE.into(),
                    ..Default::default()
                })),
            }
        ),
        format!(
            "{:?}",
            pb::ReadBlobRequest {
                lease_token: LEASE.into(),
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            pb::AppendStepLogsRequest {
                session_token: SESSION.into(),
                lease_token: LEASE.into(),
                ..Default::default()
            }
        ),
        format!(
            "{:?}",
            pb::AcknowledgeTasksRequest {
                worker_id: "w".into(),
                tasks: vec![pb::TaskAcknowledgement {
                    task_id: "t".into(),
                    lease_token: LEASE.into(),
                    attempt: 1,
                    generation: 2,
                    ..Default::default()
                }],
            }
        ),
    ];
    for text in &printed {
        assert!(!text.contains(LEASE), "{text}");
        assert!(!text.contains(SESSION), "{text}");
    }
    assert!(
        printed[2].contains("attempt: 1"),
        "the fences still print: {}",
        printed[2]
    );
}

/// The messages of the contract with a field whose name ends in `_token`, read from its descriptor.
fn with_a_token_field() -> BTreeSet<String> {
    let contracts = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../specs/001-platform-foundation/contracts");
    let set = protox::compile(["worker.proto"], [contracts]).expect("worker.proto compiles");
    let mut found = BTreeSet::new();
    let mut pending: Vec<prost_types::DescriptorProto> = set
        .file
        .iter()
        .flat_map(|file| file.message_type.iter().cloned())
        .collect();
    while let Some(message) = pending.pop() {
        if message
            .field
            .iter()
            .any(|field| field.name().ends_with("_token"))
        {
            found.insert(message.name().to_owned());
        }
        pending.extend(message.nested_type.iter().cloned());
    }
    found
}

#[test]
fn every_message_with_a_token_field_is_covered() {
    let covered: BTreeSet<String> = COVERED.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(
        with_a_token_field(),
        covered,
        "a message with a field ending in `_token` must be in `skip_debug` (crates/api-grpc/build.rs) with a Debug that \
         prints counts only, and printed by `no_token_prints`"
    );
}
