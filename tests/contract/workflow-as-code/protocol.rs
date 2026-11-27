//! The worker protocol of design 044 is an additive minor (Architecture decision 0109, Technical Plan 13.1).
//!
//! Additivity against the published baseline is `crates/api-grpc/tests/compatibility.rs`'s job: it fails when a
//! field or RPC of `worker.baseline.json` disappears or changes. This test holds the other half: the baseline was
//! regenerated with design 044's additions (so the next change is judged against them), each addition sits at the
//! number the design and the register give it, and none takes a number another design has named.

use std::collections::BTreeMap;

use prost_types::field_descriptor_proto::Label;
use serde_json::{Value, json};

use crate::{read_json, worker_proto};

const PACKAGE: &str = "sagat_octopus_orchestrator.v1";

/// The same stable shape as `crates/api-grpc/tests/compatibility.rs` writes into the baseline.
fn snapshot() -> Value {
    let set = worker_proto();
    let mut messages = BTreeMap::new();
    let mut services = BTreeMap::new();
    for file in &set.file {
        for message in &file.message_type {
            let fields: BTreeMap<String, Value> = message
                .field
                .iter()
                .map(|f| {
                    let type_name = if f.type_name().is_empty() {
                        String::new()
                    } else {
                        format!(":{}", f.type_name())
                    };
                    (
                        f.number().to_string(),
                        json!({
                            "name": f.name(),
                            "type": format!("{:?}{type_name}", f.r#type()),
                            "repeated": f.label() == Label::Repeated,
                        }),
                    )
                })
                .collect();
            messages.insert(format!("{PACKAGE}.{}", message.name()), json!(fields));
        }
        for service in &file.service {
            let methods: BTreeMap<String, Value> = service
                .method
                .iter()
                .map(|m| {
                    (
                        m.name().to_owned(),
                        json!({
                            "input": m.input_type(),
                            "output": m.output_type(),
                            "clientStreaming": m.client_streaming(),
                            "serverStreaming": m.server_streaming(),
                        }),
                    )
                })
                .collect();
            services.insert(format!("{PACKAGE}.{}", service.name()), json!(methods));
        }
    }
    json!({ "messages": messages, "services": services })
}

fn field<'a>(baseline: &'a Value, message: &str, number: u32) -> &'a Value {
    &baseline["messages"][format!("{PACKAGE}.{message}")][number.to_string()]
}

#[test]
fn the_worker_protocol_change_is_additive_to_the_baseline() {
    let baseline = read_json("specs/001-platform-foundation/contracts/worker.baseline.json");
    assert_eq!(
        snapshot(),
        baseline,
        "worker.baseline.json is behind worker.proto: regenerate it with \
         `SAGAT_UPDATE_BASELINE=1 cargo test -p sagat-api-grpc --test compatibility` and read the diff \
         (a removed or changed line is a breaking change)"
    );

    // The additions of Technical Plan 13.1, at the numbers the register gives them.
    let expected = [
        ("WorkerCapabilities", 11, "workflow_types"),
        ("WorkerCapabilities", 12, "offers_workflow_as_code"),
        ("TaskLease", 28, "workflow_task"),
        ("TaskLease", 29, "query_task"),
        ("TaskLease", 30, "replay_check_task"),
        ("CompleteStepRequest", 13, "step_logs"), // design 041 (1.7), beside 044's 14 (Decision 044-256)
        ("CompleteStepRequest", 14, "accept_workflow_task"),
        ("FailStepRequest", 13, "step_logs"),
        ("ReadBlobRequest", 9, "read_token"),
        ("HistoryCommandRecord", 5, "saga_scope"),
        ("FailWorkflowTaskRequest", 14, "actual_saga_scope"),
        ("StepCommandResponse", 5, "workflow_task"),
        ("CompleteWorkflowTaskRequest", 7, "applied_through_seq"),
        ("CompleteWorkflowTaskRequest", 8, "replayed_chain"),
        ("CompleteWorkflowTaskRequest", 10, "deferred_command"),
        ("CompleteWorkflowTaskRequest", 12, "behaviour_flags"),
        ("WorkflowTaskLease", 9, "gated_entries"),
        ("WorkflowTaskLease", 10, "state_max_bytes"),
        ("HistoryCancellationRequested", 2, "cancel_origin"),
    ];
    for (message, number, name) in expected {
        assert_eq!(
            field(&baseline, message, number)["name"],
            json!(name),
            "{message} field {number}"
        );
    }

    // Numbers other designs have named (draft 019: capability 6 and lease 19 to 22; draft 031: capability 7 and lease
    // 25; feature 006: capability 10 and lease 23 and 24; design 045: lease 26 and 27; design 041: completion 13) stay
    // free of design 044's fields.
    let named_elsewhere = [
        ("WorkerCapabilities", [6, 7, 8, 9, 10].as_slice()),
        ("TaskLease", [19, 20, 21, 22, 23, 24, 25, 26, 27].as_slice()),
        ("CompleteStepRequest", [13].as_slice()),
    ];
    for (message, numbers) in named_elsewhere {
        for number in numbers {
            let taken = field(&baseline, message, *number);
            let ours = [
                "workflow_types",
                "offers_workflow_as_code",
                "workflow_task",
                "query_task",
                "replay_check_task",
                "accept_workflow_task",
            ];
            assert!(
                !ours.iter().any(|name| taken["name"] == json!(name)),
                "{message} field {number} is named by another design"
            );
        }
    }

    let service = &baseline["services"][format!("{PACKAGE}.WorkerService")];
    for (rpc, streaming) in [
        ("CompleteWorkflowTask", false),
        ("FailWorkflowTask", false),
        ("ReadHistory", true),
        ("RespondWorkerQuery", false),
        ("RespondReplayCheck", false),
        ("ReleaseSticky", false),
    ] {
        assert_eq!(service[rpc]["serverStreaming"], json!(streaming), "{rpc}");
        assert_eq!(service[rpc]["clientStreaming"], json!(false), "{rpc}");
    }
}
