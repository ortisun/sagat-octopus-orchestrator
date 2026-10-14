//! Adapter: builds and runs untrusted scripts in containers (`FR-005-001`, `FR-005-004`,
//! `FR-005-007`).
//!
//! The only way in for the code is its input and the artifact; the only way out is one line on
//! standard output and a bounded log on standard error. Everything else the script wants (the
//! network, a secret) it asks of its supervisor, which may refuse. See [`docker`] for the
//! isolation, and `docs/adr/ADR-0013-inline-code-sandbox.md` for what it does not promise.

pub mod docker;
pub mod lockfile;
pub mod protocol;

pub use docker::{DockerSandbox, GoTarget, Images, artifact_contract, artifact_policy};
