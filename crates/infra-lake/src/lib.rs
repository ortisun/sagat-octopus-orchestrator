//! The lake runner's side of feature 035: the protocol it speaks with the sandboxed Python host, and the relay that is
//! the host's only route out of its sandbox.
//!
//! This crate is the only one that will link the object-store clients of the lake build (`gcp`, `azure`), so the control
//! plane and the execution engine never carry them (D035-058).

pub mod attempt;
pub mod control;
pub mod policy;
pub mod protocol;
pub mod relay;
pub mod staging;
pub mod supervisor;
