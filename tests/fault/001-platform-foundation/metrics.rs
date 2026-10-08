//! Unauthenticated metric-label abuse (second security review, finding 1): hyper accepts any HTTP
//! token as a method, and the Prometheus recorder keeps every series it has ever seen, so a method
//! used as a label would let an anonymous client grow process memory without bound.

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_telemetry::Metrics;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::support::api::{TestApi, start};

/// The only values the `method` label may take.
const METHOD_LABELS: [&str; 8] = [
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "OTHER",
];

/// Sends one request with an arbitrary method token over a raw socket and returns the status line.
async fn raw_request(api: &TestApi, method: &str) -> String {
    let addr = api
        .base
        .trim_start_matches("http://")
        .split('/')
        .next()
        .expect("authority");
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request =
        format!("{method} /v1/health HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .expect("the server answers or closes")
        .ok();
    String::from_utf8_lossy(&response)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Every `(label name, value)` pair of the `name` series in a Prometheus text page.
fn label_values(page: &str, name: &str, label: &str) -> Vec<String> {
    let needle = format!("{label}=\"");
    page.lines()
        .filter(|line| line.starts_with(&format!("{name}{{")))
        .filter_map(|line| {
            let start = line.find(&needle)? + needle.len();
            let end = line[start..].find('"')?;
            Some(line[start..start + end].to_owned())
        })
        .collect()
}

#[tokio::test]
async fn arbitrary_http_methods_cannot_create_unbounded_metric_series() {
    let metrics = Metrics::global();
    let api = start().await;
    let before = label_values(&metrics.render(), "http_requests_total", "method").len();

    // 150 distinct method tokens, one of them 20 KB long.
    for i in 0..150 {
        raw_request(&api, &format!("FUZZ{i:04}")).await;
    }
    raw_request(&api, &"M".repeat(20_000)).await;
    // And the standard methods that must keep their own label.
    assert_eq!(raw_request(&api, "GET").await, "HTTP/1.1 200 OK");

    let page = metrics.render();
    let methods = label_values(&page, "http_requests_total", "method");
    let distinct: BTreeSet<&str> = methods.iter().map(String::as_str).collect();

    let allowed: BTreeSet<&str> = METHOD_LABELS.into_iter().collect();
    assert!(
        distinct.is_subset(&allowed),
        "method labels outside the closed set: {:?}",
        distinct.difference(&allowed).take(5).collect::<Vec<_>>()
    );
    assert!(
        distinct.contains("OTHER") && distinct.contains("GET"),
        "unknown methods share one label and standard ones keep theirs: {distinct:?}"
    );
    // Series added by 151 unknown methods: a handful (one per status/route), never one per method.
    assert!(
        methods.len() - before.min(methods.len()) <= 8,
        "{} series before, {} after",
        before,
        methods.len()
    );
    assert!(!page.contains("FUZZ0042") && !page.contains(&"M".repeat(64)));
}
