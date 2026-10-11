//! The comparison of two workflow documents (FR-007-004, D140): which nodes and edges changed, and
//! which changes break the contract with callers and emitters or need somebody to prepare.
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use sagat_application::definitions::{IrCodec, IrError, MAX_IR_BYTES};
use sagat_application::diff::{Change, DocumentDiff};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn diff(from: &Value, to: &Value) -> DocumentDiff {
    JsonIrCodec
        .diff(from.to_string().as_bytes(), to.to_string().as_bytes())
        .expect("two objects can be compared")
}

fn base() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "demo-flow"},
        "entrypoint": "first",
        "triggers": [{"kind": "manual"}],
        "inputSchema": {
            "type": "object",
            "required": ["orderId"],
            "properties": {"orderId": {"type": "string"}, "note": {"type": "string"}}
        },
        "outputSchema": {
            "type": "object",
            "properties": {"receipt": {"type": "string"}}
        },
        "nodes": [
            {"id": "first", "type": "activity", "activity": "order.check", "next": "second"},
            {"id": "second", "type": "activity", "activity": "order.ship", "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn codes(findings: &[sagat_application::diff::DiffFinding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.code).collect()
}

fn with_node(mut doc: Value, node: Value) -> Value {
    doc["nodes"].as_array_mut().expect("nodes").push(node);
    doc
}

#[test]
fn the_same_document_twice_is_identical_whatever_the_order_of_its_keys() {
    let a = base();
    let reordered: Value = serde_json::from_str(
        r#"{"nodes":[{"next":"second","activity":"order.check","type":"activity","id":"first"},
            {"next":"done","activity":"order.ship","type":"activity","id":"second"},{"type":"end","id":"done"}],
            "entrypoint":"first","triggers":[{"kind":"manual"}],"kind":"Workflow",
            "apiVersion":"sagat-octopus-orchestrator.io/v1","metadata":{"name":"demo-flow"},
            "inputSchema":{"properties":{"note":{"type":"string"},"orderId":{"type":"string"}},"required":["orderId"],"type":"object"},
            "outputSchema":{"properties":{"receipt":{"type":"string"}},"type":"object"}}"#,
    )
    .expect("json");
    let d = diff(&a, &reordered);
    assert!(d.identical, "{d:?}");
    assert_eq!(
        d,
        DocumentDiff {
            identical: true,
            ..DocumentDiff::default()
        }
    );
}

#[test]
fn nodes_added_removed_and_changed_are_listed_with_the_fields_that_differ() {
    let mut to = with_node(
        base(),
        json!({"id": "audit", "type": "activity", "activity": "order.audit", "next": "done"}),
    );
    to["nodes"][0]["activity"] = json!("order.verify");
    to["nodes"][0]["retry"] = json!({"maxAttempts": 3});
    to["nodes"].as_array_mut().expect("nodes").remove(2); // "done" is gone
    to["nodes"][1]["next"] = json!("audit");

    let d = diff(&base(), &to);
    assert!(!d.identical);
    assert_eq!(d.nodes_added, ["audit"]);
    assert_eq!(d.nodes_removed, ["done"]);
    let changed: Vec<_> = d
        .nodes_changed
        .iter()
        .map(|c| {
            (
                c.id.as_str(),
                c.fields.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        changed,
        [
            ("first", vec!["activity", "retry"]),
            ("second", vec!["next"])
        ]
    );
}

#[test]
fn edges_are_read_from_every_field_that_points_at_another_node() {
    let from = json!({
        "metadata": {"name": "demo-flow"}, "entrypoint": "c",
        "nodes": [
            {"id": "c", "type": "condition", "expression": "x", "ifTrue": "p", "ifFalse": "w"},
            {"id": "p", "type": "parallel", "branches": ["a", "b"], "join": "all", "next": "w"},
            {"id": "w", "type": "waitEvent", "eventType": "order.paid", "timeoutMs": 1000, "onTimeout": "s", "next": "s"},
            {"id": "s", "type": "saga", "begin": "a", "next": "e", "onError": "e"},
            {"id": "a", "type": "activity", "activity": "x.a"},
            {"id": "b", "type": "activity", "activity": "x.b"},
            {"id": "e", "type": "end"}
        ]
    });
    let mut to = from.clone();
    to["nodes"][0]["ifFalse"] = json!("e");
    to["nodes"][1]["branches"] = json!(["a", "b", "s"]);
    to["nodes"][3]["begin"] = json!("b");

    let d = diff(&from, &to);
    let show = |edges: &[sagat_application::diff::DiffEdge]| {
        edges
            .iter()
            .map(|e| format!("{}-{}->{}", e.from, e.kind, e.to))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        show(&d.edges_added),
        ["c-false->e", "p-branch->s", "s-begin->b"]
    );
    assert_eq!(show(&d.edges_removed), ["c-false->w", "s-begin->a"]);
}

#[test]
fn what_is_outside_the_nodes_is_listed_by_path() {
    let mut to = base();
    to["metadata"]["description"] = json!("now with a description");
    to["entrypoint"] = json!("second");
    to["triggers"] = json!([{"kind": "manual"}, {"kind": "event", "eventType": "order.created"}]);
    let d = diff(&base(), &to);
    let listed: Vec<_> = d
        .changes
        .iter()
        .map(|c| (c.path.as_str(), c.change))
        .collect();
    assert_eq!(
        listed,
        [
            ("/entrypoint", Change::Changed),
            ("/metadata/description", Change::Added),
            ("/triggers/1", Change::Added),
        ]
    );
    assert!(
        d.breaking.is_empty(),
        "adding a trigger breaks nobody: {:?}",
        d.breaking
    );
}

#[test]
fn removing_or_changing_a_trigger_breaks_the_contract_adding_one_does_not() {
    let mut removed = base();
    removed["triggers"] = json!([]);
    assert_eq!(
        codes(&diff(&base(), &removed).breaking),
        ["TRIGGER_REMOVED"]
    );

    let mut from = base();
    from["triggers"] = json!([{"kind": "event", "eventType": "order.created"}]);
    let mut changed = base();
    changed["triggers"] = json!([{"kind": "event", "eventType": "order.placed"}]);
    let d = diff(&from, &changed);
    assert_eq!(codes(&d.breaking), ["TRIGGER_REMOVED"], "{:?}", d.breaking);

    let mut tightened = from.clone();
    tightened["triggers"][0]["filter"] = json!("data.amount > 10");
    assert_eq!(
        codes(&diff(&from, &tightened).breaking),
        ["TRIGGER_CHANGED"]
    );

    let mut added = base();
    added["triggers"] = json!([{"kind": "manual"}, {"kind": "cron", "cron": "0 * * * *"}]);
    assert!(diff(&base(), &added).breaking.is_empty());
}

#[test]
fn the_input_contract_breaks_when_a_property_goes_changes_type_or_becomes_required() {
    let mut to = base();
    to["inputSchema"]["properties"]
        .as_object_mut()
        .expect("props")
        .remove("note");
    to["inputSchema"]["properties"]["orderId"]["type"] = json!("integer");
    to["inputSchema"]["required"] = json!(["orderId", "customer"]);
    to["inputSchema"]["properties"]["customer"] = json!({"type": "string"});
    let d = diff(&base(), &to);
    assert_eq!(
        codes(&d.breaking),
        [
            "INPUT_PROPERTY_REMOVED",
            "INPUT_TYPE_CHANGED",
            "INPUT_REQUIRED_ADDED"
        ]
    );
    let paths: Vec<_> = d
        .breaking
        .iter()
        .map(|f| f.path.as_deref().expect("path"))
        .collect();
    assert_eq!(
        paths,
        [
            "/inputSchema/properties/note",
            "/inputSchema/properties/orderId/type",
            "/inputSchema/required"
        ]
    );

    // Relaxing the contract, or adding an optional property, breaks nobody.
    let mut relaxed = base();
    relaxed["inputSchema"]["required"] = json!([]);
    relaxed["inputSchema"]["properties"]["extra"] = json!({"type": "number"});
    assert!(diff(&base(), &relaxed).breaking.is_empty());
}

#[test]
fn a_required_input_added_to_a_workflow_that_took_anything_breaks_its_callers() {
    let mut from = base();
    from.as_object_mut().expect("object").remove("inputSchema");
    let d = diff(&from, &base());
    assert_eq!(
        codes(&d.breaking),
        ["INPUT_REQUIRED_ADDED"],
        "{:?}",
        d.breaking
    );
    // Describing an input without requiring anything breaks nobody.
    let mut loose = base();
    loose["inputSchema"] = json!({"type": "object", "properties": {"orderId": {"type": "string"}}});
    assert!(diff(&from, &loose).breaking.is_empty());
    // Dropping the schema altogether accepts more, and an output nobody described promised nothing.
    let mut none = base();
    none.as_object_mut().expect("object").remove("inputSchema");
    none.as_object_mut().expect("object").remove("outputSchema");
    assert!(
        diff(&base(), &none)
            .breaking
            .iter()
            .all(|f| f.code.starts_with("OUTPUT"))
    );
    assert!(
        diff(&none, &base())
            .breaking
            .iter()
            .all(|f| f.code != "OUTPUT_PROPERTY_REMOVED")
    );
}

#[test]
fn the_output_contract_breaks_when_a_property_goes_or_changes_type() {
    let mut gone = base();
    gone["outputSchema"]["properties"] = json!({});
    assert_eq!(
        codes(&diff(&base(), &gone).breaking),
        ["OUTPUT_PROPERTY_REMOVED"]
    );

    let mut retyped = base();
    retyped["outputSchema"]["properties"]["receipt"]["type"] = json!("object");
    assert_eq!(
        codes(&diff(&base(), &retyped).breaking),
        ["OUTPUT_TYPE_CHANGED"]
    );

    let mut grown = base();
    grown["outputSchema"]["properties"]["trace"] = json!({"type": "string"});
    assert!(diff(&base(), &grown).breaking.is_empty());
}

fn waiting(id: &str, event: &str) -> Value {
    json!({"id": id, "type": "waitEvent", "eventType": event, "correlationExpr": "input.orderId", "timeoutMs": 60000, "next": "done"})
}

#[test]
fn an_awaited_event_type_that_disappears_breaks_its_emitters() {
    let from = with_node(base(), waiting("paid", "order.paid"));

    let mut renamed = with_node(base(), waiting("paid", "order.settled"));
    let d = diff(&from, &renamed);
    assert_eq!(
        codes(&d.breaking),
        ["EVENT_TYPE_REMOVED"],
        "{:?}",
        d.breaking
    );
    assert_eq!(d.breaking[0].node.as_deref(), Some("paid"));

    let gone = base();
    assert_eq!(codes(&diff(&from, &gone).breaking), ["EVENT_TYPE_REMOVED"]);

    // The same event type awaited by a node with another id is the same contract.
    renamed = with_node(base(), waiting("payment_received", "order.paid"));
    assert!(diff(&from, &renamed).breaking.is_empty());
}

#[test]
fn what_somebody_must_prepare_is_flagged_but_does_not_break_the_contract() {
    let mut to = with_node(
        base(),
        json!({"id": "audit", "type": "activity", "activity": "order.audit", "next": "done"}),
    );
    to["nodes"][0]["activity"] = json!("order.verify");
    to["nodes"][1]["secrets"] = json!(["carrier-key"]);
    to["nodes"].as_array_mut().expect("nodes").push(
        json!({"id": "run", "type": "script", "artifactDigest": "a".repeat(64), "next": "done"}),
    );
    let d = diff(&base(), &to);
    assert!(d.breaking.is_empty(), "{:?}", d.breaking);
    let attention: Vec<_> = d
        .attention
        .iter()
        .map(|f| (f.code, f.node.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(
        attention,
        [
            ("ACTIVITY_ADDED", "audit"),
            ("ACTIVITY_CHANGED", "first"),
            ("SCRIPT_CHANGED", "run"),
            ("SECRET_ADDED", "second"),
        ]
    );

    let mut other_script = to.clone();
    other_script["nodes"][4]["artifactDigest"] = json!("b".repeat(64));
    let again = diff(&to, &other_script);
    assert_eq!(
        again.attention.iter().map(|f| f.code).collect::<Vec<_>>(),
        ["SCRIPT_CHANGED"]
    );
}

#[test]
fn a_workflow_with_no_version_compares_an_empty_document_with_its_draft() {
    let d = diff(&json!({}), &base());
    assert!(!d.identical);
    assert_eq!(d.nodes_added, ["done", "first", "second"]);
    assert!(d.nodes_removed.is_empty());
    assert!(
        d.breaking.is_empty(),
        "nothing existed to break: {:?}",
        d.breaking
    );
}

#[test]
fn a_draft_that_does_not_validate_can_still_be_compared() {
    let mut to = base();
    to["nodes"][0]["next"] = json!("nowhere");
    to["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(json!({"type": "activity"})); // no id
    to["unknown"] = json!(true);
    let d = diff(&base(), &to);
    assert!(!d.identical);
    assert_eq!(d.nodes_changed[0].id, "first");
    assert!(
        d.changes.iter().any(|c| c.path == "/unknown"),
        "{:?}",
        d.changes
    );
}

#[test]
fn a_side_that_is_not_a_json_object_or_is_too_large_cannot_be_compared() {
    let ok = base().to_string();
    for bad in ["[]", "42", "not json", "null", ""] {
        assert_eq!(
            JsonIrCodec.diff(ok.as_bytes(), bad.as_bytes()).unwrap_err(),
            IrError::Malformed,
            "to = {bad}"
        );
        assert_eq!(
            JsonIrCodec.diff(bad.as_bytes(), ok.as_bytes()).unwrap_err(),
            IrError::Malformed,
            "from = {bad}"
        );
    }
    let big = vec![b' '; MAX_IR_BYTES + 1];
    assert_eq!(
        JsonIrCodec.diff(ok.as_bytes(), &big).unwrap_err(),
        IrError::TooLarge
    );
}

#[test]
fn findings_never_repeat_a_value_of_the_document_only_names_and_paths() {
    let mut from = base();
    from["triggers"] = json!([{"kind": "event", "eventType": "order.created", "filter": "data.secretish == 'hunter2'"}]);
    let mut to = base();
    to["triggers"] = json!([]);
    let d = diff(&from, &to);
    for finding in d.breaking.iter().chain(&d.attention) {
        assert!(!finding.message.contains("hunter2"), "{finding:?}");
    }
}

fn node_ids() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::btree_set("[a-z]{1,4}", 1..8).prop_map(|s| s.into_iter().collect())
}

proptest! {
    #[test]
    fn comparing_a_document_with_itself_is_identical_and_the_other_way_round_swaps_added_and_removed(
        left in node_ids(),
        right in node_ids(),
    ) {
        let doc = |ids: &Vec<String>| json!({
            "metadata": {"name": "demo-flow"}, "entrypoint": ids[0],
            "nodes": ids.iter().map(|id| json!({"id": id, "type": "end"})).collect::<Vec<_>>()
        });
        let (a, b) = (doc(&left), doc(&right));
        prop_assert!(diff(&a, &a).identical);
        let forward = diff(&a, &b);
        let backward = diff(&b, &a);
        prop_assert_eq!(&forward.nodes_added, &backward.nodes_removed);
        prop_assert_eq!(&forward.nodes_removed, &backward.nodes_added);
        let mut sorted = forward.nodes_added.clone();
        sorted.sort();
        prop_assert_eq!(&forward.nodes_added, &sorted);
    }
}

// ------------------------------------------------------------------------- a hostile draft

/// A draft is stored after a size check only (a published version is limited to sixteen triggers, a
/// draft is not), so a megabyte of it can be a hundred thousand triggers. Comparing it with itself
/// must cost what reading it costs, on a thread the whole platform shares.
#[test]
fn a_draft_full_of_triggers_is_compared_in_the_time_it_takes_to_read_it() {
    let mut doc = base();
    doc["triggers"] = Value::Array((0..40_000).map(|i| json!(i)).collect());
    let started = std::time::Instant::now();
    let d = diff(&doc, &doc);
    let took = started.elapsed();
    assert!(d.identical);
    assert!(
        took < std::time::Duration::from_millis(1500),
        "comparing a document with itself took {took:?}"
    );
}

/// The answer is read by a person on a page: what is not listed is counted, not sent.
#[test]
fn an_answer_about_tens_of_thousands_of_differences_is_bounded_and_says_how_many_it_left_out() {
    let mut from = base();
    from["triggers"] = Value::Array(
        (0..15_000)
            .map(|i| json!({"kind": "event", "eventType": format!("t{i}")}))
            .collect(),
    );
    let mut to = base();
    to["triggers"] = json!([]);
    to["metadata"] = json!({"name": "demo-flow", "labels": Value::Array((0..5_000).map(|i| json!(i)).collect())});
    let d = diff(&from, &to);
    assert!(d.breaking.len() <= 101, "{} findings", d.breaking.len());
    assert!(
        d.breaking.iter().any(|f| f.code == "MORE_FINDINGS"),
        "{:?}",
        codes(&d.breaking)
    );
    assert!(d.changes.len() <= 1_000, "{} changes", d.changes.len());
    assert!(d.changes_omitted > 3_000, "{}", d.changes_omitted);
    assert!(d.attention.len() <= 101);
}

/// A document is nested up to a hundred levels and its names are as long as the megabyte allows: a path
/// built at every one of thousands of places would cost gigabytes, and a path is only worth what it
/// helps a person find.
#[test]
fn thousands_of_differences_deep_in_a_nested_document_cost_what_reading_it_costs_and_the_answer_stays_small()
 {
    // 30 levels of 7 000-character names, with three thousand properties at the bottom of the one
    // document and none in the other.
    let nest = |leaves: usize| {
        let mut bottom = serde_json::Map::new();
        for i in 0..leaves {
            bottom.insert(format!("p{i}"), json!({}));
        }
        let mut schema = json!({"type": "object", "properties": Value::Object(bottom)});
        for level in (0..30).rev() {
            let name = format!("{}{level}", "n".repeat(7_000));
            schema = json!({"type": "object", "properties": {name: schema}});
        }
        schema
    };
    let mut from = base();
    from["inputSchema"] = nest(3_000);
    from["outputSchema"] = nest(3_000);
    let mut to = base();
    to["inputSchema"] = nest(0);
    to["outputSchema"] = nest(0);
    let started = std::time::Instant::now();
    let d = diff(&from, &to);
    let took = started.elapsed();
    assert!(took < std::time::Duration::from_secs(2), "took {took:?}");
    assert!(d.breaking.iter().any(|f| f.code == "MORE_FINDINGS"));
    let answer: usize = d
        .breaking
        .iter()
        .chain(&d.attention)
        .map(|f| f.message.len() + f.path.as_ref().map_or(0, String::len))
        .sum::<usize>()
        + d.changes.iter().map(|c| c.path.len()).sum::<usize>();
    assert!(answer < 1_000_000, "the answer is {answer} bytes");
    // A path is clipped, not copied: nothing in it is longer than a name is allowed to be.
    assert!(
        d.changes.iter().all(|c| c.path.len() < 2_000),
        "a path of the changes is long"
    );
}
