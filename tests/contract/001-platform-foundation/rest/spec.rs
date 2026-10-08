//! A minimal reader of the OpenAPI document: just what the conformance checks need.

use serde_json::Value;

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

    /// The documented response object for `status`, falling back to `default`.
    fn response(&self, method: &str, template: &str, status: u16) -> Option<&Value> {
        let responses = &self.operation(method, template)["responses"];
        responses
            .get(status.to_string())
            .or_else(|| responses.get("default"))
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

    pub fn error_codes(&self) -> Vec<String> {
        self.doc["components"]["schemas"]["Error"]["properties"]["code"]["enum"]
            .as_array()
            .expect("error codes")
            .iter()
            .map(|v| v.as_str().expect("str").to_owned())
            .collect()
    }
}
