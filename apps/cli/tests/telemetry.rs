//! Onboarding telemetry is explicit, bounded and correlated with the requests the command makes.
#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn temp_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "sagat-cli-{name}-{}-{}.ndjson",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn cli(args: &[&str], configure: impl FnOnce(&mut Command)) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sagat"));
    command.args(args);
    for name in [
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_HEADERS",
        "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
        "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
        "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
        "SAGAT_ONBOARDING_TELEMETRY_FILE",
        "SAGAT_TOKEN",
    ] {
        command.env_remove(name);
    }
    configure(&mut command);
    command.output().expect("run sagat")
}

fn capture_http(response_body: &'static str) -> (String, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let address = format!("http://{}", listener.local_addr().expect("address"));
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .expect("read timeout");
                    let mut request = Vec::new();
                    let mut buf = [0_u8; 4096];
                    let header_end = loop {
                        let read = socket.read(&mut buf).expect("read request");
                        assert!(read > 0, "request ended before its headers");
                        request.extend_from_slice(&buf[..read]);
                        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    while request.len() < header_end + content_length {
                        let read = socket.read(&mut buf).expect("read body");
                        assert!(read > 0, "request body ended early");
                        request.extend_from_slice(&buf[..read]);
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    socket.write_all(response.as_bytes()).expect("respond");
                    send.send(request).expect("captured request");
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "no HTTP request arrived");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept: {error}"),
            }
        }
    });
    (address, receive)
}

fn capture_http_sequence(response_bodies: Vec<&'static str>) -> (String, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let address = format!("http://{}", listener.local_addr().expect("address"));
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for response_body in response_bodies {
            let (mut socket, _) = listener.accept().expect("accept request");
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read timeout");
            let mut request = Vec::new();
            let mut buf = [0_u8; 4096];
            let header_end = loop {
                let read = socket.read(&mut buf).expect("read request");
                assert!(read > 0, "request ended before its headers");
                request.extend_from_slice(&buf[..read]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let read = socket.read(&mut buf).expect("read body");
                assert!(read > 0, "request body ended early");
                request.extend_from_slice(&buf[..read]);
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket.write_all(response.as_bytes()).expect("respond");
            send.send(request).expect("captured request");
        }
    });
    (address, receive)
}

fn request_json(request: &[u8]) -> serde_json::Value {
    let header_end = request
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("OTLP headers")
        + 4;
    serde_json::from_slice(&request[header_end..]).expect("OTLP JSON")
}

#[test]
fn telemetry_is_off_until_the_operator_opts_in() {
    let report = temp_file("off");
    let output = cli(&["help"], |_| {});
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("sagat:"));
    assert!(output.stderr.is_empty());
    assert!(!report.exists());
}

#[test]
fn local_report_appends_bounded_logs_and_metrics_without_inputs_or_secrets() {
    let report = temp_file("local");
    let secret = "secret-token-that-must-not-be-reported";
    let mut invalid_error = String::new();
    for args in [&["help"][..], &["unknown-secret-command"][..]] {
        let output = cli(args, |command| {
            command
                .env("SAGAT_ONBOARDING_TELEMETRY_FILE", &report)
                .env("SAGAT_TOKEN", secret);
        });
        assert_eq!(output.status.success(), args[0] == "help");
        if args[0] != "help" {
            invalid_error = String::from_utf8_lossy(&output.stderr).into_owned();
        }
    }

    let text = std::fs::read_to_string(&report).expect("local report");
    assert_eq!(text.lines().count(), 4, "one log and metric per invocation");
    assert!(!text.contains(secret));
    assert!(!text.contains("unknown-secret-command"));
    assert!(!text.contains(&report.to_string_lossy().into_owned()));
    assert!(invalid_error.contains("sagat: trace "), "{invalid_error}");
    assert!(!invalid_error.contains(secret));

    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON line"))
        .collect();
    assert_eq!(rows[0]["signal"], "log");
    assert_eq!(rows[1]["signal"], "metric");
    assert_eq!(rows[2]["signal"], "log");
    assert_eq!(rows[3]["signal"], "metric");
    for row in &rows {
        assert!(matches!(row["command"].as_str(), Some("help" | "invalid")));
        assert!(matches!(row["outcome"].as_str(), Some("success" | "usage")));
        assert!(row["duration_ms"].as_u64().is_some());
    }
    assert!(
        rows[0]["trace_id"]
            .as_str()
            .is_some_and(|id| id.len() == 32)
    );
    assert!(
        rows[2]["trace_id"]
            .as_str()
            .is_some_and(|id| id.len() == 32)
    );
    assert!(
        rows[1].get("trace_id").is_none(),
        "metric labels carry no IDs"
    );
    assert!(
        rows[3].get("trace_id").is_none(),
        "metric labels carry no IDs"
    );
}

#[test]
fn all_otlp_signals_are_flushed_and_correlated_even_when_the_local_report_fails() {
    let (collector, exported) = capture_http_sequence(vec!["{}", "{}", "{}"]);
    let (idp, called) =
        capture_http(r#"{"access_token":"secret-token-returned-by-idp","expires_in":3600}"#);
    let invalid_report = temp_file("missing-parent").join("report.ndjson");
    let output = cli(&["dev", "token", "--idp", &idp], |command| {
        command
            .env("OTEL_EXPORTER_OTLP_ENDPOINT", &collector)
            .env("SAGAT_ONBOARDING_TELEMETRY_FILE", &invalid_report);
    });
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let platform_request = called
        .recv_timeout(Duration::from_secs(2))
        .expect("identity-provider request");
    let platform_text = String::from_utf8(platform_request).expect("HTTP request text");
    let traceparent = platform_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("traceparent")
                .then(|| value.trim().to_owned())
        })
        .expect("traceparent header");
    let trace_id = traceparent.split('-').nth(1).expect("trace id");

    let requests: Vec<Vec<u8>> = (0..3)
        .map(|_| {
            exported
                .recv_timeout(Duration::from_secs(2))
                .expect("OTLP request")
        })
        .collect();
    let request_lines: Vec<&str> = requests
        .iter()
        .map(|request| {
            std::str::from_utf8(request)
                .expect("request text")
                .lines()
                .next()
                .expect("request line")
        })
        .collect();
    assert_eq!(
        request_lines,
        [
            "POST /v1/traces HTTP/1.1",
            "POST /v1/logs HTTP/1.1",
            "POST /v1/metrics HTTP/1.1"
        ]
    );
    let span = request_json(&requests[0]).to_string();
    let log = request_json(&requests[1]).to_string();
    let metric = request_json(&requests[2]).to_string();
    assert!(span.contains("sagat.cli.command"));
    assert!(span.contains("dev.token"));
    assert!(span.contains(trace_id));
    assert!(log.contains("sagat.cli.command.completed"));
    assert!(log.contains(trace_id));
    assert!(log.contains("dev.token"));
    assert!(metric.contains("sagat_cli_command_duration_seconds"));
    assert!(metric.contains("dev.token"));
    assert!(metric.contains("success"));
    assert!(
        !metric.contains(trace_id),
        "metric dimensions carry no trace ID"
    );
    for encoded in [&span, &log, &metric] {
        assert!(!encoded.contains("secret-token-returned-by-idp"));
        assert!(!encoded.contains(&idp));
        assert!(!encoded.contains(&collector));
        assert!(!encoded.contains(&invalid_report.to_string_lossy().into_owned()));
    }
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("sagat: onboarding telemetry: the report file could not be opened"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn authenticated_rest_calls_continue_the_command_trace_without_exporting_credentials() {
    let (collector, exported) = capture_http("{}");
    let (api, called) = capture_http("{}");
    let token = "secret-bearer-that-must-not-be-exported";
    let output = cli(
        &[
            "--server",
            &api,
            "--namespace",
            "payments",
            "workflow",
            "inspect",
            "0199f1f0-0000-7000-8000-000000000001",
        ],
        |command| {
            command.env("SAGAT_TOKEN", token).env(
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                format!("{collector}/v1/traces"),
            );
        },
    );
    assert!(
        !output.status.success(),
        "the stub serves only the first request"
    );

    let api_request = called
        .recv_timeout(Duration::from_secs(2))
        .expect("API request");
    let api_text = String::from_utf8(api_request).expect("HTTP request text");
    assert!(api_text.contains(&format!("authorization: Bearer {token}")));
    let traceparent = api_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("traceparent")
                .then(|| value.trim().to_owned())
        })
        .expect("traceparent header");
    let trace_id = traceparent.split('-').nth(1).expect("trace id");

    let otlp_request = exported
        .recv_timeout(Duration::from_secs(2))
        .expect("OTLP request");
    let header_end = otlp_request
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("OTLP headers")
        + 4;
    let encoded = String::from_utf8_lossy(&otlp_request[header_end..]);
    assert!(encoded.contains(trace_id));
    assert!(!encoded.contains(token));
    assert!(!encoded.contains(&api));
    assert!(!encoded.contains(&collector));
}

#[test]
fn default_async_script_failures_show_a_correlated_trace_and_remediation() {
    let source_path = temp_file("failure.ts");
    let source = "export default () => 1;\n";
    std::fs::write(&source_path, source).expect("script source");
    let source_arg = source_path.to_string_lossy().into_owned();

    for (verb, final_response, expected) in [
        (
            "test",
            r#"{"status":"FAILED","wallMs":7,"failure":"SCRIPT_TIMEOUT","denied":[],"log":""}"#,
            "help: the script ran out of time",
        ),
        (
            "publish",
            r#"{"status":"FAILED","failure":"SCRIPT_BUILD_REJECTED","buildLog":"syntax error"}"#,
            "version 1 was not built",
        ),
    ] {
        let responses = vec![
            r#"{"items":[{"id":"ns-1","slug":"payments"}]}"#,
            r#"{"items":[{"id":"script-1","name":"failure","language":"typescript"}]}"#,
            r#"{"id":"script-1","name":"failure","language":"typescript","source":"export default () => 1;\n","revision":1}"#,
            if verb == "test" {
                r#"{"id":"run-1"}"#
            } else {
                r#"{"version":{"number":1}}"#
            },
            final_response,
        ];
        let (api, requests) = capture_http_sequence(responses);
        let output = cli(
            &[
                "--server",
                &api,
                "--namespace",
                "payments",
                "script",
                verb,
                &source_arg,
                "--language",
                "typescript",
                "--name",
                "failure",
            ],
            |command| {
                command.env("SAGAT_TOKEN", "secret-script-token");
            },
        );
        assert!(!output.status.success(), "{verb} unexpectedly succeeded");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stdout.contains(expected), "stdout: {stdout}");
        assert!(stdout.contains("help:"), "stdout: {stdout}");
        let trace = stderr
            .lines()
            .find_map(|line| line.strip_prefix("sagat: trace "))
            .expect("trace shown for failed command");
        for _ in 0..5 {
            let request = requests
                .recv_timeout(Duration::from_secs(2))
                .expect("script API request");
            assert!(String::from_utf8_lossy(&request).contains(trace));
        }
    }
}
