//! Closed telemetry vocabulary of the lake and the step-output destinations (feature 035, FR-035-030, NFR-035-OBS).
//!
//! The plan lists the metric families and their labels. Every label takes its values from a small closed set, and none
//! takes a connection, a table, a run, a tenant or any other identifier: those belong in a span or in the audit, never
//! in a series, because a series per table or per run is a leak of memory and of information.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use sagat_telemetry::{
    BrowseOutcome, DestinationKind, DestinationOutcome, LakeCatalogType, LakeLookup, LakeMetric,
    LakeMode, LakePhase, LakeRelayDenial, LakeSandboxExit, LakeStorageType, LakeWriteOutcome,
    Metrics, lake,
};

fn names<T: Copy>(all: &[T], name: impl Fn(T) -> &'static str) -> Vec<&'static str> {
    all.iter().copied().map(name).collect()
}

/// Every closed set, by label key: the values a series of that key may carry.
fn vocabulary() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        ("mode", names(&LakeMode::ALL, LakeMode::as_str)),
        (
            "catalog_type",
            names(&LakeCatalogType::ALL, LakeCatalogType::as_str),
        ),
        (
            "storage_type",
            names(&LakeStorageType::ALL, LakeStorageType::as_str),
        ),
        ("phase", names(&LakePhase::ALL, LakePhase::as_str)),
        ("result", names(&LakeLookup::ALL, LakeLookup::as_str)),
        (
            "kind",
            names(&DestinationKind::ALL, DestinationKind::as_str),
        ),
    ])
}

#[test]
fn the_metric_names_and_label_keys_are_those_of_the_plan() {
    let plan = [
        (
            LakeMetric::Writes,
            "lake_writes_total",
            &["mode", "catalog_type", "storage_type", "outcome"][..],
        ),
        (
            LakeMetric::WritePhaseSeconds,
            "lake_write_phase_seconds",
            &["phase"][..],
        ),
        (LakeMetric::Lookups, "lake_lookups_total", &["result"][..]),
        (
            LakeMetric::CommitConflicts,
            "lake_commit_conflicts_total",
            &[][..],
        ),
        (
            LakeMetric::SandboxExits,
            "lake_sandbox_exits_total",
            &["reason"][..],
        ),
        (
            LakeMetric::RelayDenials,
            "lake_relay_denials_total",
            &["reason"][..],
        ),
        (
            LakeMetric::DestinationWrites,
            "destination_writes_total",
            &["kind", "outcome"][..],
        ),
        (
            LakeMetric::DestinationHeld,
            "destination_held_total",
            &["kind"][..],
        ),
        (
            LakeMetric::CatalogBrowses,
            "database_catalog_browses_total",
            &["outcome"][..],
        ),
    ];
    assert_eq!(
        plan.len(),
        LakeMetric::ALL.len(),
        "a metric of the plan is not listed here"
    );
    for (metric, name, keys) in plan {
        assert_eq!(metric.as_str(), name);
        assert_eq!(metric.label_keys(), keys, "{name}");
        for forbidden in [
            "connection",
            "table",
            "namespace",
            "run",
            "step",
            "workflow",
            "tenant",
            "actor",
            "subject",
            "host",
            "bucket",
            "snapshot",
            "effect_key",
            "path",
            "file",
            "error",
        ] {
            assert!(
                metric
                    .label_keys()
                    .iter()
                    .all(|key| !key.contains(forbidden)),
                "{name} has a label that can carry an identifier: {forbidden}"
            );
        }
    }
}

#[test]
fn every_label_takes_its_values_from_a_small_closed_set() {
    let mut sets = vocabulary();
    sets.insert(
        "write outcome",
        names(&LakeWriteOutcome::ALL, LakeWriteOutcome::as_str),
    );
    sets.insert(
        "sandbox reason",
        names(&LakeSandboxExit::ALL, LakeSandboxExit::as_str),
    );
    sets.insert(
        "relay reason",
        names(&LakeRelayDenial::ALL, LakeRelayDenial::as_str),
    );
    sets.insert(
        "destination outcome",
        names(&DestinationOutcome::ALL, DestinationOutcome::as_str),
    );
    sets.insert(
        "browse outcome",
        names(&BrowseOutcome::ALL, BrowseOutcome::as_str),
    );

    for (label, values) in sets {
        assert!(
            !values.is_empty() && values.len() <= 12,
            "{label}: {} values",
            values.len()
        );
        assert_eq!(
            values.iter().collect::<BTreeSet<_>>().len(),
            values.len(),
            "{label} repeats a value"
        );
        for value in values {
            assert!(
                value.len() <= 24
                    && value
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    && value.starts_with(|c: char| c.is_ascii_lowercase()),
                "{label}: {value:?} is not a short snake_case name"
            );
        }
    }
}

/// The sets are decisions of the feature, pinned so that a value cannot be added without a reviewed change.
#[test]
fn the_closed_sets_are_the_ones_the_feature_decided() {
    let sets = vocabulary();
    assert_eq!(sets["mode"], ["append", "overwrite", "upsert"]);
    assert_eq!(sets["catalog_type"], ["rest", "glue", "hive", "jdbc"]);
    assert_eq!(sets["storage_type"], ["s3", "gcs", "adls"]);
    assert_eq!(
        sets["phase"],
        ["sandbox", "load", "lookup", "write", "commit"]
    );
    assert_eq!(sets["result"], ["found", "absent", "inconclusive"]);
    assert_eq!(
        sets["kind"],
        [
            "iceberg",
            "postgresql",
            "object_storage",
            "clickhouse",
            "broker_stream"
        ]
    );
    assert_eq!(
        names(&LakeWriteOutcome::ALL, LakeWriteOutcome::as_str),
        [
            "committed",
            "empty_no_commit",
            "reverted",
            "failed",
            "unknown"
        ]
    );
    // How a sandbox container can end (contracts/lake-runtime.md): the host answered, or something ended it first.
    assert_eq!(
        names(&LakeSandboxExit::ALL, LakeSandboxExit::as_str),
        [
            "completed",
            "cancelled",
            "lease_lost",
            "deadline",
            "heartbeat_missed",
            "memory_killed",
            "start_failed",
            "protocol_error"
        ]
    );
    // What the relay can refuse (contracts/lake-runtime.md, "The relay").
    assert_eq!(
        names(&LakeRelayDenial::ALL, LakeRelayDenial::as_str),
        [
            "endpoint_not_declared",
            "address_forbidden",
            "resolution_failed",
            "token_refused",
            "request_unsupported"
        ]
    );
    assert_eq!(
        names(&DestinationOutcome::ALL, DestinationOutcome::as_str),
        [
            "committed",
            "created",
            "already_present",
            "inserted",
            "delivered",
            "failed_continued",
            "skipped",
            "failed",
            "unknown"
        ]
    );
    assert_eq!(
        names(&BrowseOutcome::ALL, BrowseOutcome::as_str),
        ["done", "failed", "refused"]
    );
}

/// Records every combination through the typed recorders and reads the Prometheus page back: each series carries
/// exactly the label keys of its family, with values from the closed sets, and the phase latency is a histogram.
#[test]
fn what_the_recorders_emit_uses_only_the_declared_keys_and_values() {
    let (metrics, recorder) = Metrics::detached(Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        for mode in LakeMode::ALL {
            for catalog in LakeCatalogType::ALL {
                for storage in LakeStorageType::ALL {
                    for outcome in LakeWriteOutcome::ALL {
                        lake::record_write(mode, catalog, storage, outcome);
                    }
                }
            }
        }
        for phase in LakePhase::ALL {
            lake::record_phase(phase, 0.25);
        }
        for result in LakeLookup::ALL {
            lake::record_lookup(result);
        }
        lake::record_commit_conflict();
        for reason in LakeSandboxExit::ALL {
            lake::record_sandbox_exit(reason);
        }
        for reason in LakeRelayDenial::ALL {
            lake::record_relay_denial(reason);
        }
        for kind in DestinationKind::ALL {
            lake::record_destination_held(kind);
            for outcome in DestinationOutcome::ALL {
                lake::record_destination_write(kind, outcome);
            }
        }
        for outcome in BrowseOutcome::ALL {
            lake::record_catalog_browse(outcome);
        }
    });
    let page = metrics.render();

    let allowed: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::from([
        (
            "mode",
            names(&LakeMode::ALL, LakeMode::as_str)
                .into_iter()
                .collect(),
        ),
        (
            "catalog_type",
            names(&LakeCatalogType::ALL, LakeCatalogType::as_str)
                .into_iter()
                .collect(),
        ),
        (
            "storage_type",
            names(&LakeStorageType::ALL, LakeStorageType::as_str)
                .into_iter()
                .collect(),
        ),
        (
            "phase",
            names(&LakePhase::ALL, LakePhase::as_str)
                .into_iter()
                .collect(),
        ),
        (
            "result",
            names(&LakeLookup::ALL, LakeLookup::as_str)
                .into_iter()
                .collect(),
        ),
        (
            "kind",
            names(&DestinationKind::ALL, DestinationKind::as_str)
                .into_iter()
                .collect(),
        ),
    ]);
    let outcomes: BTreeSet<&str> = names(&LakeWriteOutcome::ALL, LakeWriteOutcome::as_str)
        .into_iter()
        .chain(names(&DestinationOutcome::ALL, DestinationOutcome::as_str))
        .chain(names(&BrowseOutcome::ALL, BrowseOutcome::as_str))
        .collect();
    let reasons: BTreeSet<&str> = names(&LakeSandboxExit::ALL, LakeSandboxExit::as_str)
        .into_iter()
        .chain(names(&LakeRelayDenial::ALL, LakeRelayDenial::as_str))
        .collect();

    let mut seen = BTreeSet::new();
    for line in page.lines().filter(|line| !line.starts_with('#')) {
        let Some(metric) = LakeMetric::ALL.iter().find(|metric| {
            line.starts_with(metric.as_str())
                && line[metric.as_str().len()..].starts_with(['{', '_', ' '])
        }) else {
            continue;
        };
        seen.insert(*metric);
        let labels: Vec<(&str, &str)> = line
            .split_once('{')
            .map(|(_, rest)| rest.split_once('}').map_or("", |(inside, _)| inside))
            .unwrap_or("")
            .split(',')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (key, value) = pair.split_once('=').expect("key=value");
                (key, value.trim_matches('"'))
            })
            .filter(|(key, _)| *key != "le") // histogram bucket bound, not a label of ours
            .collect();
        let keys: BTreeSet<&str> = labels.iter().map(|(key, _)| *key).collect();
        let expected: BTreeSet<&str> = metric.label_keys().iter().copied().collect();
        assert_eq!(keys, expected, "{line}");
        for (key, value) in labels {
            let fine = match key {
                "outcome" => outcomes.contains(value),
                "reason" => reasons.contains(value),
                other => allowed[other].contains(value),
            };
            assert!(fine, "{line}: {value:?} is not in the closed set of {key}");
        }
    }
    assert_eq!(
        seen.len(),
        LakeMetric::ALL.len(),
        "a family never reached the page: {seen:?}"
    );
    assert!(
        page.contains("lake_write_phase_seconds_bucket"),
        "the phase latency must be a histogram:\n{page}"
    );
}
