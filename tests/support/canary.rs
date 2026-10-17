//! Unique payload canaries and leak-safe assertions shared by feature 016 suites.
//!
//! A failing assertion names the surface and the canary class, never the canary text or the
//! captured bytes. This keeps the test harness from turning a detected leak into a CI log leak.
#![allow(dead_code)]

use base64::Engine as _;
use sagat_domain::identity::{Permission, Role};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone)]
pub struct Canary {
    class: &'static str,
    value: String,
}

impl Canary {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Clone)]
pub struct Canaries {
    pub public_value: Canary,
    pub public_key: Canary,
    pub internal_value: Canary,
    pub internal_key: Canary,
    pub pii_value: Canary,
    pub pii_key: Canary,
    pub secret_value: Canary,
    pub secret_key: Canary,
    pub unmarked_value: Canary,
    pub unmarked_key: Canary,
    pub undeclared_key: Canary,
    pub undeclared_value: Canary,
    pub invalid_value: Canary,
    pub invalid_key: Canary,
    pub legacy_value: Canary,
    pub legacy_key: Canary,
    pub event_key: Canary,
    pub event_value: Canary,
    pub dead_letter_body_key: Canary,
    pub dead_letter_body_value: Canary,
    pub dead_letter_header_key: Canary,
    pub dead_letter_header_value: Canary,
}

impl Canaries {
    #[must_use]
    pub fn unique(label: &str) -> Self {
        let suffix = Uuid::now_v7().simple().to_string();
        let make = |class: &'static str| Canary {
            class,
            value: format!("canary-{label}-{class}-{suffix}"),
        };
        Self {
            public_value: make("public-value"),
            public_key: make("public-key"),
            internal_value: make("internal-value"),
            internal_key: make("internal-key"),
            pii_value: make("pii-value"),
            pii_key: make("pii-key"),
            secret_value: make("secret-value"),
            secret_key: make("secret-key"),
            unmarked_value: make("unmarked-value"),
            unmarked_key: make("unmarked-key"),
            undeclared_key: make("undeclared-key"),
            undeclared_value: make("undeclared-value"),
            invalid_value: make("invalid-value"),
            invalid_key: make("invalid-key"),
            legacy_value: make("legacy-value"),
            legacy_key: make("legacy-key"),
            event_key: make("event-key"),
            event_value: make("event-value"),
            dead_letter_body_key: make("dead-letter-body-key"),
            dead_letter_body_value: make("dead-letter-body-value"),
            dead_letter_header_key: make("dead-letter-header-key"),
            dead_letter_header_value: make("dead-letter-header-value"),
        }
    }

    #[must_use]
    pub fn always_forbidden(&self) -> [&Canary; 6] {
        [
            &self.secret_value,
            &self.secret_key,
            &self.invalid_value,
            &self.invalid_key,
            &self.legacy_value,
            &self.legacy_key,
        ]
    }

    #[must_use]
    pub fn all(&self) -> [&Canary; 22] {
        [
            &self.public_value,
            &self.public_key,
            &self.internal_value,
            &self.internal_key,
            &self.pii_value,
            &self.pii_key,
            &self.secret_value,
            &self.secret_key,
            &self.unmarked_value,
            &self.unmarked_key,
            &self.undeclared_key,
            &self.undeclared_value,
            &self.invalid_value,
            &self.invalid_key,
            &self.legacy_value,
            &self.legacy_key,
            &self.event_key,
            &self.event_value,
            &self.dead_letter_body_key,
            &self.dead_letter_body_value,
            &self.dead_letter_header_key,
            &self.dead_letter_header_value,
        ]
    }

    #[must_use]
    pub fn classified_schema(&self) -> Value {
        let mut schema = json!({
            "type":"object", "additionalProperties":true,
            "x-sagat-classification":"public",
            "properties":{
                "publicField":{"type":"string"},
                "internalField":{"type":"string","x-sagat-classification":"internal"},
                "piiField":{"type":"string","x-sagat-classification":"pii"},
                "secretField":{"type":"string","x-sagat-classification":"secret"}
            }
        });
        for level in ["public", "internal", "pii", "secret"] {
            schema["properties"][format!("{level}Map")] = json!({
                "type":"object", "additionalProperties":true,
                "x-sagat-classification":level
            });
        }
        schema
    }

    #[must_use]
    pub fn classified_input(&self) -> Value {
        let mut input = json!({
            "publicField":self.public_value.value(),
            "internalField":self.internal_value.value(),
            "piiField":self.pii_value.value(),
            "secretField":self.secret_value.value(),
            (self.undeclared_key.value()):self.undeclared_value.value()
        });
        for (level, key, value) in [
            ("public", &self.public_key, &self.public_value),
            ("internal", &self.internal_key, &self.internal_value),
            ("pii", &self.pii_key, &self.pii_value),
            ("secret", &self.secret_key, &self.secret_value),
        ] {
            input[format!("{level}Map")] = json!({(key.value()):value.value()});
        }
        input
    }

    #[must_use]
    pub fn event_forbidden(&self) -> [&Canary; 2] {
        [&self.event_key, &self.event_value]
    }

    #[must_use]
    pub fn dead_letter_forbidden(&self) -> [&Canary; 4] {
        [
            &self.dead_letter_body_key,
            &self.dead_letter_body_value,
            &self.dead_letter_header_key,
            &self.dead_letter_header_value,
        ]
    }
}

#[derive(Clone, Copy)]
pub struct AudienceCase {
    pub subject: &'static str,
    pub role: Role,
    pub grant: bool,
    pub reveal: bool,
}

impl AudienceCase {
    #[must_use]
    pub fn may_read_runs(self) -> bool {
        self.role.grants(Permission::RunRead)
    }

    #[must_use]
    pub fn may_read_events(self) -> bool {
        self.role.grants(Permission::EventRead)
    }

    #[must_use]
    pub fn may_read_internal(self) -> bool {
        self.role.grants(Permission::PayloadReadInternal)
    }

    #[must_use]
    pub fn may_reveal(self) -> bool {
        self.grant && self.reveal && self.may_read_internal()
    }
}

#[must_use]
pub fn audience_cases() -> Vec<AudienceCase> {
    let roles = [
        ("viewer-canary", Role::Viewer),
        ("developer-canary", Role::Developer),
        ("operator-canary", Role::Operator),
        ("admin", Role::Admin),
        ("worker-canary", Role::Worker),
        ("runner-canary", Role::Runner),
    ];
    roles
        .into_iter()
        .flat_map(|(subject, role)| {
            [false, true].into_iter().flat_map(move |grant| {
                [false, true].into_iter().map(move |reveal| AudienceCase {
                    subject,
                    role,
                    grant,
                    reveal,
                })
            })
        })
        .collect()
}

pub fn assert_absent(surface: &str, bytes: impl AsRef<[u8]>, forbidden: &[&Canary]) {
    let bytes = bytes.as_ref();
    let json = serde_json::from_slice::<Value>(bytes).ok();
    for canary in forbidden {
        assert!(
            !contains(bytes, canary.value.as_bytes())
                && !json.as_ref().is_some_and(|value| {
                    encoded_json_contains(value, canary.value.as_bytes(), 0)
                }),
            "surface {surface} leaked the {} canary",
            canary.class
        );
    }
}

fn encoded_json_contains(value: &Value, needle: &[u8], depth: usize) -> bool {
    match value {
        Value::String(text) => encoded_string_contains(text, needle, depth),
        Value::Array(values) => values
            .iter()
            .any(|value| encoded_json_contains(value, needle, depth)),
        Value::Object(values) => values.iter().any(|(key, value)| {
            encoded_string_contains(key, needle, depth)
                || encoded_json_contains(value, needle, depth)
        }),
        _ => false,
    }
}

fn encoded_string_contains(text: &str, needle: &[u8], depth: usize) -> bool {
    if depth >= 2 || text.len() > 1024 * 1024 {
        return false;
    }
    for engine in [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(decoded) = engine.decode(text)
            && (contains(&decoded, needle)
                || serde_json::from_slice::<Value>(&decoded)
                    .is_ok_and(|value| encoded_json_contains(&value, needle, depth + 1)))
        {
            return true;
        }
    }
    false
}

pub fn assert_present(surface: &str, bytes: impl AsRef<[u8]>, expected: &Canary) {
    assert!(
        contains(bytes.as_ref(), expected.value.as_bytes()),
        "surface {surface} omitted the expected {} canary",
        expected.class
    );
}

pub fn assert_error(surface: &str, body: &Value, canaries: &Canaries) {
    for field in ["code", "traceId", "retryable", "remediation"] {
        assert!(
            body.get(field).is_some(),
            "surface {surface} omitted closed error field {field}"
        );
    }
    let encoded = serde_json::to_vec(body).expect("error is JSON");
    assert_absent(surface, encoded, &canaries.all());
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(test)]
mod leak_detector_tests {
    use super::*;

    #[test]
    fn encoded_dead_letter_bodies_and_keys_are_not_mistaken_for_absent_canaries() {
        let canaries = Canaries::unique("detector");
        let key = canaries.dead_letter_body_key.value();
        let value = canaries.dead_letter_body_value.value();
        let body = serde_json::to_vec(&json!({(key):value})).expect("body");
        let encoded = base64::engine::general_purpose::STANDARD.encode(body);
        let wire = serde_json::to_vec(&json!({"body":encoded})).expect("wire");
        let parsed: Value = serde_json::from_slice(&wire).expect("wire JSON");
        assert!(!contains(&wire, value.as_bytes()));
        assert!(encoded_json_contains(&parsed, key.as_bytes(), 0));
        assert!(encoded_json_contains(&parsed, value.as_bytes(), 0));
    }

    #[test]
    fn unrelated_metadata_is_safe_and_every_canary_class_has_a_distinct_value() {
        let canaries = Canaries::unique("detector");
        let values: std::collections::BTreeSet<_> =
            canaries.all().into_iter().map(Canary::value).collect();
        assert_eq!(values.len(), 22);
        assert_absent(
            "metadata",
            br#"{"status":"DEAD","bodyDisclosure":"structure-only"}"#,
            &canaries.all(),
        );
    }
}
