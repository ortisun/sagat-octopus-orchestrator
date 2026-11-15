//! Operator chains of design 042, collection half (T042-5): the closed vocabulary, which surface has which operator,
//! the parameter ranges and the chain length (FR-042-001, FR-042-004).
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_domain::operators::{
    MAX_COUNT, MAX_STAGES, Operator, OperatorChain, OperatorError, OperatorName, Surface,
};
use sagat_domain::workflow::Expression;

fn expr(text: &str) -> Expression {
    Expression::parse(text).expect("expression")
}

#[test]
fn the_vocabulary_is_the_nine_names_of_the_schema() {
    let names: Vec<&str> = OperatorName::ALL.iter().map(|n| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "filter",
            "map",
            "flatMap",
            "skip",
            "take",
            "distinct",
            "window",
            "debounce",
            "ignoreElements"
        ]
    );
    for name in OperatorName::ALL {
        assert_eq!(OperatorName::parse(name.as_str()), Some(name));
    }
    for other in [
        "", "Filter", "throttle", "buffer", "scan", "flat_map", "ignore",
    ] {
        assert_eq!(OperatorName::parse(other), None, "{other}");
    }
}

#[test]
fn a_collection_has_seven_operators_and_a_trigger_has_all_nine() {
    let on_collection: Vec<OperatorName> = OperatorName::ALL
        .into_iter()
        .filter(|n| n.is_on(Surface::Collection))
        .collect();
    assert_eq!(
        on_collection,
        [
            OperatorName::Filter,
            OperatorName::Map,
            OperatorName::FlatMap,
            OperatorName::Skip,
            OperatorName::Take,
            OperatorName::Distinct,
            OperatorName::Window
        ]
    );
    assert!(OperatorName::ALL.iter().all(|n| n.is_on(Surface::Trigger)));
    for trigger_only in [OperatorName::Debounce, OperatorName::IgnoreElements] {
        assert_eq!(
            trigger_only.require_on(Surface::Collection, 3),
            Err(OperatorError::Surface {
                stage: 3,
                operator: trigger_only,
                surface: Surface::Collection,
            })
        );
    }
    assert_eq!(OperatorName::Map.require_on(Surface::Collection, 0), Ok(()));
}

#[test]
fn every_collection_operator_names_itself() {
    let all = [
        Operator::Filter {
            predicate: expr("element > 0"),
        },
        Operator::Map {
            select: expr("element.id"),
        },
        Operator::FlatMap {
            select: expr("element.lines"),
        },
        Operator::Skip { count: 1 },
        Operator::Take { count: 1 },
        Operator::Distinct { by: None },
        Operator::Window { count: 1 },
    ];
    let names: Vec<OperatorName> = all.iter().map(Operator::name).collect();
    assert_eq!(names, &OperatorName::ALL[..7]);
    assert!(names.iter().all(|n| n.is_on(Surface::Collection)));
}

#[test]
fn a_window_of_zero_is_refused() {
    let refused = OperatorChain::collection(vec![
        Operator::Map {
            select: expr("element"),
        },
        Operator::Window { count: 0 },
    ]);
    assert_eq!(
        refused,
        Err(OperatorError::Count {
            stage: 1,
            operator: OperatorName::Window,
            count: 0,
            min: 1,
            max: MAX_COUNT,
        })
    );
}

#[test]
fn a_chain_has_one_to_eight_stages() {
    assert_eq!(MAX_STAGES, 8);
    assert_eq!(
        OperatorChain::collection(Vec::new()),
        Err(OperatorError::Length(0))
    );
    let nine = vec![Operator::Take { count: 1 }; 9];
    assert_eq!(
        OperatorChain::collection(nine),
        Err(OperatorError::Length(9))
    );
}

#[test]
fn the_expressions_of_a_chain_are_listed_in_stage_order() {
    let chain = OperatorChain::collection(vec![
        Operator::Filter {
            predicate: expr("element.total > 0"),
        },
        Operator::Distinct {
            by: Some(expr("element.customer")),
        },
        Operator::Distinct { by: None },
        Operator::Skip { count: 0 },
        Operator::FlatMap {
            select: expr("element.lines"),
        },
    ])
    .expect("chain");
    let listed: Vec<(usize, &str, &str)> = chain
        .expressions()
        .map(|(stage, field, e)| (stage, field, e.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            (0, "where", "element.total > 0"),
            (1, "by", "element.customer"),
            (4, "select", "element.lines"),
        ]
    );
    assert_eq!(chain.surface(), Surface::Collection);
    assert_eq!(chain.len(), 5);
}

fn any_expression() -> impl Strategy<Value = Expression> {
    "[a-z]{1,12}".prop_map(|t| expr(&format!("element.{t}")))
}

/// A stage whose parameters are in range.
fn valid_stage() -> impl Strategy<Value = Operator> {
    prop_oneof![
        any_expression().prop_map(|predicate| Operator::Filter { predicate }),
        any_expression().prop_map(|select| Operator::Map { select }),
        any_expression().prop_map(|select| Operator::FlatMap { select }),
        (0..=MAX_COUNT).prop_map(|count| Operator::Skip { count }),
        (0..=MAX_COUNT).prop_map(|count| Operator::Take { count }),
        proptest::option::of(any_expression()).prop_map(|by| Operator::Distinct { by }),
        (1..=MAX_COUNT).prop_map(|count| Operator::Window { count }),
    ]
}

proptest! {
    /// Any one to eight stages in range, in any order, make a chain that keeps them in their order.
    #[test]
    fn any_order_of_valid_stages_is_a_chain(stages in proptest::collection::vec(valid_stage(), 1..=MAX_STAGES)) {
        let chain = OperatorChain::collection(stages.clone()).expect("a valid chain");
        prop_assert_eq!(chain.stages(), &stages[..]);
    }

    /// Too many stages are refused whatever they are.
    #[test]
    fn more_than_eight_stages_are_refused(stages in proptest::collection::vec(valid_stage(), MAX_STAGES + 1..=24)) {
        let len = stages.len();
        prop_assert_eq!(OperatorChain::collection(stages), Err(OperatorError::Length(len)));
    }

    /// `skip` and `take` accept 0 to 1,000, and `window` 1 to 1,000; the first stage out of range is the one named.
    #[test]
    fn counts_are_refused_outside_their_range(
        prefix in proptest::collection::vec(valid_stage(), 0..MAX_STAGES),
        count in 0u32..=4_000,
        which in 0usize..3,
    ) {
        let (stage, name, min) = match which {
            0 => (Operator::Skip { count }, OperatorName::Skip, 0),
            1 => (Operator::Take { count }, OperatorName::Take, 0),
            _ => (Operator::Window { count }, OperatorName::Window, 1),
        };
        let index = prefix.len();
        let mut stages = prefix;
        stages.push(stage);
        let result = OperatorChain::collection(stages);
        if (min..=MAX_COUNT).contains(&count) {
            prop_assert!(result.is_ok(), "{result:?}");
        } else {
            prop_assert_eq!(result, Err(OperatorError::Count { stage: index, operator: name, count, min, max: MAX_COUNT }));
        }
    }

    /// The length rule is checked before any stage: a chain of nine is refused for its length even with a stage out of
    /// range in it.
    #[test]
    fn length_is_checked_first(count in MAX_COUNT + 1..=u32::MAX) {
        let mut stages = vec![Operator::Take { count: 1 }; MAX_STAGES];
        stages.push(Operator::Skip { count });
        prop_assert_eq!(OperatorChain::collection(stages), Err(OperatorError::Length(MAX_STAGES + 1)));
    }
}

// --- the chain on a `forEach` -----------------------------------------------------------------------------------------

mod on_a_collection {
    use super::expr;
    use sagat_domain::identity::Slug;
    use sagat_domain::operators::{Operator, OperatorChain};
    use sagat_domain::workflow::{
        BodyDefinition, Node, NodeId, NodeKind, WorkflowDefinition, WorkflowError, WorkflowLimits,
    };

    fn id(value: &str) -> NodeId {
        NodeId::parse(value).expect("node id")
    }

    fn definition(chain: Option<OperatorChain>) -> Result<WorkflowDefinition, WorkflowError> {
        let owner = Node::new(
            id("items"),
            NodeKind::ForEach {
                items: expr("input.items"),
                operators: chain,
                body: id("line"),
                result: expr("bodyResult"),
                max_items: 17,
                max_concurrency: 3,
            },
        )
        .with_next(id("done"));
        let body = BodyDefinition::new(
            id("line"),
            id("return"),
            vec![Node::new(
                id("return"),
                NodeKind::Return {
                    result: expr("item"),
                },
            )],
        )
        .expect("closed body");
        WorkflowDefinition::new_structured(
            Slug::parse("bounded-orders").expect("workflow name"),
            id("items"),
            vec![owner, Node::new(id("done"), NodeKind::End { output: None })],
            vec![body],
            WorkflowLimits::default(),
        )
    }

    #[test]
    fn a_collection_carries_its_chain() {
        let chain = OperatorChain::collection(vec![Operator::Take { count: 2 }]).expect("chain");
        let workflow = definition(Some(chain.clone())).expect("definition");
        match workflow.node(&id("items")).expect("owner").kind() {
            NodeKind::ForEach { operators, .. } => assert_eq!(operators.as_ref(), Some(&chain)),
            other => panic!("{other:?}"),
        }
        assert!(definition(None).is_ok());
    }

    /// A stage expression is held to the 1,024 bytes of every structured expression.
    #[test]
    fn a_stage_expression_over_the_structured_byte_limit_is_refused() {
        let long = format!("element.{}", "a".repeat(1024));
        let chain = OperatorChain::collection(vec![
            Operator::Take { count: 2 },
            Operator::Map {
                select: expr(&long),
            },
        ])
        .expect("chain");
        assert_eq!(
            definition(Some(chain)).err(),
            Some(WorkflowError::ExpressionByteLimit {
                node: id("items"),
                limit: 1024
            })
        );
    }
}
