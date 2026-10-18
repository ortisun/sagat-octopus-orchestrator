//! What the outbox really publishes on NATS, checked against the canonical AsyncAPI document: every
//! `event_outbox` row of a scenario must name a documented `sagat.v1.<type>` channel, validate
//! against that channel's message and carry no property the message does not document.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

struct Hint {
    schema: Value,
    validator: jsonschema::Validator,
}

fn hints() -> &'static BTreeMap<String, Hint> {
    static HINTS: OnceLock<BTreeMap<String, Hint>> = OnceLock::new();
    HINTS.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../specs/001-platform-foundation/contracts/events.asyncapi.yaml");
        let source = std::fs::read_to_string(path).expect("canonical AsyncAPI exists");
        let document: Value = serde_saphyr::from_str(&source).expect("AsyncAPI is YAML");
        let mut hints = BTreeMap::new();
        for channel in document["channels"].as_object().expect("channels").values() {
            let Some(address) = channel["address"].as_str() else {
                continue;
            };
            let Some(rest) = address.strip_prefix("sagat.v1.") else {
                continue;
            };
            let event_type = rest
                .split_once(".{tenant}")
                .expect("an internal hint is addressed per tenant")
                .0
                .to_owned();
            let reference = channel["messages"]
                .as_object()
                .and_then(|messages| messages.values().next())
                .and_then(|message| message["$ref"].as_str())
                .expect("the channel names its message");
            let name = reference.rsplit('/').next().expect("a component reference");
            let schema = document["components"]["messages"][name]["payload"].clone();
            let validator = jsonschema::options()
                .should_validate_formats(true)
                .build(&schema)
                .expect("the hint payload schema compiles");
            hints.insert(event_type, Hint { schema, validator });
        }
        hints
    })
}

/// Fails unless every outbox row in the scenario's database is a documented hint. Returns the event
/// types that were checked, so a caller can also assert what it expected to see.
pub(crate) async fn assert_outbox_matches_the_contract(pool: &sqlx::PgPool) -> Vec<String> {
    assert_outbox_matches_the_contract_except(pool, &[]).await
}

/// The same check, leaving out rows a test seeded itself (for example a deliberately conflicting
/// outbox id), which the platform never published.
pub(crate) async fn assert_outbox_matches_the_contract_except(
    pool: &sqlx::PgPool,
    seeded: &[uuid::Uuid],
) -> Vec<String> {
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT event_type, payload FROM event_outbox WHERE NOT (id = ANY($1)) ORDER BY id",
    )
    .bind(seeded)
    .fetch_all(pool)
    .await
    .expect("outbox rows");
    let mut seen = Vec::new();
    for (event_type, payload) in rows {
        let hint = hints()
            .get(&event_type)
            .unwrap_or_else(|| panic!("{event_type} is published but not in events.asyncapi.yaml"));
        let errors: Vec<String> = hint
            .validator
            .iter_errors(&payload)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{event_type}: {errors:?} in {payload}");
        let documented = hint.schema["properties"]
            .as_object()
            .expect("documented properties");
        for key in payload.as_object().expect("a hint is an object").keys() {
            assert!(
                documented.contains_key(key),
                "{event_type} carries undocumented `{key}`: {payload}"
            );
        }
        if !seen.contains(&event_type) {
            seen.push(event_type);
        }
    }
    seen
}

/// Each of `expected` was among the checked event types.
pub(crate) fn assert_published(published: &[String], expected: &[&str]) {
    for kind in expected {
        assert!(
            published.iter().any(|seen| seen == kind),
            "{kind} in {published:?}"
        );
    }
}

#[test]
fn capacity_finish_hint_is_closed_control_metadata() {
    let hint = hints()
        .get("capacity.member.finished")
        .expect("dormant capacity Finish has a canonical hint before any producer");
    let payload = serde_json::json!({
        "tenantId": uuid::Uuid::from_u128(1).to_string(),
        "namespaceId": uuid::Uuid::from_u128(2).to_string(),
        "groupId": uuid::Uuid::from_u128(3).to_string(),
        "memberId": uuid::Uuid::from_u128(4).to_string(),
        "chainId": uuid::Uuid::from_u128(5).to_string(),
        "headRunId": uuid::Uuid::from_u128(6).to_string(),
        "resultingRevision": 2,
        "epoch": 2,
        "retired": false,
    });
    assert!(hint.validator.is_valid(&payload));
    for key in payload.as_object().expect("reference object").keys() {
        let mut missing = payload.clone();
        missing
            .as_object_mut()
            .expect("reference object")
            .remove(key);
        assert!(!hint.validator.is_valid(&missing), "required {key}");
    }
    for (key, value) in [
        ("credentials", serde_json::json!({"password": "test-only"})),
        ("payload", serde_json::json!({})),
        ("resultingRevision", serde_json::json!(0)),
        ("resultingRevision", serde_json::json!(1)),
        ("epoch", serde_json::json!(0)),
        ("retired", serde_json::json!("false")),
        ("tenantId", serde_json::json!(1)),
    ] {
        let mut invalid = payload.clone();
        invalid[key] = value;
        assert!(!hint.validator.is_valid(&invalid), "invalid {key}");
    }
    let mut traced = payload;
    traced["traceId"] = serde_json::json!("0123456789abcdef0123456789abcdef");
    assert!(hint.validator.is_valid(&traced));
    traced["traceId"] = Value::Null;
    assert!(hint.validator.is_valid(&traced));
}

fn scoped_capacity_finish_payload() -> Value {
    serde_json::json!({
        "tenantId": uuid::Uuid::from_u128(1).to_string(),
        "namespaceId": uuid::Uuid::from_u128(2).to_string(),
        "groupId": uuid::Uuid::from_u128(3).to_string(),
        "memberId": uuid::Uuid::from_u128(4).to_string(),
        "originalRunId": uuid::Uuid::from_u128(5).to_string(),
        "rootOwnerNode": "a",
        "iterationIndex": 0,
        "resultingRevision": 2,
        "epoch": 1,
        "retired": false,
        "traceId": "0123456789abcdef0123456789abcdef",
    })
}

#[test]
fn capacity_scoped_finish_hint_accepts_the_closed_reference_payload() {
    let hint = hints()
        .get("capacity.scoped_member.finished")
        .expect("dormant scoped capacity Finish has a distinct canonical hint before any producer");
    let payload = scoped_capacity_finish_payload();
    assert!(hint.validator.is_valid(&payload));

    let mut maxima = payload.clone();
    maxima["rootOwnerNode"] = serde_json::json!("a".repeat(64));
    maxima["iterationIndex"] = serde_json::json!(9_999);
    maxima["resultingRevision"] = serde_json::json!(9_223_372_036_854_775_807_i64);
    maxima["epoch"] = serde_json::json!(9_223_372_036_854_775_807_i64);
    assert!(hint.validator.is_valid(&maxima));

    let mut null_trace = payload.clone();
    null_trace["traceId"] = Value::Null;
    assert!(hint.validator.is_valid(&null_trace));

    let mut absent_trace = payload;
    absent_trace
        .as_object_mut()
        .expect("reference object")
        .remove("traceId");
    assert!(hint.validator.is_valid(&absent_trace));
}

#[test]
fn capacity_scoped_finish_hint_rejects_each_invalid_shape_independently() {
    let hint = hints()
        .get("capacity.scoped_member.finished")
        .expect("dormant scoped capacity Finish has a distinct canonical hint before any producer");
    let payload = scoped_capacity_finish_payload();

    for key in [
        "tenantId",
        "namespaceId",
        "groupId",
        "memberId",
        "originalRunId",
        "rootOwnerNode",
        "iterationIndex",
        "resultingRevision",
        "epoch",
        "retired",
    ] {
        let mut invalid = payload.clone();
        invalid
            .as_object_mut()
            .expect("reference object")
            .remove(key);
        assert!(!hint.validator.is_valid(&invalid), "required {key}");
    }

    for (key, value) in [
        ("tenantId", serde_json::json!(1)),
        ("namespaceId", serde_json::json!(false)),
        ("groupId", serde_json::json!([])),
        ("memberId", serde_json::json!({})),
        ("originalRunId", Value::Null),
        ("rootOwnerNode", serde_json::json!(1)),
        ("iterationIndex", serde_json::json!("0")),
        ("resultingRevision", serde_json::json!("2")),
        ("epoch", serde_json::json!(true)),
        ("retired", serde_json::json!("false")),
        ("traceId", serde_json::json!(1)),
    ] {
        let mut invalid = payload.clone();
        invalid[key] = value;
        assert!(!hint.validator.is_valid(&invalid), "wrong type for {key}");
    }

    for key in [
        "tenantId",
        "namespaceId",
        "groupId",
        "memberId",
        "originalRunId",
    ] {
        let mut invalid = payload.clone();
        invalid[key] = serde_json::json!("not-a-uuid");
        assert!(!hint.validator.is_valid(&invalid), "UUID format for {key}");
    }

    for owner in ["", "_owner", "owner/child"] {
        let mut invalid = payload.clone();
        invalid["rootOwnerNode"] = serde_json::json!(owner);
        assert!(!hint.validator.is_valid(&invalid), "owner {owner:?}");
    }
    let mut owner_too_long = payload.clone();
    owner_too_long["rootOwnerNode"] = serde_json::json!("a".repeat(65));
    assert!(!hint.validator.is_valid(&owner_too_long), "owner maxLength");

    for (key, value) in [
        ("iterationIndex", serde_json::json!(-1)),
        ("iterationIndex", serde_json::json!(10_000)),
        ("resultingRevision", serde_json::json!(1)),
        (
            "resultingRevision",
            serde_json::json!(9_223_372_036_854_775_808_u64),
        ),
        ("epoch", serde_json::json!(0)),
        ("epoch", serde_json::json!(9_223_372_036_854_775_808_u64)),
    ] {
        let mut invalid = payload.clone();
        invalid[key] = value;
        assert!(!hint.validator.is_valid(&invalid), "bound for {key}");
    }

    for trace in [
        "0123456789abcdef0123456789abcde",
        "0123456789abcdef0123456789abcdef0",
        "0123456789abcdef0123456789abcdeg",
        "0123456789abcdef0123456789abcdeF",
    ] {
        let mut invalid = payload.clone();
        invalid["traceId"] = serde_json::json!(trace);
        assert!(!hint.validator.is_valid(&invalid), "trace {trace:?}");
    }

    for key in [
        "secret",
        "credentials",
        "payload",
        "chainId",
        "headRunId",
        "unexpected",
    ] {
        let mut invalid = payload.clone();
        invalid[key] = serde_json::json!("forbidden");
        assert!(!hint.validator.is_valid(&invalid), "extra {key}");
    }
}
