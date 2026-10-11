//! What the log and the exporter hide, and what they must leave alone (FR-008-006).

use sagat_telemetry::{MAX_VALUE_CHARS, REDACTED, is_sensitive_name, scrub};

#[test]
fn a_name_that_says_the_value_is_private_is_recognised() {
    for name in [
        "authorization",
        "Authorization",
        "lease_token",
        "session-token",
        "client.secret",
        "password",
        "db_passwd",
        "cookie",
        "set-cookie",
        "api_key",
        "x-api-key",
        "private_key",
        "request_body",
        "payload",
        "webhook_signature",
        "bearer",
        "credentials",
    ] {
        assert!(is_sensitive_name(name), "{name} should be hidden");
    }
}

#[test]
fn a_name_that_is_only_an_identifier_or_a_measure_is_not() {
    for name in [
        "trace_id",
        "run_id",
        "tenant_id",
        "route",
        "status",
        "latency_ms",
        "attempt",
        "node",
        "method",
        "error",
        "code",
    ] {
        assert!(!is_sensitive_name(name), "{name} should be kept");
    }
}

#[test]
fn a_web_token_is_scrubbed_wherever_it_stands() {
    let jwt = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r";
    for text in [
        format!("token {jwt} rejected"),
        format!("Authorization: Bearer {jwt}"),
        format!("{{\"id_token\":\"{jwt}\"}}"),
    ] {
        let scrubbed = scrub(&text);
        assert!(!scrubbed.contains("eyJ"), "{scrubbed}");
        assert!(scrubbed.contains(REDACTED), "{scrubbed}");
    }
}

#[test]
fn the_credential_after_a_scheme_is_scrubbed_and_the_words_around_it_are_not() {
    assert_eq!(
        scrub("sent Bearer abcDEF123456789xyz to the gateway"),
        format!("sent Bearer {REDACTED} to the gateway")
    );
    assert_eq!(
        scrub("Basic dXNlcjpwYXNzd29yZA== was refused"),
        format!("Basic {REDACTED} was refused")
    );
    // The word "basic" in a sentence is not a scheme.
    let sentence = "the basic configuration was kept";
    assert_eq!(scrub(sentence), sentence);
}

#[test]
fn the_password_of_a_url_is_scrubbed_and_its_host_is_kept() {
    assert_eq!(
        scrub("connecting to postgres://sagat:hunter2@db.internal:5432/sagat failed"),
        format!("connecting to postgres://{REDACTED}@db.internal:5432/sagat failed")
    );
    assert_eq!(
        scrub("https://user@example.com/path?x=1 and https://example.org/"),
        format!("https://{REDACTED}@example.com/path?x=1 and https://example.org/")
    );
    let plain = "fetching https://example.com/a@b/c";
    assert_eq!(
        scrub(plain),
        plain,
        "an @ after the authority is not userinfo"
    );
}

#[test]
fn keys_with_a_known_prefix_are_scrubbed() {
    for key in [
        "sk-live1234567890abcdef",
        "ghp_abcdefghijklmnopqrstuv",
        "xoxb-1234567890-abc",
        "AKIAIOSFODNN7EXAMPLE",
    ] {
        let text = format!("using {key} now");
        assert_eq!(scrub(&text), format!("using {REDACTED} now"), "{key}");
    }
}

#[test]
fn ordinary_text_and_identifiers_are_left_alone() {
    for text in [
        "request completed",
        "4bf92f3577b34da6a3ce929d0e0e4736",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "tenants/9f8e7d6c-5b4a-3210-fedc-ba9876543210/runs",
        "sk-short",
        "",
    ] {
        assert_eq!(scrub(text), text);
    }
}

#[test]
fn a_long_value_is_cut_and_says_by_how_much() {
    let long = "x".repeat(MAX_VALUE_CHARS + 500);
    let cut = scrub(&long);
    assert!(cut.starts_with(&"x".repeat(MAX_VALUE_CHARS)));
    assert!(
        cut.ends_with("[500 characters cut]"),
        "{}",
        &cut[MAX_VALUE_CHARS..]
    );
}

#[test]
fn text_with_multibyte_characters_is_scrubbed_without_panicking() {
    let text = "ação: Bearer abcdef0123456789é — postgres://üser:pässwörd@hôst/db ✓";
    let scrubbed = scrub(text);
    assert!(!scrubbed.contains("pässwörd"), "{scrubbed}");
    assert!(scrubbed.contains("ação"), "{scrubbed}");
}
