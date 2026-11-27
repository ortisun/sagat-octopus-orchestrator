//! gRPC adapter of the worker protocol (`specs/001-platform-foundation/contracts/worker.proto`).
//!
//! The stubs are generated at build time from the contract that every SDK also uses, so the Rust
//! server and the SDKs cannot drift apart. The adapter authenticates with the same bearer-token
//! pipeline as the REST API ([`sagat_api_http::authenticate_bearer`]), maps errors with the same
//! classification, and delegates every rule to `sagat_application::worker::WorkerService`.

mod acknowledgement;
mod blobs;
/// The wire answer to a managed database or lake step (`response`).
pub mod database;
pub mod lake_connection;
mod limits;
mod request;
mod server;
mod service;
mod status;
mod undelivered;

/// Generated messages, server and client of `sagat_octopus_orchestrator.v1`.
#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    missing_docs,
    unreachable_pub
)]
pub mod pb {
    tonic::include_proto!("sagat_octopus_orchestrator.v1");
}

impl std::fmt::Debug for pb::ResolvedSecret {
    /// The value of a secret never prints.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedSecret")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("value", &"<redacted>")
            .finish()
    }
}

pub use blobs::{BLOB_CHUNK_BYTES, BlobTransfer};
pub use server::{AdmissionProvider, GrpcConfig, TlsConfig, serve, worker_service};
pub use service::WorkerGrpc;
pub use status::{ERROR_METADATA, RETRY_AFTER_METADATA, to_status};
pub use undelivered::{
    Abandonment, DEFAULT_WITHDRAWAL_CONCURRENCY, DeliveryProbe, Withdrawals, withdrawal_concurrency,
};

impl std::fmt::Debug for pb::ResolvedDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResolvedDatabase { <redacted> }")
    }
}
/// A lake connection prints the names of its credentials and never their values.
fn debug_connection(
    f: &mut std::fmt::Formatter<'_>,
    name: &str,
    credentials: &std::collections::HashMap<String, Vec<u8>>,
) -> std::fmt::Result {
    let mut names: Vec<&String> = credentials.keys().collect();
    names.sort();
    let redacted: Vec<String> = names
        .into_iter()
        .map(|key| format!("{key}: <redacted>"))
        .collect();
    f.debug_struct(name)
        .field("credentials", &redacted)
        .finish_non_exhaustive()
}
impl std::fmt::Debug for pb::IcebergConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_connection(f, "IcebergConnection", &self.credentials)
    }
}
impl std::fmt::Debug for pb::ObjectStorageConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_connection(f, "ObjectStorageConnection", &self.credentials)
    }
}
impl std::fmt::Debug for pb::ClickHouseConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_connection(f, "ClickHouseConnection", &self.credentials)
    }
}
impl std::fmt::Debug for pb::DatabaseValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DatabaseValue { <redacted> }")
    }
}
impl std::fmt::Debug for pb::database_value::Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DatabaseValue::Value(<redacted>)")
    }
}

// A step-log batch (feature 041): only counts print, never a message, an exception, a field, a token
// or the identity of the attempt (FR-041-007 (d)).
impl std::fmt::Debug for pb::AppendStepLogsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppendStepLogsRequest")
            .field("records", &self.records.len())
            .finish_non_exhaustive()
    }
}
// The last records of an attempt on its report (design 041 section 2a): its counts only.
impl std::fmt::Debug for pb::StepLogCarry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogCarry")
            .field("records", &self.records.len())
            .field("highest_sequence", &self.highest_sequence)
            .finish()
    }
}
impl std::fmt::Debug for pb::StepLogRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogRecord")
            .field("sequence", &self.sequence)
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::StepLogField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepLogField { <redacted> }")
    }
}
impl std::fmt::Debug for pb::step_log_field::Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepLogField::Value(<redacted>)")
    }
}

// Every message with a field whose name ends in `_token` (a lease token or a session token, the bearer authority of a worker)
// skips the generated `Debug` and prints counts and fences only (delivery acknowledgement, Task 006-043; `stream-completion.md`,
// "Wire"): no token, no identity, no payload.
impl std::fmt::Debug for pb::RegisterWorkerResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisterWorkerResponse")
            .field("accepted_protocol_version", &self.accepted_protocol_version)
            .field("step_logs", &self.step_logs.is_some())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::PollTasksRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollTasksRequest")
            .field("max_tasks", &self.max_tasks)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::TaskLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskLease")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .field("input_bytes", &self.input_json.len())
            .field("acknowledge_before_start", &self.acknowledge_before_start)
            .field("attempt_deadline_epoch_ms", &self.attempt_deadline_epoch_ms)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::HeartbeatRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeartbeatRequest")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::CompleteStepRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompleteStepRequest")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::FailStepRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailStepRequest")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::ResolveSecretsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolveSecretsRequest")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::WriteBlobHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteBlobHeader")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::ReadBlobRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadBlobRequest")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::TaskAcknowledgement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskAcknowledgement")
            .field("attempt", &self.attempt)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
