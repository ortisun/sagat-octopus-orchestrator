//! The continuation-chain contract, read from the canonical schema and OpenAPI rather than
//! restated: segment and timeout bounds, explicit transfer, the chain command key, the paged list
//! of a chain's runs and the additive segment metadata of a run (T014-034, D293, D299).

use jsonschema::Validator;
use serde_json::{Value, json};
use std::sync::OnceLock;

fn workflow_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = super::repository_root()
            .join("specs/001-platform-foundation/contracts/workflow.schema.json");
        serde_json::from_str(&std::fs::read_to_string(path).expect("workflow schema"))
            .expect("workflow schema is JSON")
    })
}

fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        jsonschema::options()
            .build(workflow_schema())
            .expect("workflow schema compiles")
    })
}

fn openapi() -> &'static Value {
    static OPENAPI: OnceLock<Value> = OnceLock::new();
    OPENAPI.get_or_init(|| {
        let path =
            super::repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
        serde_saphyr::from_str(&std::fs::read_to_string(path).expect("openapi"))
            .expect("canonical OpenAPI is YAML")
    })
}

fn continuing(policy: Value) -> Value {
    let mut document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "chain-contract"},
        "entrypoint": "rollover",
        "nodes": [{"id": "rollover", "type": "continueAsNew", "input": "input"}]
    });
    if !policy.is_null() {
        document["chainPolicy"] = policy;
    }
    document
}

fn accepted(document: &Value) -> bool {
    validator().is_valid(document)
}

#[test]
fn max_segments_is_one_to_ten_thousand_and_counts_the_initial_run() {
    for valid in [1, 2, 9_999, 10_000] {
        assert!(
            accepted(&continuing(
                json!({"maxSegments": valid, "timeoutMs": 60_000})
            )),
            "{valid}"
        );
    }
    for invalid in [
        json!(0),
        json!(-1),
        json!(10_001),
        json!(1.5),
        json!("3"),
        Value::Null,
    ] {
        assert!(
            !accepted(&continuing(
                json!({"maxSegments": invalid, "timeoutMs": 60_000})
            )),
            "{invalid}"
        );
    }
    assert!(!accepted(&continuing(json!({"timeoutMs": 60_000}))));
    // The limit and the segment index agree: 10,000 segments are indexes 0..9,999.
    let components = &openapi()["components"]["schemas"];
    for (name, path) in [
        (
            "ChainSegment",
            &components["ChainSegment"]["properties"]["index"],
        ),
        ("ChainRun", &components["ChainRun"]["properties"]["index"]),
        (
            "Run.chain",
            &components["Run"]["properties"]["chain"]["properties"]["segmentIndex"],
        ),
        (
            "RuntimeChain",
            &components["RuntimeChain"]["properties"]["segmentIndex"],
        ),
    ] {
        assert_eq!(path["minimum"], 0, "{name}");
        assert_eq!(path["maximum"], 9_999, "{name}");
    }
    let description = workflow_schema()["$defs"]["chainPolicy"].to_string();
    assert!(
        description.contains("maxSegments"),
        "the schema names the limit it bounds"
    );
}

#[test]
fn the_original_timeout_is_required_positive_and_bounded() {
    assert!(accepted(&continuing(
        json!({"maxSegments": 3, "timeoutMs": 1})
    )));
    assert!(accepted(&continuing(
        json!({"maxSegments": 3, "timeoutMs": 31_622_400_000_u64})
    )));
    for invalid in [
        json!(0),
        json!(-5),
        json!(31_622_400_001_u64),
        json!("60s"),
        Value::Null,
    ] {
        assert!(
            !accepted(&continuing(json!({"maxSegments": 3, "timeoutMs": invalid}))),
            "{invalid}"
        );
    }
    assert!(
        !accepted(&continuing(json!({"maxSegments": 3}))),
        "a continuation without an original timeout is refused"
    );
    assert!(
        !accepted(&continuing(Value::Null)),
        "a continuation without a chain policy is refused"
    );
    let mut extra = continuing(json!({"maxSegments": 3, "timeoutMs": 60_000}));
    extra["chainPolicy"]["resetTimeout"] = json!(true);
    assert!(!accepted(&extra), "the timeout cannot be reset by policy");
}

#[test]
fn a_successor_receives_only_the_input_and_state_the_node_names() {
    let policy = json!({"maxSegments": 3, "timeoutMs": 60_000});
    let mut document = continuing(policy);
    assert!(accepted(&document));
    // The input is required, and nothing implicit (run output, step outputs) is transferred.
    document["nodes"][0]
        .as_object_mut()
        .expect("node")
        .remove("input");
    assert!(
        !accepted(&document),
        "an explicit input expression is required"
    );
    for hidden in ["output", "outputs", "carry", "stateRevision"] {
        let mut document = continuing(json!({"maxSegments": 3, "timeoutMs": 60_000}));
        document["nodes"][0][hidden] = json!("input");
        assert!(!accepted(&document), "{hidden} is not a transfer");
    }
    // State moves only when it is declared and named.
    let mut stateful = continuing(json!({"maxSegments": 3, "timeoutMs": 60_000}));
    stateful["nodes"][0]["state"] = json!("state");
    assert!(
        !accepted(&stateful),
        "a state transfer without a declared state is refused"
    );
}

#[test]
fn chain_commands_use_the_shared_idempotency_key_bounds() {
    let spec = openapi();
    let key = &spec["components"]["parameters"]["IdempotencyKey"];
    assert_eq!(key["required"], true);
    assert_eq!(key["schema"]["minLength"], 8);
    assert_eq!(key["schema"]["maxLength"], 160);
    for (path, method) in [
        (
            "/namespaces/{namespaceId}/chains/{chainId}/signals/{signalName}",
            "post",
        ),
        ("/namespaces/{namespaceId}/chains/{chainId}/cancel", "post"),
    ] {
        let parameters = spec["paths"][path][method]["parameters"]
            .as_array()
            .unwrap_or_else(|| panic!("{path} has parameters"));
        assert!(
            parameters
                .iter()
                .any(|parameter| parameter["$ref"] == "#/components/parameters/IdempotencyKey"),
            "{path} takes the shared Idempotency-Key (8..160), not a 1..128 key of its own"
        );
        let request = spec["paths"][path][method]["requestBody"]["content"]["application/json"]
            ["schema"]["$ref"]
            .as_str()
            .expect("request schema")
            .rsplit('/')
            .next()
            .expect("schema name")
            .to_owned();
        let required = spec["components"]["schemas"][&request]["required"]
            .as_array()
            .expect("required");
        assert!(
            required.iter().any(|name| name == "expectedRunId"),
            "{request} requires the head the caller saw"
        );
    }
}

#[test]
fn the_runs_of_a_chain_are_a_bounded_cursor_page_without_payloads() {
    let spec = openapi();
    let operation = &spec["paths"]["/namespaces/{namespaceId}/chains/{chainId}/runs"]["get"];
    assert_eq!(operation["operationId"], "listExecutionChainRuns");
    let parameters = operation["parameters"].to_string();
    for shared in ["Cursor", "Limit", "NamespaceId"] {
        assert!(
            parameters.contains(&format!("#/components/parameters/{shared}")),
            "{shared}"
        );
    }
    let limit = &spec["components"]["parameters"]["Limit"]["schema"];
    assert_eq!(
        (limit["minimum"].as_i64(), limit["maximum"].as_i64()),
        (Some(1), Some(200))
    );
    assert_eq!(
        operation["responses"]["200"]["headers"]["Cache-Control"]["schema"]["const"],
        "no-store"
    );
    for status in ["400", "401", "403", "404"] {
        assert!(operation["responses"][status].is_object(), "{status}");
    }

    let page = &spec["components"]["schemas"]["ChainRunPage"];
    assert_eq!(page["additionalProperties"], false);
    assert_eq!(page["properties"]["items"]["maxItems"], 200);
    assert_eq!(page["required"], json!(["items", "nextCursor"]));
    let item = &spec["components"]["schemas"]["ChainRun"];
    assert_eq!(item["additionalProperties"], false);
    let mut names: Vec<&str> = item["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "completionKind",
            "createdAt",
            "finishedAt",
            "index",
            "predecessorRunId",
            "runId",
            "status",
            "successorRunId"
        ],
        "a page lists lineage and lifecycle; it never carries an input, output or state"
    );
    assert_eq!(
        item["properties"]["completionKind"]["enum"],
        json!(["CONTINUED", "FINAL", null])
    );
}

#[test]
fn a_run_names_its_chain_segment_additively() {
    let run = &openapi()["components"]["schemas"]["Run"];
    let required = run["required"].as_array().expect("required");
    assert!(
        !required.iter().any(|name| name == "chain"),
        "the segment is additive: a client that predates it keeps parsing a run"
    );
    let chain = &run["properties"]["chain"];
    assert_eq!(chain["additionalProperties"], false);
    assert_eq!(
        chain["properties"]["completionKind"]["enum"],
        json!(["CONTINUED", "FINAL", null])
    );
    assert!(
        chain["description"]
            .as_str()
            .expect("description")
            .contains("not the process outcome"),
        "the contract says why a SUCCEEDED segment is not a process outcome"
    );
    // The run status enum is the one clients already parse: no CONTINUED status.
    let statuses = run["properties"]["status"]["enum"].to_string();
    assert!(!statuses.contains("CONTINUED"), "{statuses}");
}
