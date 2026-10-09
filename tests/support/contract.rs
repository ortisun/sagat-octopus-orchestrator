//! Machinery shared by the REST conformance suites: a minimal OpenAPI reader and a scenario
//! checker that holds every real response to the contract (documented status, schema, headers,
//! `traceId == X-Trace-Id`).
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use reqwest::Method;
use serde_json::{Value, json};

use super::api::TestApi;

pub struct OpenApi {
    doc: Value,
}

impl OpenApi {
    pub fn load() -> Self {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../specs/001-platform-foundation/contracts/openapi.yaml"
        );
        let text = std::fs::read_to_string(path).expect("openapi.yaml");
        let doc: Value = serde_yaml_ng::from_str(&text).expect("valid YAML");
        Self { doc }
    }

    /// The run statuses the contract names (the enumeration of `Run.status`).
    pub fn run_statuses(&self) -> Vec<String> {
        self.doc["components"]["schemas"]["Run"]["properties"]["status"]["enum"]
            .as_array()
            .expect("Run.status is an enumeration")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    }

    fn operation(&self, method: &str, template: &str) -> &Value {
        self.doc["paths"][template][method.to_ascii_lowercase()]
            .as_object()
            .map(|_| &self.doc["paths"][template][method.to_ascii_lowercase()])
            .unwrap_or_else(|| panic!("the contract has no operation {method} {template}"))
    }

    /// Resolves a `$ref` to a component (only local references are used in this contract).
    fn resolve<'a>(&'a self, node: &'a Value) -> &'a Value {
        match node.get("$ref").and_then(Value::as_str) {
            Some(reference) => {
                let path = reference.strip_prefix("#/").expect("local $ref");
                path.split('/').fold(&self.doc, |n, key| &n[key])
            }
            None => node,
        }
    }

    /// The documented response object for `status`. The catch-all `default` only stands in for the
    /// failures every endpoint can have whatever it does (a timeout, an oversized or unsupported
    /// body, an unexpected error); every other status must be declared by the operation, or the
    /// catch-all would make any status look documented.
    fn response(&self, method: &str, template: &str, status: u16) -> Option<&Value> {
        const COVERED_BY_DEFAULT: [u16; 4] = [408, 413, 415, 500];
        let responses = &self.operation(method, template)["responses"];
        responses
            .get(status.to_string())
            .or_else(|| {
                COVERED_BY_DEFAULT
                    .contains(&status)
                    .then(|| responses.get("default"))
                    .flatten()
            })
            .map(|r| self.resolve(r))
    }

    /// Whether `status` is documented (explicitly or through `default`).
    pub fn documents(&self, method: &str, template: &str, status: u16) -> bool {
        self.response(method, template, status).is_some()
    }

    /// Validates a JSON body against the documented schema of that response. `None` if the response
    /// documents no JSON body.
    pub fn validate_body(
        &self,
        method: &str,
        template: &str,
        status: u16,
        body: &Value,
    ) -> Option<Result<(), String>> {
        let response = self.response(method, template, status)?;
        let schema = response
            .get("content")?
            .get("application/json")?
            .get("schema")?;
        Some(self.validate(schema, body))
    }

    /// Names of the headers documented for that response.
    pub fn documented_headers(&self, method: &str, template: &str, status: u16) -> Vec<String> {
        self.response(method, template, status)
            .and_then(|r| r.get("headers"))
            .and_then(Value::as_object)
            .map(|h| h.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Schema (pattern) of a documented response header.
    pub fn header_schema(&self, method: &str, template: &str, status: u16, header: &str) -> Value {
        let response = self
            .response(method, template, status)
            .expect("documented response");
        let header = self.resolve(&response["headers"][header]);
        header["schema"].clone()
    }

    pub fn validate(&self, schema: &Value, instance: &Value) -> Result<(), String> {
        // Validate against the schema with the document's components alongside, so local `$ref`s
        // such as `#/components/schemas/Error` resolve.
        let mut root = schema.clone();
        root.as_object_mut()
            .expect("schema object")
            .insert("components".into(), self.doc["components"].clone());
        let validator = jsonschema::options()
            .should_validate_formats(true)
            .build(&root)
            .map_err(|e| format!("the schema itself is invalid: {e}"))?;
        let errors: Vec<String> = validator
            .iter_errors(instance)
            .map(|e| format!("{e} (at {})", e.instance_path()))
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// `(method, template)` of every operation tagged `tag`.
    pub fn operations_tagged(&self, tag: &str) -> Vec<(String, String)> {
        let mut found = Vec::new();
        for (path, item) in self.doc["paths"].as_object().expect("paths") {
            for (method, op) in item.as_object().expect("path item") {
                let tagged = op["tags"]
                    .as_array()
                    .is_some_and(|t| t.iter().any(|v| v == tag));
                if tagged {
                    found.push((method.to_ascii_uppercase(), path.clone()));
                }
            }
        }
        found.sort();
        found
    }

    /// `(method, template)` of every operation whose vendor extension `key` equals `value`.
    pub fn operations_marked(&self, key: &str, value: &str) -> Vec<(String, String)> {
        let mut found = Vec::new();
        for (path, item) in self.doc["paths"].as_object().expect("paths") {
            for (method, op) in item.as_object().expect("path item") {
                if op.get(key).and_then(Value::as_str) == Some(value) {
                    found.push((method.to_ascii_uppercase(), path.clone()));
                }
            }
        }
        found.sort();
        found
    }

    /// Every operation of the document: `(method, template, operationId, implemented)`, where
    /// "implemented" means a feature built it (`x-implemented`, or the identity operations).
    pub fn all_operations(&self) -> Vec<(String, String, String, bool)> {
        let mut found = Vec::new();
        for (path, item) in self.doc["paths"].as_object().expect("paths") {
            for (method, op) in item.as_object().expect("path item") {
                if !["get", "put", "post", "delete", "patch"].contains(&method.as_str()) {
                    continue;
                }
                let id = op["operationId"].as_str().expect("operationId").to_owned();
                let identity = op["tags"]
                    .as_array()
                    .is_some_and(|t| t.iter().any(|v| v == "Identity"));
                let implemented = op.get("x-implemented").is_some() || identity;
                found.push((method.to_ascii_uppercase(), path.clone(), id, implemented));
            }
        }
        found.sort();
        found
    }

    pub fn error_codes(&self) -> Vec<String> {
        self.doc["components"]["schemas"]["Error"]["properties"]["code"]["enum"]
            .as_array()
            .expect("error codes")
            .iter()
            .map(|v| v.as_str().expect("str").to_owned())
            .collect()
    }
}

#[derive(Clone)]
pub enum As {
    Anonymous,
    Garbage,
    User(&'static str, &'static str),
}

pub struct Scenario {
    pub method: &'static str,
    pub template: &'static str,
    pub path: String,
    pub caller: As,
    pub body: Option<Value>,
    pub expect: u16,
    /// Extra request headers (namespace, idempotency key, ...).
    pub headers: Vec<(&'static str, String)>,
}

pub fn scenario(
    method: &'static str,
    template: &'static str,
    path: &str,
    caller: As,
    body: Option<Value>,
    expect: u16,
) -> Scenario {
    Scenario {
        method,
        template,
        path: path.to_owned(),
        caller,
        body,
        expect,
        headers: Vec::new(),
    }
}

impl Scenario {
    #[must_use]
    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// Sends the scenario and checks it against the contract. Returns `(method, template, status)`.
pub async fn check(api: &TestApi, spec: &OpenApi, s: &Scenario) -> (String, String, u16) {
    let method = Method::from_bytes(s.method.as_bytes()).expect("method");
    let mut request = api.request(method, &s.path);
    request = match &s.caller {
        As::Anonymous => request,
        As::Garbage => request.bearer_auth("not.a.jwt"),
        As::User(sub, tenant) => request.bearer_auth(api.idp.token(sub, tenant).build()),
    };
    for (name, value) in &s.headers {
        request = request.header(*name, value);
    }
    if let Some(body) = &s.body {
        request = request.json(body);
    }
    let response = request.send().await.expect("send");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let text = response.text().await.expect("body");
    let label = format!("{} {} -> {status}", s.method, s.path);

    assert_eq!(status, s.expect, "unexpected status for {label}: {text}");
    assert!(
        spec.documents(s.method, s.template, status),
        "{label}: status {status} is NOT documented for {} {}",
        s.method,
        s.template
    );

    if !text.is_empty() {
        let body: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{label}: body is not JSON ({e}): {text}"));
        if let Some(result) = spec.validate_body(s.method, s.template, status, &body) {
            result.unwrap_or_else(|e| {
                panic!("{label}: body does not conform to the contract: {e}\n{body}")
            });
        }
        if status >= 400 {
            assert_eq!(
                body["traceId"],
                headers["x-trace-id"].to_str().expect("ascii"),
                "{label}: error traceId == X-Trace-Id"
            );
        }
    }
    for name in spec.documented_headers(s.method, s.template, status) {
        let value = headers
            .get(name.to_ascii_lowercase().as_str())
            .unwrap_or_else(|| panic!("{label}: documented header {name} is missing"));
        let schema = spec.header_schema(s.method, s.template, status, &name);
        let as_json = value
            .to_str()
            .expect("ascii")
            .parse::<u64>()
            .map_or_else(|_| json!(value.to_str().expect("ascii")), |n| json!(n));
        spec.validate(&schema, &as_json)
            .unwrap_or_else(|e| panic!("{label}: header {name} violates the contract: {e}"));
    }
    (s.method.to_owned(), s.template.to_owned(), status)
}
