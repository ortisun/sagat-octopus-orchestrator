//! The exporter's settings from the standard OpenTelemetry variables.

use std::collections::HashMap;
use std::time::Duration;

use sagat_telemetry::OtlpConfig;

fn from(vars: &[(&str, &str)]) -> Result<Option<OtlpConfig>, String> {
    let map: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    OtlpConfig::from_lookup(&|name| map.get(name).cloned(), "sagat-engine").map_err(|e| e.to_string())
}

#[test]
fn nothing_is_exported_unless_an_endpoint_is_set() {
    assert!(from(&[]).expect("ok").is_none());
    assert!(from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "  ")]).expect("ok").is_none());
    assert!(from(&[("OTEL_SERVICE_NAME", "x"), ("OTEL_TRACES_SAMPLER", "always_on")]).expect("ok").is_none());
}

#[test]
fn the_sdk_can_be_switched_off_whatever_else_is_set() {
    let off = from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318"), ("OTEL_SDK_DISABLED", "TRUE")]);
    assert!(off.expect("ok").is_none());
}

#[test]
fn a_base_endpoint_gets_the_traces_path_and_a_traces_endpoint_is_taken_as_it_is() {
    let base = from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318/")]).expect("ok").expect("on");
    assert_eq!(base.endpoint, "http://collector:4318/v1/traces");
    assert_eq!(base.service_name, "sagat-engine");
    assert!((base.sample_ratio - 1.0).abs() < f64::EPSILON);
    let full = from(&[
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://ignored:4318"),
        ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "https://otlp.example.com/custom/traces"),
    ])
    .expect("ok")
    .expect("on");
    assert_eq!(full.endpoint, "https://otlp.example.com/custom/traces");
}

#[test]
fn the_service_name_headers_and_timeout_follow_the_variables() {
    let config = from(&[
        ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://otlp.example.com"),
        ("OTEL_SERVICE_NAME", "orchestrator-eu"),
        ("OTEL_EXPORTER_OTLP_HEADERS", "x-api-key=abc, authorization = Bearer t0k3n"),
        ("OTEL_EXPORTER_OTLP_TIMEOUT", "2500"),
        ("SAGAT_OTLP_QUEUE_CAPACITY", "512"),
    ])
    .expect("ok")
    .expect("on");
    assert_eq!(config.service_name, "orchestrator-eu");
    assert_eq!(config.headers, vec![
        ("x-api-key".to_owned(), "abc".to_owned()),
        ("authorization".to_owned(), "Bearer t0k3n".to_owned()),
    ]);
    assert_eq!(config.timeout, Duration::from_millis(2500));
    assert_eq!(config.queue_capacity, 512);
}

#[test]
fn the_sampler_is_the_ratio_the_variables_say() {
    let on = |extra: &[(&str, &str)]| {
        let mut vars = vec![("OTEL_EXPORTER_OTLP_ENDPOINT", "http://c:4318")];
        vars.extend_from_slice(extra);
        from(&vars)
    };
    assert!((on(&[("OTEL_TRACES_SAMPLER", "always_off")]).expect("ok").expect("on").sample_ratio).abs() < f64::EPSILON);
    let ratio = on(&[("OTEL_TRACES_SAMPLER", "parentbased_traceidratio"), ("OTEL_TRACES_SAMPLER_ARG", "0.25")]);
    assert!((ratio.expect("ok").expect("on").sample_ratio - 0.25).abs() < f64::EPSILON);
    assert!(on(&[("OTEL_TRACES_SAMPLER", "traceidratio")]).is_err(), "a ratio sampler needs its ratio");
    assert!(on(&[("OTEL_TRACES_SAMPLER", "traceidratio"), ("OTEL_TRACES_SAMPLER_ARG", "1.5")]).is_err());
    assert!(on(&[("OTEL_TRACES_SAMPLER", "traceidratio"), ("OTEL_TRACES_SAMPLER_ARG", "NaN")]).is_err());
    assert!(on(&[("OTEL_TRACES_SAMPLER", "jaeger_remote")]).is_err());
}

#[test]
fn a_setting_the_exporter_cannot_honour_is_an_error_and_never_a_silent_fallback() {
    let with = |extra: (&str, &str)| from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://c:4318"), extra]);
    for (extra, why) in [
        (("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"), "grpc is not spoken"),
        (("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "http/protobuf"), "protobuf is not spoken"),
        (("OTEL_EXPORTER_OTLP_HEADERS", "no-equals-sign"), "a header without a value"),
        (("OTEL_EXPORTER_OTLP_HEADERS", "=value"), "a header without a name"),
        (("OTEL_EXPORTER_OTLP_TIMEOUT", "5"), "a timeout below 100 ms"),
        (("OTEL_EXPORTER_OTLP_TIMEOUT", "soon"), "a timeout that is not a number"),
        (("SAGAT_OTLP_QUEUE_CAPACITY", "3"), "a queue too small to batch"),
    ] {
        assert!(with(extra).is_err(), "{why}");
    }
    assert!(with(("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json")).is_ok());
    assert!(from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "ftp://c")]).is_err());
    assert!(from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "collector:4318")]).is_err());
}

#[test]
fn credentials_in_the_endpoint_are_refused_and_their_value_is_not_repeated() {
    let error = from(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "https://user:hunter2@otlp.example.com")]).expect_err("refused");
    assert!(error.contains("OTEL_EXPORTER_OTLP_ENDPOINT"), "{error}");
    assert!(!error.contains("hunter2"), "{error}");
}
