//! Metrics of the Iceberg write steps and of the step-output destinations (feature 035, FR-035-030).
//!
//! Every label takes its values from a closed set defined here, and the recorders accept those types and nothing else,
//! so a table, a connection, a run or a tenant cannot become a series: those go in a span or in the audit. The sets are
//! pinned by `tests/lake_vocabulary.rs`; adding a value is a reviewed change.

/// A closed set of label values: the enum, the list of its variants (built from the same rows, so it cannot miss one)
/// and the text each one has on the page.
macro_rules! closed {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident => $text:literal,)+ }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vdoc])* $variant,)+
        }
        impl $name {
            pub const ALL: [Self; [$(Self::$variant),+].len()] = [$(Self::$variant),+];

            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }
        }
    };
}

closed! {
    /// The write mode of a step (the `mode` label).
    LakeMode {
        Append => "append",
        Overwrite => "overwrite",
        Upsert => "upsert",
    }
}
closed! {
    /// The catalog type of the connection (the `catalog_type` label).
    LakeCatalogType {
        Rest => "rest",
        Glue => "glue",
        Hive => "hive",
        Jdbc => "jdbc",
    }
}
closed! {
    /// The storage type of the connection (the `storage_type` label).
    LakeStorageType {
        S3 => "s3",
        Gcs => "gcs",
        Adls => "adls",
    }
}
closed! {
    /// How a write attempt ended. `Unknown` is an inconclusive outcome, which is never repeated blindly.
    LakeWriteOutcome {
        Committed => "committed",
        EmptyNoCommit => "empty_no_commit",
        Reverted => "reverted",
        Failed => "failed",
        Unknown => "unknown",
    }
}
closed! {
    /// A phase of a write attempt (the `phase` label of the latency histogram).
    LakePhase {
        Sandbox => "sandbox",
        Load => "load",
        Lookup => "lookup",
        Write => "write",
        Commit => "commit",
    }
}
closed! {
    /// The result of the effect-key lookup in the snapshot history (D035-030).
    LakeLookup {
        Found => "found",
        Absent => "absent",
        Inconclusive => "inconclusive",
    }
}
closed! {
    /// How a sandbox container ended (contracts/lake-runtime.md): the host answered, or something ended it first.
    LakeSandboxExit {
        Completed => "completed",
        Cancelled => "cancelled",
        LeaseLost => "lease_lost",
        Deadline => "deadline",
        HeartbeatMissed => "heartbeat_missed",
        MemoryKilled => "memory_killed",
        StartFailed => "start_failed",
        ProtocolError => "protocol_error",
    }
}
closed! {
    /// What the relay refused (contracts/lake-runtime.md, "The relay"); the value of the destination is never a label.
    LakeRelayDenial {
        EndpointNotDeclared => "endpoint_not_declared",
        AddressForbidden => "address_forbidden",
        ResolutionFailed => "resolution_failed",
        TokenRefused => "token_refused",
        RequestUnsupported => "request_unsupported",
    }
}
closed! {
    /// The kind of a step-output destination.
    DestinationKind {
        Iceberg => "iceberg",
        Postgresql => "postgresql",
        ObjectStorage => "object_storage",
        Clickhouse => "clickhouse",
        BrokerStream => "broker_stream",
    }
}
closed! {
    /// How a destination write ended: the outcomes of the receipts, and the two that have none.
    DestinationOutcome {
        Committed => "committed",
        Created => "created",
        AlreadyPresent => "already_present",
        Inserted => "inserted",
        Delivered => "delivered",
        FailedContinued => "failed_continued",
        Skipped => "skipped",
        Failed => "failed",
        Unknown => "unknown",
    }
}
closed! {
    /// How a catalog browse ended.
    BrowseOutcome {
        Done => "done",
        Failed => "failed",
        /// Refused before it ran: permission, another tenant's connection, a limit.
        Refused => "refused",
    }
}

/// The metric families and their only permitted label keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LakeMetric {
    Writes,
    WritePhaseSeconds,
    Lookups,
    CommitConflicts,
    SandboxExits,
    RelayDenials,
    DestinationWrites,
    DestinationHeld,
    CatalogBrowses,
}

impl LakeMetric {
    pub const ALL: [Self; 9] = [
        Self::Writes,
        Self::WritePhaseSeconds,
        Self::Lookups,
        Self::CommitConflicts,
        Self::SandboxExits,
        Self::RelayDenials,
        Self::DestinationWrites,
        Self::DestinationHeld,
        Self::CatalogBrowses,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Writes => "lake_writes_total",
            Self::WritePhaseSeconds => "lake_write_phase_seconds",
            Self::Lookups => "lake_lookups_total",
            Self::CommitConflicts => "lake_commit_conflicts_total",
            Self::SandboxExits => "lake_sandbox_exits_total",
            Self::RelayDenials => "lake_relay_denials_total",
            Self::DestinationWrites => "destination_writes_total",
            Self::DestinationHeld => "destination_held_total",
            Self::CatalogBrowses => "database_catalog_browses_total",
        }
    }

    #[must_use]
    pub const fn label_keys(self) -> &'static [&'static str] {
        match self {
            Self::Writes => &["mode", "catalog_type", "storage_type", "outcome"],
            Self::WritePhaseSeconds => &["phase"],
            Self::Lookups => &["result"],
            Self::CommitConflicts => &[],
            Self::SandboxExits | Self::RelayDenials => &["reason"],
            Self::DestinationWrites => &["kind", "outcome"],
            Self::DestinationHeld => &["kind"],
            Self::CatalogBrowses => &["outcome"],
        }
    }
}

pub fn record_write(
    mode: LakeMode,
    catalog: LakeCatalogType,
    storage: LakeStorageType,
    outcome: LakeWriteOutcome,
) {
    metrics::counter!(
        LakeMetric::Writes.as_str(),
        "mode" => mode.as_str(),
        "catalog_type" => catalog.as_str(),
        "storage_type" => storage.as_str(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

pub fn record_phase(phase: LakePhase, seconds: f64) {
    metrics::histogram!(LakeMetric::WritePhaseSeconds.as_str(), "phase" => phase.as_str())
        .record(seconds);
}

pub fn record_lookup(result: LakeLookup) {
    metrics::counter!(LakeMetric::Lookups.as_str(), "result" => result.as_str()).increment(1);
}

pub fn record_commit_conflict() {
    metrics::counter!(LakeMetric::CommitConflicts.as_str()).increment(1);
}

pub fn record_sandbox_exit(reason: LakeSandboxExit) {
    metrics::counter!(LakeMetric::SandboxExits.as_str(), "reason" => reason.as_str()).increment(1);
}

pub fn record_relay_denial(reason: LakeRelayDenial) {
    metrics::counter!(LakeMetric::RelayDenials.as_str(), "reason" => reason.as_str()).increment(1);
}

pub fn record_destination_write(kind: DestinationKind, outcome: DestinationOutcome) {
    metrics::counter!(
        LakeMetric::DestinationWrites.as_str(),
        "kind" => kind.as_str(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

pub fn record_destination_held(kind: DestinationKind) {
    metrics::counter!(LakeMetric::DestinationHeld.as_str(), "kind" => kind.as_str()).increment(1);
}

pub fn record_catalog_browse(outcome: BrowseOutcome) {
    metrics::counter!(LakeMetric::CatalogBrowses.as_str(), "outcome" => outcome.as_str())
        .increment(1);
}
