//! The platform's validator of the run-input subset (design 043, T043-2): the pre-check, the compiled form, the work
//! counter and its budget, the refusal and its pointer, the cache, and the oracle property test against `jsonschema`.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_domain::identity::TenantId;
use sagat_domain::workflow::InputSchema;
use sagat_infra_schema::run_input::{
    CompiledInputSchema, InputSchemaIssue, InputSchemas, Limits, PathSegment, RUN_INPUT_WORK_UNITS,
    Rule, SchemaRefusal, Unusable, pointer_of, precheck,
};
use serde_json::{Value, json};

fn bytes(schema: &Value) -> Vec<u8> {
    serde_json::to_vec(schema).expect("schema bytes")
}

fn compiled(schema: &Value) -> CompiledInputSchema {
    CompiledInputSchema::compile(&bytes(schema))
        .unwrap_or_else(|e| panic!("{schema} compiles: {e:?}"))
}

fn refused(schema: &Value) -> Unusable {
    match CompiledInputSchema::compile(&bytes(schema)) {
        Err(unusable) => unusable,
        Ok(_) => panic!("{schema} must be refused"),
    }
}

fn check(schema: &Value, input: &Value) -> Result<(), SchemaRefusal> {
    compiled(schema).validate(&bytes(input))
}

/// The field and rule a refusal answers, as the gate will report them.
fn field_rule(result: Result<(), SchemaRefusal>) -> (String, &'static str) {
    match result {
        Err(SchemaRefusal::Invalid { segments, rule }) => (pointer_of(&segments), rule.as_str()),
        other => panic!("expected an invalid input, got {other:?}"),
    }
}

// ------------------------------------------------------------------ the vocabulary

#[test]
fn every_keyword_outside_the_subset_is_refused_with_its_name_and_pointer() {
    for keyword in [
        "patternProperties",
        "uniqueItems",
        "not",
        "if",
        "then",
        "else",
        "contains",
        "dependencies",
        "dependentRequired",
        "dependentSchemas",
        "unevaluatedProperties",
        "unevaluatedItems",
        "additionalItems",
        "propertyNames",
        "prefixItems",
        "$dynamicRef",
        "$recursiveRef",
        "$anchor",
        "$dynamicAnchor",
        "$recursiveAnchor",
        "id",
        "contentMediaType",
        "contentEncoding",
        "x-other",
    ] {
        let value = match keyword {
            "uniqueItems" => json!(true),
            "$dynamicRef" | "$recursiveRef" => json!("#"),
            "$anchor" | "$dynamicAnchor" | "id" | "contentMediaType" | "contentEncoding" => {
                json!("a")
            }
            "$recursiveAnchor" => json!(true),
            "dependentRequired" => json!({"a": ["b"]}),
            "prefixItems" => json!([{}]),
            _ => json!({}),
        };
        let mut inner = serde_json::Map::new();
        inner.insert(keyword.to_owned(), value);
        let schema = json!({"type": "object", "properties": {"a": Value::Object(inner)}});
        let unusable = refused(&schema);
        assert_eq!(
            unusable.issue,
            InputSchemaIssue::KeywordUnsupported,
            "{keyword}"
        );
        assert_eq!(unusable.keyword, keyword);
        assert_eq!(
            unusable.at,
            format!(
                "/properties/a/{}",
                keyword.replace('~', "~0").replace('/', "~1")
            )
        );
    }
}

#[test]
fn the_subset_rules_that_keep_the_drafts_equal_are_enforced() {
    let cases: Vec<(Value, InputSchemaIssue, &str, &str)> = vec![
        (
            json!({"items": [{}]}),
            InputSchemaIssue::KeywordUnsupported,
            "items",
            "/items",
        ),
        (
            json!({"exclusiveMinimum": true, "minimum": 1}),
            InputSchemaIssue::KeywordUnsupported,
            "exclusiveMinimum",
            "/exclusiveMinimum",
        ),
        (
            json!({"multipleOf": 0.01}),
            InputSchemaIssue::KeywordUnsupported,
            "multipleOf",
            "/multipleOf",
        ),
        (
            json!({"multipleOf": 0}),
            InputSchemaIssue::KeywordUnsupported,
            "multipleOf",
            "/multipleOf",
        ),
        (
            json!({"$defs": {"a": {}}, "properties": {"x": {"$ref": "#/$defs/a", "type": "string"}}}),
            InputSchemaIssue::ReferenceInvalid,
            "$ref",
            "/properties/x/$ref",
        ),
        (
            json!({"properties": {"x": {"$id": "https://example.invalid/x"}}}),
            InputSchemaIssue::KeywordUnsupported,
            "$id",
            "/properties/x/$id",
        ),
        (
            json!({"properties": {"x": {"$schema": "https://json-schema.org/draft/2020-12/schema"}}}),
            InputSchemaIssue::KeywordUnsupported,
            "$schema",
            "/properties/x/$schema",
        ),
        (
            json!({"$schema": "http://json-schema.org/draft-04/schema#"}),
            InputSchemaIssue::KeywordUnsupported,
            "$schema",
            "/$schema",
        ),
        (
            json!({"$schema": "https://json-schema.org/draft/2019-09/schema"}),
            InputSchemaIssue::KeywordUnsupported,
            "$schema",
            "/$schema",
        ),
        (
            json!({"type": "date"}),
            InputSchemaIssue::KeywordUnsupported,
            "type",
            "/type",
        ),
        (
            json!({"required": ["a", "a"]}),
            InputSchemaIssue::KeywordUnsupported,
            "required",
            "/required",
        ),
        (
            json!({"allOf": []}),
            InputSchemaIssue::KeywordUnsupported,
            "allOf",
            "/allOf",
        ),
        (
            json!({"minLength": -1}),
            InputSchemaIssue::KeywordUnsupported,
            "minLength",
            "/minLength",
        ),
        (
            json!({"properties": {"x": 3}}),
            InputSchemaIssue::KeywordUnsupported,
            "",
            "/properties/x",
        ),
    ];
    for (schema, issue, keyword, at) in cases {
        let unusable = refused(&schema);
        assert_eq!(
            (
                unusable.issue,
                unusable.keyword.as_str(),
                unusable.at.as_str()
            ),
            (issue, keyword, at),
            "{schema}"
        );
    }
    // Names are not keywords: a property called `id`, `$ref` or `patternProperties` is a name.
    let names = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://example.invalid/root",
        "type": "object",
        "properties": {"id": {"type": "string"}, "$ref": {"type": "integer"}, "patternProperties": {}},
        "$defs": {"$id": {}}, "definitions": {"not": true},
        "title": "t", "description": "d", "default": {}, "examples": [{}], "$comment": "c", "format": "anything",
        "x-sagat-classification": "public"
    });
    precheck(&bytes(&names))
        .expect("names, annotations and the root $schema and $id are in the subset");
    assert_eq!(check(&names, &json!({"id": "a", "$ref": 1})), Ok(()));
    assert_eq!(
        field_rule(check(&names, &json!({"$ref": "x"}))),
        ("/$ref".to_owned(), "type")
    );
    precheck(&bytes(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema"}),
    ))
    .expect("2020-12");
}

#[test]
fn format_and_the_annotations_are_inert() {
    let schema =
        json!({"type": "string", "format": "email", "x-sagat-classification": "pii", "title": "t"});
    assert_eq!(
        check(&schema, &json!("not an email")),
        Ok(()),
        "format is an annotation in every dialect"
    );
    let draft07 = json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "string", "format": "date"});
    assert_eq!(
        check(&draft07, &json!("tomorrow")),
        Ok(()),
        "also under draft-07"
    );
}

// ------------------------------------------------------------------ the pattern allowlist

#[test]
fn the_pattern_allowlist_refuses_what_ecma_262_and_the_regex_engine_read_differently() {
    for pattern in [
        r"\d",
        r"\D",
        r"\w",
        r"\W",
        r"\s",
        r"\S",
        r"\b",
        r"\B",
        r"[\d]",
        r"[\w-]",
        r"[^\s]",
        r"(?i)a",
        r"\A",
        r"\z",
        r"(?P<n>a)",
        r"(?<n>a)",
        r"[[:alpha:]]",
        r"\p{L}",
        r"\pL",
        r"[\p{L}]",
        r"[a&&b]",
        r"[a--b]",
        r"[a~~b]",
        r"[a[b]]",
        r"[]a]",
        r"[^]",
        r"[^]a]",
        r"a{101}",
        r"a{1,200}",
        r"(a)\1",
        r"(?=a)",
        r"(?!a)",
        r"(?<=a)b",
        "\\u0041",
        r"\x{41}",
        r"\a",
        r"\<",
        r"\0",
        "a{",
    ] {
        let unusable = refused(&json!({"properties": {"code": {"pattern": pattern}}}));
        assert_eq!(
            unusable.issue,
            InputSchemaIssue::PatternUnsupported,
            "{pattern}"
        );
        assert_eq!(
            (unusable.keyword.as_str(), unusable.at.as_str()),
            ("pattern", "/properties/code/pattern")
        );
    }
    for pattern in [
        r"^[a-z]{2,8}-[0-9]+$",
        r"\.",
        r"\n\r\t",
        r"\x41",
        r"[^a-c]",
        r"(?:ab)+|c*?",
        r"^$",
        r"a{100}",
        r"[\]]",
        r"\/",
    ] {
        precheck(&bytes(&json!({"pattern": pattern})))
            .unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
    }
    let dot = json!({"pattern": "^a.b$"});
    assert_eq!(check(&dot, &json!("axb")), Ok(()));
    for terminator in ["a\u{2028}b", "a\u{2029}b", "a\nb", "a\rb"] {
        assert_eq!(
            field_rule(check(&dot, &json!(terminator))),
            ("/".to_owned(), "pattern"),
            "{terminator:?}"
        );
    }
    assert_eq!(
        check(&json!({"pattern": "b"}), &json!("abc")),
        Ok(()),
        "unanchored search"
    );
}

#[test]
fn a_pattern_past_its_states_its_program_or_the_count_is_refused() {
    let states = refused(&json!({"pattern": "a{100}b{100}c{100}d{100}e{100}f{100}"}));
    assert_eq!(states.issue, InputSchemaIssue::PatternUnsupported);
    let mut many = serde_json::Map::new();
    for i in 0..33 {
        many.insert(format!("p{i:02}"), json!({"pattern": "a"}));
    }
    let count = refused(&json!({"properties": many}));
    assert_eq!(
        (count.issue, count.at.as_str()),
        (
            InputSchemaIssue::PatternUnsupported,
            "/properties/p32/pattern"
        )
    );
    let mut thirty_two = serde_json::Map::new();
    for i in 0..32 {
        thirty_two.insert(format!("p{i:02}"), json!({"pattern": "a"}));
    }
    precheck(&bytes(&json!({"properties": thirty_two}))).expect("32 patterns are allowed");
}

// ------------------------------------------------------------------ references

#[test]
fn a_reference_must_name_a_schema_position_of_the_document() {
    let base = |reference: &str| {
        json!({
            "$defs": {
                "a": {"type": "string"}, "a%b": {"type": "integer"}, "a~b": {"type": "boolean"}, "a/b": {"type": "null"}, "~1": {"type": "array"},
                "h": {"enum": [{}], "default": {}, "examples": [{}], "x-sagat-classification": {"k": {}}, "properties": {"p": {}}}
            },
            "properties": {"x": {"$ref": reference}}
        })
    };
    for target in [
        "#/$defs/h/enum/0",
        "#/$defs/h/default",
        "#/$defs/h/examples/0",
        "#/$defs/h/x-sagat-classification/k",
        "#/$defs/h/properties",
        "#/$defs",
        "#/$defs/missing",
        "https://example.invalid/s.json",
        "other.json#/$defs/a",
        "#a",
        "#/$defs/a%2",
        "#/$defs/a~2",
        "#/$defs/%FF",
        "#/allOf/01",
    ] {
        let unusable = refused(&base(target));
        assert_eq!(
            (
                unusable.issue,
                unusable.keyword.as_str(),
                unusable.at.as_str()
            ),
            (
                InputSchemaIssue::ReferenceInvalid,
                "$ref",
                "/properties/x/$ref"
            ),
            "{target}"
        );
    }
    for (target, accepted, refused_value) in [
        ("#/$defs/a", json!("s"), json!(1)),
        ("#/$defs/a%25b", json!(1), json!("s")),
        ("#/$defs/a~0b", json!(true), json!(1)),
        ("#/$defs/a~1b", json!(null), json!(1)),
        ("#/$defs/~01", json!([]), json!(1)),
    ] {
        let schema = base(target);
        assert_eq!(check(&schema, &json!({"x": accepted})), Ok(()), "{target}");
        assert_eq!(
            field_rule(check(&schema, &json!({"x": refused_value}))),
            ("/x".to_owned(), "type"),
            "{target}"
        );
    }
    let allof = json!({"allOf": [{"type": "string"}, {"$ref": "#/allOf/0"}]});
    assert_eq!(
        check(&allof, &json!("s")),
        Ok(()),
        "an index without a leading zero names its branch"
    );
}

#[test]
fn a_cycle_is_found_by_the_marked_search_through_all_of_and_through_properties() {
    let through_all_of = json!({
        "$defs": {"a": {"allOf": [{"$ref": "#/$defs/b"}]}, "b": {"allOf": [{"$ref": "#/$defs/a"}]}},
        "$ref": "#/$defs/a"
    });
    assert_eq!(
        refused(&through_all_of).issue,
        InputSchemaIssue::ReferenceInvalid
    );
    let through_properties = json!({"type": "object", "properties": {"child": {"$ref": "#"}}});
    let unusable = refused(&through_properties);
    assert_eq!(
        (
            unusable.issue,
            unusable.keyword.as_str(),
            unusable.at.as_str()
        ),
        (
            InputSchemaIssue::ReferenceInvalid,
            "$ref",
            "/properties/child/$ref"
        )
    );
    let unused = json!({"$defs": {"loop": {"$ref": "#/$defs/loop"}}});
    assert_eq!(
        refused(&unused).issue,
        InputSchemaIssue::ReferenceInvalid,
        "a cycle in an unused definition too"
    );
}

/// A schema whose root reaches `leaf` through `levels` levels of a `×fan` `allOf` of references.
fn dag(leaf: Value, levels: usize, fan: usize) -> Value {
    let mut defs = serde_json::Map::new();
    defs.insert("t0".to_owned(), leaf);
    for level in 1..=levels {
        let below = json!({"$ref": format!("#/$defs/t{}", level - 1)});
        defs.insert(format!("t{level}"), json!({"allOf": vec![below; fan]}));
    }
    json!({"$defs": defs, "$ref": format!("#/$defs/t{levels}")})
}

#[test]
fn a_reference_dag_compiles_to_an_arena_the_size_of_the_document() {
    let schema = dag(json!({"type": "object"}), 12, 2);
    let compiled = compiled(&schema);
    // the root, the leaf, and per level the allOf node and its two members
    assert_eq!(compiled.node_count(), 2 + 12 * 3, "{compiled:?}");
    assert_eq!(check(&schema, &json!({})), Ok(()));
    assert_eq!(
        field_rule(check(&schema, &json!([]))),
        ("/".to_owned(), "type")
    );
}

#[test]
fn an_application_path_of_129_is_refused_and_128_is_not() {
    let chain = |links: usize| {
        let mut defs = serde_json::Map::new();
        defs.insert(format!("d{links}"), json!({}));
        for i in (1..links).rev() {
            defs.insert(
                format!("d{i}"),
                json!({"$ref": format!("#/$defs/d{}", i + 1)}),
            );
        }
        json!({"$defs": defs, "$ref": "#/$defs/d1"})
    };
    let allowed = compiled(&chain(127));
    assert_eq!(allowed.application_path(), 128);
    let unusable = refused(&chain(128));
    assert_eq!(unusable.issue, InputSchemaIssue::TooDeep);
}

#[test]
fn size_and_depth_are_bounded_before_anything_else() {
    let names: Vec<String> = (0..7_000).map(|i| format!("name{i:05}")).collect();
    let large = refused(&json!({"required": names}));
    assert_eq!(large.issue, InputSchemaIssue::TooLarge);
    let mut deep = json!({});
    for _ in 0..33 {
        deep = json!({"allOf": [deep]});
    }
    assert_eq!(refused(&deep).issue, InputSchemaIssue::TooDeep);
}

// ------------------------------------------------------------------ the lower bound at publication

#[test]
fn a_schema_every_accepted_input_of_which_passes_the_budget_is_refused_at_publication() {
    let members: Vec<u32> = (0..1_000).collect();
    let costly = dag(json!({"enum": members}), 10, 2);
    let unusable = refused(&costly);
    assert_eq!(
        unusable.issue,
        InputSchemaIssue::TooCostly,
        "a ×2 allOf tree of depth 10 over 1,000 members"
    );
    let cheaper = dag(json!({"enum": members}), 9, 2);
    assert!(
        compiled(&cheaper).lower_bound() > 500_000,
        "depth 9 is publishable, and costs"
    );
    // anyOf counts its cheapest branch: one cheap and one costly branch is publishable.
    let mut either = dag(json!({"enum": members}), 10, 2);
    either.as_object_mut().unwrap().remove("$ref");
    either["anyOf"] = json!([{}, {"$ref": "#/$defs/t10"}]);
    let compiled = compiled(&either);
    assert!(compiled.lower_bound() < 10, "{}", compiled.lower_bound());
    assert_eq!(compiled.validate(&bytes(&json!(5))), Ok(()));
    // a bound that would overflow saturates instead
    let huge = dag(json!({"enum": members}), 22, 8);
    assert_eq!(refused(&huge).issue, InputSchemaIssue::TooCostly);
}

// ------------------------------------------------------------------ the refusal

#[test]
fn the_refusal_names_the_rule_and_a_pointer_of_declared_names_and_indices_only() {
    let schema = json!({
        "type": "object",
        "required": ["orderId"],
        "properties": {
            "orderId": {"type": "string", "maxLength": 8},
            "lines": {"type": "array", "items": {"type": "object", "properties": {"qty": {"type": "integer", "minimum": 1}}}}
        },
        "additionalProperties": {"type": "string"}
    });
    assert_eq!(
        field_rule(check(&schema, &json!({"orderId": 42}))),
        ("/orderId".to_owned(), "type")
    );
    assert_eq!(
        field_rule(check(&schema, &json!({}))),
        ("/orderId".to_owned(), "required")
    );
    assert_eq!(
        field_rule(check(&schema, &json!({"orderId": "123456789"}))),
        ("/orderId".to_owned(), "maxLength")
    );
    assert_eq!(
        field_rule(check(
            &schema,
            &json!({"orderId": "a", "lines": [{"qty": 2}, {"qty": 0}]})
        )),
        ("/lines/1/qty".to_owned(), "minimum")
    );
    let (field, rule) = field_rule(check(
        &schema,
        &json!({"orderId": "a", "sk_live_caller_key": 7}),
    ));
    assert_eq!(
        (field.as_str(), rule),
        ("/", "additionalProperties"),
        "the caller's key is never in the pointer"
    );
    let closed = json!({"properties": {"a": {}}, "additionalProperties": false});
    assert_eq!(
        field_rule(check(&closed, &json!({"a": 1, "zzz": 1}))),
        ("/".to_owned(), "additionalProperties")
    );
    let choice = json!({"properties": {"v": {"anyOf": [{"type": "string"}, {"type": "null"}]}}});
    assert_eq!(
        field_rule(check(&choice, &json!({"v": 1}))),
        ("/v".to_owned(), "anyOf")
    );
    let exactly = json!({"oneOf": [{"type": "integer"}, {"minimum": 0}]});
    assert_eq!(
        field_rule(check(&exactly, &json!(3))),
        ("/".to_owned(), "oneOf")
    );
    assert_eq!(check(&exactly, &json!(-3)), Ok(()));
    let numbers = json!({"properties": {"n": {"exclusiveMinimum": 1, "exclusiveMaximum": 10, "multipleOf": 3}}});
    assert_eq!(
        field_rule(check(&numbers, &json!({"n": 1}))),
        ("/n".to_owned(), "exclusiveMinimum")
    );
    assert_eq!(
        field_rule(check(&numbers, &json!({"n": 12}))),
        ("/n".to_owned(), "exclusiveMaximum")
    );
    assert_eq!(
        field_rule(check(&numbers, &json!({"n": 4}))),
        ("/n".to_owned(), "multipleOf")
    );
    assert_eq!(check(&numbers, &json!({"n": 6.0})), Ok(()));
    let values =
        json!({"properties": {"e": {"enum": [1, "a", [1, 2], {"k": null}]}, "c": {"const": 1}}});
    assert_eq!(
        check(&values, &json!({"e": 1.0, "c": 1.0})),
        Ok(()),
        "numbers compare by value"
    );
    assert_eq!(check(&values, &json!({"e": [1.0, 2], "c": 1})), Ok(()));
    assert_eq!(
        field_rule(check(&values, &json!({"e": "b"}))),
        ("/e".to_owned(), "enum")
    );
    assert_eq!(
        field_rule(check(&values, &json!({"c": 2}))),
        ("/c".to_owned(), "const")
    );
    let integer = json!({"type": "integer"});
    assert_eq!(
        check(&integer, &json!(2.0)),
        Ok(()),
        "a number with no fractional part is an integer"
    );
    assert_eq!(
        field_rule(check(&integer, &json!(2.5))),
        ("/".to_owned(), "type")
    );
    let length = json!({"minLength": 2, "maxLength": 2});
    assert_eq!(
        check(&length, &json!("é€")),
        Ok(()),
        "lengths are in code points"
    );
}

#[test]
fn an_input_deeper_than_64_is_max_depth_and_one_deeper_than_serde_parses_too() {
    let schema = json!({"type": "object"});
    let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let compiled = compiled(&schema);
    assert_eq!(
        compiled.validate(format!("{{\"a\":{}}}", nested(63)).as_bytes()),
        Ok(())
    );
    assert_eq!(
        compiled.validate(format!("{{\"a\":{}}}", nested(64)).as_bytes()),
        Err(SchemaRefusal::TooDeep)
    );
    assert_eq!(
        compiled.validate(nested(500).as_bytes()),
        Err(SchemaRefusal::TooDeep)
    );
    assert_eq!(
        compiled.validate(
            br#"{"a":"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[["}"#
        ),
        Ok(()),
        "brackets in a string are not depth"
    );
}

#[test]
fn the_backstop_stops_a_validation_past_its_time_transiently() {
    let schema = json!({"items": {"type": "integer"}});
    let input = bytes(&json!(vec![1; 50_000]));
    let (result, counters) = compiled(&schema).validate_with(
        &input,
        Limits {
            units: RUN_INPUT_WORK_UNITS,
            backstop: Duration::ZERO,
        },
    );
    assert_eq!(result, Err(SchemaRefusal::Backstop));
    assert!(
        counters.units >= 4_096 && counters.units < 8_192,
        "the clock is read every 4,096 units: {counters:?}"
    );
}

#[test]
fn two_validators_from_the_same_bytes_report_the_same_field_and_rule() {
    let schema = json!({
        "type": "object", "required": ["z", "a"],
        "properties": {"m": {"type": "string"}, "b": {"type": "integer"}, "x": {"items": {"enum": [1, 2]}}},
        "additionalProperties": {"type": "boolean"}
    });
    let inputs = [
        json!({"z": 1, "a": 1, "m": 1, "b": "x"}),
        json!({"a": 1}),
        json!({"z": 1, "a": 1, "x": [1, 3], "q": 1}),
        json!({"z": 0, "a": 0, "k": 2, "b": 1.5}),
    ];
    let one = compiled(&schema);
    let other = std::thread::spawn({
        let schema = bytes(&schema);
        move || CompiledInputSchema::compile(&schema).expect("compiles")
    })
    .join()
    .expect("thread");
    for input in &inputs {
        let first = one.validate(&bytes(input));
        assert!(first.is_err(), "{input}");
        assert_eq!(first, other.validate(&bytes(input)), "{input}");
    }
}

// ------------------------------------------------------------------ the cache

#[test]
fn compiled_schemas_are_shared_within_a_tenant_never_across_and_a_failure_is_remembered() {
    let schemas = InputSchemas::default();
    let schema = |value: Value| {
        let canonical = bytes(&value);
        let digest: [u8; 32] = sha_of(&canonical);
        InputSchema::new(canonical, digest)
    };
    let (one, two) = (TenantId::generate(), TenantId::generate());
    let object = schema(json!({"type": "object"}));
    for _ in 0..1_000 {
        schemas.check(one, &object, true, b"{}").expect("valid");
    }
    assert_eq!(
        schemas.cache_stats().misses,
        1,
        "one compile for 1,000 starts"
    );
    let same_bytes_other_version = schema(json!({"type": "object"}));
    schemas
        .check(one, &same_bytes_other_version, true, b"{}")
        .expect("valid");
    assert_eq!(
        schemas.cache_stats().misses,
        1,
        "byte-identical schemas of one tenant share an entry"
    );
    schemas.check(two, &object, true, b"{}").expect("valid");
    assert_eq!(
        schemas.cache_stats().misses,
        2,
        "two tenants never share one"
    );
    let broken = schema(json!({"patternProperties": {}}));
    for _ in 0..1_000 {
        assert!(matches!(
            schemas.check(one, &broken, true, b"{}"),
            Err(SchemaRefusal::Unusable(_))
        ));
    }
    let stats = schemas.cache_stats();
    assert_eq!(
        (stats.misses, stats.failures),
        (3, 1),
        "one remembered failure for 1,000 starts"
    );
    schemas.check(one, &object, false, b"{}").expect("unshared");
    assert_eq!(
        schemas.cache_stats().misses,
        3,
        "an unshared (ad hoc) key is compiled for the call and not cached"
    );
}

fn sha_of(bytes: &[u8]) -> [u8; 32] {
    // a stand-in digest for the cache key: any function of the bytes keeps the property the test needs
    let mut out = [0u8; 32];
    for (i, byte) in bytes.iter().enumerate() {
        out[i % 32] = out[i % 32].wrapping_mul(31).wrapping_add(*byte);
    }
    out[31] ^= bytes.len() as u8;
    out
}

// ------------------------------------------------------------------ the budget: adversarial cases

/// Primitive operations per unit the validator may spend, whatever the keyword (G043-017 records the measured k).
const K: u64 = 32;

/// Runs `schema` on `input` and holds the operations to `K` per unit, the work to the budget and the time to the
/// backstop. Answers the result, the counters and the elapsed time.
fn adversarial(
    name: &str,
    schema: &Value,
    input: &[u8],
) -> (Result<(), SchemaRefusal>, u64, u64, Duration) {
    let compiled = compiled(schema);
    let started = Instant::now();
    let (result, counters) = compiled.validate_with(input, Limits::default());
    let elapsed = started.elapsed();
    assert!(
        counters.operations > 0,
        "{name}: the test-support counter is on"
    );
    assert!(
        counters.units <= RUN_INPUT_WORK_UNITS,
        "{name}: {counters:?}"
    );
    assert!(
        counters.operations <= K * counters.units.max(1),
        "{name}: {} operations for {} units",
        counters.operations,
        counters.units
    );
    assert!(elapsed < Duration::from_secs(5), "{name}: {elapsed:?}");
    eprintln!(
        "adversarial {name}: units {} operations {} k {:.2} elapsed {elapsed:?}",
        counters.units,
        counters.operations,
        counters.operations as f64 / counters.units.max(1) as f64
    );
    (result, counters.units, counters.operations, elapsed)
}

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("n{i:04}")).collect()
}

/// The adversarial cases of design 043, one per keyword: each must stop at `workLimit` within the budget.
fn adversarial_cases() -> Vec<(&'static str, Value, Vec<u8>)> {
    let required = names(6_000);
    let all_present: serde_json::Map<String, Value> =
        required.iter().map(|n| (n.clone(), json!(0))).collect();
    let declared = names(2_000);
    let properties: serde_json::Map<String, Value> =
        declared.iter().map(|n| (n.clone(), json!({}))).collect();
    let present: serde_json::Map<String, Value> =
        declared.iter().map(|n| (n.clone(), json!(1))).collect();
    let extra: serde_json::Map<String, Value> =
        names(3_000).into_iter().map(|n| (n, json!("v"))).collect();
    let members: Vec<String> = (0..1_000)
        .map(|i| format!("{i:04}{}", "m".repeat(46)))
        .collect();
    // the last member: every application compares all 1,000 and passes, so the DAG applies it again and again
    let last = members[999].clone();
    let constant: serde_json::Map<String, Value> =
        names(1_500).into_iter().map(|n| (n, json!(true))).collect();
    let long = "a".repeat(256 * 1024);
    let ab: String = (0..256 * 1024)
        .map(|i| if i % 3 == 0 { 'b' } else { 'a' })
        .collect();
    let mut numbers = vec![json!(0); 39_999];
    numbers.push(json!("last"));
    vec![
        (
            "required",
            dag(json!({"required": required}), 8, 2),
            bytes(&Value::Object(all_present)),
        ),
        (
            "properties",
            dag(json!({"properties": properties}), 8, 2),
            bytes(&Value::Object(present)),
        ),
        (
            "additionalProperties",
            dag(json!({"additionalProperties": {"type": "string"}}), 9, 2),
            bytes(&Value::Object(extra)),
        ),
        (
            "enum",
            dag(json!({"enum": members}), 8, 2),
            bytes(&json!(last)),
        ),
        (
            "const",
            dag(json!({"const": constant.clone()}), 8, 2),
            bytes(&Value::Object(constant)),
        ),
        (
            "type",
            json!({"items": {"type": ["null", "boolean", "object", "array", "string", "integer", "number"]}}),
            bytes(&json!(vec![0; 120_000])),
        ),
        (
            "items",
            json!({"items": {"allOf": [{}, {}, {}, {}]}}),
            bytes(&json!(vec![0; 120_000])),
        ),
        (
            "allOf",
            dag(json!({"items": {}}), 6, 2),
            bytes(&json!(vec![0; 20_000])),
        ),
        (
            "anyOf",
            json!({"anyOf": vec![json!({"items": {"type": "integer"}}); 30]}),
            bytes(&json!(numbers.clone())),
        ),
        (
            "oneOf",
            json!({"oneOf": vec![json!({"items": {"type": "integer"}}); 30]}),
            bytes(&json!(numbers)),
        ),
        (
            "minLength",
            dag(json!({"minLength": 1}), 3, 2),
            bytes(&json!(long)),
        ),
        (
            "pattern",
            json!({"pattern": "[ab]*a[ab]{12}"}),
            bytes(&json!(ab)),
        ),
        // a pattern that runs, again and again, until the budget: what a unit of a match costs
        (
            "pattern matched",
            dag(json!({"pattern": "a[ab]{12}c"}), 6, 2),
            // the only match is at the end: every application scans the whole string, then matches
            bytes(&json!(format!("{}a{}c", &ab[..2_034], "b".repeat(12)))),
        ),
    ]
}

#[test]
fn every_adversarial_case_stops_at_the_budget_with_operations_bounded_by_the_units() {
    for (name, schema, input) in adversarial_cases() {
        let (result, units, _, _) = adversarial(name, &schema, &input);
        assert_eq!(
            result,
            Err(SchemaRefusal::WorkLimit),
            "{name} after {units} units"
        );
    }
}

#[test]
fn one_of_a_cheap_match_and_a_costly_match_is_a_work_limit_never_an_acceptance() {
    let costly = dag(json!({"minLength": 1}), 4, 2);
    let mut schema = costly.clone();
    schema.as_object_mut().unwrap().remove("$ref");
    schema["oneOf"] = json!([{}, {"$ref": "#/$defs/t4"}]);
    let (result, _, _, _) = adversarial(
        "oneOf cheap and costly",
        &schema,
        &bytes(&json!("a".repeat(200_000))),
    );
    assert_eq!(
        result,
        Err(SchemaRefusal::WorkLimit),
        "the costly branch stops the whole validation"
    );
    assert_eq!(
        compiled(&schema).validate(&bytes(&json!("a"))),
        Err(SchemaRefusal::Invalid {
            segments: vec![],
            rule: Rule::OneOf
        }),
        "both match a short string"
    );
}

// ------------------------------------------------------------------ no value in a refusal

mod sentinel {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        /// No generated value or key of the input appears in a refusal (FR-043-014).
        #[test]
        fn no_generated_value_or_key_appears_in_a_refusal(
            secret in "[A-Z]{12}",
            number in any::<i64>(),
            depth in 0usize..4,
        ) {
            let schema = json!({
                "type": "object", "required": ["a"],
                "properties": {"a": {"type": "integer", "maximum": -1}, "s": {"enum": ["x"]}, "l": {"items": {"type": "boolean"}}},
                "additionalProperties": {"type": "integer"}
            });
            let mut value = json!(secret.clone());
            for _ in 0..depth {
                value = json!({ secret.clone(): value });
            }
            for input in [
                json!({"a": number.abs(), "s": secret.clone()}),
                json!({"a": -2, "s": "x", secret.clone(): value.clone()}),
                json!({"a": -2, "l": [true, secret.clone()]}),
                json!({secret.clone(): secret.clone()}),
            ] {
                let refusal = compiled(&schema).validate(&bytes(&input));
                let shown = format!("{refusal:?}");
                prop_assert!(refusal.is_err(), "{}", input);
                prop_assert!(!shown.contains(&secret), "{}", shown);
                prop_assert!(!shown.contains(&number.abs().to_string()) || number.abs() < 10, "{}", shown);
            }
        }
    }
}

// ------------------------------------------------------------------ the oracle

mod oracle {
    use std::cell::Cell;

    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::{Config, TestRunner};

    struct Refuse;

    impl jsonschema::Retrieve for Refuse {
        fn retrieve(
            &self,
            _uri: &jsonschema::Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            Err("no remote reference is resolved".into())
        }
    }

    const KEYS: [&str; 4] = ["a", "b", "c", "d"];
    /// Within the allowlist. The alphabet of the strings has no `\r`, U+2028 or U+2029, which `jsonschema` lets `.`
    /// match and ECMA-262 (and the subset) does not (D043-082).
    const PATTERNS: [&str; 12] = [
        "^a",
        "b$",
        "a+b",
        "^[ab]{1,2}$",
        "(?:ab)*c",
        "^[^a]*$",
        "a.c",
        "x|y",
        "\\.",
        "[a-c]",
        "^$",
        "é",
    ];

    fn number() -> impl Strategy<Value = Value> {
        prop_oneof![
            (-5i64..10).prop_map(|n| json!(n)),
            prop_oneof![Just(i64::MIN), Just(i64::MAX), Just(-1i64)].prop_map(|n| json!(n)),
            ((i64::MAX as u64 + 1)..=u64::MAX).prop_map(|n| json!(n)),
            (-4i32..8).prop_map(|n| json!(f64::from(n))),
            Just(json!(-0.0)),
            prop_oneof![Just(2.5f64), Just(-1.5), Just(0.1)].prop_map(|n| json!(n)),
            prop_oneof![
                Just("18446744073709551616"),
                Just("100000000000000000000"),
                Just("-36893488147419103232")
            ]
            .prop_map(|text| serde_json::from_str::<Value>(text).expect("a big integer")),
        ]
    }

    fn text() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just('a'),
                Just('b'),
                Just('c'),
                Just('x'),
                Just('.'),
                Just('é'),
                Just('\n')
            ],
            0..5,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    fn scalar() -> impl Strategy<Value = Value> {
        prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            number(),
            text().prop_map(Value::String)
        ]
    }

    pub(super) fn instance() -> impl Strategy<Value = Value> {
        scalar().prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                proptest::collection::btree_map(
                    proptest::sample::select(&KEYS[..]).prop_map(str::to_owned),
                    inner,
                    0..4
                )
                .prop_map(|map| Value::Object(map.into_iter().collect())),
            ]
        })
    }

    fn type_name() -> impl Strategy<Value = &'static str> {
        proptest::sample::select(
            &[
                "null", "boolean", "object", "array", "number", "string", "integer",
            ][..],
        )
    }

    /// One keyword of the subset that holds no subschema.
    fn assertion() -> impl Strategy<Value = (String, Value)> {
        let small = 0u64..4;
        prop_oneof![
            type_name().prop_map(|t| ("type".to_owned(), json!(t))),
            proptest::sample::subsequence(
                vec![
                    "null", "boolean", "object", "array", "number", "string", "integer"
                ],
                1..3
            )
            .prop_map(|names| ("type".to_owned(), json!(names))),
            proptest::collection::vec(scalar(), 1..4)
                .prop_map(|members| ("enum".to_owned(), Value::Array(members))),
            instance().prop_map(|value| ("const".to_owned(), value)),
            number().prop_map(|n| ("minimum".to_owned(), n)),
            number().prop_map(|n| ("maximum".to_owned(), n)),
            number().prop_map(|n| ("exclusiveMinimum".to_owned(), n)),
            number().prop_map(|n| ("exclusiveMaximum".to_owned(), n)),
            (1u64..5).prop_map(|n| ("multipleOf".to_owned(), json!(n))),
            small
                .clone()
                .prop_map(|n| ("minLength".to_owned(), json!(n))),
            small
                .clone()
                .prop_map(|n| ("maxLength".to_owned(), json!(n))),
            small
                .clone()
                .prop_map(|n| ("minItems".to_owned(), json!(n))),
            small
                .clone()
                .prop_map(|n| ("maxItems".to_owned(), json!(n))),
            small
                .clone()
                .prop_map(|n| ("minProperties".to_owned(), json!(n))),
            small.prop_map(|n| ("maxProperties".to_owned(), json!(n))),
            proptest::sample::select(&PATTERNS[..]).prop_map(|p| ("pattern".to_owned(), json!(p))),
            proptest::sample::subsequence(KEYS.to_vec(), 1..3)
                .prop_map(|names| ("required".to_owned(), json!(names))),
            Just(("format".to_owned(), json!("email"))),
            Just(("title".to_owned(), json!("t"))),
        ]
    }

    fn object_of(pairs: Vec<(String, Value)>) -> Value {
        Value::Object(pairs.into_iter().collect())
    }

    /// A schema of the subset that may refer to the definitions `refs`.
    fn schema(refs: &'static [&'static str]) -> BoxedStrategy<Value> {
        let leaf = prop_oneof![
            8 => proptest::collection::vec(assertion(), 0..3).prop_map(object_of),
            1 => any::<bool>().prop_map(Value::Bool),
            2 => if refs.is_empty() { Just(json!({})).boxed() } else {
                proptest::sample::select(refs).prop_map(|name| json!({"$ref": format!("#/$defs/{name}")})).boxed()
            },
        ];
        leaf.prop_recursive(3, 16, 3, |inner| {
            let with = |key: &'static str, value: BoxedStrategy<Value>| {
                (proptest::collection::vec(assertion(), 0..2), value).prop_map(
                    move |(mut pairs, value)| {
                        pairs.push((key.to_owned(), value));
                        object_of(pairs)
                    },
                )
            };
            prop_oneof![
                with(
                    "properties",
                    proptest::collection::btree_map(
                        proptest::sample::select(&KEYS[..]).prop_map(str::to_owned),
                        inner.clone(),
                        1..3
                    )
                    .prop_map(|map| Value::Object(map.into_iter().collect()))
                    .boxed()
                ),
                with("items", inner.clone().boxed()),
                with("additionalProperties", inner.clone().boxed()),
                with(
                    "allOf",
                    proptest::collection::vec(inner.clone(), 1..3)
                        .prop_map(Value::Array)
                        .boxed()
                ),
                with(
                    "anyOf",
                    proptest::collection::vec(inner.clone(), 1..3)
                        .prop_map(Value::Array)
                        .boxed()
                ),
                with(
                    "oneOf",
                    proptest::collection::vec(inner, 1..3)
                        .prop_map(Value::Array)
                        .boxed()
                ),
            ]
        })
        .boxed()
    }

    /// A root with two definitions, the second of which may refer to the first, and the root to both.
    fn document() -> impl Strategy<Value = Value> {
        (schema(&[]), schema(&["d0"]), schema(&["d0", "d1"])).prop_map(|(d0, d1, root)| {
            let mut root = match root {
                Value::Bool(accepts) => json!({"allOf": [accepts]}),
                other => other,
            };
            root["$defs"] = json!({"d0": d0, "d1": d1});
            root
        })
    }

    const DIALECTS: [Option<&str>; 3] = [
        None,
        Some("http://json-schema.org/draft-07/schema#"),
        Some("https://json-schema.org/draft/2020-12/schema"),
    ];

    /// T043-2, FR-043-002, D043-060: over generated schemas of the subset and inputs, the platform's validator and
    /// `jsonschema` 0.58.6 (formats off, every remote reference refused) give the same verdict, in each dialect choice.
    /// Verdicts the platform gives as `workLimit`, `maxDepth` or the backstop are skipped, and fail the test above 5%.
    #[test]
    fn the_platform_validator_agrees_with_jsonschema_over_the_subset() {
        let compared = Cell::new(0u64);
        let skipped = Cell::new(0u64);
        let mut runner = TestRunner::new(Config {
            cases: 768,
            ..Config::default()
        });
        let result = runner.run(
            &(document(), proptest::collection::vec(instance(), 1..8)),
            |(schema, inputs)| {
                let ours = CompiledInputSchema::compile(&bytes(&schema)).map_err(|e| {
                    TestCaseError::fail(format!("{schema} is in the subset: {e:?}"))
                })?;
                for dialect in DIALECTS {
                    let mut dialected = schema.clone();
                    if let Some(uri) = dialect {
                        dialected["$schema"] = json!(uri);
                    }
                    let theirs = jsonschema::options()
                        .should_validate_formats(false)
                        .with_retriever(Refuse)
                        .build(&dialected)
                        .map_err(|e| {
                            TestCaseError::fail(format!("jsonschema refuses {dialected}: {e}"))
                        })?;
                    for input in &inputs {
                        let verdict = match ours.validate(&bytes(input)) {
                            Ok(()) => true,
                            Err(SchemaRefusal::Invalid { .. }) => false,
                            Err(_) => {
                                skipped.set(skipped.get() + 1);
                                continue;
                            }
                        };
                        compared.set(compared.get() + 1);
                        prop_assert_eq!(
                            verdict,
                            theirs.is_valid(input),
                            "schema {} input {} dialect {:?}",
                            dialected,
                            input,
                            dialect
                        );
                    }
                }
                Ok(())
            },
        );
        if let Err(failure) = result {
            panic!("{failure}");
        }
        let (compared, skipped) = (compared.get(), skipped.get());
        let ratio = skipped as f64 / (compared + skipped).max(1) as f64;
        eprintln!(
            "oracle: {compared} verdicts compared, {skipped} skipped ({:.3}%)",
            ratio * 100.0
        );
        assert!(compared > 1_000, "{compared}");
        assert!(ratio <= 0.05, "skip ratio {ratio}");
    }
}

// ------------------------------------------------------------------ measurement (G043-017)

/// Measures, on this machine, the primitive operations per unit of each adversarial case, the time of a full budget,
/// and the time per (byte × state) of a pattern the lazy DFA does not help: the numbers of G043-017. Run it in release:
/// `cargo test --release -p sagat-infra-schema --test run_input -- --ignored --nocapture measure`.
#[test]
#[ignore = "a measurement, run by hand in release for G043-017"]
fn measure_k_and_the_cost_of_a_full_budget() {
    let mut worst = Duration::ZERO;
    for (name, schema, input) in adversarial_cases() {
        let compiled = compiled(&schema);
        let mut best = Duration::MAX;
        let mut counters = Default::default();
        for _ in 0..5 {
            let started = Instant::now();
            let (_, spent) = compiled.validate_with(&input, Limits::default());
            best = best.min(started.elapsed());
            counters = spent;
        }
        let sagat_infra_schema::run_input::Counters { units, operations } = counters;
        let per_unit = best.as_nanos() as f64 / units.max(1) as f64;
        let full = Duration::from_nanos((per_unit * RUN_INPUT_WORK_UNITS as f64) as u64);
        // a case that stops before it spends half the budget (the long pattern, refused before it runs) says
        // nothing about the time of a unit
        if units * 2 >= RUN_INPUT_WORK_UNITS {
            worst = worst.max(full);
        }
        println!(
            "measure {name}: units {units} operations {operations} k {:.2} time {best:?} ns/unit {per_unit:.2} full budget {full:?}",
            operations as f64 / units.max(1) as f64
        );
    }
    println!("measure: the most a full budget costs over the cases {worst:?}");
    // The pattern divisor: the raw match of a DFA-defeating pattern, against its charge. The configuration of the
    // validator's patterns, with the literal prefilter off as well: `c` never occurs in the haystack, and a prefilter
    // would answer without running an automaton at all. The lazy DFA of `a[ab]{12}c` needs 2^13 states, more than its
    // 64 KiB cache holds, so this is the case the charge is written for.
    let pattern = "a[ab]{12}c";
    let regex = regex_automata::meta::Builder::new()
        .configure(
            regex_automata::meta::Config::new()
                .which_captures(regex_automata::nfa::thompson::WhichCaptures::None)
                .hybrid_cache_capacity(64 * 1024)
                .dfa(false)
                .onepass(false)
                .backtrack(false)
                .auto_prefilter(false),
        )
        .build(pattern)
        .expect("pattern");
    let nfa = regex_automata::nfa::thompson::Compiler::new()
        .configure(
            regex_automata::nfa::thompson::Config::new()
                .which_captures(regex_automata::nfa::thompson::WhichCaptures::None),
        )
        .build(pattern)
        .expect("nfa");
    let states = nfa.states().len() as f64;
    let haystack: String = (0..256 * 1024u64)
        .map(|i| {
            if (i.wrapping_mul(2_654_435_761) >> 7) % 2 == 0 {
                'b'
            } else {
                'a'
            }
        })
        .collect();
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let started = Instant::now();
        assert!(!regex.is_match(&haystack));
        best = best.min(started.elapsed());
    }
    let charge = (haystack.len() as f64 + 1.0) * states;
    println!(
        "measure pattern: {states} states, {} bytes, {best:?}, ns per (byte × state) {:.4}",
        haystack.len(),
        best.as_nanos() as f64 / charge
    );
}

#[test]
fn the_compiled_schema_is_shared_by_arc_for_the_gate() {
    let schemas = InputSchemas::default();
    let canonical = bytes(&json!({"type": "object"}));
    let schema = InputSchema::new(canonical, [7; 32]);
    let tenant = TenantId::generate();
    let a = schemas.compiled(tenant, &schema, true).expect("compiles");
    let b = schemas.compiled(tenant, &schema, true).expect("compiles");
    assert!(Arc::ptr_eq(&a, &b));
    assert!(a.weight() > 0);
    let _: Option<PathSegment> = None;
}
