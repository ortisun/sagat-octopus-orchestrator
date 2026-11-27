//! Workflow as code (design 044): what the domain knows of a code run. So far: the command kinds and the command chain
//! (Technical Plan 6.2) and the activity envelope with its precedence and the publisher's fields (Technical Plan 3.1
//! and 5.1); the code definition, the visible and waking events and the code run's state follow with the rest of Task
//! 044-4.

mod chain;
mod envelope;

pub use chain::{ChainError, ChainedCommand, CommandChain, CommandDigester, CommandKind};
pub use envelope::{
    ActivityEntry, ActivityEnvelope, AdmittedActivity, ApprovalPolicyName, EntryName,
    EnvelopeError, EnvelopeRefusal, MAX_ACTIVITY_ENTRIES, MAX_ENTRY_SECRETS, PublisherFields,
};
