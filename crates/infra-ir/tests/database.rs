//! Shared managed-database IR and redaction contract (T017-011).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::IrCodec;
use sagat_domain::{
    database::{DatabaseType, Parameter, TypedValue},
    workflow::NodeId,
};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn document(node: Value) -> Value {
    json!({"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow",
        "metadata":{"name":"database-flow"},"entrypoint":"query","nodes":[node]})
}

fn database() -> Value {
    json!({"id":"query","type":"database","connectionRef":{"name":"warehouse","revision":2},
        "operation":"query","statement":"SELECT $1::bigint, $2::text",
        "parameters":[{"literal":{"type":"integer","value":"9223372036854775807"}},
            {"type":"text","expression":"input.label"}]})
}

#[test]
fn canonical_database_node_is_supported() {
    let parsed = JsonIrCodec.parse(document(database()).to_string().as_bytes());
    assert!(parsed.is_ok(), "{parsed:?}");
}

#[test]
fn ordinary_activities_cannot_forge_managed_database_names() {
    for field in ["activity", "compensateWith", "reconcileWith"] {
        let mut node = json!({"id":"query","type":"activity","activity":"regular"});
        node[field] = json!("sagat.database.postgresql");
        assert!(
            JsonIrCodec
                .parse(document(node).to_string().as_bytes())
                .is_err(),
            "{field}"
        );
    }
}

#[test]
fn database_schema_errors_do_not_echo_sql_or_parameters() {
    for field in [
        "operation",
        "connectionRef",
        "statement",
        "parameters",
        "limits",
        "unknown",
    ] {
        let mut node = database();
        node[field] = json!({"SENSITIVE_DATABASE_CANARY":"SENSITIVE_DATABASE_CANARY"});
        let error = JsonIrCodec
            .parse(document(node).to_string().as_bytes())
            .expect_err("invalid");
        assert!(
            !format!("{error:?} {error}").contains("SENSITIVE_DATABASE_CANARY"),
            "{error:?}"
        );
    }
}

#[test]
fn database_writes_cannot_override_retry_or_effect_safety() {
    for field in [
        "retry",
        "activity",
        "secrets",
        "highRisk",
        "irreversible",
        "compensateWith",
        "reconcileWith",
    ] {
        let mut node = database();
        node["operation"] = json!("execute");
        node[field] = match field {
            "retry" => json!({"maxAttempts":2}),
            "highRisk" | "irreversible" => json!(false),
            "secrets" => json!(["password"]),
            _ => json!("regular"),
        };
        assert!(
            JsonIrCodec
                .parse(document(node).to_string().as_bytes())
                .is_err(),
            "{field}"
        );
    }
}

#[test]
fn shared_lossless_value_fixture_and_write_policy() {
    let fixture = include_bytes!(
        "../../../tests/contract/database-connections/typed-parameters.workflow.json"
    );
    let parsed = JsonIrCodec.parse(fixture).expect("shared fixture");
    let id = NodeId::parse("query").expect("id");
    let node = parsed.definition.node(&id).expect("node");
    let operation = node.database_operation().expect("managed operation");
    assert_eq!(operation.connection.revision, Some(2));
    assert!(
        matches!(&operation.parameters()[2], Parameter::Literal(TypedValue::Integer(s)) if s == "9223372036854775807")
    );
    assert!(
        matches!(&operation.parameters()[3], Parameter::Literal(TypedValue::Decimal(s)) if s == "12345678901234567890.123456789")
    );
    assert!(
        matches!(&operation.parameters()[8], Parameter::Literal(TypedValue::Binary(v)) if v == &[0,255])
    );
    assert!(matches!(
        &operation.parameters()[10],
        Parameter::Expression {
            value_type: DatabaseType::Integer,
            ..
        }
    ));
    assert_eq!(node.retry_policy().max_attempts(), 1);
    assert!(!node.effect().high_risk);
    let mut write = database();
    write["operation"] = json!("execute");
    let parsed = JsonIrCodec
        .parse(document(write).to_string().as_bytes())
        .expect("write");
    let node = parsed.definition.node(&id).expect("write node");
    assert_eq!(node.retry_policy().max_attempts(), 1);
    assert!(node.effect().high_risk && node.effect().irreversible);
}

#[test]
fn typed_parameters_and_utf8_byte_ceilings_fail_closed() {
    for parameter in [
        json!({"literal":{"type":"integer","value":9007199254740991_i64}}),
        json!({"literal":{"type":"decimal","value":"NaN"}}),
        json!({"literal":{"type":"binary","value":"not base64"}}),
        json!({"literal":{"type":"date","value":"2026-02-30"}}),
        json!({"literal":{"type":"text","value":"nul\u{0}"}}),
        json!({"literal":{"type":"null","value":0}}),
        json!({"type":"text","expression":"input.SENSITIVE_DATABASE_CANARY["}),
        json!({"type":"text","expression":"input.label","literal":{"type":"null","value":null}}),
    ] {
        let mut node = database();
        node["parameters"] = json!([parameter]);
        let error = JsonIrCodec
            .parse(document(node).to_string().as_bytes())
            .expect_err("invalid value");
        assert!(!format!("{error:?}").contains("SENSITIVE_DATABASE_CANARY"));
    }
    let mut node = database();
    node["statement"] = json!("é".repeat(8193));
    assert!(
        JsonIrCodec
            .parse(document(node.clone()).to_string().as_bytes())
            .is_err()
    );
    node["statement"] = json!("SELECT $1");
    node["parameters"] = json!([{"literal":{"type":"text","value":"é".repeat(32769)}}]);
    assert!(
        JsonIrCodec
            .parse(document(node).to_string().as_bytes())
            .is_err()
    );
}

#[test]
fn expression_parameters_preserve_precision_and_never_report_sensitive_input() {
    use sagat_application::execution::{ExpressionEvaluator, RunData};
    use sagat_domain::workflow::Expression;
    use sagat_infra_ir::JsonEvaluator;
    let parameter = |kind| Parameter::Expression {
        value_type: kind,
        expression: Expression::parse("input.value").expect("expression"),
    };
    let input = |value| RunData {
        input: serde_json::to_vec(&json!({"value":value})).expect("json"),
        ..RunData::default()
    };
    assert_eq!(
        JsonEvaluator
            .database_parameter(
                &parameter(DatabaseType::Integer),
                &input(json!("9223372036854775807"))
            )
            .expect("integer"),
        TypedValue::Integer("9223372036854775807".into())
    );
    assert_eq!(
        JsonEvaluator
            .database_parameter(
                &parameter(DatabaseType::Decimal),
                &input(json!("12345678901234567890.123456789"))
            )
            .expect("decimal"),
        TypedValue::Decimal("12345678901234567890.123456789".into())
    );
    for value in [
        json!(9223372036854775807_i64),
        json!("SENSITIVE_CANARY"),
        json!(null),
    ] {
        let error = JsonEvaluator
            .database_parameter(&parameter(DatabaseType::Integer), &input(value))
            .expect_err("invalid integer");
        assert!(!format!("{error:?}").contains("SENSITIVE_CANARY"));
    }
    assert!(
        JsonEvaluator
            .database_parameter(
                &parameter(DatabaseType::Timestamp),
                &input(json!("2026-02-30T00:00:00Z"))
            )
            .is_err()
    );
    assert_eq!(
        JsonEvaluator
            .database_parameter(&parameter(DatabaseType::Binary), &input(json!("AP8=")))
            .expect("binary"),
        TypedValue::Binary(vec![0, 255])
    );
}
