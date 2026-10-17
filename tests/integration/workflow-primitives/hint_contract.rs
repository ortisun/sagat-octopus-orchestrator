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
