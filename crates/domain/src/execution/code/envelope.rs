//! The activity envelope of a code version (design 044, Technical Plan 3.1 and 5.1; Decision 044-241).
//!
//! A code version's envelope lists the activities its commands may schedule: each entry is an exact activity name or a
//! prefix ending in `.*`. Every name has one covering entry: an exact entry wins over a prefix entry and a longer prefix
//! over a shorter one. The covering entry owns the publisher's fields of the activities it covers (the secrets, the
//! risk metadata and the approval that gates them); a command's fragment may repeat one of them only with the entry's
//! value, never add or change one. Only an exact entry may release secrets (constitution 5.0.0), and the compensation
//! or the lookup of an activity runs with the secrets of its own entry, never with the forward activity's.

use std::collections::BTreeSet;

use crate::secrets::SecretName;
use crate::workflow::ActivityName;

/// The most activity entries an envelope holds.
pub const MAX_ACTIVITY_ENTRIES: usize = 64;
/// The most secrets one exact entry releases (the graph activity node's bound).
pub const MAX_ENTRY_SECRETS: usize = 16;
/// The activity names a code version may not list: the platform's script and workflow-task names.
const RESERVED_PREFIXES: [&str; 2] = ["script.", "sagat."];

/// The name of an envelope entry: an exact activity name, or the prefix of a `<stem>.*` entry (kept with its dot).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryName {
    Exact(ActivityName),
    Prefix(String),
}

impl EntryName {
    /// # Errors
    /// [`EnvelopeError::InvalidName`] unless the text is an activity name or `<activity name>.*`, and
    /// [`EnvelopeError::ReservedName`] for a name under `script.` or `sagat.`.
    pub fn parse(text: &str) -> Result<Self, EnvelopeError> {
        let name = match text.strip_suffix(".*") {
            Some(stem) => ActivityName::parse(stem)
                .map(|_| Self::Prefix(format!("{stem}.")))
                .map_err(|_| EnvelopeError::InvalidName(clip(text)))?,
            None => ActivityName::parse(text)
                .map(Self::Exact)
                .map_err(|_| EnvelopeError::InvalidName(clip(text)))?,
        };
        let reserved = match &name {
            Self::Exact(exact) => RESERVED_PREFIXES
                .iter()
                .any(|prefix| exact.as_str().starts_with(prefix)),
            Self::Prefix(prefix) => RESERVED_PREFIXES.iter().any(|reserved| {
                prefix.starts_with(reserved) || reserved.starts_with(prefix.as_str())
            }),
        };
        if reserved {
            return Err(EnvelopeError::ReservedName(clip(text)));
        }
        Ok(name)
    }

    /// Whether this entry names `activity`.
    #[must_use]
    pub fn covers(&self, activity: &ActivityName) -> bool {
        match self {
            Self::Exact(exact) => exact == activity,
            Self::Prefix(prefix) => activity.as_str().starts_with(prefix.as_str()),
        }
    }

    /// The order of precedence among the entries that cover one name: an exact entry first, then the longer prefix.
    fn precedence(&self) -> (bool, usize) {
        match self {
            Self::Exact(_) => (true, 0),
            Self::Prefix(prefix) => (false, prefix.len()),
        }
    }

    /// The entry's name as written in the envelope.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::Exact(exact) => exact.as_str().to_owned(),
            Self::Prefix(prefix) => format!("{prefix}*"),
        }
    }
}

/// The name of an approval policy of `envelope.approvalPolicies`, which an entry's `requiresApproval` names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApprovalPolicyName(String);

impl ApprovalPolicyName {
    /// # Errors
    /// [`EnvelopeError::InvalidName`] unless it follows the IR's name rule (`^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$`, the
    /// rule of an activity name).
    pub fn parse(text: &str) -> Result<Self, EnvelopeError> {
        ActivityName::parse(text)
            .map(|_| Self(text.to_owned()))
            .map_err(|_| EnvelopeError::InvalidName(clip(text)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The publisher's fields an entry owns, each absent when the entry does not set it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublisherFields {
    pub secrets: Option<BTreeSet<SecretName>>,
    pub irreversible: Option<bool>,
    pub high_risk: Option<bool>,
    pub compensate_with: Option<ActivityName>,
    pub reconcile_with: Option<ActivityName>,
    pub requires_approval: Option<ApprovalPolicyName>,
}

/// One entry of `envelope.activities`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityEntry {
    name: EntryName,
    fields: PublisherFields,
}

impl ActivityEntry {
    /// # Errors
    /// [`EnvelopeError::SecretsOnPrefix`] for a prefix entry that lists secrets, and
    /// [`EnvelopeError::TooManySecrets`] beyond [`MAX_ENTRY_SECRETS`].
    pub fn new(name: EntryName, fields: PublisherFields) -> Result<Self, EnvelopeError> {
        if let Some(secrets) = &fields.secrets {
            if matches!(name, EntryName::Prefix(_)) {
                return Err(EnvelopeError::SecretsOnPrefix(name.as_text()));
            }
            if secrets.len() > MAX_ENTRY_SECRETS {
                return Err(EnvelopeError::TooManySecrets(name.as_text()));
            }
        }
        Ok(Self { name, fields })
    }

    #[must_use]
    pub const fn name(&self) -> &EntryName {
        &self.name
    }

    #[must_use]
    pub const fn fields(&self) -> &PublisherFields {
        &self.fields
    }

    /// The secrets a step of this entry may resolve: the exact entry's list, none for a prefix entry.
    #[must_use]
    pub fn secrets(&self) -> Vec<&SecretName> {
        self.fields
            .secrets
            .as_ref()
            .map(|secrets| secrets.iter().collect())
            .unwrap_or_default()
    }
}

/// The activities a code version's commands may schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityEnvelope {
    entries: Vec<ActivityEntry>,
}

impl ActivityEnvelope {
    /// # Errors
    /// [`EnvelopeError::TooManyEntries`] beyond [`MAX_ACTIVITY_ENTRIES`], [`EnvelopeError::DuplicateEntry`] for a name
    /// listed twice, and [`EnvelopeError::RiskNameNotAnEntry`] for a `compensateWith` or `reconcileWith` that no entry
    /// covers (its own entry gives the derived step its secrets).
    pub fn new(entries: Vec<ActivityEntry>) -> Result<Self, EnvelopeError> {
        if entries.len() > MAX_ACTIVITY_ENTRIES {
            return Err(EnvelopeError::TooManyEntries(entries.len()));
        }
        let mut names = BTreeSet::new();
        for entry in &entries {
            if !names.insert(&entry.name) {
                return Err(EnvelopeError::DuplicateEntry(entry.name.as_text()));
            }
        }
        let envelope = Self { entries };
        for entry in &envelope.entries {
            for (member, name) in [
                ("compensateWith", &entry.fields.compensate_with),
                ("reconcileWith", &entry.fields.reconcile_with),
            ] {
                if let Some(name) = name
                    && envelope.covering(name).is_none()
                {
                    return Err(EnvelopeError::RiskNameNotAnEntry {
                        entry: entry.name.as_text(),
                        member,
                        activity: name.as_str().to_owned(),
                    });
                }
            }
        }
        Ok(envelope)
    }

    #[must_use]
    pub fn entries(&self) -> &[ActivityEntry] {
        &self.entries
    }

    /// The one entry that covers `activity`: the exact entry if there is one, otherwise the longest prefix entry.
    #[must_use]
    pub fn covering(&self, activity: &ActivityName) -> Option<&ActivityEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.name.covers(activity))
            .max_by_key(|entry| entry.name.precedence())
    }

    /// Admits an activity command: its covering entry, the fields the fragment repeats checked against it, and the
    /// entries its compensation and its lookup run under.
    ///
    /// # Errors
    /// [`EnvelopeRefusal::NotCovered`] when no entry covers the activity, and [`EnvelopeRefusal::PublisherField`],
    /// naming the member, when the fragment repeats a publisher field the entry does not set or sets otherwise
    /// (`secrets` compared as sets). Both are `COMMAND_OUTSIDE_ENVELOPE` (Technical Plan 6.3).
    pub fn admit(
        &self,
        activity: &ActivityName,
        repeated: &PublisherFields,
    ) -> Result<AdmittedActivity<'_>, EnvelopeRefusal> {
        let entry = self
            .covering(activity)
            .ok_or_else(|| EnvelopeRefusal::NotCovered(activity.as_str().to_owned()))?;
        let owned = &entry.fields;
        let checks = [
            (
                "secrets",
                repeated.secrets.is_some(),
                repeated.secrets == owned.secrets,
            ),
            (
                "irreversible",
                repeated.irreversible.is_some(),
                repeated.irreversible == owned.irreversible,
            ),
            (
                "highRisk",
                repeated.high_risk.is_some(),
                repeated.high_risk == owned.high_risk,
            ),
            (
                "compensateWith",
                repeated.compensate_with.is_some(),
                repeated.compensate_with == owned.compensate_with,
            ),
            (
                "reconcileWith",
                repeated.reconcile_with.is_some(),
                repeated.reconcile_with == owned.reconcile_with,
            ),
            (
                "requiresApproval",
                repeated.requires_approval.is_some(),
                repeated.requires_approval == owned.requires_approval,
            ),
        ];
        if let Some((member, _, _)) = checks
            .iter()
            .find(|(_, present, equal)| *present && !*equal)
        {
            return Err(EnvelopeRefusal::PublisherField {
                activity: activity.as_str().to_owned(),
                member,
            });
        }
        let derived =
            |name: &Option<ActivityName>| name.as_ref().and_then(|name| self.covering(name));
        Ok(AdmittedActivity {
            entry,
            compensation: derived(&owned.compensate_with),
            reconciliation: derived(&owned.reconcile_with),
        })
    }
}

/// An admitted activity command: the entry it runs under and the entries of its derived steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmittedActivity<'a> {
    pub entry: &'a ActivityEntry,
    /// The entry of the `compensateWith` activity, whose secrets the `~c` step resolves.
    pub compensation: Option<&'a ActivityEntry>,
    /// The entry of the `reconcileWith` activity, whose secrets the `~r` step resolves.
    pub reconciliation: Option<&'a ActivityEntry>,
}

/// Why an envelope cannot be published (Task 044-3 maps each to the publication's refusal).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    #[error("an envelope entry is an activity name or a prefix ending in `.*`: {0}")]
    InvalidName(String),
    #[error("an envelope entry may not be under `script.` or `sagat.`: {0}")]
    ReservedName(String),
    #[error("only an exact envelope entry may list secrets: {0}")]
    SecretsOnPrefix(String),
    #[error("an envelope entry lists at most {MAX_ENTRY_SECRETS} secrets: {0}")]
    TooManySecrets(String),
    #[error("an envelope lists at most {MAX_ACTIVITY_ENTRIES} activities, found {0}")]
    TooManyEntries(usize),
    #[error("the envelope lists {0} twice")]
    DuplicateEntry(String),
    #[error("{member} of the envelope entry {entry} names {activity}, which no entry covers")]
    RiskNameNotAnEntry {
        entry: String,
        member: &'static str,
        activity: String,
    },
}

/// Why a command is outside its version's envelope (`COMMAND_OUTSIDE_ENVELOPE`, Technical Plan 6.3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeRefusal {
    #[error("no envelope entry covers the activity {0}")]
    NotCovered(String),
    #[error(
        "the command for {activity} repeats {member} with a value its envelope entry does not give it"
    )]
    PublisherField {
        activity: String,
        member: &'static str,
    },
}

impl EnvelopeRefusal {
    /// The member of the fragment that is outside the envelope: the activity name, or the repeated field.
    #[must_use]
    pub const fn member(&self) -> &'static str {
        match self {
            Self::NotCovered(_) => "activity",
            Self::PublisherField { member, .. } => member,
        }
    }
}

/// A name echoed in an error, cut to the length of a name.
fn clip(text: &str) -> String {
    text.chars().take(130).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> ActivityName {
        ActivityName::parse(text).expect("an activity name")
    }

    fn secrets(names: &[&str]) -> Option<BTreeSet<SecretName>> {
        Some(
            names
                .iter()
                .map(|n| SecretName::parse(n).expect("a secret name"))
                .collect(),
        )
    }

    fn entry(text: &str, fields: PublisherFields) -> ActivityEntry {
        ActivityEntry::new(EntryName::parse(text).expect("an entry name"), fields)
            .expect("an entry")
    }

    /// The envelope of Technical Plan 3.1's example.
    fn envelope() -> ActivityEnvelope {
        ActivityEnvelope::new(vec![
            entry("inventory.*", PublisherFields::default()),
            entry(
                "inventory.reserve",
                PublisherFields {
                    compensate_with: Some(name("inventory.release")),
                    ..PublisherFields::default()
                },
            ),
            entry("inventory.release", PublisherFields::default()),
            entry(
                "inventory.bulk.*",
                PublisherFields {
                    irreversible: Some(true),
                    ..PublisherFields::default()
                },
            ),
            entry(
                "payments.charge",
                PublisherFields {
                    secrets: secrets(&["payments-api"]),
                    high_risk: Some(true),
                    compensate_with: Some(name("payments.refund")),
                    reconcile_with: Some(name("payments.lookup")),
                    requires_approval: Some(
                        ApprovalPolicyName::parse("large-payments").expect("a policy"),
                    ),
                    ..PublisherFields::default()
                },
            ),
            entry(
                "payments.lookup",
                PublisherFields {
                    secrets: secrets(&["payments-read"]),
                    ..PublisherFields::default()
                },
            ),
            entry(
                "payments.refund",
                PublisherFields {
                    secrets: secrets(&["payments-refunds"]),
                    ..PublisherFields::default()
                },
            ),
            entry("payments.*", PublisherFields::default()),
        ])
        .expect("the example envelope")
    }

    #[test]
    fn an_exact_entry_wins_over_a_prefix_entry() {
        let envelope = envelope();
        let covering = |activity: &str| {
            envelope
                .covering(&name(activity))
                .map(|entry| entry.name().as_text())
        };
        assert_eq!(
            covering("inventory.reserve").as_deref(),
            Some("inventory.reserve")
        );
        assert_eq!(covering("inventory.count").as_deref(), Some("inventory.*"));
        assert_eq!(
            covering("inventory.bulk.reserve").as_deref(),
            Some("inventory.bulk.*"),
            "a longer prefix wins over a shorter one"
        );
        assert_eq!(
            covering("inventoryx.count"),
            None,
            "a prefix covers names under its dot only"
        );
        assert_eq!(
            covering("inventory"),
            None,
            "a prefix does not cover its own stem"
        );
        assert_eq!(
            covering("payments.charge").as_deref(),
            Some("payments.charge")
        );
        let reversed = ActivityEnvelope::new(envelope.entries().iter().rev().cloned().collect())
            .expect("the same entries in another order");
        assert_eq!(
            reversed
                .covering(&name("inventory.bulk.reserve"))
                .map(|entry| entry.name().as_text())
                .as_deref(),
            Some("inventory.bulk.*"),
            "the precedence does not depend on the order of the entries"
        );
    }

    #[test]
    fn a_command_outside_the_envelope_is_refused_with_its_member() {
        let envelope = envelope();
        let outside = envelope
            .admit(&name("shipping.book"), &PublisherFields::default())
            .expect_err("no entry covers shipping.book");
        assert_eq!(
            outside,
            EnvelopeRefusal::NotCovered("shipping.book".to_owned())
        );
        assert_eq!(outside.member(), "activity");

        let added = envelope
            .admit(
                &name("inventory.count"),
                &PublisherFields {
                    high_risk: Some(false),
                    ..PublisherFields::default()
                },
            )
            .expect_err("the prefix entry sets no highRisk");
        assert_eq!(
            added.member(),
            "highRisk",
            "a field the entry does not set cannot be added"
        );

        let changed = envelope
            .admit(
                &name("inventory.bulk.reserve"),
                &PublisherFields {
                    irreversible: Some(false),
                    ..PublisherFields::default()
                },
            )
            .expect_err("the entry says irreversible");
        assert_eq!(
            changed.member(),
            "irreversible",
            "a field cannot be changed"
        );

        let gate_dropped = envelope
            .admit(
                &name("payments.charge"),
                &PublisherFields {
                    requires_approval: Some(
                        ApprovalPolicyName::parse("small-payments").expect("a policy"),
                    ),
                    ..PublisherFields::default()
                },
            )
            .expect_err("another approval policy");
        assert_eq!(gate_dropped.member(), "requiresApproval");

        let repeated = envelope
            .admit(
                &name("payments.charge"),
                &PublisherFields {
                    secrets: secrets(&["payments-api"]),
                    high_risk: Some(true),
                    compensate_with: Some(name("payments.refund")),
                    ..PublisherFields::default()
                },
            )
            .expect("repeating the entry's own values is allowed");
        assert_eq!(repeated.entry.name().as_text(), "payments.charge");
    }

    #[test]
    fn an_activity_command_cannot_carry_a_secret_of_another_activity_entry() {
        let envelope = envelope();
        for (activity, carried) in [
            ("payments.capture", &["payments-api"][..]),
            ("payments.charge", &["payments-api", "payments-refunds"][..]),
            ("payments.charge", &["payments-refunds"][..]),
            ("payments.charge", &[][..]),
        ] {
            let refusal = envelope
                .admit(
                    &name(activity),
                    &PublisherFields {
                        secrets: secrets(carried),
                        ..PublisherFields::default()
                    },
                )
                .expect_err("only the covering entry's own secrets, exactly");
            assert_eq!(refusal.member(), "secrets", "{activity} with {carried:?}");
        }
        let admitted = envelope
            .admit(&name("payments.capture"), &PublisherFields::default())
            .expect("covered by payments.*");
        assert!(
            admitted.entry.secrets().is_empty(),
            "a prefix entry releases no secret"
        );
    }

    #[test]
    fn a_compensation_receives_the_secrets_of_its_own_entry() {
        let envelope = envelope();
        let admitted = envelope
            .admit(&name("payments.charge"), &PublisherFields::default())
            .expect("covered");
        let own = |entry: Option<&ActivityEntry>| {
            entry
                .map(|entry| {
                    entry
                        .secrets()
                        .into_iter()
                        .map(|s| s.as_str().to_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        assert_eq!(own(Some(admitted.entry)), ["payments-api"]);
        assert_eq!(
            own(admitted.compensation),
            ["payments-refunds"],
            "the refund runs with its own secrets"
        );
        assert_eq!(
            own(admitted.reconciliation),
            ["payments-read"],
            "the lookup runs with its own secrets"
        );
        let reserve = envelope
            .admit(&name("inventory.reserve"), &PublisherFields::default())
            .expect("covered");
        assert_eq!(
            reserve
                .compensation
                .map(|entry| entry.name().as_text())
                .as_deref(),
            Some("inventory.release")
        );
        assert!(own(reserve.compensation).is_empty());
    }

    #[test]
    fn an_envelope_that_breaks_its_rules_is_refused() {
        assert_eq!(
            ActivityEntry::new(
                EntryName::parse("payments.*").expect("a prefix"),
                PublisherFields {
                    secrets: secrets(&["payments-api"]),
                    ..PublisherFields::default()
                },
            ),
            Err(EnvelopeError::SecretsOnPrefix("payments.*".to_owned()))
        );
        for reserved in [
            "script.run",
            "sagat.workflow:order",
            "sagat.*",
            "script.x.*",
        ] {
            assert!(
                matches!(
                    EntryName::parse(reserved),
                    Err(EnvelopeError::ReservedName(_))
                ),
                "{reserved} is reserved"
            );
        }
        for invalid in ["*", "a.*.*", "", ".*", "a b"] {
            assert!(
                matches!(
                    EntryName::parse(invalid),
                    Err(EnvelopeError::InvalidName(_))
                ),
                "{invalid:?} is not an entry name"
            );
        }
        let dangling = ActivityEnvelope::new(vec![entry(
            "orders.place",
            PublisherFields {
                compensate_with: Some(name("orders.cancel")),
                ..PublisherFields::default()
            },
        )]);
        assert_eq!(
            dangling,
            Err(EnvelopeError::RiskNameNotAnEntry {
                entry: "orders.place".to_owned(),
                member: "compensateWith",
                activity: "orders.cancel".to_owned(),
            })
        );
        let twice = ActivityEnvelope::new(vec![
            entry("orders.place", PublisherFields::default()),
            entry("orders.place", PublisherFields::default()),
        ]);
        assert_eq!(
            twice,
            Err(EnvelopeError::DuplicateEntry("orders.place".to_owned()))
        );
        let many = (0..=MAX_ACTIVITY_ENTRIES)
            .map(|index| entry(&format!("a{index}"), PublisherFields::default()))
            .collect();
        assert_eq!(
            ActivityEnvelope::new(many),
            Err(EnvelopeError::TooManyEntries(MAX_ACTIVITY_ENTRIES + 1))
        );
    }
}
