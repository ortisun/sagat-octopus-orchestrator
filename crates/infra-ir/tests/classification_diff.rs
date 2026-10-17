//! Classification changes need attention without changing the workflow compatibility verdict.
#![allow(clippy::expect_used)]
use sagat_application::definitions::IrCodec;
use sagat_infra_ir::JsonIrCodec;
use serde_json::json;

#[test]
fn effective_levels_are_compared_per_path_with_inherited_children() {
    let before = json!({"inputSchema":{"type":"object","x-sagat-classification":"internal",
        "properties":{"receipt":{"type":"string"}}}});
    let mut after = before.clone();
    after["inputSchema"]["x-sagat-classification"] = json!("public");
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(diff.breaking.is_empty());
    let changes: Vec<_> = diff
        .attention
        .iter()
        .filter(|row| row.code == "CLASSIFICATION_RELAXED")
        .collect();
    assert_eq!(changes.len(), 2);
    assert!(
        changes
            .iter()
            .any(|row| row.path.as_deref() == Some("/inputSchema"))
    );
    assert!(
        changes
            .iter()
            .any(|row| row.path.as_deref() == Some("/inputSchema/properties/receipt"))
    );
    let reverse = JsonIrCodec
        .diff(
            &serde_json::to_vec(&after).expect("JSON"),
            &serde_json::to_vec(&before).expect("JSON"),
        )
        .expect("diff");
    assert_eq!(
        reverse
            .attention
            .iter()
            .filter(|row| row.code == "CLASSIFICATION_TIGHTENED")
            .count(),
        2
    );
    assert!(reverse.breaking.is_empty());
}

#[test]
fn named_body_carriers_with_the_same_node_id_remain_independent() {
    let before = json!({"bodies":{
        "east":{"entrypoint":"work","nodes":[{"id":"work","type":"activity","outputSchema":{"type":"string","x-sagat-classification":"pii"}}]},
        "west":{"entrypoint":"work","nodes":[{"id":"work","type":"activity","outputSchema":{"type":"string","x-sagat-classification":"pii"}}]}
    }});
    let mut after = before.clone();
    after["bodies"]["east"]["nodes"][0]["outputSchema"]["x-sagat-classification"] = json!("public");
    after["bodies"]["west"]["nodes"][0]["outputSchema"]["x-sagat-classification"] = json!("secret");
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(
        diff.attention
            .iter()
            .any(|row| row.code == "CLASSIFICATION_RELAXED"
                && row.path.as_deref() == Some("/bodies/east/nodes/0/outputSchema"))
    );
    assert!(
        diff.attention
            .iter()
            .any(|row| row.code == "CLASSIFICATION_TIGHTENED"
                && row.path.as_deref() == Some("/bodies/west/nodes/0/outputSchema"))
    );
    assert!(diff.breaking.is_empty());
}

#[test]
fn classification_after_the_human_report_cap_is_still_compared() {
    let properties = (0..550)
        .map(|index| {
            (
                format!("p{index:03}"),
                json!({"type":"string", "x-sagat-classification":"pii"}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let before =
        json!({"inputSchema":{"x-sagat-classification":"public", "properties":properties}});
    let mut after = before.clone();
    after["inputSchema"]["properties"]["p549"]["x-sagat-classification"] = json!("public");
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(
        diff.attention
            .iter()
            .any(|row| row.code == "CLASSIFICATION_RELAXED"
                && row.path.as_deref() == Some("/inputSchema/properties/p549"))
    );
}

#[test]
fn classification_findings_are_bounded_and_the_more_count_is_exact() {
    let properties = (0..125)
        .map(|index| {
            (
                format!("p{index:03}"),
                json!({"type":"string", "x-sagat-classification":"pii"}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let before =
        json!({"inputSchema":{"x-sagat-classification":"public", "properties":properties}});
    let mut after = before.clone();
    for index in 0..125 {
        after["inputSchema"]["properties"][format!("p{index:03}")]["x-sagat-classification"] =
            json!("public");
    }
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert_eq!(
        diff.attention
            .iter()
            .filter(|row| row.code == "CLASSIFICATION_RELAXED")
            .count(),
        100
    );
    assert!(
        diff.attention
            .iter()
            .any(|row| row.code == "MORE_FINDINGS" && row.message.starts_with("25 more"))
    );
}

#[test]
fn wildcard_and_numeric_property_names_do_not_collide_with_applicators() {
    let before = json!({"inputSchema":{
        "properties": {
            "*":{"x-sagat-classification":"pii"},
            "0":{"x-sagat-classification":"pii"}
        },
        "additionalProperties":{"x-sagat-classification":"public"},
        "prefixItems":[{"x-sagat-classification":"public"}],
        "items":{"x-sagat-classification":"pii"}
    }});
    let mut after = before.clone();
    after["inputSchema"]["properties"]["*"]["x-sagat-classification"] = json!("secret");
    after["inputSchema"]["properties"]["0"]["x-sagat-classification"] = json!("secret");
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert_eq!(
        diff.attention
            .iter()
            .filter(|row| row.code == "CLASSIFICATION_TIGHTENED")
            .count(),
        2
    );
}

#[test]
fn node_and_combiner_reordering_do_not_create_classification_findings() {
    let before = json!({
        "nodes":[
            {"id":"a","type":"activity","outputSchema":{"allOf":[
                {"properties":{"x":{"x-sagat-classification":"public"}}},
                {"additionalProperties":{"x-sagat-classification":"pii"}}
            ]}},
            {"id":"b","type":"activity","outputSchema":{"x-sagat-classification":"secret"}}
        ]
    });
    let after = json!({
        "nodes":[
            {"id":"b","type":"activity","outputSchema":{"x-sagat-classification":"secret"}},
            {"id":"a","type":"activity","outputSchema":{"allOf":[
                {"additionalProperties":{"x-sagat-classification":"pii"}},
                {"properties":{"x":{"x-sagat-classification":"public"}}}
            ]}}
        ]
    });
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(
        !diff
            .attention
            .iter()
            .any(|row| row.code.starts_with("CLASSIFICATION_"))
    );
}

#[test]
fn removed_locations_use_object_and_array_fallbacks() {
    let before = json!({"inputSchema":{
        "properties":{"gone":{"x-sagat-classification":"secret"}},
        "prefixItems":[{"x-sagat-classification":"secret"}]
    }});
    let after = json!({"inputSchema":{
        "additionalProperties":{"x-sagat-classification":"public"},
        "items":{"x-sagat-classification":"internal"}
    }});
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    let relaxed: Vec<_> = diff
        .attention
        .iter()
        .filter(|row| row.code == "CLASSIFICATION_RELAXED")
        .collect();
    assert!(
        relaxed
            .iter()
            .any(|row| row.path.as_deref() == Some("/inputSchema/properties/gone"))
    );
    assert!(
        relaxed
            .iter()
            .any(|row| row.path.as_deref() == Some("/inputSchema/prefixItems/0"))
    );
}

#[test]
fn exact_json_pointer_source_paths_escape_slashes_and_tildes() {
    let before = json!({"inputSchema":{"x-sagat-classification":"public", "properties":{"a/~b":{"x-sagat-classification":"pii"}}}});
    let mut after = before.clone();
    after["inputSchema"]["properties"]["a/~b"]["x-sagat-classification"] = json!("public");
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(
        diff.attention
            .iter()
            .any(|row| row.code == "CLASSIFICATION_RELAXED"
                && row.path.as_deref() == Some("/inputSchema/properties/a~1~0b"))
    );
}

#[test]
fn incomplete_carrier_is_reported_once_and_its_prefix_is_not_compared() {
    let before = json!({"inputSchema":{"x-sagat-classification":"secret"}});
    let after = json!({"inputSchema":{
        "x-sagat-classification":"public",
        "$ref":"https://outside.invalid/schema"
    }});
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(
        diff.attention
            .iter()
            .any(|row| row.code == "CLASSIFICATION_UNRESOLVED_REF")
    );
    assert!(
        !diff
            .attention
            .iter()
            .any(|row| row.code == "CLASSIFICATION_RELAXED")
    );
}

#[test]
fn absent_carrier_is_not_treated_as_an_implicit_pii_carrier() {
    let before = json!({});
    let after = json!({"inputSchema":{"x-sagat-classification":"public"}});
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(!diff.attention.iter().any(|row| matches!(
        row.code,
        "CLASSIFICATION_RELAXED" | "CLASSIFICATION_TIGHTENED"
    )));
}

#[test]
fn a_shared_reference_is_resolved_in_each_inherited_context() {
    let before = json!({"inputSchema":{
        "x-sagat-classification":"public",
        "$defs":{"shared":{"type":"string"}},
        "properties":{
            "open":{"$ref":"#/$defs/shared", "x-sagat-classification":"public"},
            "private":{"$ref":"#/$defs/shared", "x-sagat-classification":"secret"}
        }
    }});
    let mut after = before.clone();
    after["inputSchema"]["properties"]["open"]["x-sagat-classification"] = json!("internal");
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert_eq!(
        diff.attention
            .iter()
            .filter(|row| row.code == "CLASSIFICATION_TIGHTENED")
            .count(),
        1
    );
    assert!(diff.attention.iter().any(|row| {
        row.code == "CLASSIFICATION_TIGHTENED"
            && row.path.as_deref() == Some("/inputSchema/properties/open")
    }));
}

#[test]
fn object_fallback_does_not_jump_past_an_explicit_ancestor() {
    let before = json!({"inputSchema":{
        "x-sagat-classification":"public",
        "properties":{"a":{
            "type":"object", "x-sagat-classification":"public"
        }},
        "additionalProperties":{
            "type":"object", "x-sagat-classification":"public",
            "properties":{"b":{"x-sagat-classification":"public"}}
        }
    }});
    let mut after = before.clone();
    after["inputSchema"]["properties"]["a"]["properties"] =
        json!({"b":{"x-sagat-classification":"public"}});
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(diff.attention.iter().any(|row| {
        row.code == "CLASSIFICATION_RELAXED"
            && row.path.as_deref() == Some("/inputSchema/properties/a/properties/b")
    }));
}

#[test]
fn array_fallback_does_not_jump_past_an_explicit_ancestor() {
    let before = json!({"inputSchema":{
        "type":"array", "x-sagat-classification":"public",
        "prefixItems":[{
            "type":"array", "x-sagat-classification":"public"
        }],
        "items":{
            "type":"array", "x-sagat-classification":"public",
            "prefixItems":[{"x-sagat-classification":"public"}]
        }
    }});
    let mut after = before.clone();
    after["inputSchema"]["prefixItems"][0]["prefixItems"] =
        json!([{"x-sagat-classification":"public"}]);
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(diff.attention.iter().any(|row| {
        row.code == "CLASSIFICATION_RELAXED"
            && row.path.as_deref() == Some("/inputSchema/prefixItems/0/prefixItems/0")
    }));
}

#[test]
fn missing_location_floor_keeps_a_secret_ancestor() {
    let before = json!({"inputSchema":{
        "x-sagat-classification":"secret",
        "properties":{"gone":{"type":"string"}}
    }});
    let after = json!({"inputSchema":{
        "x-sagat-classification":"secret"
    }});
    let diff = JsonIrCodec
        .diff(
            &serde_json::to_vec(&before).expect("JSON"),
            &serde_json::to_vec(&after).expect("JSON"),
        )
        .expect("diff");
    assert!(!diff.attention.iter().any(|row| {
        matches!(
            row.code,
            "CLASSIFICATION_RELAXED" | "CLASSIFICATION_TIGHTENED"
        )
    }));
}
