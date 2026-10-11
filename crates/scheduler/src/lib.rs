//! The scheduler: the execution plane's passes that need no caller (`FR-002-004`, `FR-002-006`,
//! `FR-002-007`).
//!
//! A suspended run costs one database row. When its timer is due, its wait deadline passes, its
//! retry backoff ends, its worker stops heartbeating or its run deadline arrives, a pass here finds
//! it with an index scan and applies the matching domain command under the run's row lock. The
//! domain re-checks the condition, so a candidate someone else already handled is a harmless skip:
//! running several schedulers at once is safe, only redundant.

//!
//! The [`OutboxDispatcher`] is the other half: it moves committed outbox messages to the broker,
//! at-least-once, so a lost wake-up costs latency but never correctness.

mod deliveries;
mod events;
mod monitor;
mod outbox;
mod passes;
mod telemetry;

pub use deliveries::{DeliveryRound, DeliverySettings, DeliveryWorker};
pub use events::{EventDispatchSettings, EventDispatcher, EventRound};
pub use monitor::{Monitor, MonitorRound, MonitorSettings};
pub use outbox::{DispatchSettings, Dispatched, OutboxDispatcher};
pub use passes::{Report, Scheduler, SchedulerSettings};
pub use telemetry::{
    ProjectionRound, ProjectorSettings, TelemetryProjector, state as projection_state,
};
