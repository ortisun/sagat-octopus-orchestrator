//! Design 041 FR-041-007 (c), P5, task T041-6: the server scrubs every part of a line with the shapes of both of its
//! scrubbers (`sagat_domain::scripts::redact_credentials` and `sagat_telemetry::scrub_shapes_with`) before any size is
//! measured, and hides the value of any type of a field whose key is a sensitive name. Asserted on the stored rows.

use std::time::Duration;

use sagat_application::step_logs::StepLogRecordIn;
use sagat_domain::step_logs::{FieldValue, StepLogLevel};

use crate::support::bounded;
use crate::support::engine::world;
use crate::support::step_logs::{ingest, leased, send};

const LIMIT: Duration = Duration::from_secs(120);

/// Each shape, with the part of it that must never be stored. The first three are the script logs' shapes
/// (`redact_credentials`), the rest the telemetry's (`scrub_shapes_with`).
const SHAPES: [(&str, &str); 10] = [
    ("ASIAABCDEFGHIJKLMNOP", "ASIAABCDEFGHIJKLMNOP"),
    (
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEsecretbody\n-----END RSA PRIVATE KEY-----",
        "MIIEsecretbody",
    ),
    ("Bearer tok3n-abcdefgh", "tok3n-abcdefgh"),
    ("postgres://admin:hunter2@db.internal/app", "hunter2"),
    ("Basic dXNlcjpwYXNzd29yZDEy", "dXNlcjpwYXNzd29yZDEy"),
    (
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl",
        "eyJhbGciOiJIUzI1NiJ9",
    ),
    ("sk-abcdefghijklmnop1234", "sk-abcdefghijklmnop1234"),
    ("ghp_abcdefghijklmnopqrstuv", "ghp_abcdefghijklmnopqrstuv"),
    ("xoxb-1234567890ab", "xoxb-1234567890ab"),
    ("AKIAIOSFODNN7EXAMPLE", "AKIAIOSFODNN7EXAMPLE"),
];

/// The names `sagat_telemetry::is_sensitive_name` matches a part of.
const SENSITIVE: [&str; 14] = [
    "authorization",
    "token",
    "secret",
    "password",
    "passwd",
    "cookie",
    "credential",
    "api_key",
    "apikey",
    "private_key",
    "payload",
    "body",
    "bearer",
    "signature",
];

fn line(
    sequence: u64,
    message: String,
    exception: String,
    fields: Vec<(String, FieldValue)>,
) -> StepLogRecordIn {
    StepLogRecordIn {
        sequence,
        level: Some(StepLogLevel::Warn),
        worker_time: None,
        message,
        exception,
        fields,
        truncated: false,
    }
}

/// Each shape in the message, the exception, a field key and a string field value: the stored row holds none of the
/// secret parts and `[REDACTED]` in each of the four places.
#[tokio::test]
async fn each_shape_of_both_scrubbers_is_scrubbed_from_every_part_of_a_line() {
    bounded::within("shapes", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let batch: Vec<StepLogRecordIn> = SHAPES
            .iter()
            .zip(1..)
            .map(|((shape, _), sequence)| {
                line(
                    sequence,
                    format!("m {shape} m"),
                    format!("e {shape} e"),
                    vec![(
                        format!("k {shape}"),
                        FieldValue::String(format!("v {shape} v")),
                    )],
                )
            })
            .collect();
        send(&ingest(&w), &a, batch).await;
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT l.sequence, l.message, l.exception, l.fields FROM step_log_lines l \
               JOIN step_log_streams s ON s.tenant_id = l.tenant_id AND s.stream_id = l.stream_id \
              WHERE s.run_id = $1 ORDER BY l.sequence",
        )
        .bind(a.run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("the lines");
        assert_eq!(rows.len(), SHAPES.len());
        for ((sequence, message, exception, fields), (shape, secret)) in rows.iter().zip(SHAPES) {
            for (part, text) in [
                ("message", message),
                ("exception", exception),
                ("fields", fields),
            ] {
                assert!(
                    !text.contains(secret),
                    "{sequence} {part}: {shape:?} stored as {text:?}"
                );
                assert!(text.contains("[REDACTED]"), "{sequence} {part}: {text:?}");
            }
            let object: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(fields).expect("canonical JSON");
            let (key, value) = object.iter().next().expect("one field");
            let value = value.as_str().expect("a string value");
            for (part, text) in [("key", key.as_str()), ("value", value)] {
                assert!(!text.contains(secret), "{sequence} {part}: {text:?}");
                assert!(text.contains("[REDACTED]"), "{sequence} {part}: {text:?}");
            }
        }
    })
    .await;
}

/// Each sensitive field name with a value of each type (string, integer, double, boolean): the stored field's value
/// is the string `[REDACTED]`.
#[tokio::test]
async fn a_sensitive_field_name_hides_its_value_whatever_its_type() {
    bounded::within("field names", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let values = [
            FieldValue::String("plain".into()),
            FieldValue::Int(42),
            FieldValue::Double(1.5),
            FieldValue::Bool(true),
        ];
        let batch: Vec<StepLogRecordIn> = SENSITIVE
            .iter()
            .flat_map(|name| values.iter().map(move |value| (*name, value.clone())))
            .zip(1..)
            .map(|((name, value), sequence)| {
                line(
                    sequence,
                    "m".into(),
                    String::new(),
                    vec![(format!("x_{name}_y"), value)],
                )
            })
            .collect();
        let expected: Vec<String> = SENSITIVE
            .iter()
            .flat_map(|name| {
                values
                    .iter()
                    .map(move |_| format!("{{\"x_{name}_y\":\"[REDACTED]\"}}"))
            })
            .collect();
        send(&ingest(&w), &a, batch).await;
        let stored: Vec<String> = sqlx::query_scalar(
            "SELECT l.fields FROM step_log_lines l \
               JOIN step_log_streams s ON s.tenant_id = l.tenant_id AND s.stream_id = l.stream_id \
              WHERE s.run_id = $1 ORDER BY l.sequence",
        )
        .bind(a.run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("the fields");
        assert_eq!(stored, expected);
    })
    .await;
}
