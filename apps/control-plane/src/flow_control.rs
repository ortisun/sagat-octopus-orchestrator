//! `control-plane flow-control-activate` (feature 022, T022-094; FR-022-053, ADR-0089 decision 6): the operator's
//! activation of `concurrencyKeys` or `rateLimits` for the fleet, through feature 018's instance-capability registry.
//!
//! It records that every control plane and every engine must list the capability, which the registry allows only while
//! every live process of both kinds lists it. A process lists it only when its binary runs the gate, its switch is on
//! and it holds the deployment's digest keys (D022-057): this build runs both gates and lists both capabilities so
//! (`concurrency-keys` since D022-057, `rate-limits` since D022-064), and an activation is refused while any live process
//! does not list the one asked for. It runs with the schema-owning role: the activation functions are not the runtime
//! role's (decision D022-077 item 8).

use sagat_application::error::StoreError;
use sagat_application::flow_control::FlowControlCapability;
use sagat_application::instances::{Activated, Activation, ChangeRefused, InstanceKind};
use sagat_domain::identity::Subject;
use sagat_infra_postgres::{PgInstanceStore, live_capable_step};

use crate::RunError;
use crate::config::{ConfigError, DatabaseConfig};

/// Both kinds run a gate: the control plane admits runs, the engine admits runs and makes steps ready.
const KINDS: [InstanceKind; 2] = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];

/// The capability named on the command line: `concurrency-keys` or `rate-limits`.
#[must_use]
pub fn capability_of(name: &str) -> Option<FlowControlCapability> {
    match name {
        "concurrency-keys" => Some(FlowControlCapability::ConcurrencyKeys),
        "rate-limits" => Some(FlowControlCapability::RateLimits),
        _ => None,
    }
}

/// Records the requirement, or says why not, in words for the operator.
///
/// # Errors
/// [`RunError::Declined`] when live processes lack the capability or the capability is recorded for other kinds;
/// [`RunError::Database`] when the registry cannot be used.
pub async fn activate(
    db: &DatabaseConfig,
    capability: FlowControlCapability,
    operator: &str,
) -> Result<String, RunError> {
    let actor = Subject::parse(operator).map_err(|error| ConfigError::Invalid {
        name: "--operator",
        reason: error.to_string(),
    })?;
    let pool = crate::open_pool(db, db.statement_timeout).await?;
    let outcome = PgInstanceStore::new(pool.clone())
        .activate(
            Activation {
                capability: capability.registry(),
                kinds: &KINDS,
                actor: &actor,
            },
            // the requirement, once a live process of each kind runs the gate (an empty fleet would serve nothing
            // after it); the digest key version has its own command (`flow_control_key_activate`)
            live_capable_step(capability.registry(), &KINDS),
        )
        .await;
    pool.close().await;
    said(capability, outcome)
}

fn said(
    capability: FlowControlCapability,
    outcome: Result<Activated, ChangeRefused>,
) -> Result<String, RunError> {
    let name = capability.as_str();
    match outcome {
        Ok(Activated::Recorded { epoch }) => {
            tracing::info!(capability = name, epoch, "a flow-control capability was activated for the fleet");
            Ok(format!("{name} is activated for the fleet (requirement epoch {epoch})."))
        }
        Ok(Activated::AlreadyRecorded) => Ok(format!("{name} was activated before; nothing was done.")),
        Err(ChangeRefused::InstancesLackCapability { missing, examples }) => Err(RunError::Declined(format!(
            "{missing} live control plane(s) or engine(s) do not run the {name} gate (instance ids: {}). Upgrade them to a \
             build that lists `{}` and run the command again. Nothing was changed.",
            examples
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            capability.registry().as_str()
        ))),
        Err(ChangeRefused::KindsDiffer) => Err(RunError::Declined(format!(
            "{name} is recorded for other kinds of process; remove it first. Nothing was changed."
        ))),
        Err(ChangeRefused::Store(StoreError::Internal { detail })) if detail.contains("permission denied") => {
            Err(RunError::Declined(
                "permission denied: the activation calls functions the runtime role may not execute. Run it with the \
                 schema-owning database role."
                    .to_owned(),
            ))
        }
        Err(ChangeRefused::Store(error)) => Err(RunError::Database(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_names_the_two_capabilities_and_nothing_else() {
        assert_eq!(
            capability_of("concurrency-keys"),
            Some(FlowControlCapability::ConcurrencyKeys)
        );
        assert_eq!(
            capability_of("rate-limits"),
            Some(FlowControlCapability::RateLimits)
        );
        assert_eq!(capability_of("schedules"), None);
    }

    #[test]
    fn a_refusal_says_what_to_do_and_that_nothing_changed() {
        let refused = said(
            FlowControlCapability::RateLimits,
            Err(ChangeRefused::InstancesLackCapability {
                missing: 2,
                examples: vec![uuid::Uuid::nil()],
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(
            refused.contains("2 live")
                && refused.contains("flow-control-rate-v1")
                && refused.contains("Nothing was changed"),
            "{refused}"
        );
        assert!(
            said(
                FlowControlCapability::ConcurrencyKeys,
                Ok(Activated::AlreadyRecorded)
            )
            .is_ok()
        );
    }

    /// D022-057, T022-072: a process of this build serves both gates (switched on, with its keys), so it lists both and
    /// an activation of either can pass once every live process is of this build.
    #[test]
    fn this_build_lists_both_gates() {
        use sagat_application::flow_control::{FlowControlSwitches, served};
        use sagat_application::run_input::InputValidationMode;
        let on = FlowControlSwitches {
            concurrency_keys: true,
            rate_limits: true,
        };
        let listed: Vec<_> = [InputValidationMode::Enforce, InputValidationMode::Report]
            .into_iter()
            .flat_map(|mode| crate::registry::capabilities(mode, &served(on, true)))
            .collect();
        assert!(
            listed.contains(&sagat_application::instances::Capability::FlowControlConcurrencyV1)
        );
        assert!(listed.contains(&sagat_application::instances::Capability::FlowControlRateV1));
    }
}
