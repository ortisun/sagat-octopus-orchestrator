//! `control-plane lease-withdrawal-activate` (feature 006, decision D006-004 of `specs/006-sdk-multilanguage/research.md`;
//! ADR-0098; the pattern of ADR-0082): the operator command that records the requirement `lease-withdrawal-v1` of every
//! control plane and execution engine. From the next heartbeat of each engine on, a lease the engine committed and never
//! handed to a host is withdrawn (`step.lease_withdrawn`) instead of lapsing into a review.
//!
//! It runs with the schema-owning role, in one bounded transaction (the registry's), with no step of its own but a second
//! count of the fleet under the epoch lock. It takes the number of live processes the operator expects (both kinds
//! together), because an activation that finds none would pass vacuously; it has a dry run that verifies everything and
//! changes nothing; a second run says the requirement is recorded already. It has a way back: removing the requirement
//! (`docs/runbooks/lease-withdrawal.md`), which stops new withdrawals within one heartbeat.

use std::fmt;

use sagat_application::instances::{
    Activated, Activation, Capability, ChangeRefused, InstanceKind,
};
use sagat_domain::identity::Subject;
use sagat_infra_postgres::{
    CapabilityCensus, PgInstanceStore, capability_census, census_step, may_activate,
};

use crate::config::{ConfigError, DatabaseConfig};
use crate::handover::{shown, unprintable};
use crate::{RunError, open_pool};

const CAPABILITY: Capability = Capability::LeaseWithdrawalV1;
const KINDS: &[InstanceKind] = &InstanceKind::ALL;

/// The flags of `lease-withdrawal-activate`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivationArgs {
    /// `--operator NAME`: who records it, kept with the database role of the session in the requirement's history.
    pub operator: Option<String>,
    /// `--expect-live N`: how many live control planes and execution engines, together, the operator expects.
    pub expect_live: Option<u32>,
    /// `--dry-run`: verify and change nothing.
    pub dry_run: bool,
}

/// What the command did, in words for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationOutcome {
    /// The requirement is recorded in this run.
    Activated { live: u32, epoch: i64 },
    /// It was recorded before; nothing was done.
    AlreadyActive,
    /// A dry run found nothing that would stop the activation.
    WouldActivate { live: u32 },
}

impl fmt::Display for ActivationOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Activated { live, epoch } => write!(
                f,
                "activated: lease-withdrawal-v1 is required of every control plane and execution engine ({live} live \
                 process(es) list it, requirement epoch {epoch}). Each engine withdraws the leases it never handed to a \
                 host from its next heartbeat on (within about ten seconds). To stop, remove the requirement: \
                 docs/runbooks/lease-withdrawal.md."
            ),
            Self::AlreadyActive => write!(
                f,
                "already active: lease-withdrawal-v1 is a recorded requirement; nothing was changed"
            ),
            Self::WouldActivate { live } => write!(
                f,
                "dry run: nothing stops the activation ({live} live control plane(s) and execution engine(s), all \
                 listing lease-withdrawal-v1, and this database role may record it). Nothing was changed."
            ),
        }
    }
}

fn declined(message: impl Into<String>) -> RunError {
    RunError::Declined(message.into())
}

fn store_failure(error: &impl ToString) -> RunError {
    let text = error.to_string();
    if text.contains("permission denied") {
        declined(
            "permission denied: the activation calls functions the runtime role may not execute. Run it with the \
             schema-owning database role (the one that runs `migrate`), not the one the API and the engines use.",
        )
    } else {
        RunError::Database(text)
    }
}

fn lacking(census: &CapabilityCensus) -> RunError {
    let names = census
        .lacking_examples
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    declined(format!(
        "{} live process(es) do not list lease-withdrawal-v1 (instance ids: {names}). Every control plane and engine \
         of a build that knows the event lists it: upgrade those processes (or stop them), wait for them to register, and \
         run the command again. Nothing was changed.",
        census.lacking
    ))
}

/// The count of live processes against what the operator expects. Without `--expect-live` none live is a refusal.
fn check_live(live: u32, expect: Option<u32>) -> Result<(), RunError> {
    match expect {
        Some(expected) if live != expected => Err(declined(format!(
            "{live} control plane(s) and execution engine(s) are live (seen within 30 seconds) and --expect-live says \
             {expected}. If fewer are live, one has not registered yet or is down; if more, one is not yours or is not \
             stopped. Nothing was changed."
        ))),
        None if live == 0 => Err(declined(
            "no control plane or execution engine is live (seen within 30 seconds), so there is nothing to verify. Start \
             them and run the command again, or pass --expect-live 0 if that is what you mean. Nothing was changed.",
        )),
        _ => Ok(()),
    }
}

/// Records the requirement `lease-withdrawal-v1` of control planes and engines.
///
/// # Errors
/// [`RunError::Declined`] when the operator is told no (a process that lacks the capability, a count that is not the one
/// expected, a role that may not record it), with the sentence that says what to do; [`RunError::Config`] for an operator
/// name that is not a subject; [`RunError::Database`] otherwise.
pub async fn lease_withdrawal_activate(
    db: &DatabaseConfig,
    args: &ActivationArgs,
) -> Result<ActivationOutcome, RunError> {
    let operator = match (&args.operator, args.dry_run) {
        (Some(name), _) => {
            if let Some(reason) = unprintable(name) {
                return Err(ConfigError::Invalid {
                    name: "--operator",
                    reason: reason.to_owned(),
                }
                .into());
            }
            Some(Subject::parse(name).map_err(|error| ConfigError::Invalid {
                name: "--operator",
                reason: error.to_string(),
            })?)
        }
        (None, true) => None,
        (None, false) => {
            return Err(declined(
                "--operator NAME is required: the activation is recorded with who made it",
            ));
        }
    };
    let pool = open_pool(db, db.statement_timeout).await?;
    let result = run(&pool, args, operator.as_ref()).await;
    pool.close().await;
    result
}

async fn run(
    pool: &sagat_infra_postgres::PgPool,
    args: &ActivationArgs,
    operator: Option<&Subject>,
) -> Result<ActivationOutcome, RunError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| RunError::Database(error.to_string()))?;
    let census = capability_census(&mut conn, CAPABILITY, KINDS)
        .await
        .map_err(|error| store_failure(&error))?;
    if census.recorded {
        return Ok(ActivationOutcome::AlreadyActive);
    }
    check_live(census.live, args.expect_live)?;
    if census.lacking > 0 {
        return Err(lacking(&census));
    }
    if args.dry_run {
        if !may_activate(&mut conn)
            .await
            .map_err(|error| store_failure(&error))?
        {
            return Err(store_failure(
                &"permission denied for function platform_requirement_activate",
            ));
        }
        return Ok(ActivationOutcome::WouldActivate { live: census.live });
    }
    drop(conn);
    let Some(operator) = operator else {
        return Err(declined("--operator NAME is required"));
    };
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counted = std::sync::Arc::clone(&seen);
    let expect = args.expect_live;
    let first = census.live;
    // under the epoch lock the fleet is counted again: a process that registered or went silent since the first count
    // changes nothing
    let judge = move |census: &CapabilityCensus| {
        counted.store(census.live, std::sync::atomic::Ordering::Relaxed);
        census.live != expect.unwrap_or(first) || census.lacking > 0
    };
    let activated = PgInstanceStore::new(pool.clone())
        .activate(
            Activation {
                capability: CAPABILITY,
                kinds: KINDS,
                actor: operator,
            },
            census_step(CAPABILITY, KINDS, judge),
        )
        .await;
    match activated {
        Ok(Activated::Recorded { epoch }) => {
            let live = seen.load(std::sync::atomic::Ordering::Relaxed);
            tracing::info!(operator = %shown(operator.as_str()), epoch, live, "lease-withdrawal-v1 was recorded");
            Ok(ActivationOutcome::Activated { live, epoch })
        }
        Ok(Activated::AlreadyRecorded) => Ok(ActivationOutcome::AlreadyActive),
        Err(ChangeRefused::InstancesLackCapability { missing, examples }) => {
            Err(lacking(&CapabilityCensus {
                live: 0,
                lacking: missing,
                lacking_examples: examples,
                recorded: false,
            }))
        }
        Err(ChangeRefused::KindsDiffer) => Err(declined(
            "lease-withdrawal-v1 is recorded for other kinds than control planes and engines; remove it first \
             (docs/runbooks/lease-withdrawal.md). Nothing was changed.",
        )),
        Err(ChangeRefused::Store(sagat_application::error::StoreError::StateConflict(_))) => {
            Err(declined(
                "the live control planes and execution engines changed while the activation was being made. Nothing was \
                 changed; run the command again.",
            ))
        }
        Err(ChangeRefused::Store(error)) => Err(store_failure(&error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_count_must_be_the_one_expected_and_none_live_is_refused_without_it() {
        assert!(check_live(3, Some(3)).is_ok());
        assert!(check_live(0, Some(0)).is_ok());
        assert!(check_live(2, None).is_ok());
        for (live, expect) in [(2, Some(3)), (4, Some(3)), (0, None)] {
            assert!(
                matches!(check_live(live, expect), Err(RunError::Declined(_))),
                "{live} {expect:?}"
            );
        }
    }

    #[test]
    fn what_the_operator_reads_names_the_capability_and_the_way_back() {
        let activated = ActivationOutcome::Activated { live: 3, epoch: 9 }.to_string();
        assert!(activated.contains("lease-withdrawal-v1") && activated.contains("epoch 9"));
        assert!(activated.contains("docs/runbooks/lease-withdrawal.md"));
        assert!(
            ActivationOutcome::WouldActivate { live: 2 }
                .to_string()
                .contains("Nothing was changed")
        );
    }
}
