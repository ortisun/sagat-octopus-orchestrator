//! Object-store adapter of the blob port (FR-002-008, ADR-0023).
//!
//! One implementation of [`sagat_application::blobs::BlobBackend`] over the `object_store` crate
//! serves every provider, because they all speak the same protocol: Amazon S3, Cloudflare R2 (an S3
//! API with its own endpoint and region `auto`), and, for development and tests, memory and a local
//! directory. The provider is chosen at start-up from the environment ([`BlobSettings`]); nothing
//! outside this crate knows which one it is. Adding Google Cloud Storage or Azure Blob later is a new
//! [`BlobStoreConfig`] variant and a builder, not a change to any caller.

mod backend;
mod config;

pub use backend::{ObjectStoreBackend, build_backend};
pub use config::{
    BlobSettings, BlobStoreConfig, ConfigError, Credentials, Jurisdiction, PartSize, R2Config,
    S3Config, Secret,
};
