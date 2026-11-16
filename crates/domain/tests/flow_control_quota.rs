//! The quota resources of feature 022's concurrency gate (T022-013, 0.3.0 slice; FR-022-021, FR-022-034, FR-022-047,
//! D022-024, D022-040): each has the default and the hard ceiling of the plan, a test at and over its bound, and zero
//! refuses (SC-022-011). The effective value is the lowest of the declaration, the namespace and the deployment.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::{activity, id};
use sagat_domain::identity::Slug;
use sagat_domain::quota::{self, QuotaError, QuotaKind, QuotaResource, QuotaScope, QuotaUnit};
use sagat_domain::workflow::flow_control::MAX_QUEUE_TIMEOUT;
use sagat_domain::workflow::{
    ConcurrencyPolicy, FlowControl, FlowControlCeilings, FlowControlLimit, FlowGroup, GateKind,
    WorkflowDefinition,
};

const NEW: [QuotaResource; 6] = [
    QuotaResource::QueuedRuns,
    QuotaResource::ConcurrencyLiveKeys,
    QuotaResource::FlowControlKeyWaiters,
    QuotaResource::ConcurrencyLimit,
    QuotaResource::FlowControlQueueTimeoutMs,
    QuotaResource::FlowControlNodeDeclarations,
];

#[test]
fn each_resource_has_its_name_and_is_found_by_name_once() {
    let names = [
        "queued_runs",
        "concurrency_live_keys",
        "flow_control_key_waiters",
        "concurrency_limit",
        "flow_control_queue_timeout_ms",
        "flow_control_node_declarations",
    ];
    for (resource, name) in NEW.into_iter().zip(names) {
        assert_eq!(resource.as_str(), name);
        assert_eq!(name.parse::<QuotaResource>(), Ok(resource));
        assert_eq!(
            QuotaResource::ALL
                .iter()
                .filter(|r| **r == resource)
                .count(),
            1,
            "{name} is listed once"
        );
        assert_eq!(resource.scope(), QuotaScope::Namespace, "{name}");
        assert!(!resource.is_database(), "{name}");
        assert!(!resource.limit_name().is_empty(), "{name}");
    }
}

/// Counts have a usage the quota view reports; the per-declaration bounds are policies without one.
#[test]
fn the_waiters_and_the_keys_are_counts_and_the_declaration_bounds_are_policies() {
    for count in [
        QuotaResource::QueuedRuns,
        QuotaResource::ConcurrencyLiveKeys,
        QuotaResource::FlowControlKeyWaiters,
    ] {
        assert_eq!(count.kind(), QuotaKind::Count, "{count}");
        assert_eq!(count.unit(), QuotaUnit::Items, "{count}");
        assert!(count.has_usage(), "{count}");
    }
    for policy in [
        QuotaResource::ConcurrencyLimit,
        QuotaResource::FlowControlQueueTimeoutMs,
        QuotaResource::FlowControlNodeDeclarations,
    ] {
        assert_eq!(policy.kind(), QuotaKind::Policy, "{policy}");
        assert!(!policy.has_usage(), "{policy}");
    }
    assert_eq!(
        QuotaResource::FlowControlQueueTimeoutMs.unit(),
        QuotaUnit::Milliseconds
    );
}

/// The design values of the plan ("Finite ceilings", "Ceilings, revised").
#[test]
fn the_defaults_and_the_hard_ceilings_are_those_of_the_plan() {
    let expected = [
        (QuotaResource::QueuedRuns, 1_000, 10_000),
        (QuotaResource::ConcurrencyLiveKeys, 10_000, 100_000),
        (QuotaResource::FlowControlKeyWaiters, 100, 10_000),
        (QuotaResource::ConcurrencyLimit, 10_000, 10_000),
        (
            QuotaResource::FlowControlQueueTimeoutMs,
            86_400_000,
            86_400_000,
        ),
        (QuotaResource::FlowControlNodeDeclarations, 16, 64),
    ];
    for (resource, default, maximum) in expected {
        assert_eq!(resource.default_limit(), default, "{resource}");
        assert_eq!(resource.maximum(), maximum, "{resource}");
        assert!(
            resource.default_ceiling() <= resource.maximum(),
            "{resource}"
        );
    }
    // one key's waiters can never fill the namespace by default (D022-024)
    assert!(
        QuotaResource::FlowControlKeyWaiters.default_limit()
            < QuotaResource::QueuedRuns.default_limit()
    );
}

/// SC-022-011: a value at the hard ceiling is accepted, one over it is refused, for setting and for granting.
#[test]
fn every_ceiling_accepts_its_bound_and_refuses_one_more() {
    for resource in NEW {
        let max = resource.maximum();
        assert_eq!(
            quota::validate(resource, true, max, max),
            Ok(max),
            "{resource} at its bound"
        );
        assert_eq!(
            quota::validate(resource, true, max + 1, max),
            Err(QuotaError::TooLarge),
            "{resource} over its bound"
        );
        assert_eq!(quota::validate_ceiling(resource, max), Ok(max));
        assert_eq!(
            quota::validate_ceiling(resource, max + 1),
            Err(QuotaError::TooLarge)
        );
    }
}

/// Zero is never "unlimited": a count of zero is a valid setting that refuses every new waiter or key, and a policy of
/// zero is refused as a value (nothing could ever be declared under it).
#[test]
fn zero_refuses() {
    for count in [
        QuotaResource::QueuedRuns,
        QuotaResource::ConcurrencyLiveKeys,
        QuotaResource::FlowControlKeyWaiters,
    ] {
        assert_eq!(quota::validate(count, true, 0, count.maximum()), Ok(0));
    }
    for policy in [
        QuotaResource::ConcurrencyLimit,
        QuotaResource::FlowControlQueueTimeoutMs,
        QuotaResource::FlowControlNodeDeclarations,
    ] {
        assert_eq!(
            quota::validate(policy, true, 0, policy.maximum()),
            Err(QuotaError::OutsideRange),
            "{policy}"
        );
    }
    let closed = FlowControlCeilings {
        queued_runs: 0,
        live_keys: 0,
        key_waiters: 0,
        ..FlowControlCeilings::DEFAULT
    };
    assert_eq!(
        closed.admits_waiter(0, 0, false),
        Err(FlowControlLimit::QueuedRuns)
    );
    assert_eq!(closed.admits_key(0), Err(FlowControlLimit::LiveKeys));
}

/// The namespace's counts judged against their ceilings, after this transaction's own rows: one below the bound
/// admits, at the bound refuses (the new waiter or key would be one more).
#[test]
fn a_waiter_and_a_key_are_admitted_below_their_ceilings_and_refused_at_them() {
    let ceilings = FlowControlCeilings {
        queued_runs: 3,
        live_keys: 2,
        key_waiters: 2,
        ..FlowControlCeilings::DEFAULT
    };
    assert_eq!(ceilings.admits_waiter(2, 1, false), Ok(()));
    assert_eq!(
        ceilings.admits_waiter(3, 1, false),
        Err(FlowControlLimit::QueuedRuns)
    );
    assert_eq!(
        ceilings.admits_waiter(3, 1, true),
        Ok(()),
        "a run that waits at its other gate is counted once"
    );
    assert_eq!(
        ceilings.admits_waiter(2, 2, false),
        Err(FlowControlLimit::KeyWaiters)
    );
    assert_eq!(ceilings.admits_key(1), Ok(()));
    assert_eq!(ceilings.admits_key(2), Err(FlowControlLimit::LiveKeys));
}

/// FR-022-034: effective = min(declaration, namespace, deployment). The namespace value already holds the deployment's
/// ceiling (`quota::resolve`), so the gate takes the lower of the declaration and the resolved value.
#[test]
fn the_effective_limit_and_timeout_are_the_lowest_of_the_declaration_and_the_namespace() {
    let declared =
        ConcurrencyPolicy::new(FlowGroup::Derived, None, 50, Duration::from_millis(600_000))
            .expect("valid");
    let open = FlowControlCeilings::DEFAULT;
    assert_eq!(open.concurrency_limit(&declared), 50);
    assert_eq!(
        open.queue_timeout(declared.queue_timeout()),
        Duration::from_millis(600_000)
    );
    let lowered = FlowControlCeilings {
        concurrency_limit: 7,
        queue_timeout_ms: 1_000,
        ..FlowControlCeilings::DEFAULT
    };
    assert_eq!(lowered.concurrency_limit(&declared), 7);
    assert_eq!(
        lowered.queue_timeout(declared.queue_timeout()),
        Duration::from_millis(1_000)
    );
    // a namespace value above what any declaration may hold never raises it
    let raised = FlowControlCeilings {
        concurrency_limit: u64::MAX,
        queue_timeout_ms: u64::MAX,
        ..FlowControlCeilings::DEFAULT
    };
    assert_eq!(raised.concurrency_limit(&declared), 50);
    assert_eq!(raised.queue_timeout(MAX_QUEUE_TIMEOUT), MAX_QUEUE_TIMEOUT);
}

#[test]
fn the_defaults_are_the_resolved_defaults_of_the_resources() {
    let d = FlowControlCeilings::DEFAULT;
    assert_eq!(d.queued_runs, QuotaResource::QueuedRuns.default_limit());
    assert_eq!(
        d.live_keys,
        QuotaResource::ConcurrencyLiveKeys.default_limit()
    );
    assert_eq!(
        d.key_waiters,
        QuotaResource::FlowControlKeyWaiters.default_limit()
    );
    assert_eq!(
        d.concurrency_limit,
        QuotaResource::ConcurrencyLimit.default_limit()
    );
    assert_eq!(
        d.queue_timeout_ms,
        QuotaResource::FlowControlQueueTimeoutMs.default_limit()
    );
    assert_eq!(
        d.node_declarations,
        QuotaResource::FlowControlNodeDeclarations.default_limit()
    );
}

fn concurrency_nodes(count: usize) -> WorkflowDefinition {
    let nodes: Vec<_> = (0..count)
        .map(|i| {
            let policy =
                ConcurrencyPolicy::new(FlowGroup::Derived, None, 1, Duration::from_secs(60))
                    .expect("valid");
            let node = activity(&format!("n{i}"))
                .with_flow_control(FlowControl::new(Some(policy), None))
                .expect("gated");
            if i + 1 < count {
                node.with_next(id(&format!("n{}", i + 1)))
            } else {
                node
            }
        })
        .collect();
    WorkflowDefinition::new(
        Slug::parse("many-gates").expect("slug"),
        nodes[0].id().clone(),
        nodes,
    )
    .expect("a definition")
}

/// 16 node-level declarations of one kind by default; a namespace may raise it to the domain's 64.
#[test]
fn a_definition_holds_the_namespace_ceiling_of_node_declarations_and_not_one_more() {
    let default = FlowControlCeilings::DEFAULT;
    assert_eq!(default.admits_declarations(&concurrency_nodes(16)), Ok(()));
    assert_eq!(
        default.admits_declarations(&concurrency_nodes(17)),
        Err(FlowControlLimit::NodeDeclarations {
            kind: GateKind::Concurrency,
            max: 16
        })
    );
    let raised = FlowControlCeilings {
        node_declarations: 64,
        ..FlowControlCeilings::DEFAULT
    };
    assert_eq!(raised.admits_declarations(&concurrency_nodes(64)), Ok(()));
    let one = FlowControlCeilings {
        node_declarations: 1,
        ..FlowControlCeilings::DEFAULT
    };
    assert_eq!(one.admits_declarations(&concurrency_nodes(1)), Ok(()));
    assert!(one.admits_declarations(&concurrency_nodes(2)).is_err());
}

/// Correctness review F5, architecture review m5 (2026-11-16): one key's run waiters stay below the namespace's
/// `queued_runs` whatever an administrator sets, so one hot key cannot take every slot of the namespace.
#[test]
fn one_key_never_holds_every_waiting_run_of_the_namespace() {
    let lowered = FlowControlCeilings {
        queued_runs: 50,
        key_waiters: 100,
        ..FlowControlCeilings::DEFAULT
    };
    assert_eq!(lowered.effective_key_waiters(), 49);
    assert_eq!(lowered.admits_waiter(48, 48, false), Ok(()));
    assert_eq!(
        lowered.admits_waiter(49, 49, false),
        Err(FlowControlLimit::KeyWaiters)
    );
    assert_eq!(
        FlowControlCeilings::DEFAULT.effective_key_waiters(),
        100,
        "the defaults already hold it"
    );
}
