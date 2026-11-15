#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::time::Duration;

use proptest::prelude::*;
use sagat_application::collection_chain::{
    ChainAnswer, ChainCeiling, ChainCeilings, ChainFailure, ChainInput, ChainOutput, StageFault,
};
use sagat_application::execution::{ExpressionEvaluator, RunData};
use sagat_domain::operators::{Operator, OperatorChain, OperatorName};
use sagat_domain::workflow::{Expression, NodeId};
use sagat_infra_expr::{Compiled, Env};
use serde_json::{Value, json};

use super::distinct_form;
use crate::JsonEvaluator;

fn expression(text: &str) -> Expression {
    Expression::parse(text).expect("expression")
}

fn chain(stages: Vec<Operator>) -> OperatorChain {
    OperatorChain::collection(stages).expect("a valid chain")
}

fn filter(text: &str) -> Operator {
    Operator::Filter {
        predicate: expression(text),
    }
}

fn map(text: &str) -> Operator {
    Operator::Map {
        select: expression(text),
    }
}

fn flat_map(text: &str) -> Operator {
    Operator::FlatMap {
        select: expression(text),
    }
}

fn data(input: &Value) -> RunData {
    RunData {
        input: serde_json::to_vec(input).expect("json"),
        ..RunData::default()
    }
}

fn run(
    chain: &OperatorChain,
    input: &Value,
    ceilings: &ChainCeilings,
) -> Result<ChainOutput, ChainFailure> {
    let items = expression("input.list");
    match JsonEvaluator.collection_chain(
        ChainInput::Items(&items),
        chain,
        &data(input),
        ceilings,
    )? {
        ChainAnswer::Output(output) => Ok(output),
        ChainAnswer::Blob(_) => panic!("an inline list is not a blob"),
    }
}

fn values(output: &ChainOutput) -> Vec<Value> {
    output
        .items
        .iter()
        .map(|item| serde_json::from_slice(item).expect("json"))
        .collect()
}

fn shaped(stages: Vec<Operator>, list: Value) -> Vec<Value> {
    let output = run(
        &chain(stages),
        &json!({ "list": list }),
        &ChainCeilings::COLLECTION,
    )
    .expect("the chain applies");
    values(&output)
}

#[test]
fn each_operator_shapes_the_list_as_declared() {
    let list = json!([3, -1, 4, -1, 5, 9, 2, 6]);
    assert_eq!(
        shaped(vec![filter("element > 0")], list.clone()),
        [3, 4, 5, 9, 2, 6]
    );
    assert_eq!(shaped(vec![map("element * 10")], json!([1, 2])), [10, 20]);
    assert_eq!(
        shaped(vec![flat_map("[element, element]")], json!([1, [2]])),
        vec![json!(1), json!(1), json!([2]), json!([2])],
        "one level only: a list element stays a list"
    );
    assert_eq!(
        shaped(vec![Operator::Skip { count: 6 }], list.clone()),
        [2, 6]
    );
    assert_eq!(
        shaped(vec![Operator::Take { count: 2 }], list.clone()),
        [3, -1]
    );
    assert_eq!(
        shaped(vec![Operator::Distinct { by: None }], list.clone()),
        [3, -1, 4, 5, 9, 2, 6]
    );
    assert_eq!(
        shaped(
            vec![Operator::Distinct {
                by: Some(expression("element % 2"))
            }],
            list.clone()
        ),
        [3, -1, 4],
        "the first element of each canonical key: 1, -1 and 0 (`%` keeps the sign)"
    );
    assert_eq!(
        shaped(vec![Operator::Window { count: 3 }], list.clone()),
        vec![json!([3, -1, 4]), json!([-1, 5, 9]), json!([2, 6])],
        "the last window may be shorter"
    );
    assert_eq!(
        shaped(
            vec![Operator::Skip { count: 0 }, Operator::Take { count: 0 }],
            list
        ),
        Vec::<Value>::new()
    );
}

#[test]
fn an_empty_output_is_an_empty_collection() {
    let output = run(
        &chain(vec![filter("false")]),
        &json!({"list": [1, 2, 3]}),
        &ChainCeilings::COLLECTION,
    )
    .expect("empty is allowed");
    assert!(output.items.is_empty());
    assert_eq!(output.source_count, 3);
    let output = run(
        &chain(vec![map("element")]),
        &json!({"list": []}),
        &ChainCeilings::COLLECTION,
    )
    .expect("an empty input is allowed");
    assert!(output.items.is_empty());
}

#[test]
fn stages_see_element_position_and_what_items_sees() {
    let mut data = data(&json!({"list": ["a", "b", "c"], "keep": 1, "suffix": "!"}));
    data.insert_root_output(
        NodeId::parse("prior").expect("node"),
        br#"{"tag":"t"}"#.to_vec(),
    );
    data.state = Some(br#"{"n": 7}"#.to_vec());
    let chain = chain(vec![
        filter("position != input.keep"),
        map("[element + input.suffix, position, steps.prior.output.tag, state.n]"),
    ]);
    let items = expression("input.list");
    let ChainAnswer::Output(output) = JsonEvaluator
        .collection_chain(
            ChainInput::Items(&items),
            &chain,
            &data,
            &ChainCeilings::COLLECTION,
        )
        .expect("applies")
    else {
        panic!("not a blob");
    };
    assert_eq!(
        values(&output),
        vec![json!(["a!", 0, "t", 7]), json!(["c!", 1, "t", 7])],
        "`position` is the index in the stage's own input"
    );
    assert_eq!(output.chain_digest, crate::operators::chain_digest(&chain));
    assert_eq!(
        output
            .stages
            .iter()
            .map(|s| (s.operator, s.input, s.output))
            .collect::<Vec<_>>(),
        vec![(OperatorName::Filter, 3, 2), (OperatorName::Map, 2, 2)]
    );
}

#[test]
fn every_element_of_a_stage_runs_before_the_next_stage_and_the_first_failure_is_named() {
    // Element by element, stage 1 would fail first (position 0 is an integer with no field `b`); stage by stage,
    // stage 0 fails at position 2 before stage 1 sees anything.
    let failure = run(
        &chain(vec![map("element.a"), filter("element.b")]),
        &json!({"list": [{"a": 1}, {"a": 2}, {}]}),
        &ChainCeilings::COLLECTION,
    )
    .expect_err("stage 0 fails");
    assert_eq!(
        failure,
        ChainFailure::Stage {
            stage: 0,
            operator: OperatorName::Map,
            position: 2,
            fault: StageFault::Expression
        }
    );
}

#[test]
fn a_filter_that_is_not_boolean_and_a_flat_map_that_is_not_a_list_name_their_stage() {
    assert_eq!(
        run(
            &chain(vec![map("element"), filter("element")]),
            &json!({"list": [true, 1]}),
            &ChainCeilings::COLLECTION
        ),
        Err(ChainFailure::Stage {
            stage: 1,
            operator: OperatorName::Filter,
            position: 1,
            fault: StageFault::NotBoolean
        })
    );
    assert_eq!(
        run(
            &chain(vec![flat_map("element")]),
            &json!({"list": [[1], 2]}),
            &ChainCeilings::COLLECTION
        ),
        Err(ChainFailure::Stage {
            stage: 0,
            operator: OperatorName::FlatMap,
            position: 1,
            fault: StageFault::NotAList
        })
    );
    assert_eq!(
        run(
            &chain(vec![map("element")]),
            &json!({"list": 3}),
            &ChainCeilings::COLLECTION
        ),
        Err(ChainFailure::Items),
        "an items expression that yields neither a list nor a blob"
    );
}

#[test]
fn an_items_expression_that_yields_a_blob_reference_asks_for_the_blob() {
    let blob = json!({"$blob": {
        "id": "0190f0aa-0000-7000-8000-000000000001", "size": 10,
        "sha256": "aa".repeat(32), "contentType": "application/json"}});
    let items = expression("input.source");
    let answer = JsonEvaluator
        .collection_chain(
            ChainInput::Items(&items),
            &chain(vec![map("element")]),
            &data(&json!({ "source": blob })),
            &ChainCeilings::COLLECTION,
        )
        .expect("a blob reference");
    assert!(matches!(answer, ChainAnswer::Blob(_)), "{answer:?}");
}

#[test]
fn materialised_elements_are_the_input_when_the_source_was_a_blob() {
    let elements = vec![b"1".to_vec(), b"2".to_vec(), br#"{"a":1}"#.to_vec()];
    let answer = JsonEvaluator
        .collection_chain(
            ChainInput::Elements(&elements),
            &chain(vec![Operator::Skip { count: 1 }]),
            &RunData::default(),
            &ChainCeilings::COLLECTION,
        )
        .expect("applies");
    let ChainAnswer::Output(output) = answer else {
        panic!("elements are not a blob")
    };
    assert_eq!(output.items, vec![b"2".to_vec(), br#"{"a":1}"#.to_vec()]);
    assert_eq!(output.source_count, 3);
}

// --- ceilings, each crossed by one and held exactly (SC-042-004) ------------------------------------------------------

fn budget(
    ceiling: ChainCeiling,
    stage: Option<usize>,
    operator: Option<OperatorName>,
    position: Option<usize>,
) -> ChainFailure {
    ChainFailure::Budget {
        ceiling,
        stage,
        operator,
        position,
    }
}

#[test]
fn the_input_holds_exactly_1000_elements_and_not_1001() {
    let at: Vec<u32> = (0..1000).collect();
    let output = run(
        &chain(vec![map("element")]),
        &json!({ "list": at }),
        &ChainCeilings::COLLECTION,
    )
    .expect("at the ceiling");
    assert_eq!(output.items.len(), 1000);
    let over: Vec<u32> = (0..1001).collect();
    assert_eq!(
        run(
            &chain(vec![map("element")]),
            &json!({ "list": over }),
            &ChainCeilings::COLLECTION
        ),
        Err(budget(ChainCeiling::InputElements, None, None, None))
    );
    let elements: Vec<Vec<u8>> = (0..1001).map(|n: u32| n.to_string().into_bytes()).collect();
    assert_eq!(
        JsonEvaluator.collection_chain(
            ChainInput::Elements(&elements),
            &chain(vec![map("element")]),
            &RunData::default(),
            &ChainCeilings::COLLECTION
        ),
        Err(budget(ChainCeiling::InputElements, None, None, None)),
        "a blob's elements are bounded the same way"
    );
}

#[test]
fn a_stage_produces_exactly_1000_elements_and_not_1001() {
    let at: Vec<u32> = (0..500).collect();
    let output = run(
        &chain(vec![flat_map("[element, element]")]),
        &json!({ "list": at }),
        &ChainCeilings::COLLECTION,
    )
    .expect("at the ceiling");
    assert_eq!(output.items.len(), 1000);
    let mut over: Vec<u32> = (0..500).collect();
    over.push(500);
    // The 1001st element is produced from position 500, the first of its pair.
    assert_eq!(
        run(
            &chain(vec![flat_map("[element, element]")]),
            &json!({ "list": over }),
            &ChainCeilings::COLLECTION
        ),
        Err(budget(
            ChainCeiling::StageElements,
            Some(0),
            Some(OperatorName::FlatMap),
            Some(500)
        ))
    );
}

/// A JSON string whose compact form is exactly `bytes` long.
fn string_of(bytes: usize) -> Value {
    Value::String("x".repeat(bytes - 2))
}

#[test]
fn an_element_holds_exactly_256_kib_and_not_one_byte_more() {
    let limit = 256 * 1024;
    // `[element]` adds two bytes to the element it wraps.
    let output = run(
        &chain(vec![map("element"), map("[element]")]),
        &json!({ "list": [string_of(limit - 2)] }),
        &ChainCeilings::COLLECTION,
    )
    .expect("at the ceiling");
    assert_eq!(output.items[0].len(), limit);
    assert_eq!(
        run(
            &chain(vec![map("element"), map("[element]")]),
            &json!({ "list": [string_of(limit - 1)] }),
            &ChainCeilings::COLLECTION
        ),
        Err(budget(
            ChainCeiling::ElementBytes,
            Some(1),
            Some(OperatorName::Map),
            Some(0)
        ))
    );
}

#[test]
fn a_stage_holds_exactly_4_mib_and_not_one_byte_more() {
    let element = 256 * 1024;
    let at: Vec<Vec<u8>> = (0..16)
        .map(|_| serde_json::to_vec(&string_of(element)).expect("json"))
        .collect();
    let answer = JsonEvaluator
        .collection_chain(
            ChainInput::Elements(&at),
            &chain(vec![map("element")]),
            &RunData::default(),
            &ChainCeilings::COLLECTION,
        )
        .expect("at the ceiling");
    let ChainAnswer::Output(output) = answer else {
        panic!("not a blob")
    };
    assert_eq!(
        output.items.iter().map(Vec::len).sum::<usize>(),
        4 * 1024 * 1024
    );
    let mut over = at;
    over.push(b"1".to_vec());
    assert_eq!(
        JsonEvaluator.collection_chain(
            ChainInput::Elements(&over),
            &chain(vec![map("element")]),
            &RunData::default(),
            &ChainCeilings::COLLECTION
        ),
        Err(budget(
            ChainCeiling::StageBytes,
            Some(0),
            Some(OperatorName::Map),
            Some(16)
        ))
    );
}

#[test]
fn the_wall_time_is_checked_between_elements() {
    let spent = ChainCeilings {
        wall_time: Duration::ZERO,
        ..ChainCeilings::COLLECTION
    };
    assert_eq!(
        run(&chain(vec![map("element")]), &json!({"list": [1]}), &spent),
        Err(budget(
            ChainCeiling::WallTime,
            Some(0),
            Some(OperatorName::Map),
            Some(0)
        ))
    );
    // No element, nothing to check between: an empty input finishes even with no time at all.
    assert!(run(&chain(vec![map("element")]), &json!({"list": []}), &spent).is_ok());
}

#[test]
fn a_failure_never_carries_a_value() {
    let mark = "MARKED-VALUE-7f3a";
    let failures = [
        run(
            &chain(vec![map("element.missing")]),
            &json!({"list": [mark]}),
            &ChainCeilings::COLLECTION,
        ),
        run(
            &chain(vec![filter("element")]),
            &json!({"list": [mark]}),
            &ChainCeilings::COLLECTION,
        ),
        run(
            &chain(vec![map("element + element")]),
            &json!({"list": [mark.repeat(5000)]}),
            &ChainCeilings {
                max_element_bytes: 100,
                ..ChainCeilings::COLLECTION
            },
        ),
    ];
    for failure in failures {
        let failure = failure.expect_err("fails");
        let text = format!("{failure} {failure:?}");
        assert!(!text.contains("MARKED"), "{text}");
    }
}

// --- the canonical form of `distinct` (FR-042-011) --------------------------------------------------------------------

fn form(text: &str) -> String {
    String::from_utf8(distinct_form(&serde_json::from_str(text).expect("json"))).expect("utf-8")
}

#[test]
fn the_canonical_form_writes_integral_doubles_as_integers_and_sorts_keys() {
    assert_eq!(form("1"), "1");
    assert_eq!(form("1.0"), "1");
    assert_eq!(form("-0"), "0");
    assert_eq!(form("-0.0"), "0");
    assert_eq!(form("0.0"), "0");
    assert_eq!(form("2.5"), "2.5");
    assert_eq!(form("-3.0"), "-3");
    assert_eq!(form("9007199254740992.0"), "9007199254740992");
    assert_eq!(form("-9007199254740992.0"), "-9007199254740992");
    assert_ne!(
        form("18014398509481984.0"),
        "18014398509481984",
        "beyond 2^53 a double stays a double (G042-010)"
    );
    assert_eq!(
        form(r#"{"b": 1.0, "a": [2.0, {"d": 0, "c": -0.0}]}"#),
        r#"{"a":[2,{"c":0,"d":0}],"b":1}"#
    );
    assert_eq!(
        form(r#"{"é": 1, "z": 2, "a": 3}"#),
        r#"{"a":3,"z":2,"é":1}"#,
        "code-point order"
    );
    assert_ne!(
        form("\"\u{e9}\""),
        form("\"e\u{301}\""),
        "strings are compared as they are, without normalisation"
    );
}

#[test]
fn distinct_keeps_the_first_of_values_that_compare_equal() {
    let list: Value = serde_json::from_str(
        r#"[1, 1.0, -0, 0, 0.0, {"a":1,"b":2}, {"b":2.0,"a":1}, [1,2], [1.0,2.0], [2,1]]"#,
    )
    .expect("json");
    let expected: Vec<Value> =
        serde_json::from_str(r#"[1, -0, {"a":1,"b":2}, [1,2], [2,1]]"#).expect("json");
    assert_eq!(
        shaped(vec![Operator::Distinct { by: None }], list),
        expected
    );
}

// --- the property test against a reference implementation ------------------------------------------------------------

/// The expressions a generated chain uses, each with what it means for the reference.
#[derive(Debug, Clone, Copy)]
enum Predicate {
    EvenPosition,
    Early,
    Always,
}

#[derive(Debug, Clone, Copy)]
enum Projection {
    Same,
    WithPosition,
    Position,
}

#[derive(Debug, Clone, Copy)]
enum Spread {
    Twice,
    Nothing,
    Position,
}

#[derive(Debug, Clone, Copy)]
enum Key {
    Element,
    ByElement,
    Parity,
}

#[derive(Debug, Clone, Copy)]
enum Stage {
    Filter(Predicate),
    Map(Projection),
    FlatMap(Spread),
    Skip(u32),
    Take(u32),
    Distinct(Key),
    Window(u32),
}

impl Stage {
    fn operator(self) -> Operator {
        match self {
            Self::Filter(p) => filter(match p {
                Predicate::EvenPosition => "position % 2 == 0",
                Predicate::Early => "position < 3",
                Predicate::Always => "true",
            }),
            Self::Map(p) => map(match p {
                Projection::Same => "element",
                Projection::WithPosition => "[element, position]",
                Projection::Position => "position",
            }),
            Self::FlatMap(s) => flat_map(match s {
                Spread::Twice => "[element, element]",
                Spread::Nothing => "[]",
                Spread::Position => "[position]",
            }),
            Self::Skip(count) => Operator::Skip { count },
            Self::Take(count) => Operator::Take { count },
            Self::Distinct(key) => Operator::Distinct {
                by: match key {
                    Key::Element => None,
                    Key::ByElement => Some(expression("element")),
                    Key::Parity => Some(expression("position % 2")),
                },
            },
            Self::Window(count) => Operator::Window { count },
        }
    }
}

/// `==` of the expression language: the reference for `distinct`, independent of the canonical form.
fn language_equal(a: &Value, b: &Value) -> bool {
    let equal = Compiled::compile("a == b").expect("compiles");
    equal
        .eval_bool(&Env::new().with("a", a.clone()).with("b", b.clone()))
        .expect("== is total")
}

/// The reference: a plain reading of FR-042-004 over values, with no ceilings and no canonical form.
fn reference(stages: &[Stage], mut list: Vec<Value>) -> Vec<Value> {
    for stage in stages {
        list = match *stage {
            Stage::Filter(p) => list
                .into_iter()
                .enumerate()
                .filter(|(i, _)| match p {
                    Predicate::EvenPosition => i % 2 == 0,
                    Predicate::Early => *i < 3,
                    Predicate::Always => true,
                })
                .map(|(_, v)| v)
                .collect(),
            Stage::Map(p) => list
                .into_iter()
                .enumerate()
                .map(|(i, v)| match p {
                    Projection::Same => v,
                    Projection::WithPosition => json!([v, i]),
                    Projection::Position => json!(i),
                })
                .collect(),
            Stage::FlatMap(s) => list
                .into_iter()
                .enumerate()
                .flat_map(|(i, v)| match s {
                    Spread::Twice => vec![v.clone(), v],
                    Spread::Nothing => vec![],
                    Spread::Position => vec![json!(i)],
                })
                .collect(),
            Stage::Skip(n) => list.into_iter().skip(n as usize).collect(),
            Stage::Take(n) => list.into_iter().take(n as usize).collect(),
            Stage::Distinct(key) => {
                let mut kept: Vec<(Value, Value)> = Vec::new();
                for (i, v) in list.into_iter().enumerate() {
                    let k = match key {
                        Key::Element | Key::ByElement => v.clone(),
                        Key::Parity => json!(i % 2),
                    };
                    if !kept.iter().any(|(seen, _)| language_equal(seen, &k)) {
                        kept.push((k, v));
                    }
                }
                kept.into_iter().map(|(_, v)| v).collect()
            }
            Stage::Window(n) => list
                .chunks(n as usize)
                .map(|chunk| Value::Array(chunk.to_vec()))
                .collect(),
        };
    }
    list
}

/// Elements written as text, so that `1` and `1.0`, `-0` and `0`, and keys in different orders all occur.
const ELEMENTS: [&str; 18] = [
    "1",
    "1.0",
    "-0",
    "0",
    "-0.0",
    "0.0",
    "2.5",
    "2",
    "\"a\"",
    "\"b\"",
    "null",
    "true",
    r#"{"a":1,"b":2}"#,
    r#"{"b":2,"a":1}"#,
    r#"{"b":2.0,"a":1.0}"#,
    "[1,2]",
    "[1.0,2.0]",
    "[2,1]",
];

fn stage_strategy() -> impl Strategy<Value = Stage> {
    prop_oneof![
        prop_oneof![
            Just(Predicate::EvenPosition),
            Just(Predicate::Early),
            Just(Predicate::Always)
        ]
        .prop_map(Stage::Filter),
        prop_oneof![
            Just(Projection::Same),
            Just(Projection::WithPosition),
            Just(Projection::Position)
        ]
        .prop_map(Stage::Map),
        prop_oneof![
            Just(Spread::Twice),
            Just(Spread::Nothing),
            Just(Spread::Position)
        ]
        .prop_map(Stage::FlatMap),
        (0u32..6).prop_map(Stage::Skip),
        (0u32..6).prop_map(Stage::Take),
        prop_oneof![Just(Key::Element), Just(Key::ByElement), Just(Key::Parity)]
            .prop_map(Stage::Distinct),
        (1u32..4).prop_map(Stage::Window),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn a_chain_gives_what_the_reference_gives(
        stages in prop::collection::vec(stage_strategy(), 1..=8),
        picks in prop::collection::vec(0..ELEMENTS.len(), 0..24),
    ) {
        let text = format!("[{}]", picks.iter().map(|i| ELEMENTS[*i]).collect::<Vec<_>>().join(","));
        let list: Vec<Value> = serde_json::from_str(&text).expect("json");
        let expected = reference(&stages, list.clone());
        let chain = chain(stages.iter().map(|s| s.operator()).collect());
        let output = run(&chain, &json!({ "list": list }), &ChainCeilings::COLLECTION)
            .expect("the generated chains cannot fail");
        prop_assert_eq!(values(&output), expected);
        // The bytes are the compact JSON of each value, what the snapshot stores.
        for item in &output.items {
            let value: Value = serde_json::from_slice(item).expect("json");
            prop_assert_eq!(&serde_json::to_vec(&value).expect("json"), item);
        }
    }

    #[test]
    fn values_that_compare_equal_have_one_canonical_form(
        a in 0..ELEMENTS.len(),
        b in 0..ELEMENTS.len(),
    ) {
        let a: Value = serde_json::from_str(ELEMENTS[a]).expect("json");
        let b: Value = serde_json::from_str(ELEMENTS[b]).expect("json");
        prop_assert_eq!(language_equal(&a, &b), distinct_form(&a) == distinct_form(&b));
    }

    #[test]
    fn integers_and_integral_doubles_within_2_pow_53_share_a_form(n in -(1i64 << 53)..=(1i64 << 53)) {
        #[allow(clippy::cast_precision_loss)]
        let double = n as f64;
        prop_assert_eq!(distinct_form(&json!(n)), distinct_form(&json!(double)));
    }
}

/// T042-9: the evaluator alone at the ceilings, measured, not asserted. Run with
/// `cargo test -p sagat-infra-ir --lib --release -- --ignored measure_the_chain --nocapture`.
#[test]
#[ignore = "a measurement (T042-9): run on purpose and record the result"]
fn measure_the_chain_at_its_ceilings() {
    let rounds = 20;
    let measure = |name: &str, chain: &OperatorChain, elements: &[Vec<u8>]| {
        let mut samples: Vec<Duration> = (0..rounds)
            .map(|_| {
                let started = std::time::Instant::now();
                let answer = JsonEvaluator.collection_chain(
                    ChainInput::Elements(elements),
                    chain,
                    &RunData::default(),
                    &ChainCeilings::COLLECTION,
                );
                let elapsed = started.elapsed();
                assert!(matches!(answer, Ok(ChainAnswer::Output(_))), "{answer:?}");
                elapsed
            })
            .collect();
        samples.sort();
        println!(
            "T042-9 evaluator {name}: n={rounds} p50={:.2} ms p95={:.2} ms max={:.2} ms",
            samples[rounds / 2].as_secs_f64() * 1e3,
            samples[(rounds * 95) / 100].as_secs_f64() * 1e3,
            samples[rounds - 1].as_secs_f64() * 1e3
        );
    };
    let small: Vec<Vec<u8>> = (0..1000).map(|n: u32| n.to_string().into_bytes()).collect();
    measure(
        "eight stages over 1,000 small elements",
        &chain(vec![
            map("element + 1"),
            filter("element > 0"),
            map("element * 2"),
            Operator::Distinct { by: None },
            map("element - 1"),
            Operator::Skip { count: 0 },
            map("element"),
            Operator::Take { count: 1000 },
        ]),
        &small,
    );
    let big: Vec<Vec<u8>> = (0..16)
        .map(|_| serde_json::to_vec(&string_of(256 * 1024)).expect("json"))
        .collect();
    measure(
        "eight map stages of 4 MiB (sixteen 256 KiB elements)",
        &chain((0..8).map(|_| map("element")).collect()),
        &big,
    );
    let many: Vec<Vec<u8>> = (0..1000)
        .map(|_| serde_json::to_vec(&string_of(4 * 1024)).expect("json"))
        .collect();
    measure(
        "eight map stages of 4 MiB (1,000 elements of 4 KiB)",
        &chain((0..8).map(|_| map("element")).collect()),
        &many,
    );
}

// --- the review of S1 (2026-11-15) -----------------------------------------------------------------------------------

fn elements_of(values: &[Value]) -> Vec<Vec<u8>> {
    values
        .iter()
        .map(|value| serde_json::to_vec(value).expect("json"))
        .collect()
}

/// A stage expression that names a 256 KiB element about 120 times is refused by the bound on what one evaluation
/// copies, and the stage fails typed, instead of building hundreds of MiB for a boolean.
#[test]
fn a_stage_expression_that_copies_its_element_many_times_fails_its_stage() {
    let sixty = vec!["element"; 60].join(", ");
    let elements = elements_of(&[string_of(256 * 1024)]);
    assert_eq!(
        JsonEvaluator.collection_chain(
            ChainInput::Elements(&elements),
            &chain(vec![filter(&format!("size([[{sixty}], [{sixty}]]) > 0"))]),
            &RunData::default(),
            &ChainCeilings::COLLECTION,
        ),
        Err(ChainFailure::Stage {
            stage: 0,
            operator: OperatorName::Filter,
            position: 0,
            fault: StageFault::Expression
        })
    );
}

/// The same bound holds for `items`, before any stage, and for a collection without operators.
#[test]
fn an_items_expression_that_copies_a_value_many_times_fails() {
    let sixty = vec!["input.big"; 60].join(", ");
    let items = expression(&format!("[[{sixty}], [{sixty}]]"));
    let data = data(&json!({ "big": string_of(256 * 1024 - 64) }));
    assert_eq!(
        JsonEvaluator.collection_chain(
            ChainInput::Items(&items),
            &chain(vec![map("element")]),
            &data,
            &ChainCeilings::COLLECTION,
        ),
        Err(ChainFailure::Items)
    );
    assert!(
        JsonEvaluator.collection_source(&items, &data).is_err(),
        "a collection without operators is bounded the same way"
    );
}

/// An element nested too deep would be stored and then refused by every parser that reads it back (the JSON parser
/// stops below 128 levels, and the snapshot array and a task's input wrap an item in more), so the run would be retried
/// at every tick: it is refused when produced, with the budget code.
#[test]
fn an_element_deeper_than_120_levels_fails_with_the_budget_code() {
    // The language nests at most seven lists in one expression, so each `map` below adds seven levels.
    let seven = format!("{}element{}", "[".repeat(7), "]".repeat(7));
    let nested = |levels: usize| {
        let mut value = json!(1);
        for _ in 0..levels {
            value = json!([value]);
        }
        value
    };
    // 120 levels in, 127 after the first stage: over the 120 an element may nest, the first stage fails, typed.
    let elements = elements_of(&[nested(120)]);
    assert_eq!(
        JsonEvaluator.collection_chain(
            ChainInput::Elements(&elements),
            &chain((0..8).map(|_| map(&seven)).collect()),
            &RunData::default(),
            &ChainCeilings::COLLECTION,
        ),
        Err(budget(
            ChainCeiling::ElementDepth,
            Some(0),
            Some(OperatorName::Map),
            Some(0)
        ))
    );
    // 113 + 7: 120 nested lists are accepted, and the snapshot array that holds them reads back.
    let elements = elements_of(&[nested(113)]);
    let answer = JsonEvaluator
        .collection_chain(
            ChainInput::Elements(&elements),
            &chain(vec![map(&seven)]),
            &RunData::default(),
            &ChainCeilings::COLLECTION,
        )
        .expect("128 levels");
    let ChainAnswer::Output(output) = answer else {
        panic!("not a blob")
    };
    let snapshot = format!(
        "[{}]",
        String::from_utf8(output.items[0].clone()).expect("utf-8")
    );
    assert!(serde_json::from_str::<Value>(&snapshot).is_ok());
    // A window adds a level too.
    let elements = elements_of(&[nested(120)]);
    assert!(matches!(
        JsonEvaluator.collection_chain(
            ChainInput::Elements(&elements),
            &chain(vec![Operator::Window { count: 1 }]),
            &RunData::default(),
            &ChainCeilings::COLLECTION,
        ),
        Err(ChainFailure::Budget {
            ceiling: ChainCeiling::ElementDepth,
            ..
        })
    ));
}

/// A variable only a stage reads that cannot be read names that stage, not `items`.
#[test]
fn a_stage_variable_that_cannot_be_read_fails_that_stage() {
    let mut data = data(&json!({"list": [1, 2]}));
    data.state = Some(b"{not json".to_vec());
    let items = expression("input.list");
    assert_eq!(
        JsonEvaluator.collection_chain(
            ChainInput::Items(&items),
            &chain(vec![map("element"), filter("state.on")]),
            &data,
            &ChainCeilings::COLLECTION,
        ),
        Err(ChainFailure::Stage {
            stage: 1,
            operator: OperatorName::Filter,
            position: 0,
            fault: StageFault::Expression
        })
    );
}
