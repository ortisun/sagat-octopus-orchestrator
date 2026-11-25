//! Closed telemetry vocabulary of flow control (feature 022, T022-014, T022-082 in part; FR-022-033, FR-022-049): the
//! metric families and label keys of the plan, each label from a closed set, and no series that carries a tenant, a
//! run, a group or a key.

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_telemetry::{
    ConcurrencyOutcome, FlowControlMetric, FlowControlScope, FlowControlSpan, Metrics,
    RateLimitOutcome, flow_control,
};

#[test]
fn the_metric_names_and_label_keys_are_those_of_the_plan() {
    let plan: [(FlowControlMetric, &str, &[&str]); 8] = [
        (
            FlowControlMetric::ConcurrencyAcquisitions,
            "concurrency_acquisitions_total",
            &["scope", "outcome"],
        ),
        (
            FlowControlMetric::ConcurrencyWaitSeconds,
            "concurrency_wait_seconds",
            &["scope"],
        ),
        (
            FlowControlMetric::ConcurrencyWaiters,
            "concurrency_waiters",
            &["scope"],
        ),
        (
            FlowControlMetric::RateLimitDecisions,
            "rate_limit_decisions_total",
            &["scope", "outcome"],
        ),
        (
            FlowControlMetric::RateLimitWaitSeconds,
            "rate_limit_wait_seconds",
            &["scope"],
        ),
        (
            FlowControlMetric::RateLimitWaiters,
            "rate_limit_waiters",
            &["scope"],
        ),
        (
            FlowControlMetric::RateLimitGrantLagSeconds,
            "rate_limit_grant_lag_seconds",
            &[],
        ),
        (
            FlowControlMetric::RateLimitUnleasedGrants,
            "rate_limit_unleased_grants",
            &[],
        ),
    ];
    assert_eq!(plan.len(), FlowControlMetric::ALL.len());
    for (metric, name, keys) in plan {
        assert_eq!(metric.as_str(), name);
        assert_eq!(metric.label_keys(), keys, "{name}");
    }
    assert_eq!(
        FlowControlScope::ALL.map(FlowControlScope::as_str),
        ["run", "step"]
    );
    assert_eq!(
        ConcurrencyOutcome::ALL.map(ConcurrencyOutcome::as_str),
        [
            "acquired",
            "queued",
            "refused_full",
            "refused_reset",
            "granted",
            "skipped_ahead",
            "timed_out",
            "dequeued"
        ]
    );
    assert_eq!(
        RateLimitOutcome::ALL.map(RateLimitOutcome::as_str),
        [
            "granted",
            "queued",
            "granted_from_queue",
            "refused_full",
            "refused_reset",
            "skipped_ahead",
            "timed_out",
            "dequeued",
            "slot_returned"
        ]
    );
    assert_eq!(
        FlowControlSpan::ALL.map(FlowControlSpan::as_str),
        [
            "workflow.concurrency.acquire",
            "workflow.concurrency.grant",
            "workflow.rate_limit.admit",
            "workflow.rate_limit.grant"
        ]
    );
    for span in FlowControlSpan::ALL {
        assert!(
            span.group_attribute().ends_with(".group"),
            "a span names a group, never a key"
        );
    }
}

/// Every series the recorders write carries only the keys of its family, each from its closed set: a label that is not
/// in the vocabulary (a tenant, a run, a group, a key) fails here.
#[test]
fn every_recorded_series_uses_only_the_closed_vocabulary() {
    let (telemetry, recorder) = Metrics::detached(Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        for scope in FlowControlScope::ALL {
            for outcome in ConcurrencyOutcome::ALL {
                flow_control::record_concurrency(scope, outcome);
            }
            for outcome in RateLimitOutcome::ALL {
                flow_control::record_rate_limit(scope, outcome);
            }
            flow_control::record_concurrency_wait(scope, 0.5);
            flow_control::record_rate_limit_wait(scope, 0.5);
            flow_control::set_concurrency_waiters(scope, 3.0);
            flow_control::set_rate_limit_waiters(scope, 3.0);
        }
        flow_control::record_grant_lag(0.25);
        flow_control::set_unleased_grants(1.0);
    });
    let rendered = telemetry.render();
    let (seen, violations) = check(&rendered);
    assert!(violations.is_empty(), "{violations:?}");
    assert_eq!(seen.len(), FlowControlMetric::ALL.len(), "{rendered}");
}

/// The families a rendered page holds and every label on it that is not in the closed vocabulary: a key that is not a
/// label of its family (a tenant, a run, a group, a key), or a value outside its closed set.
fn check(rendered: &str) -> (BTreeSet<&'static str>, Vec<String>) {
    let mut allowed = BTreeSet::new();
    for scope in FlowControlScope::ALL {
        allowed.insert(format!("scope=\"{}\"", scope.as_str()));
    }
    for outcome in ConcurrencyOutcome::ALL
        .map(ConcurrencyOutcome::as_str)
        .into_iter()
        .chain(RateLimitOutcome::ALL.map(RateLimitOutcome::as_str))
    {
        allowed.insert(format!("outcome=\"{outcome}\""));
    }
    let mut seen = BTreeSet::new();
    let mut violations = Vec::new();
    for line in rendered
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let name = line.split(['{', ' ']).next().expect("a name");
        let Some(family) = FlowControlMetric::ALL
            .into_iter()
            .find(|metric| name.starts_with(metric.as_str()))
        else {
            violations.push(format!("a series outside the vocabulary: {line}"));
            continue;
        };
        seen.insert(family.as_str());
        if let Some(labels) = line
            .split_once('{')
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(l, _)| l)
        {
            for label in labels
                .split(',')
                .filter(|label| !label.starts_with("le=") && !label.starts_with("quantile="))
            {
                let key = label.split('=').next().expect("a key");
                if !family.label_keys().contains(&key) {
                    violations.push(format!(
                        "{key} is not a label of {}: {line}",
                        family.as_str()
                    ));
                } else if !allowed.contains(label) {
                    violations.push(format!("{label} is not in the closed set: {line}"));
                }
            }
        }
    }
    (seen, violations)
}

/// T022-082: the check refuses what the vocabulary does not hold: a group or a tenant as a label of a family, a scope
/// outside its closed set, a label on a family that has none, and a series of no family of the vocabulary.
#[test]
fn the_vocabulary_check_rejects_unknown_labels_and_values() {
    let (telemetry, recorder) = Metrics::detached(Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        metrics::gauge!("rate_limit_waiters", "scope" => "run", "group" => "accounts").set(1.0);
        metrics::gauge!("concurrency_waiters", "scope" => "tenant").set(1.0);
        metrics::histogram!("rate_limit_grant_lag_seconds", "tenant" => "t-1").record(0.5);
        metrics::counter!("rate_limit_keys_total").increment(1);
    });
    let (_, violations) = check(&telemetry.render());
    for expected in [
        "group is not a label of rate_limit_waiters",
        "scope=\"tenant\" is not in the closed set",
        "tenant is not a label of rate_limit_grant_lag_seconds",
        "a series outside the vocabulary: rate_limit_keys_total",
    ] {
        assert!(
            violations.iter().any(|v| v.contains(expected)),
            "{expected} was not refused: {violations:?}"
        );
    }
}
