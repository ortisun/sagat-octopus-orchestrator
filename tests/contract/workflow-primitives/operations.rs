//! Additive v1 declaration contract for bounded Query and Update operations.

use std::sync::OnceLock;

use jsonschema::Validator;
use sagat_application::definitions::{IrCodec, IrError};
use sagat_domain::execution::{
    MAX_OPERATION_NAME_BYTES, MAX_OPERATION_VALUE_BYTES, MAX_UPDATE_KEY_BYTES, ResultDisclosure,
};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = super::repository_root()
            .join("specs/001-platform-foundation/contracts/workflow.schema.json");
        let source = std::fs::read_to_string(path).expect("canonical workflow schema exists");
        serde_json::from_str(&source).expect("workflow schema is JSON")
    })
}

fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        jsonschema::options()
            .build(schema())
            .expect("workflow schema compiles")
    })
}

fn document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "account-state"},
        "entrypoint": "done",
        "nodes": [{"id": "done", "type": "end"}],
        "state": {
            "schema": {"type": "integer"},
            "initial": "0",
            "queries": {
                "balance.read": {
                    "argsSchema": {"type": "object"},
                    "resultSchema": {"type": "integer"},
                    "expression": "state",
                    "disclosure": "declared-business-result-v1"
                }
            },
            "updates": {
                "balance:credit": {
                    "argsSchema": {"type": "integer"},
                    "validate": "args > 0",
                    "state": "state + args",
                    "result": "newState",
                    "resultSchema": {"type": "integer"},
                    "disclosure": "declared-business-result-v1"
                }
            }
        }
    })
}

fn assert_valid(value: &Value) {
    let errors = validator()
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn explicit_disclosure_is_typed_and_the_bounded_runtime_slice_is_publishable() {
    let value = document();
    assert_valid(&value);
    assert_eq!(
        ResultDisclosure::DeclaredBusinessResult.as_str(),
        "declared-business-result-v1"
    );

    let bytes = serde_json::to_vec(&value).expect("fixture JSON");
    JsonIrCodec
        .parse(&bytes)
        .expect("bounded declared operations are publishable");
    assert!(matches!(
        JsonIrCodec.parse_structured(&bytes),
        Err(IrError::UnsupportedDeclaration { field: "state", .. })
    ));
    let parsed = JsonIrCodec
        .parse_declared_state(&bytes)
        .expect("internal declared-state parser");
    assert_eq!(parsed.state().initial().as_str(), "0");
    assert_eq!(parsed.state().queries().count(), 1);
    assert_eq!(parsed.state().updates().count(), 1);

    let mut chain = value.clone();
    chain["chainPolicy"] = json!({"maxSegments": 2, "timeoutMs": 1_000});
    assert!(JsonIrCodec.parse(chain.to_string().as_bytes()).is_err());

    let mut structured = value;
    structured["entrypoint"] = json!("review");
    structured["nodes"] = json!([
        {
            "id": "review",
            "type": "approval",
            "policy": {"roles": ["admin"]},
            "timeoutMs": 1_000,
            "onApprove": "approved",
            "onReject": "rejected",
            "onExpire": "expired"
        },
        {"id": "approved", "type": "end"},
        {"id": "rejected", "type": "end"},
        {"id": "expired", "type": "end"}
    ]);
    assert!(matches!(
        JsonIrCodec.parse(structured.to_string().as_bytes()),
        Err(IrError::Unsupported { kind, .. }) if kind == "approval"
    ));
}

#[test]
fn disclosure_is_never_inferred_from_a_schema() {
    for pointer in [
        "/state/queries/balance.read/disclosure",
        "/state/updates/balance:credit/disclosure",
    ] {
        let mut missing = document();
        missing
            .pointer_mut(pointer.rsplit_once('/').expect("field pointer").0)
            .expect("operation")
            .as_object_mut()
            .expect("operation object")
            .remove("disclosure");
        assert!(!validator().is_valid(&missing), "{pointer}");

        let mut implicit = document();
        *implicit.pointer_mut(pointer).expect("disclosure") = json!("schema-implies-access");
        assert!(!validator().is_valid(&implicit), "{pointer}");
    }
}

#[test]
fn operation_counts_names_expressions_and_unknown_handler_fields_are_bounded() {
    assert_eq!(
        schema()["$defs"]["name"]["maxLength"],
        MAX_OPERATION_NAME_BYTES
    );
    assert_eq!(MAX_UPDATE_KEY_BYTES, 128);

    for collection in ["queries", "updates"] {
        let mut excessive = document();
        let template = excessive["state"][collection]
            .as_object()
            .expect("operation map")
            .values()
            .next()
            .expect("operation")
            .clone();
        excessive["state"][collection] = Value::Object(
            (0..33)
                .map(|index| (format!("operation{index}"), template.clone()))
                .collect(),
        );
        assert!(!validator().is_valid(&excessive), "{collection}");
    }

    let mut long_name = document();
    let query = long_name["state"]["queries"]
        .as_object_mut()
        .expect("queries")
        .remove("balance.read")
        .expect("query");
    long_name["state"]["queries"]["x".repeat(129)] = query;
    assert!(!validator().is_valid(&long_name));

    let mut long_expression = document();
    long_expression["state"]["updates"]["balance:credit"]["state"] = json!("x".repeat(1025));
    assert!(!validator().is_valid(&long_expression));

    for forbidden in ["handler", "callback", "network", "wait"] {
        let mut imperative = document();
        imperative["state"]["updates"]["balance:credit"][forbidden] = json!("doWork");
        assert!(!validator().is_valid(&imperative), "{forbidden}");
    }
}

#[test]
fn result_schema_requires_finite_shapes_and_matches_the_value_ceiling() {
    assert_eq!(MAX_OPERATION_VALUE_BYTES, 256 * 1024);
    assert_eq!(
        schema()["$defs"]["resultSchema"]["x-maxSchemaBytes"],
        65_536
    );

    let mut bounded = document();
    bounded["state"]["queries"]["balance.read"]["resultSchema"] = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "entries": {
                "type": "array",
                "items": {"type": "string", "maxLength": 64},
                "maxItems": 100
            }
        },
        "required": ["entries"]
    });
    assert_valid(&bounded);

    let mut open = bounded.clone();
    open["state"]["queries"]["balance.read"]["resultSchema"]["additionalProperties"] = json!(true);
    assert!(!validator().is_valid(&open));

    let mut unbounded_string = document();
    unbounded_string["state"]["queries"]["balance.read"]["resultSchema"] =
        json!({"type": "string"});
    assert!(!validator().is_valid(&unbounded_string));

    let mut unbounded_array = document();
    unbounded_array["state"]["queries"]["balance.read"]["resultSchema"] =
        json!({"type": "array", "items": {"type": "integer"}});
    assert!(!validator().is_valid(&unbounded_array));
}

/// The REST precondition is optional and never negative; the HTTP adapter reads it as `u64`, so a
/// negative value is refused before any lookup.
#[test]
fn expected_state_revision_is_an_optional_nonnegative_integer() {
    let path =
        super::repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    let source = std::fs::read_to_string(path).expect("canonical OpenAPI exists");
    let openapi: Value = serde_saphyr::from_str(&source).expect("canonical OpenAPI is YAML");
    for request in ["DeclaredUpdateRequest", "ChainDeclaredUpdateRequest"] {
        let schema = &openapi["components"]["schemas"][request];
        let required = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{request} lists required fields"));
        assert!(
            !required
                .iter()
                .any(|field| field == "expectedStateRevision"),
            "{request}: the precondition must stay optional"
        );
        let revision = &schema["properties"]["expectedStateRevision"];
        assert_eq!(revision["type"], "integer", "{request}");
        assert_eq!(revision["minimum"], 0, "{request}");
        assert_eq!(
            schema["properties"]["updateKey"]["minLength"], 1,
            "{request}"
        );
        assert_eq!(
            schema["properties"]["updateKey"]["maxLength"], MAX_UPDATE_KEY_BYTES,
            "{request}"
        );
    }
}

#[test]
fn declared_state_schema_uses_the_same_closed_bounded_value_shapes() {
    assert_eq!(
        schema()["$defs"]["state"]["properties"]["schema"]["$ref"],
        "#/$defs/resultSchema"
    );

    let mut bounded = document();
    bounded["state"]["schema"] = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {"balance": {"type": "integer"}},
        "required": ["balance"]
    });
    assert_valid(&bounded);

    let mut open = bounded;
    open["state"]["schema"]["additionalProperties"] = json!(true);
    assert!(!validator().is_valid(&open));
}
