//! The operator factory of a collection and `for_each_with_operators` / `map_with_operators` (design 042).
#![allow(clippy::expect_used)]
use sagat_octopus_orchestrator_sdk::{
    json,
    workflow::{Builder, CollectionOptions, StepOptions, ops},
};

fn line() -> Builder {
    let body = Builder::flow();
    body.step("price", "line.price", StepOptions::default())
        .return_value("return", "steps.price.output");
    body
}

fn options(max_items: u64, max_concurrency: u64) -> CollectionOptions {
    CollectionOptions {
        items: "input.lines".into(),
        body: "line".into(),
        result: "bodyResult".into(),
        max_items,
        max_concurrency,
    }
}

#[test]
fn the_factory_writes_one_stage_per_operator_keyed_by_its_name() {
    assert_eq!(
        ops::filter("element.total > 0").to_value(),
        json!({"filter": {"where": "element.total > 0"}})
    );
    assert_eq!(
        ops::map("element.lines").to_value(),
        json!({"map": {"select": "element.lines"}})
    );
    assert_eq!(
        ops::flat_map("element").to_value(),
        json!({"flatMap": {"select": "element"}})
    );
    assert_eq!(ops::skip(0).to_value(), json!({"skip": {"count": 0}}));
    assert_eq!(ops::take(1000).to_value(), json!({"take": {"count": 1000}}));
    assert_eq!(
        ops::distinct_by("element.sku").to_value(),
        json!({"distinct": {"by": "element.sku"}})
    );
    assert_eq!(ops::distinct().to_value(), json!({"distinct": {}}));
    assert_eq!(ops::window(25).to_value(), json!({"window": {"count": 25}}));
    assert_eq!(ops::flat_map("element").name(), "flatMap");
}

#[test]
fn the_factory_only_assembles_and_leaves_ranges_to_the_server() {
    assert_eq!(ops::window(0).to_value(), json!({"window": {"count": 0}}));
    assert_eq!(ops::take(1001).to_value(), json!({"take": {"count": 1001}}));
}

#[test]
fn for_each_and_map_carry_the_operators_in_the_order_given() {
    let workflow = Builder::new("chained-flow");
    workflow
        .body("line", &line())
        .for_each_with_operators(
            "lines",
            options(9, 2),
            [ops::filter("element.ok"), ops::distinct(), ops::window(2)],
        )
        .map_with_operators("prices", options(9, 2), vec![ops::take(3)])
        .end("done", None);
    let document = workflow.build().expect("valid workflow");
    assert_eq!(
        document["nodes"][0]["operators"],
        json!([{"filter": {"where": "element.ok"}}, {"distinct": {}}, {"window": {"count": 2}}])
    );
    assert_eq!(
        document["nodes"][1]["operators"],
        json!([{"take": {"count": 3}}])
    );
    assert_eq!(document["nodes"][1]["type"], "forEach");
}

#[test]
fn every_present_collection_call_still_builds_without_operators() {
    let workflow = Builder::new("plain-flow");
    workflow
        .body("line", &line())
        .for_each("lines", options(17, 3))
        .map("prices", options(17, 3))
        .for_each_with_operators("empty", options(17, 3), Vec::new())
        .end("done", None);
    let document = workflow.build().expect("valid workflow");
    for position in 0..3 {
        assert!(document["nodes"][position].get("operators").is_none());
    }
    let present = CollectionOptions {
        items: "input.lines".into(),
        body: "line".into(),
        result: "bodyResult".into(),
        max_items: 2,
        max_concurrency: 1,
    };
    assert_eq!(present.max_items, 2);
}

#[test]
fn a_chained_collection_still_checks_its_bounds() {
    let workflow = Builder::new("bounded-flow");
    workflow
        .body("line", &line())
        .for_each_with_operators("lines", options(0, 1), [ops::take(1)])
        .end("done", None);
    assert!(workflow.build().is_err());
}
