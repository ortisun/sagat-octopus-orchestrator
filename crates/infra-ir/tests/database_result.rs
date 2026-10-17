#![allow(clippy::unwrap_used)]
use sagat_application::database::DatabaseResultValidator;
use sagat_domain::database::DatabaseLimits;
use sagat_infra_ir::JsonDatabaseResultValidator;

#[test]
fn retained_database_result_has_one_bounded_closed_lossless_shape() {
    let validator = JsonDatabaseResultValidator;
    let good=br#"{"columns":[{"name":"same","type":"integer"},{"name":"same","type":"decimal"},{"name":"bin","type":"binary"}],"rows":[[{"type":"integer","value":"9223372036854775807"},{"type":"decimal","value":"12345678901234567890.001230"},{"type":"binary","value":"AP8q"}]],"affectedRows":"1","truncated":false}"#;
    assert!(validator.validate(good, DatabaseLimits::default()).is_ok());
    let mut value: serde_json::Value = serde_json::from_slice(good).unwrap();
    let bad = |v: &serde_json::Value| {
        validator
            .validate(&serde_json::to_vec(v).unwrap(), DatabaseLimits::default())
            .is_err()
    };
    value["rows"][0][0]["value"] = serde_json::json!(123);
    assert!(bad(&value));
    value["rows"][0][0] = serde_json::json!({"type":"null","value":null});
    assert!(!bad(&value));
    value["rows"][0][0] = serde_json::json!({"type":"text","value":"wrong column type"});
    assert!(bad(&value));
    value["rows"][0][0] = serde_json::json!({"type":"integer","value":"1","extra":"canary"});
    assert!(bad(&value));
    value = serde_json::from_slice(good).unwrap();
    value["rows"][0].as_array_mut().unwrap().pop();
    assert!(bad(&value));
    value = serde_json::from_slice(good).unwrap();
    value["truncated"] = serde_json::json!(true);
    assert!(bad(&value));
    value = serde_json::from_slice(good).unwrap();
    value["affectedRows"] = serde_json::json!("-1");
    assert!(bad(&value));
    assert!(
        validator
            .validate(
                good,
                DatabaseLimits {
                    max_result_bytes: 16,
                    ..DatabaseLimits::default()
                }
            )
            .is_err()
    );
    let duplicate =
        br#"{"columns":[],"rows":[],"affectedRows":"0","truncated":false,"truncated":true}"#;
    assert!(
        validator
            .validate(duplicate, DatabaseLimits::default())
            .is_err()
    );
    let unknown =
        br#"{"columns":[],"rows":[],"affectedRows":"0","truncated":false,"secret":"canary"}"#;
    assert!(
        validator
            .validate(unknown, DatabaseLimits::default())
            .is_err()
    );
}

#[test]
fn published_result_schema_agrees_with_the_retained_result_contract() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../specs/017-database-connections/contracts/database-result.schema.json"
    ))
    .unwrap();
    let schema = jsonschema::validator_for(&schema).unwrap();
    let good = serde_json::json!({"columns":[{"name":"v","type":"integer"}],"rows":[[{"type":"integer","value":"9223372036854775807"}]],"affectedRows":"1","truncated":false});
    assert!(schema.is_valid(&good));
    let mut bad = good;
    bad["rows"][0][0]["value"] = serde_json::json!(12);
    assert!(!schema.is_valid(&bad));
}
