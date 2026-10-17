//! Unique payload canaries and leak-safe assertions shared by feature 016 suites.
//!
//! A failing assertion names the surface and the canary class, never the canary text or the
//! captured bytes. This keeps the test harness from turning a detected leak into a CI log leak.
#![allow(dead_code)]

use sagat_domain::identity::{Permission, Role};
use serde_json::Value;
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
    pub internal_value: Canary,
    pub pii_value: Canary,
    pub secret_value: Canary,
    pub secret_key: Canary,
    pub unmarked_value: Canary,
    pub undeclared_key: Canary,
    pub undeclared_value: Canary,
    pub invalid_value: Canary,
    pub legacy_value: Canary,
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
            internal_value: make("internal-value"),
            pii_value: make("pii-value"),
            secret_value: make("secret-value"),
            secret_key: make("secret-key"),
            unmarked_value: make("unmarked-value"),
            undeclared_key: make("undeclared-key"),
            undeclared_value: make("undeclared-value"),
            invalid_value: make("invalid-value"),
            legacy_value: make("legacy-value"),
            event_key: make("event-key"),
            event_value: make("event-value"),
            dead_letter_body_key: make("dead-letter-body-key"),
            dead_letter_body_value: make("dead-letter-body-value"),
            dead_letter_header_key: make("dead-letter-header-key"),
            dead_letter_header_value: make("dead-letter-header-value"),
        }
    }

    #[must_use]
    pub fn always_forbidden(&self) -> [&Canary; 4] {
        [
            &self.secret_value,
            &self.secret_key,
            &self.invalid_value,
            &self.legacy_value,
        ]
    }

    #[must_use]
    pub fn all(&self) -> [&Canary; 16] {
        [
            &self.public_value,
            &self.internal_value,
            &self.pii_value,
            &self.secret_value,
            &self.secret_key,
            &self.unmarked_value,
            &self.undeclared_key,
            &self.undeclared_value,
            &self.invalid_value,
            &self.legacy_value,
            &self.event_key,
            &self.event_value,
            &self.dead_letter_body_key,
            &self.dead_letter_body_value,
            &self.dead_letter_header_key,
            &self.dead_letter_header_value,
        ]
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
    for canary in forbidden {
        assert!(
            !contains(bytes, canary.value.as_bytes()),
            "surface {surface} leaked the {} canary",
            canary.class
        );
    }
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
