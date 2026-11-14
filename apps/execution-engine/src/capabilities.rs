//! What the engine can do (`FR-018-052`, `data-model.md` rule 5), announced to the instance-capability registry before the
//! engine runs its first pass.
//!
//! A capability is listed when the **process** will honour it, not when the binary could: it is the code of the binary
//! narrowed by the passes the configuration turns on. `schedules-v1` is the promise that this engine fires the schedules
//! (and, after the handover, the cron triggers that became schedules); an engine with `SAGAT_SCHEDULES_ENABLED` unset runs
//! no schedule pass, so it does not list it, an activation that requires it from every live engine refuses it by name,
//! and one that is restarted without the pass after the handover is refused at registration and closes its whole gate.
//! The flags are read from the environment once, so the list is true for the life of the process.

use sagat_application::instances::Capability;

use crate::EngineConfig;
use crate::settings::{PASS_SCHEDULES, optional_passes};

/// The capabilities of the engine that `config` configures, in the order of `Capability::ALL`.
#[must_use]
pub fn capabilities(config: &EngineConfig) -> Vec<Capability> {
    let passes = optional_passes(config);
    let mut listed = Vec::new();
    // feature 018: the schedule pass, and the handover of the legacy cron pass to it
    if passes.contains(&PASS_SCHEDULES) {
        listed.push(Capability::SchedulesV1);
    }
    listed
}
