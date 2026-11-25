//! `control-plane capability-activate <capability>` (feature 006, decision D006-034 of
//! `specs/006-sdk-multilanguage/research.md`; ADR-0098 decision 4; the pattern of ADR-0082): the one operator command that
//! records a fleet requirement of the instance-capability registry. Each capability it may activate is an entry of
//! [`ACTIVATABLE`]: the kinds the requirement covers, whether an empty fleet may be activated, and what follows. Feature 022
//! adds its own entries. The one-way handover of `schedules-v1` (`schedule-trigger-handover`) is not one of them: it
//! changes the platform's state in the same transaction and has no way back.
//!
//! It runs with the schema-owning role, in one bounded transaction (the registry's), with a second count of the fleet under
//! the epoch lock. It takes the number of live processes the operator expects (the kinds together), because an activation
//! that finds none would pass vacuously; it has a dry run that verifies everything and changes nothing; a second run says
//! the requirement is recorded already.

use std::fmt;

use sagat_application::error::StoreError;
use sagat_application::instances::{
    Activated, Activation, Capability, ChangeRefused, InstanceKind,
};
use sagat_domain::identity::Subject;
use sagat_infra_postgres::{
    CapabilityCensus, FOLLOWED_REQUIREMENT_MISSING, PgInstanceStore, capability_census,
    census_step, may_activate, requirement_recorded,
};

use crate::config::{ConfigError, DatabaseConfig};
use crate::handover::{shown, unprintable};
use crate::{RunError, open_pool};

/// A capability the operator may activate with this command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Activatable {
    pub capability: Capability,
    /// The kinds of process the requirement covers.
    pub kinds: &'static [InstanceKind],
    /// Whether `--expect-live 0` may activate it with nothing live (a requirement nothing serves under yet).
    pub empty_fleet: bool,
    /// Whether every kind it covers must have at least one live process that lists it (decision D006-082; feature 022's
    /// rule). `--expect-live` still counts the kinds together.
    pub each_kind_live: bool,
    /// A requirement that must be recorded for every kind before this one may be (decision D006-096): the command refuses
    /// while it is not, before its census and again under the epoch lock, where its removal cannot cross the activation.
    pub follows: Option<Capability>,
    /// What follows the activation, in words for the operator, and where the runbook is.
    pub after: &'static str,
}

/// The capabilities this command activates (decision D006-034). A capability listed in the registry but not here has its
/// own command or none.
pub const ACTIVATABLE: &[Activatable] = &[
    Activatable {
        capability: Capability::LeaseWithdrawalV1,
        kinds: &InstanceKind::ALL,
        empty_fleet: true,
        each_kind_live: false,
        follows: None,
        after: "Each engine withdraws the leases it never handed to a host from its next heartbeat on (within about ten \
                seconds). The activation is one-way once a lease is withdrawn; to stop, set SAGAT_LEASE_WITHDRAWAL=off on \
                the engines: docs/runbooks/lease-withdrawal.md.",
    },
    // the floor that a build of protocol 1.8 rolls back to (request (9) of decision D006-059, decision D006-096)
    Activatable {
        capability: Capability::LeaseWithdrawalV2,
        kinds: &InstanceKind::ALL,
        empty_fleet: true,
        each_kind_live: false,
        follows: Some(Capability::LeaseWithdrawalV1),
        after: "From now on a process that does not list lease-withdrawal-v2 (an older build, one of lease-withdrawal-v1 \
                alone among them) is refused registration. The requirement can be removed again until a journal holds a \
                withdrawal with the cause NOT_ACKNOWLEDGED: docs/runbooks/lease-withdrawal.md.",
    },
];

/// The entry of `token`, the registry token the command line names.
#[must_use]
pub fn activatable(token: &str) -> Option<&'static Activatable> {
    ACTIVATABLE
        .iter()
        .find(|entry| entry.capability.as_str() == token)
}

/// The flags of `capability-activate`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivationArgs {
    /// The registry token of the capability, one of [`ACTIVATABLE`].
    pub capability: String,
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
    Activated {
        entry: &'static Activatable,
        live: u32,
        epoch: i64,
    },
    /// It was recorded before; nothing was done.
    AlreadyActive { entry: &'static Activatable },
    /// A dry run found nothing that would stop the activation.
    WouldActivate {
        entry: &'static Activatable,
        live: u32,
    },
}

impl fmt::Display for ActivationOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Activated { entry, live, epoch } => write!(
                f,
                "activated: {} is required of every {} ({live} live process(es) list it, requirement epoch {epoch}). {}",
                entry.capability.as_str(),
                kinds_named(entry),
                entry.after
            ),
            Self::AlreadyActive { entry } => write!(
                f,
                "already active: {} is a recorded requirement; nothing was changed",
                entry.capability.as_str()
            ),
            Self::WouldActivate { entry, live } => write!(
                f,
                "dry run: nothing stops the activation ({live} live process(es) of the kinds it covers, all listing {}, \
                 and this database role may record it). Nothing was changed.",
                entry.capability.as_str()
            ),
        }
    }
}

fn kinds_named(entry: &Activatable) -> &'static str {
    match entry.kinds {
        [InstanceKind::ControlPlane] => "control plane",
        [InstanceKind::ExecutionEngine] => "execution engine",
        _ => "control plane and execution engine",
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

/// The refusal of an entry whose followed requirement is not recorded for every kind (decision D006-096).
fn not_followed(entry: &Activatable, follows: Capability) -> RunError {
    declined(format!(
        "{} follows {}, which is not recorded for control planes and execution engines. Activate {} first \
         (docs/runbooks/lease-withdrawal.md) and run the command again. Nothing was changed.",
        entry.capability.as_str(),
        follows.as_str(),
        follows.as_str()
    ))
}

fn lacking(entry: &Activatable, census: &CapabilityCensus) -> RunError {
    let names = census
        .lacking_examples
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    declined(format!(
        "{} live process(es) do not list {} (instance ids: {names}). Upgrade those processes to a build that lists it (or \
         stop them), wait for them to register, and run the command again. Nothing was changed.",
        census.lacking,
        entry.capability.as_str()
    ))
}

/// The count of live processes against what the operator expects. Without `--expect-live` none live is a refusal, and so
/// is none live for a capability that an empty fleet may not activate.
fn check_live(entry: &Activatable, live: u32, expect: Option<u32>) -> Result<(), RunError> {
    if live == 0 && !entry.empty_fleet {
        return Err(declined(format!(
            "no process of the kinds {} covers is live (seen within 30 seconds), and it is not activated with none live. \
             Start them and run the command again. Nothing was changed.",
            entry.capability.as_str()
        )));
    }
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

/// The kinds with a live process against an entry that requires each of its kinds live (decision D006-082): every kind
/// the entry covers must be among `live`. An entry that does not require it accepts any.
fn check_kinds(entry: &Activatable, live: &[InstanceKind]) -> Result<(), RunError> {
    if !entry.each_kind_live {
        return Ok(());
    }
    let missing: Vec<&str> = entry
        .kinds
        .iter()
        .filter(|kind| !live.contains(kind))
        .map(|kind| kind.as_str())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(declined(format!(
        "{} requires at least one live process (seen within 30 seconds) of each kind it covers, and none of {} is live. \
         Start one and run the command again. Nothing was changed.",
        entry.capability.as_str(),
        missing.join(", ")
    )))
}

/// Whether the second count, under the epoch lock, refuses the activation: the live count is not the one the first count
/// agreed with, a live process lacks the capability, none is live for an entry an empty fleet may not activate, or a kind
/// the entry requires live has none (decision D006-082).
fn refused_under_lock(entry: &Activatable, census: &CapabilityCensus, expected: u32) -> bool {
    census.live != expected
        || census.lacking > 0
        || (census.live == 0 && !entry.empty_fleet)
        || check_kinds(entry, &census.kinds_live).is_err()
}

/// Records the requirement `args.capability` of the kinds its entry covers.
///
/// # Errors
/// [`RunError::Declined`] when the operator is told no (a capability this command does not activate, a process that lacks
/// the capability, a count that is not the one expected, a role that may not record it), with the sentence that says what
/// to do; [`RunError::Config`] for an operator name that is not a subject; [`RunError::Database`] otherwise.
pub async fn capability_activate(
    db: &DatabaseConfig,
    args: &ActivationArgs,
) -> Result<ActivationOutcome, RunError> {
    let Some(entry) = activatable(&args.capability) else {
        return Err(declined(not_activatable(&args.capability)));
    };
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
    let result = run(&pool, entry, args, operator.as_ref()).await;
    pool.close().await;
    result
}

/// The sentence for a capability this command does not activate, with the ones it does.
#[must_use]
pub fn not_activatable(token: &str) -> String {
    let known = ACTIVATABLE
        .iter()
        .map(|entry| entry.capability.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("capability-activate does not activate '{token}'; it activates: {known}")
}

async fn run(
    pool: &sagat_infra_postgres::PgPool,
    entry: &'static Activatable,
    args: &ActivationArgs,
    operator: Option<&Subject>,
) -> Result<ActivationOutcome, RunError> {
    let capability = entry.capability;
    let kinds = entry.kinds;
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| RunError::Database(error.to_string()))?;
    let census = capability_census(&mut conn, capability, kinds)
        .await
        .map_err(|error| store_failure(&error))?;
    if census.recorded {
        return Ok(ActivationOutcome::AlreadyActive { entry });
    }
    if let Some(follows) = entry.follows
        && !requirement_recorded(&mut conn, follows, &InstanceKind::ALL)
            .await
            .map_err(|error| store_failure(&error))?
    {
        return Err(not_followed(entry, follows));
    }
    check_live(entry, census.live, args.expect_live)?;
    check_kinds(entry, &census.kinds_live)?;
    if census.lacking > 0 {
        return Err(lacking(entry, &census));
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
        return Ok(ActivationOutcome::WouldActivate {
            entry,
            live: census.live,
        });
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
        refused_under_lock(entry, census, expect.unwrap_or(first))
    };
    let activated = PgInstanceStore::new(pool.clone())
        .activate(
            Activation {
                capability,
                kinds,
                actor: operator,
            },
            census_step(capability, kinds, entry.follows, judge),
        )
        .await;
    match activated {
        Ok(Activated::Recorded { epoch }) => {
            let live = seen.load(std::sync::atomic::Ordering::Relaxed);
            tracing::info!(capability = capability.as_str(), operator = %shown(operator.as_str()), epoch, live,
                "a fleet requirement was recorded");
            Ok(ActivationOutcome::Activated { entry, live, epoch })
        }
        Ok(Activated::AlreadyRecorded) => Ok(ActivationOutcome::AlreadyActive { entry }),
        Err(ChangeRefused::InstancesLackCapability { missing, examples }) => Err(lacking(
            entry,
            &CapabilityCensus {
                live: 0,
                lacking: missing,
                lacking_examples: examples,
                recorded: false,
                kinds_live: Vec::new(),
            },
        )),
        Err(ChangeRefused::KindsDiffer) => Err(declined(format!(
            "{} is recorded for other kinds of process than this command records; remove it first \
             (docs/runbooks/instance-registry.md, section 5). Nothing was changed.",
            capability.as_str()
        ))),
        Err(ChangeRefused::Store(StoreError::StateConflict(reason)))
            if reason == FOLLOWED_REQUIREMENT_MISSING =>
        {
            Err(entry
                .follows
                .map_or_else(|| declined(reason), |follows| not_followed(entry, follows)))
        }
        Err(ChangeRefused::Store(StoreError::StateConflict(_))) => Err(declined(
            "the live processes changed while the activation was being made. Nothing was changed; run the command \
                 again.",
        )),
        Err(ChangeRefused::Store(error)) => Err(store_failure(&error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEASE: &Activatable = &ACTIVATABLE[0];

    #[test]
    fn the_live_count_must_be_the_one_expected_and_none_live_is_refused_without_it() {
        assert!(check_live(LEASE, 3, Some(3)).is_ok());
        assert!(check_live(LEASE, 0, Some(0)).is_ok());
        assert!(check_live(LEASE, 2, None).is_ok());
        for (live, expect) in [(2, Some(3)), (4, Some(3)), (0, None)] {
            assert!(
                matches!(check_live(LEASE, live, expect), Err(RunError::Declined(_))),
                "{live} {expect:?}"
            );
        }
        let no_empty_fleet = Activatable {
            empty_fleet: false,
            ..*LEASE
        };
        assert!(matches!(
            check_live(&no_empty_fleet, 0, Some(0)),
            Err(RunError::Declined(_))
        ));
        assert!(check_live(&no_empty_fleet, 1, Some(1)).is_ok());
    }

    /// Decision D006-082: an entry that requires a live process of each kind refuses, naming the kinds with none, unless
    /// every kind it covers has one; an entry that does not require it judges the count alone.
    #[test]
    fn an_entry_that_requires_each_kind_live_refuses_a_kind_with_none() {
        let each = Activatable {
            each_kind_live: true,
            empty_fleet: false,
            ..*LEASE
        };
        let both = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];
        assert!(check_kinds(&each, &both).is_ok());
        for live in [
            &[InstanceKind::ControlPlane][..],
            &[InstanceKind::ExecutionEngine][..],
            &[][..],
        ] {
            let Err(RunError::Declined(message)) = check_kinds(&each, live) else {
                panic!("{live:?} must be refused");
            };
            for kind in both.iter().filter(|kind| !live.contains(kind)) {
                assert!(message.contains(kind.as_str()), "{message}");
            }
            assert!(message.contains("Nothing was changed"), "{message}");
        }
        // the entry of lease-withdrawal-v1 does not require it
        assert!(!LEASE.each_kind_live);
        assert!(check_kinds(LEASE, &[]).is_ok());
        assert!(check_kinds(LEASE, &[InstanceKind::ExecutionEngine]).is_ok());
    }

    /// The judgement under the epoch lock (decision D006-082): every reason the first count refuses is refused again.
    #[test]
    fn the_count_under_the_lock_refuses_what_the_first_count_refuses() {
        let census = |live, lacking, kinds: &[InstanceKind]| CapabilityCensus {
            live,
            lacking,
            lacking_examples: Vec::new(),
            recorded: false,
            kinds_live: kinds.to_vec(),
        };
        let both = InstanceKind::ALL;
        let each = Activatable {
            each_kind_live: true,
            empty_fleet: false,
            ..*LEASE
        };
        assert!(!refused_under_lock(&each, &census(2, 0, &both), 2));
        assert!(
            refused_under_lock(&each, &census(3, 0, &both), 2),
            "the count moved"
        );
        assert!(
            refused_under_lock(&each, &census(2, 1, &both), 2),
            "a process lacks it"
        );
        assert!(
            refused_under_lock(&each, &census(2, 0, &[InstanceKind::ExecutionEngine]), 2),
            "a kind has none live"
        );
        assert!(
            refused_under_lock(&each, &census(0, 0, &[]), 0),
            "an empty fleet"
        );
        assert!(
            !refused_under_lock(LEASE, &census(0, 0, &[]), 0),
            "the lease entry may be empty"
        );
        assert!(!refused_under_lock(
            LEASE,
            &census(1, 0, &[InstanceKind::ExecutionEngine]),
            1
        ));
    }

    #[test]
    fn the_command_activates_only_its_listed_capabilities() {
        assert_eq!(
            activatable("lease-withdrawal-v1").map(|entry| entry.capability),
            Some(Capability::LeaseWithdrawalV1)
        );
        // the floor of protocol 1.8 is activatable since its follow-up (request (9) of decision D006-059, D006-096)
        assert_eq!(
            activatable("lease-withdrawal-v2").map(|entry| entry.capability),
            Some(Capability::LeaseWithdrawalV2)
        );
        for token in [
            "schedules-v1",
            "lease-withdrawal-v3",
            "",
            "LEASE-WITHDRAWAL-V1",
            "LEASE-WITHDRAWAL-V2",
        ] {
            assert_eq!(activatable(token), None, "{token:?}");
        }
        assert!(not_activatable("schedules-v1").contains("lease-withdrawal-v1"));
        // every entry is a capability the registry knows, once
        for (index, entry) in ACTIVATABLE.iter().enumerate() {
            assert!(Capability::ALL.contains(&entry.capability));
            assert!(
                ACTIVATABLE[..index]
                    .iter()
                    .all(|other| other.capability != entry.capability)
            );
        }
    }

    /// Decision D006-096: `lease-withdrawal-v2` covers both kinds, may be activated with nothing live, follows
    /// `lease-withdrawal-v1`, and says what it refuses from now on; the refusal names both requirements and changes nothing.
    #[test]
    fn the_entry_of_lease_withdrawal_v2_follows_lease_withdrawal_v1() {
        let entry = activatable("lease-withdrawal-v2").expect("an entry");
        assert_eq!(entry.kinds, &InstanceKind::ALL);
        assert!(entry.empty_fleet && !entry.each_kind_live);
        assert_eq!(entry.follows, Some(Capability::LeaseWithdrawalV1));
        assert!(
            entry.after.contains("refused registration"),
            "{}",
            entry.after
        );
        assert!(entry.after.contains("NOT_ACKNOWLEDGED"), "{}", entry.after);
        assert_eq!(LEASE.follows, None, "lease-withdrawal-v1 follows nothing");
        let RunError::Declined(message) = not_followed(entry, Capability::LeaseWithdrawalV1) else {
            panic!("a refusal in words");
        };
        assert!(
            message.contains("lease-withdrawal-v2 follows lease-withdrawal-v1"),
            "{message}"
        );
        assert!(message.contains("Nothing was changed"), "{message}");
        // every entry follows a capability the command also activates, never itself
        for entry in ACTIVATABLE {
            if let Some(follows) = entry.follows {
                assert_ne!(follows, entry.capability);
                assert!(activatable(follows.as_str()).is_some(), "{follows:?}");
            }
        }
    }

    #[test]
    fn what_the_operator_reads_names_the_capability_and_what_follows() {
        let activated = ActivationOutcome::Activated {
            entry: LEASE,
            live: 3,
            epoch: 9,
        }
        .to_string();
        assert!(activated.contains("lease-withdrawal-v1") && activated.contains("epoch 9"));
        assert!(activated.contains("docs/runbooks/lease-withdrawal.md"));
        assert!(activated.contains("SAGAT_LEASE_WITHDRAWAL=off"));
        assert!(
            ActivationOutcome::WouldActivate {
                entry: LEASE,
                live: 2
            }
            .to_string()
            .contains("Nothing was changed")
        );
    }
}
