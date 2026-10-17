//! The platform's log lines: what is on them, and what is not (FR-008-001, FR-008-006).
#![allow(clippy::expect_used, clippy::panic)]

use std::io;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;

use sagat_telemetry::{LogLayer, Style};

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("buffer").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl MakeWriter<'_> for Sink {
    type Writer = Self;
    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn lines(style: Style, work: impl FnOnce()) -> Vec<String> {
    let sink = Sink::default();
    let subscriber =
        tracing_subscriber::registry().with(LogLayer::new(sink.clone(), "sagat-test", style));
    tracing::subscriber::with_default(subscriber, work);
    let text = String::from_utf8(sink.0.lock().expect("buffer").clone()).expect("utf-8");
    text.lines().map(str::to_owned).collect()
}

fn json_lines(work: impl FnOnce()) -> Vec<Value> {
    lines(Style::Json, work)
        .iter()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}")))
        .collect()
}

#[test]
fn a_line_is_one_json_object_with_the_service_the_level_and_the_message() {
    let out = json_lines(|| tracing::warn!(route = "/runs", status = 503, "request completed"));
    assert_eq!(out.len(), 1);
    let line = &out[0];
    assert_eq!(line["service"], "sagat-test");
    assert_eq!(line["level"], "WARN");
    assert_eq!(line["message"], "request completed");
    assert_eq!(line["route"], "/runs");
    assert_eq!(line["status"], 503);
    assert!(
        line["timestamp"]
            .as_str()
            .is_some_and(|t| t.ends_with('Z') || t.contains('+'))
    );
}

#[test]
fn a_line_carries_the_fields_of_the_spans_it_happens_in_and_the_innermost_wins() {
    let out = json_lines(|| {
        let outer = tracing::info_span!(
            "scheduler.pass",
            run_id = "run-1",
            node = "outer",
            attempt = 2
        );
        let _outer = outer.enter();
        let inner = tracing::info_span!("lease", node = "inner");
        let _inner = inner.enter();
        tracing::info!(extra = true, "leased");
    });
    let line = &out[0];
    assert_eq!(line["run_id"], "run-1");
    assert_eq!(line["attempt"], 2);
    assert_eq!(line["node"], "inner");
    assert_eq!(line["extra"], true);
}

#[test]
fn a_field_recorded_on_a_span_after_it_was_created_is_on_the_line() {
    let out = json_lines(|| {
        let span = tracing::info_span!("http.request", status = tracing::field::Empty);
        let _entered = span.enter();
        span.record("status", 201);
        tracing::info!("done");
    });
    assert_eq!(out[0]["status"], 201);
}

#[test]
fn a_value_under_a_sensitive_name_never_reaches_the_line() {
    let out = json_lines(|| {
        let span = tracing::info_span!("request", authorization = "Bearer abc");
        let _entered = span.enter();
        tracing::error!(lease_token = "s3cr3t-lease", password = %"hunter2", payload_bytes = 12, "failed");
    });
    let raw = serde_json::to_string(&out).expect("json");
    for hidden in ["abc", "s3cr3t-lease", "hunter2"] {
        assert!(!raw.contains(hidden), "{hidden} reached the log: {raw}");
    }
    assert_eq!(out[0]["lease_token"], "[redacted]");
    assert_eq!(out[0]["authorization"], "[redacted]");
    // A number under such a name is a measure, not a secret.
    assert_eq!(out[0]["payload_bytes"], 12);
}

#[test]
fn a_credential_inside_the_message_or_an_error_is_scrubbed() {
    let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJlMTIzNDU2";
    let out = json_lines(|| {
        tracing::warn!(error = %format!("rejected {jwt}"), "dialing postgres://sagat:hunter2@db:5432/x");
    });
    let raw = serde_json::to_string(&out).expect("json");
    assert!(!raw.contains("eyJ") && !raw.contains("hunter2"), "{raw}");
    assert!(raw.contains("@db:5432/x"), "the host is kept: {raw}");
}

#[test]
fn a_field_cannot_forge_the_level_the_time_or_the_service() {
    let out = json_lines(|| {
        tracing::info!(
            level = "FATAL",
            service = "attacker",
            timestamp = "1970",
            "hello"
        )
    });
    let line = &out[0];
    assert_eq!(line["level"], "INFO");
    assert_eq!(line["service"], "sagat-test");
    assert_ne!(line["timestamp"], "1970");
    assert_eq!(line["fields.level"], "FATAL");
    assert_eq!(line["fields.service"], "attacker");
}

#[test]
fn a_line_break_in_a_value_cannot_start_a_second_line() {
    let out = lines(Style::Json, || {
        tracing::info!(
            note = "first\n{\"level\":\"ERROR\",\"message\":\"forged\"}",
            "one event"
        );
    });
    assert_eq!(out.len(), 1, "{out:?}");
}

#[test]
fn the_text_style_is_one_line_per_event_with_the_same_redaction() {
    let out = lines(Style::Text, || {
        tracing::info!(route = "/runs", token = "abc123", "request completed");
    });
    assert_eq!(out.len(), 1);
    assert!(
        out[0].contains("request completed") && out[0].contains("route=/runs"),
        "{}",
        out[0]
    );
    assert!(!out[0].contains("abc123"), "{}", out[0]);
}

#[test]
fn a_line_break_cannot_start_a_second_line_in_the_text_style_either() {
    let out = lines(Style::Text, || {
        tracing::info!(
            note = "first\r\n2026-01-01T00:00:00Z ERROR forged: entry",
            "one\nevent"
        );
    });
    assert_eq!(out.len(), 1, "{out:?}");
}

#[test]
fn a_very_long_value_is_cut() {
    let out = json_lines(|| tracing::info!(note = %"y".repeat(10_000), "long"));
    let note = out[0]["note"].as_str().expect("string");
    assert!(note.chars().count() < 2_200, "{}", note.chars().count());
    assert!(note.contains("characters cut"));
}

#[test]
fn postgres_driver_diagnostics_never_reach_either_log_format() {
    for style in [Style::Json, Style::Text] {
        let out = lines(style, || {
            tracing::debug!(target: "tokio_postgres::prepare", "SELECT driver-sql-canary");
            tracing::debug!(target: "sqlparser::parser", "SELECT parser-sql-canary");
            tracing::info!(target: "tokio_postgres::connection", "NOTICE driver-password-canary");
            tracing::info!(target: "sagat_infra_database", outcome = "failed", "database execution completed");
        });
        assert_eq!(out.len(), 1);
        assert!(!out.join("\n").contains("canary"));
        assert!(out[0].contains("database execution completed"));
    }
}
