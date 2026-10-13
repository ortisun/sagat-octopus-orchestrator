//! gRPC adapter of the worker protocol (`specs/001-platform-foundation/contracts/worker.proto`).
//!
//! The stubs are generated at build time from the contract that every SDK also uses, so the Rust
//! server and the SDKs cannot drift apart. The adapter authenticates with the same bearer-token
//! pipeline as the REST API ([`sagat_api_http::authenticate_bearer`]), maps errors with the same
//! classification, and delegates every rule to `sagat_application::worker::WorkerService`.

mod blobs;
mod limits;
mod request;
mod server;
mod service;
mod status;

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
