#![doc = include_str!("../README.md")]
//! Async native SDK for Sagat's public REST and worker protocols.
//! The server owns orchestration, durable state, retries and capability authorization.
mod client;
mod error;
mod tls;
mod token;
mod types;
pub use client::{
    ApprovalDecisionRequest, CallOptions, ChainCommand, ChainQueryRequest, ChainUpdateRequest,
    Client, ClientOptions, ListOptions, OperationOptions, PageOptions, PublishEvent, RetryPolicy,
    RunDeadline, StartOptions, StepOutcome, UpdateRequest, UploadRunInputOptions, WaitOptions,
};
pub use error::SagatError;
pub use serde_json::{Value, json};
pub use tls::{TlsIdentity, TlsOptions};
pub use token::{TokenProviderError, TokenSource};
pub use tokio_util::sync::CancellationToken;
pub use types::{
    ApprovalDecision, ApprovalDecisionReceipt, ApprovalListItem, ApprovalPage, ApprovalStatus,
    BlobDescriptor, Cause, ChainInspection, ChainLink, ChainRun, ChainRunPage, ChainSegment,
    CompletionKind, EventReceipt, EventStatus, JournalEvent, JournalPage, OperationResult, Page,
    QueryResult, Run, RunPage, RunParent, RunStatus, ScopeSegment, SignalResult, UpdateReceipt,
};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Wire types stay private: applications use capability-scoped public contexts.
#[allow(dead_code, clippy::all)]
mod protocol {
    tonic::include_proto!("sagat_octopus_orchestrator.v1");
}

mod worker_rpc;

mod activity;
mod worker;
pub use activity::{
    ActivityContext, ActivityFailure, CancellationReason, ExecutionInstanceIdentity, TaskInfo,
    TaskKind,
};
pub use worker::{WorkerHost, WorkerOptions, WorkerTransport};

mod json;

mod capabilities;
pub use capabilities::{SecretSet, SecretValue};

mod blob;
pub use blob::{BlobOutput, WriteBlobOptions};

pub use capabilities::BlobReader;

pub use activity::Verdict;

mod telemetry;
pub use telemetry::{EventKind, SdkEvent, SdkTransport, Telemetry};

pub mod workflow;
