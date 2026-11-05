//! The lake runner's side of feature 035: the protocol it speaks with the sandboxed Python host.
//!
//! This crate is the only one that will link the object-store clients of the lake build (`gcp`, `azure`), so the control
//! plane and the execution engine never carry them (D035-058).

pub mod protocol;
