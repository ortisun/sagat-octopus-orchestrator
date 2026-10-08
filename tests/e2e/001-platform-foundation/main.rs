//! End-to-end scenarios of feature 001 over HTTP: real router, real PostgreSQL, fake OIDC provider.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod us01;
mod us02;
mod us04;

use reqwest::Response;
use serde_json::Value;

/// `X-Trace-Id` of a response; asserts it is a well-formed W3C trace id.
fn trace_id(response: &Response) -> String {
    let id = response
        .headers()
        .get("x-trace-id")
        .expect("every response carries X-Trace-Id")
        .to_str()
        .expect("ascii")
        .to_owned();
    assert!(
        id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "bad trace id {id}"
    );
    id
}

/// An error body with its `traceId` removed, so bodies of different requests can be compared.
fn without_trace(mut body: Value) -> Value {
    body.as_object_mut()
        .expect("object")
        .remove("traceId")
        .expect("error bodies carry traceId");
    body
}
