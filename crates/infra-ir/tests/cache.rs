//! The definition cache is bounded by weight as well as by count (a security-review finding: 1024
//! entries of 8 MB each are 8 GB in a 512 MB container).
#![allow(clippy::expect_used)]

use std::sync::Arc;

use sagat_application::execution::DefinitionCache;
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_ir::JsonIrCodec;
use serde_json::json;

/// A chain of `n` activities as a workflow document.
fn chain(name: &str, n: usize) -> Vec<u8> {
    let nodes: Vec<_> = (0..n)
        .map(|i| {
            let mut node = json!({"id": format!("n{i}"), "type": "activity", "activity": "act.x"});
            if i + 1 < n {
                node["next"] = json!(format!("n{}", i + 1));
            }
            node
        })
        .collect();
    serde_json::to_vec(&json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "n0", "nodes": nodes,
    }))
    .expect("json")
}

#[test]
fn old_entries_are_evicted_by_weight_not_only_by_count() {
    let codec = Arc::new(JsonIrCodec);
    // Room for about two thousand-node definitions, though the count limit would allow a hundred.
    let one = JsonIrCodec;
    let sample = sagat_application::definitions::IrCodec::parse(&one, &chain("sample", 1000))
        .expect("parses")
        .definition
        .footprint();
    let cache = DefinitionCache::with_budget(codec, 100, sample * 5 / 2);

    let (a, b, c) = (
        WorkflowVersionId::generate(),
        WorkflowVersionId::generate(),
        WorkflowVersionId::generate(),
    );
    for (id, name) in [(a, "aaa-flow"), (b, "bbb-flow"), (c, "ccc-flow")] {
        cache.get_or_parse(id, &chain(name, 1000)).expect("parses");
    }
    assert!(
        cache.get(a).is_none(),
        "the least recently used was evicted to stay within the weight"
    );
    assert!(cache.get(b).is_some() && cache.get(c).is_some());

    // Using an entry protects it: `b` is now the most recent, so the next insertion evicts `c`.
    assert!(cache.get(b).is_some());
    let d = WorkflowVersionId::generate();
    cache
        .get_or_parse(d, &chain("ddd-flow", 1000))
        .expect("parses");
    assert!(cache.get(c).is_none());
    assert!(cache.get(b).is_some() && cache.get(d).is_some());
}

#[test]
fn a_definition_heavier_than_the_whole_budget_still_works_and_is_the_only_one_kept() {
    let cache = DefinitionCache::with_budget(Arc::new(JsonIrCodec), 100, 1);
    let (a, b) = (WorkflowVersionId::generate(), WorkflowVersionId::generate());
    cache
        .get_or_parse(a, &chain("aaa-flow", 50))
        .expect("parses");
    assert!(
        cache.get(a).is_some(),
        "the entry just parsed is always usable"
    );
    cache
        .get_or_parse(b, &chain("bbb-flow", 50))
        .expect("parses");
    assert!(cache.get(b).is_some());
    assert!(cache.get(a).is_none(), "and it displaced the other");
}
