//! Workflow as code (design 044): what the domain knows of a code run. This slice holds the command kinds and the
//! command chain (Technical Plan 6.2); the code definition, the visible and waking events and the code run's state
//! follow with the rest of Task 044-4.

mod chain;

pub use chain::{ChainError, ChainedCommand, CommandChain, CommandDigester, CommandKind};
