//! Invariant tests for the workflow definition model (T002-004; FR-002-001, FR-002-005).
#![allow(clippy::expect_used)]

use std::time::Duration;

use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ActivityName, BackoffStrategy, DefinitionWarning, Digest, EventType, Expression,
    ExpressionViolation, JoinPolicy, MAX_BRANCHES, MAX_NODES, MAX_RETRY_ATTEMPTS, MAX_WAIT, Node,
    NodeId, NodeKind, RetryPolicy, RetryViolation, WorkflowDefinition, WorkflowError,
};

fn id(s: &str) -> NodeId {
    NodeId::parse(s).expect("valid node id")
}

fn expr(s: &str) -> Expression {
    Expression::parse(s).expect("valid expression")
}

fn activity(node: &str) -> Node {
    Node::new(
        id(node),
        NodeKind::Activity {
            activity: ActivityName::parse(&format!("act.{node}")).expect("name"),
            retry: RetryPolicy::DEFAULT,
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        },
    )
}

fn timer(node: &str, millis: u64) -> Node {
    Node::new(
        id(node),
        NodeKind::Timer {
            delay: Duration::from_millis(millis),
        },
    )
}

fn parallel(node: &str, branches: &[&str], join: JoinPolicy) -> Node {
    Node::new(
        id(node),
        NodeKind::Parallel {
            branches: branches.iter().map(|b| id(b)).collect(),
            join,
        },
    )
}

fn end(node: &str) -> Node {
    Node::new(id(node), NodeKind::End { output: None })
}

fn build(entry: &str, nodes: Vec<Node>) -> Result<WorkflowDefinition, WorkflowError> {
    WorkflowDefinition::new(Slug::parse("demo-flow").expect("slug"), id(entry), nodes)
}

// ------------------------------------------------------------ names and values

#[test]
fn node_ids_are_path_safe_and_bounded() {
    for ok in ["a", "validate", "wait_confirmation", "step-1", "v1.2", "A9"] {
        assert!(NodeId::parse(ok).is_ok(), "{ok}");
    }
    let too_long = "a".repeat(65);
    for bad in [
        "",
        "-x",
        "_x",
        ".x",
        "a/b",
        "a b",
        "a\n",
        "é",
        too_long.as_str(),
    ] {
        assert!(NodeId::parse(bad).is_err(), "{bad:?}");
    }
    assert!(NodeId::parse(&"a".repeat(64)).is_ok());
}

#[test]
fn activity_and_event_names_allow_namespacing_but_not_separators() {
    for ok in ["payment.validate", "limit.reserve", "a:b", "x-y_z"] {
        assert!(ActivityName::parse(ok).is_ok(), "{ok}");
        assert!(EventType::parse(ok).is_ok(), "{ok}");
    }
    for bad in ["", ".x", "a/b", "a b", &"a".repeat(129)] {
        assert!(ActivityName::parse(bad).is_err(), "{bad:?}");
        assert!(EventType::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn rejected_names_are_clipped_in_the_error() {
    let long = format!("{}/", "x".repeat(500));
    let message = NodeId::parse(&long).expect_err("invalid").to_string();
    assert!(
        message.len() < 120,
        "unbounded echo: {} bytes",
        message.len()
    );
}

#[test]
fn expressions_are_bounded_text() {
    assert!(Expression::parse("input.paymentId").is_ok());
    assert_eq!(
        Expression::parse("   "),
        Err(WorkflowError::InvalidExpression(ExpressionViolation::Empty))
    );
    assert_eq!(
        Expression::parse(&"x".repeat(2049)),
        Err(WorkflowError::InvalidExpression(
            ExpressionViolation::TooLong
        ))
    );
    assert_eq!(
        Expression::parse("a\u{0}b"),
        Err(WorkflowError::InvalidExpression(
            ExpressionViolation::ControlCharacter
        ))
    );
}

#[test]
fn digests_round_trip_through_hex() {
    let digest = Digest::new([0xAB; 32]);
    let hex = digest.to_hex();
    assert_eq!(hex.len(), 64);
    assert_eq!(hex, "ab".repeat(32));
    assert_eq!(Digest::from_hex(&hex), Some(digest));
    assert_eq!(Digest::from_hex(&hex.to_uppercase()), Some(digest));
    for bad in [
        "",
        "ab",
        &"zz".repeat(32),
        &"é".repeat(32),
        &"ab".repeat(33),
    ] {
        assert_eq!(Digest::from_hex(bad), None, "{bad:?}");
    }
}

// ------------------------------------------------------------ retry policy

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn retry_policy_enforces_its_bounds() {
    let ok = RetryPolicy::new(3, ms(500), ms(10_000), BackoffStrategy::Exponential, true);
    assert!(ok.is_ok());
    let cases = [
        (0, ms(1), ms(1), RetryViolation::Attempts),
        (
            MAX_RETRY_ATTEMPTS + 1,
            ms(1),
            ms(1),
            RetryViolation::Attempts,
        ),
        (3, Duration::ZERO, ms(10), RetryViolation::InitialDelay),
        (
            3,
            Duration::from_secs(86_401),
            Duration::from_secs(86_401),
            RetryViolation::InitialDelay,
        ),
        (3, ms(100), ms(99), RetryViolation::MaxDelay),
        (
            3,
            ms(100),
            Duration::from_secs(86_401),
            RetryViolation::MaxDelay,
        ),
    ];
    for (attempts, initial, max, violation) in cases {
        assert_eq!(
            RetryPolicy::new(attempts, initial, max, BackoffStrategy::Fixed, false),
            Err(WorkflowError::InvalidRetry(violation)),
            "{attempts} {initial:?} {max:?}"
        );
    }
}

#[test]
fn attempts_left_counts_attempts_already_used() {
    let policy = RetryPolicy::new(3, ms(1), ms(1), BackoffStrategy::Fixed, false).expect("policy");
    assert!(policy.has_attempts_left(0));
    assert!(policy.has_attempts_left(2));
    assert!(!policy.has_attempts_left(3));
    assert!(!RetryPolicy::no_retry().has_attempts_left(1));
}

#[test]
fn fixed_backoff_is_constant() {
    let policy =
        RetryPolicy::new(10, ms(250), ms(5_000), BackoffStrategy::Fixed, false).expect("p");
    for attempt in 1..=10 {
        assert_eq!(policy.delay_after(attempt, 0.5), ms(250));
    }
}

#[test]
fn exponential_backoff_doubles_and_is_capped() {
    let policy =
        RetryPolicy::new(10, ms(100), ms(1_000), BackoffStrategy::Exponential, false).expect("p");
    let delays: Vec<_> = (1..=6).map(|a| policy.delay_after(a, 0.0)).collect();
    assert_eq!(
        delays,
        [ms(100), ms(200), ms(400), ms(800), ms(1_000), ms(1_000)]
    );
}

#[test]
fn exponential_backoff_cannot_overflow() {
    let policy = RetryPolicy::new(
        100,
        ms(1),
        Duration::from_secs(86_400),
        BackoffStrategy::Exponential,
        false,
    )
    .expect("p");
    for attempt in [1, 40, 41, 63, 64, 65, 1_000, u32::MAX] {
        let delay = policy.delay_after(attempt, 0.0);
        assert!(
            delay >= ms(1) && delay <= Duration::from_secs(86_400),
            "{attempt}: {delay:?}"
        );
    }
}

#[test]
fn jitter_stays_between_half_and_the_whole_base_delay() {
    let policy = RetryPolicy::new(
        10,
        ms(1_000),
        ms(60_000),
        BackoffStrategy::Exponential,
        true,
    )
    .expect("p");
    for attempt in 1..=8 {
        let base = RetryPolicy::new(
            10,
            ms(1_000),
            ms(60_000),
            BackoffStrategy::Exponential,
            false,
        )
        .expect("p")
        .delay_after(attempt, 0.0);
        for step in 0..=20 {
            let sample = f64::from(step) / 20.0;
            let delay = policy.delay_after(attempt, sample);
            assert!(
                delay >= base / 2,
                "{attempt} {sample}: {delay:?} < {:?}",
                base / 2
            );
            assert!(delay <= base, "{attempt} {sample}: {delay:?} > {base:?}");
        }
    }
}

#[test]
fn jitter_ignores_a_nonsensical_sample() {
    let policy =
        RetryPolicy::new(5, ms(1_000), ms(60_000), BackoffStrategy::Fixed, true).expect("p");
    for sample in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -3.0, 7.5] {
        let delay = policy.delay_after(1, sample);
        assert!(
            delay >= ms(500) && delay <= ms(1_000),
            "{sample}: {delay:?}"
        );
    }
}

// ------------------------------------------------------------ valid definitions

fn payment_flow() -> WorkflowDefinition {
    build(
        "validate",
        vec![
            activity("validate").with_next(id("reserve")),
            Node::new(
                id("reserve"),
                NodeKind::Activity {
                    activity: ActivityName::parse("limit.reserve").expect("n"),
                    retry: RetryPolicy::new(
                        3,
                        ms(500),
                        ms(10_000),
                        BackoffStrategy::Exponential,
                        true,
                    )
                    .expect("r"),
                    compensate_with: Some(ActivityName::parse("limit.release").expect("n")),
                    reconcile_with: None,
                    idempotency_template: Some(expr("run.id")),
                    secrets: Vec::new(),
                },
            )
            .with_next(id("wait_confirmation")),
            Node::new(
                id("wait_confirmation"),
                NodeKind::WaitEvent {
                    event_type: EventType::parse("payment.confirmed").expect("e"),
                    correlation: expr("input.paymentId"),
                    timeout: Duration::from_secs(300),
                    on_timeout: Some(id("manual_review")),
                    lookback: Duration::ZERO,
                },
            )
            .with_next(id("notify")),
            activity("notify"),
            activity("manual_review"),
        ],
    )
    .expect("valid definition")
}

#[test]
fn the_documented_payment_flow_is_valid_and_chains_may_end_without_an_end_node() {
    let def = payment_flow();
    assert_eq!(def.len(), 5);
    assert_eq!(def.entrypoint(), &id("validate"));
    assert_eq!(def.name().as_str(), "demo-flow");
    assert!(def.node(&id("notify")).expect("node").next().is_none());
}

#[test]
fn a_compensation_outside_a_saga_is_reported_because_it_will_never_run() {
    let warnings = payment_flow().warnings();
    assert_eq!(
        warnings,
        [DefinitionWarning::CompensationOutsideSaga {
            node: id("reserve")
        }]
    );
    assert!(warnings[0].to_string().contains("saga"));
}

#[test]
fn successors_list_every_edge_in_a_stable_order() {
    let def = payment_flow();
    let wait = def.node(&id("wait_confirmation")).expect("node");
    let order: Vec<_> = wait.successors().into_iter().map(NodeId::as_str).collect();
    assert_eq!(order, ["manual_review", "notify"]);
}

#[test]
fn non_activities_never_retry() {
    let def = build("t", vec![timer("t", 10)]).expect("def");
    assert_eq!(
        def.node(&id("t"))
            .expect("node")
            .retry_policy()
            .max_attempts(),
        1
    );
}

#[test]
fn a_long_chain_is_validated_without_recursion() {
    let nodes: Vec<Node> = (0..MAX_NODES)
        .map(|i| {
            let node = activity(&format!("n{i}"));
            if i + 1 < MAX_NODES {
                node.with_next(id(&format!("n{}", i + 1)))
            } else {
                node
            }
        })
        .collect();
    let def = build("n0", nodes).expect("a 10 000-node chain is valid");
    assert_eq!(def.len(), MAX_NODES);
}

#[test]
fn conditions_route_to_two_existing_nodes() {
    let def = build(
        "check",
        vec![
            Node::new(
                id("check"),
                NodeKind::Condition {
                    expression: expr("input.amount > 100"),
                    if_true: id("big"),
                    if_false: id("small"),
                },
            ),
            activity("big"),
            activity("small"),
        ],
    );
    assert!(def.is_ok(), "{def:?}");
}

// ------------------------------------------------------------ invalid definitions

#[test]
fn an_empty_or_oversized_graph_is_rejected() {
    assert_eq!(build("a", vec![]), Err(WorkflowError::NoNodes));
    let nodes: Vec<Node> = (0..=MAX_NODES)
        .map(|i| activity(&format!("n{i}")))
        .collect();
    assert_eq!(
        build("n0", nodes),
        Err(WorkflowError::TooManyNodes(MAX_NODES + 1))
    );
}

#[test]
fn duplicate_ids_and_unknown_entrypoints_are_rejected() {
    assert_eq!(
        build("a", vec![activity("a"), activity("a")]),
        Err(WorkflowError::DuplicateNode(id("a")))
    );
    assert_eq!(
        build("zzz", vec![activity("a")]),
        Err(WorkflowError::UnknownEntrypoint(id("zzz")))
    );
}

#[test]
fn every_kind_of_dangling_reference_is_reported() {
    let cases: Vec<(&str, Node)> = vec![
        ("next", activity("a").with_next(id("ghost"))),
        ("on_error", activity("a").with_on_error(id("ghost"))),
        (
            "if_true",
            Node::new(
                id("a"),
                NodeKind::Condition {
                    expression: expr("x"),
                    if_true: id("ghost"),
                    if_false: id("a"),
                },
            ),
        ),
        (
            "on_timeout",
            Node::new(
                id("a"),
                NodeKind::WaitEvent {
                    event_type: EventType::parse("e").expect("e"),
                    correlation: expr("x"),
                    timeout: ms(10),
                    on_timeout: Some(id("ghost")),
                    lookback: Duration::ZERO,
                },
            ),
        ),
        ("branch", parallel("a", &["ghost", "a2"], JoinPolicy::All)),
    ];
    for (what, node) in cases {
        let result = build("a", vec![node, activity("a2")]);
        assert_eq!(
            result,
            Err(WorkflowError::UnknownTarget {
                from: id("a"),
                to: id("ghost")
            }),
            "{what}"
        );
    }
}

#[test]
fn cycles_are_rejected_including_self_loops_and_error_edges() {
    let self_loop = build("a", vec![activity("a").with_next(id("a"))]);
    assert_eq!(self_loop, Err(WorkflowError::Cycle(id("a"))));

    let ring = build(
        "a",
        vec![
            activity("a").with_next(id("b")),
            activity("b").with_next(id("c")),
            activity("c").with_next(id("a")),
        ],
    );
    assert!(matches!(ring, Err(WorkflowError::Cycle(_))), "{ring:?}");

    let via_error = build(
        "a",
        vec![
            activity("a").with_next(id("b")),
            activity("b").with_on_error(id("a")),
        ],
    );
    assert!(
        matches!(via_error, Err(WorkflowError::Cycle(_))),
        "{via_error:?}"
    );
}

#[test]
fn nodes_that_cannot_be_reached_are_rejected() {
    let result = build("a", vec![activity("a"), activity("orphan")]);
    assert_eq!(result, Err(WorkflowError::Unreachable(id("orphan"))));
}

#[test]
fn delays_and_timeouts_must_be_positive_and_bounded() {
    for (what, node) in [
        ("zero timer", timer("a", 0)),
        (
            "huge timer",
            Node::new(
                id("a"),
                NodeKind::Timer {
                    delay: MAX_WAIT + ms(1),
                },
            ),
        ),
        (
            "zero node timeout",
            activity("a").with_timeout(Duration::ZERO),
        ),
        (
            "huge node timeout",
            activity("a").with_timeout(MAX_WAIT + ms(1)),
        ),
    ] {
        let result = build("a", vec![node]);
        assert!(
            matches!(result, Err(WorkflowError::DurationOutOfRange { .. })),
            "{what}: {result:?}"
        );
    }
    assert!(
        build(
            "a",
            vec![Node::new(id("a"), NodeKind::Timer { delay: MAX_WAIT })]
        )
        .is_ok()
    );
}

// ------------------------------------------------------------ parallel regions

fn fork_join() -> Vec<Node> {
    vec![
        parallel("fork", &["left", "right"], JoinPolicy::All).with_next(id("after")),
        activity("left").with_next(id("left2")),
        activity("left2"),
        activity("right"),
        activity("after"),
    ]
}

#[test]
fn a_fork_and_join_has_one_region_per_branch() {
    let def = build("fork", fork_join()).expect("valid");
    let regions = def.branch_regions(&id("fork")).expect("regions");
    assert_eq!(regions.len(), 2);
    fn names(r: &std::collections::BTreeSet<NodeId>) -> Vec<&str> {
        r.iter().map(NodeId::as_str).collect()
    }
    assert_eq!(names(&regions[0]), ["left", "left2"]);
    assert_eq!(names(&regions[1]), ["right"]);
    assert!(def.branch_regions(&id("left")).is_none());
}

#[test]
fn a_parallel_needs_two_to_max_distinct_branches() {
    let one = build(
        "fork",
        vec![parallel("fork", &["a"], JoinPolicy::All), activity("a")],
    );
    assert_eq!(one, Err(WorkflowError::InvalidBranchCount(id("fork"))));
    let dup = build(
        "fork",
        vec![
            parallel("fork", &["a", "a"], JoinPolicy::Any),
            activity("a"),
        ],
    );
    assert_eq!(dup, Err(WorkflowError::InvalidBranchCount(id("fork"))));

    let names: Vec<String> = (0..=MAX_BRANCHES).map(|i| format!("b{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut nodes = vec![parallel("fork", &refs, JoinPolicy::All)];
    nodes.extend(names.iter().map(|n| activity(n)));
    assert_eq!(
        build("fork", nodes),
        Err(WorkflowError::InvalidBranchCount(id("fork")))
    );
}

#[test]
fn branches_may_not_share_nodes() {
    let result = build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::All),
            activity("l").with_next(id("shared")),
            activity("r").with_next(id("shared")),
            activity("shared"),
        ],
    );
    assert!(
        matches!(result, Err(WorkflowError::BranchOverlap { .. })),
        "{result:?}"
    );
}

#[test]
fn a_branch_may_not_run_into_the_continuation_of_the_join() {
    let result = build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::All).with_next(id("after")),
            activity("l").with_next(id("after")),
            activity("r"),
            activity("after"),
        ],
    );
    assert_eq!(
        result,
        Err(WorkflowError::BranchBoundary {
            parallel: id("fork"),
            node: id("after")
        })
    );
}

#[test]
fn nothing_outside_may_jump_into_a_branch() {
    let result = build(
        "start",
        vec![
            Node::new(
                id("start"),
                NodeKind::Condition {
                    expression: expr("x"),
                    if_true: id("fork"),
                    if_false: id("inside"),
                },
            ),
            parallel("fork", &["l", "r"], JoinPolicy::All),
            activity("l").with_next(id("inside")),
            activity("inside"),
            activity("r"),
        ],
    );
    assert!(
        matches!(result, Err(WorkflowError::BranchBoundary { .. })),
        "{result:?}"
    );
}

#[test]
fn a_workflow_end_cannot_sit_inside_a_branch() {
    let result = build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::Any),
            activity("l").with_next(id("done")),
            end("done"),
            activity("r"),
        ],
    );
    assert_eq!(
        result,
        Err(WorkflowError::EndInsideBranch {
            parallel: id("fork"),
            node: id("done")
        })
    );
}

#[test]
fn parallels_can_nest_when_each_region_is_closed() {
    let def = build(
        "outer",
        vec![
            parallel("outer", &["a", "inner"], JoinPolicy::All).with_next(id("after")),
            activity("a"),
            parallel("inner", &["b", "c"], JoinPolicy::Any).with_next(id("tail")),
            activity("b"),
            activity("c"),
            activity("tail"),
            activity("after"),
        ],
    )
    .expect("nested fork/join is valid");
    let outer = def.branch_regions(&id("outer")).expect("outer");
    assert!(outer[1].contains(&id("tail")) && outer[1].contains(&id("b")));
    assert_eq!(def.branch_regions(&id("inner")).expect("inner").len(), 2);
}

#[test]
fn an_error_edge_inside_a_branch_must_stay_inside_it() {
    let escaping = build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::All),
            activity("l").with_on_error(id("r")),
            activity("r"),
        ],
    );
    assert!(
        matches!(
            escaping,
            Err(WorkflowError::BranchOverlap { .. } | WorkflowError::BranchBoundary { .. })
        ),
        "{escaping:?}"
    );

    let contained = build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::All),
            activity("l").with_on_error(id("cleanup")),
            activity("cleanup"),
            activity("r"),
        ],
    );
    assert!(contained.is_ok(), "{contained:?}");
}

// ------------------------------------------------------------------ the cost of validating

mod cost {
    use std::time::{Duration, Instant};

    use sagat_domain::identity::Slug;
    use sagat_domain::workflow::{
        ActivityName, JoinPolicy, MAX_PARALLELS, Node, NodeId, NodeKind, RetryPolicy,
        WorkflowDefinition, WorkflowError,
    };

    fn id(s: &str) -> NodeId {
        NodeId::parse(s).expect("id")
    }

    fn activity(n: &str) -> Node {
        Node::new(
            id(n),
            NodeKind::Activity {
                activity: ActivityName::parse("act.x").expect("name"),
                retry: RetryPolicy::DEFAULT,
                compensate_with: None,
                reconcile_with: None,
                idempotency_template: None,
                secrets: Vec::new(),
            },
        )
    }

    /// `parallels` parallel nodes nested one inside the next, with `chain` activities in a line
    /// inside the innermost branch — so every enclosing parallel's region contains the whole chain,
    /// the worst shape for the work of finding regions.
    fn nested(parallels: usize, chain: usize) -> Vec<Node> {
        let mut nodes = Vec::new();
        for k in 0..parallels {
            let first = if k + 1 < parallels {
                format!("p{}", k + 1)
            } else {
                "l0".to_owned()
            };
            nodes.push(
                Node::new(
                    id(&format!("p{k}")),
                    NodeKind::Parallel {
                        branches: vec![id(&first), id(&format!("side{k}"))],
                        join: JoinPolicy::All,
                    },
                )
                .with_next(id(&format!("x{k}"))),
            );
            nodes.push(activity(&format!("side{k}")));
            nodes.push(activity(&format!("x{k}")));
        }
        for i in 0..chain {
            let node = activity(&format!("l{i}"));
            nodes.push(if i + 1 < chain {
                node.with_next(id(&format!("l{}", i + 1)))
            } else {
                node
            });
        }
        nodes
    }

    fn define(nodes: Vec<Node>) -> Result<WorkflowDefinition, WorkflowError> {
        WorkflowDefinition::new(Slug::parse("deep-flow").expect("slug"), id("p0"), nodes)
    }

    #[test]
    fn a_workflow_may_not_hold_more_parallel_nodes_than_the_limit() {
        assert_eq!(
            define(nested(MAX_PARALLELS + 1, 3)),
            Err(WorkflowError::TooManyParallels(MAX_PARALLELS + 1))
        );
        assert!(
            define(nested(MAX_PARALLELS, 3)).is_ok(),
            "the limit itself is allowed"
        );
    }

    #[test]
    fn the_largest_allowed_shape_validates_in_bounded_time_and_reports_its_weight() {
        let started = Instant::now();
        let definition = define(nested(MAX_PARALLELS, 9_000)).expect("valid");
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(20),
            "32 nested parallels over 9000 nodes took {took:?}: validation is no longer bounded"
        );
        // 32 regions each holding the chain: the weight reflects it.
        let weight = definition.footprint();
        assert!(weight > 25_000_000, "{weight}");
        assert!(weight < 256 * 1024 * 1024, "{weight}");
    }

    #[test]
    fn a_plain_chain_of_ten_thousand_nodes_is_valid_and_weighs_what_it_should() {
        let nodes: Vec<Node> = (0..10_000)
            .map(|i| {
                let node = activity(&format!("n{i}"));
                if i + 1 < 10_000 {
                    node.with_next(id(&format!("n{}", i + 1)))
                } else {
                    node
                }
            })
            .collect();
        let definition =
            WorkflowDefinition::new(Slug::parse("long-flow").expect("slug"), id("n0"), nodes)
                .expect("valid");
        let weight = definition.footprint();
        assert!((6_000_000..40_000_000).contains(&weight), "{weight}");
    }
}

mod declared_secrets {
    use sagat_domain::identity::Slug;
    use sagat_domain::secrets::{MAX_DECLARED_SECRETS, SecretName};
    use sagat_domain::workflow::{
        ActivityName, Node, NodeId, NodeKind, RetryPolicy, WorkflowDefinition, WorkflowError,
    };

    /// A chain of activities; activity `i` declares the secrets `names(i)`.
    fn chain(count: usize, names: impl Fn(usize) -> Vec<String>) -> Vec<Node> {
        (0..count)
            .map(|i| {
                let node = Node::new(
                    NodeId::parse(&format!("n{i}")).expect("id"),
                    NodeKind::Activity {
                        activity: ActivityName::parse("act.x").expect("name"),
                        retry: RetryPolicy::DEFAULT,
                        compensate_with: None,
                        reconcile_with: None,
                        idempotency_template: None,
                        secrets: names(i)
                            .iter()
                            .map(|n| SecretName::parse(n).expect("secret"))
                            .collect(),
                    },
                );
                if i + 1 < count {
                    node.with_next(NodeId::parse(&format!("n{}", i + 1)).expect("id"))
                } else {
                    node
                }
            })
            .collect()
    }

    fn define(nodes: Vec<Node>) -> Result<WorkflowDefinition, WorkflowError> {
        WorkflowDefinition::new(
            Slug::parse("secret-flow").expect("slug"),
            NodeId::parse("n0").expect("id"),
            nodes,
        )
    }

    #[test]
    fn a_workflow_may_not_declare_more_distinct_secrets_than_the_limit() {
        // One distinct secret per activity, spread over many activities.
        assert_eq!(
            define(chain(MAX_DECLARED_SECRETS + 1, |i| vec![format!(
                "key-{i}"
            )])),
            Err(WorkflowError::TooManySecrets(MAX_DECLARED_SECRETS + 1))
        );
        assert!(
            define(chain(MAX_DECLARED_SECRETS, |i| vec![format!("key-{i}")])).is_ok(),
            "the limit itself is allowed"
        );
    }

    #[test]
    fn the_limit_counts_distinct_names_and_not_how_often_they_are_used() {
        // A thousand activities that all use the same two secrets declare two.
        assert!(
            define(chain(1_000, |_| vec![
                "stripe-key".to_owned(),
                "db-password".to_owned()
            ]))
            .is_ok()
        );
    }
}
