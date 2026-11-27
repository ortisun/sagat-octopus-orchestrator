//! The REST contract of design 044 (Technical Plan 13.2 and 13.3, Task 044-2) in the canonical
//! `specs/001-platform-foundation/contracts/openapi.yaml`: every route of the table exists with its statuses and the
//! permissions its description names, the new operations are merged before their routes (`x-planned: '044'`), the
//! specific codes of table 13.3 refine the closed `Error.code` set through `details.reason`, and an old client reads a
//! code run and a move job without meeting a new value in an enumeration it already knew.

use std::collections::BTreeSet;

use serde_json::Value;

fn openapi() -> Value {
    let path =
        super::repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_saphyr::from_str(&source).expect("canonical OpenAPI is YAML")
}

fn schema<'a>(openapi: &'a Value, name: &str) -> &'a Value {
    let found = &openapi["components"]["schemas"][name];
    assert!(found.is_object(), "components.schemas.{name} exists");
    found
}

fn strings(value: &Value, what: &str) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{what} is a list"))
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// A route of Technical Plan 13.2.
struct Route {
    method: &'static str,
    path: &'static str,
    operation: &'static str,
    /// A new operation of design 044, rather than an existing one it extends.
    new: bool,
    statuses: &'static [&'static str],
    permissions: &'static [&'static str],
    /// The `details.reason` codes of table 13.3 its description names.
    reasons: &'static [&'static str],
}

const ROUTES: &[Route] = &[
    Route {
        method: "post",
        path: "/runs/{runId}/operations",
        operation: "operateRun",
        new: false,
        statuses: &["202", "404", "409", "422"],
        permissions: &["run:operate"],
        reasons: &["IDEMPOTENCY_KEY_REUSED"],
    },
    Route {
        method: "post",
        path: "/runs/{runId}/code-version",
        operation: "moveRunCodeVersion",
        new: true,
        statuses: &["202", "404", "409", "422"],
        permissions: &["run:operate", "version:activate"],
        reasons: &["MOVE_TARGET_INVALID"],
    },
    Route {
        method: "post",
        path: "/namespaces/{namespaceId}/batch-operations",
        operation: "createBatchOperation",
        new: false,
        statuses: &["202", "409", "422"],
        permissions: &["run:batch", "run:operate", "version:activate"],
        reasons: &["MOVE_TARGET_INVALID"],
    },
    Route {
        method: "get",
        path: "/runs/{runId}/history",
        operation: "exportRunHistory",
        new: true,
        statuses: &["200", "404"],
        permissions: &["run:read", "history:export"],
        reasons: &[],
    },
    Route {
        method: "post",
        path: "/runs/{runId}/updates/{operationName}",
        operation: "updateRunState",
        new: false,
        statuses: &["200", "202", "409", "429"],
        permissions: &["run:update"],
        reasons: &[
            "UPDATE_PENDING",
            "IDEMPOTENCY_KEY_REUSED",
            "UPDATE_BACKLOG",
            "HISTORY_HEADROOM",
            "RUN_CLOSING",
            "RUN_NOT_MUTABLE",
        ],
    },
    Route {
        method: "get",
        path: "/runs/{runId}/update-receipts/{updateKey}",
        operation: "getRunUpdateReceipt",
        new: true,
        statuses: &["200", "404"],
        permissions: &["run:update", "run:operate"],
        reasons: &[],
    },
    Route {
        method: "get",
        path: "/namespaces/{namespaceId}/chains/{chainId}/update-receipts/{updateKey}",
        operation: "getExecutionChainUpdateReceipt",
        new: true,
        statuses: &["200", "404"],
        permissions: &["run:update", "run:operate"],
        reasons: &[],
    },
    Route {
        method: "post",
        path: "/runs/{runId}/signals/{signalName}",
        operation: "signalRun",
        new: false,
        statuses: &["202", "409"],
        permissions: &["run:signal"],
        reasons: &[
            "SIGNAL_NOT_DECLARED",
            "SIGNAL_INBOX_FULL",
            "HISTORY_HEADROOM",
            "RUN_CLOSING",
        ],
    },
    Route {
        method: "post",
        path: "/runs/{runId}/worker-queries/{queryName}",
        operation: "queryRunWorker",
        new: true,
        statuses: &["200", "404", "409", "422", "429", "504"],
        permissions: &["run:read", "run:query"],
        reasons: &["QUERY_TIMEOUT", "QUERY_COMMAND", "QUERY_UNAVAILABLE"],
    },
    Route {
        method: "put",
        path: "/namespaces/{namespaceId}/code-workflows/pause",
        operation: "pauseCodeWorkflows",
        new: true,
        statuses: &["204", "404"],
        permissions: &["run:operate", "worker:manage"],
        reasons: &[],
    },
    Route {
        method: "delete",
        path: "/namespaces/{namespaceId}/code-workflows/pause",
        operation: "resumeCodeWorkflows",
        new: true,
        statuses: &["204", "404"],
        permissions: &["run:operate", "worker:manage"],
        reasons: &[],
    },
    Route {
        method: "get",
        path: "/namespaces/{namespaceId}/workflows/{workflowId}/code-versions",
        operation: "listCodeVersions",
        new: true,
        statuses: &["200", "404"],
        permissions: &["definition:read"],
        reasons: &[],
    },
    Route {
        method: "put",
        path: "/namespaces/{namespaceId}/worker-bindings/{subject}",
        operation: "bindWorker",
        new: false,
        statuses: &["200", "422"],
        permissions: &["worker:bind", "definition:publish"],
        reasons: &[],
    },
];

/// The codes of table 13.3 that have an HTTP status. None of them is an `Error.code`: each refines the generic code
/// of its status in `details.reason`.
const SPECIFIC_CODES: &[&str] = &[
    "SIGNAL_NOT_DECLARED",
    "SIGNAL_INBOX_FULL",
    "HISTORY_HEADROOM",
    "RUN_CLOSING",
    "UPDATE_PENDING",
    "UPDATE_BACKLOG",
    "RUN_NOT_MUTABLE",
    "QUERY_TIMEOUT",
    "QUERY_COMMAND",
    "QUERY_UNAVAILABLE",
    "MOVE_TARGET_INVALID",
    "CAPABILITY_NOT_ACTIVE",
    "IDEMPOTENCY_KEY_REUSED",
];

/// An operation of design 044 is planned while no route serves it and implemented once one does; either way it is
/// design 044's.
fn is_design_044s(operation: &Value) -> bool {
    operation["x-planned"] == "044" || operation["x-implemented"] == "044"
}

fn no_store(response: &Value) -> bool {
    response["headers"]["Cache-Control"]["schema"]["const"] == "no-store"
}

#[test]
fn the_openapi_routes_of_design_044_exist_with_their_errors_and_permissions() {
    let openapi = openapi();
    for route in ROUTES {
        let name = format!("{} {} ({})", route.method, route.path, route.operation);
        let operation = &openapi["paths"][route.path][route.method];
        assert!(operation.is_object(), "{name}: the route exists");
        assert_eq!(operation["operationId"], route.operation, "{name}");
        if route.new {
            assert!(
                is_design_044s(operation),
                "{name}: a new operation is merged before its route with `x-planned: '044'`"
            );
        } else {
            assert!(
                operation.get("x-implemented").is_some(),
                "{name}: an existing operation keeps its `x-implemented`"
            );
        }
        let responses = operation["responses"]
            .as_object()
            .unwrap_or_else(|| panic!("{name}: responses"));
        for status in route.statuses {
            assert!(
                responses.contains_key(*status),
                "{name}: documents {status}"
            );
        }
        let description = operation["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: a description"));
        assert!(
            description.contains("Design 044"),
            "{name}: the description says what design 044 changes"
        );
        for permission in route.permissions {
            assert!(
                description.contains(&format!("`{permission}`")),
                "{name}: the description names the permission `{permission}`"
            );
        }
        for reason in route.reasons {
            assert!(
                description.contains(&format!("`{reason}`")),
                "{name}: the description names the reason `{reason}`"
            );
        }
    }

    // Every operation the contract marks as design 044's is a route of the table, and the cross-tenant battery lists
    // it until a route serves it (the battery's own check needs a database; this one reads the list).
    let battery = std::fs::read_to_string(
        super::repository_root().join("tests/contract/security-multitenancy/rest/battery.rs"),
    )
    .expect("the cross-tenant battery");
    let known: BTreeSet<&str> = ROUTES.iter().map(|route| route.operation).collect();
    let mut planned = 0;
    for item in openapi["paths"].as_object().expect("paths").values() {
        for operation in item.as_object().expect("a path item").values() {
            if operation["x-planned"] != "044" {
                continue;
            }
            planned += 1;
            let id = operation["operationId"].as_str().expect("an operationId");
            assert!(
                known.contains(id),
                "{id} is planned by design 044 but is not a route of its table"
            );
            assert!(
                battery.contains(&format!("\"{id}\",")),
                "{id} is planned but missing from `not_built` of the cross-tenant battery"
            );
        }
    }
    assert!(planned <= ROUTES.iter().filter(|route| route.new).count());

    // Answers that carry business data or a worker's answer are never cached.
    for (path, method, status) in [
        ("/runs/{runId}/history", "get", "200"),
        ("/runs/{runId}/updates/{operationName}", "post", "200"),
        ("/runs/{runId}/updates/{operationName}", "post", "202"),
        ("/runs/{runId}/update-receipts/{updateKey}", "get", "200"),
        (
            "/namespaces/{namespaceId}/chains/{chainId}/update-receipts/{updateKey}",
            "get",
            "200",
        ),
        ("/runs/{runId}/worker-queries/{queryName}", "post", "200"),
    ] {
        assert!(
            no_store(&openapi["paths"][path][method]["responses"][status]),
            "{method} {path} {status}: Cache-Control no-store"
        );
    }

    // The history export is JSON lines, paged by `fromSeq`, and its first line is the header.
    let export = &openapi["paths"]["/runs/{runId}/history"]["get"];
    assert!(
        export["responses"]["200"]["content"]["application/x-ndjson"].is_object(),
        "the export is application/x-ndjson"
    );
    assert!(
        export["parameters"]
            .as_array()
            .expect("parameters")
            .iter()
            .any(|parameter| parameter["name"] == "fromSeq" && parameter["in"] == "query"),
        "the export is paged by fromSeq"
    );
    let export_text = export["description"].as_str().expect("a description");
    for needle in [
        "\"exportFormat\": 1",
        "HistoryEvent",
        "worker.proto",
        "specs/044-workflow-as-code/contracts/history-export.baseline.json",
        "audited",
        "graph run",
    ] {
        assert!(
            export_text.contains(needle),
            "the export's description names {needle}"
        );
    }
    let header = schema(&openapi, "HistoryExportHeader");
    assert_eq!(header["properties"]["exportFormat"]["const"], 1);
    assert_eq!(
        strings(&header["required"], "HistoryExportHeader.required"),
        set(&["exportFormat", "gatedEntries", "stateMaxBytes"])
    );

    // Request bounds of the table.
    let update = schema(&openapi, "DeclaredUpdateRequest");
    assert_eq!(update["properties"]["waitMs"]["minimum"], 0);
    assert_eq!(update["properties"]["waitMs"]["maximum"], 60_000);
    assert_eq!(update["properties"]["acceptPending"]["type"], "boolean");
    let admission = schema(&openapi, "CodeUpdateAdmission");
    assert_eq!(
        strings(
            &admission["properties"]["status"]["enum"],
            "CodeUpdateAdmission.status"
        ),
        set(&["ADMITTED", "ACCEPTED"])
    );
    let query = schema(&openapi, "WorkerQueryRequest");
    assert_eq!(query["properties"]["timeoutMs"]["minimum"], 1);
    assert_eq!(query["properties"]["timeoutMs"]["maximum"], 30_000);
    assert_eq!(
        strings(
            &schema(&openapi, "WorkerQueryResponse")["required"],
            "WorkerQueryResponse.required"
        ),
        set(&["answer", "committedThroughSeq"])
    );
    let do_move = schema(&openapi, "MoveCodeVersionRequest");
    assert_eq!(do_move["properties"]["toVersion"]["minimum"], 1);
    assert_eq!(do_move["properties"]["dryRun"]["default"], false);
    assert_eq!(do_move["properties"]["reason"]["minLength"], 8);
    assert_eq!(do_move["properties"]["reason"]["maxLength"], 2000);
    let cancel = &schema(&openapi, "OperationRequest")["properties"]["graceMs"];
    assert_eq!(cancel["minimum"], 0, "graceMs: only 0 is accepted");
    assert_eq!(cancel["maximum"], 0, "graceMs: only 0 is accepted");
    for binding in ["SetWorkerBindingRequest", "WorkerBinding"] {
        let types = &schema(&openapi, binding)["properties"]["workflowTypes"];
        assert_eq!(types["maxItems"], 64, "{binding}.workflowTypes");
        assert_eq!(
            types["items"]["pattern"], "^[A-Za-z][A-Za-z0-9_-]{0,47}$",
            "{binding}.workflowTypes"
        );
    }
    assert_eq!(
        schema(&openapi, "SetWorkerBindingRequest")["properties"]["activities"]["minItems"],
        0,
        "activities may be empty when workflowTypes is not"
    );
    let version = schema(&openapi, "CodeVersionSummary");
    assert_eq!(
        strings(&version["required"], "CodeVersionSummary.required"),
        set(&[
            "versionNumber",
            "workflowType",
            "codeDigest",
            "openRuns",
            "servingSessions",
            "oldestOpenRunAgeSeconds",
        ])
    );

    // The specific codes keep the closed `Error.code` set: none is a value of it, each is a documented reason, and the
    // gateway timeout of a worker query is the generic `UNAVAILABLE`.
    let error = schema(&openapi, "Error");
    let codes = strings(&error["properties"]["code"]["enum"], "Error.code");
    let reasons = error["properties"]["details"]["description"]
        .as_str()
        .expect("Error.details describes its reasons");
    for code in SPECIFIC_CODES {
        assert!(
            !codes.contains(*code),
            "{code} must stay a details.reason, not an Error.code"
        );
        assert!(
            reasons.contains(&format!("`{code}`")),
            "Error.details names {code}"
        );
    }
    let timeout = &openapi["components"]["responses"]["GatewayTimeout"];
    let timeout_text = timeout["description"]
        .as_str()
        .expect("GatewayTimeout describes itself");
    assert!(timeout_text.contains("`UNAVAILABLE`") && timeout_text.contains("`QUERY_TIMEOUT`"));
    assert_eq!(
        openapi["paths"]["/runs/{runId}/worker-queries/{queryName}"]["post"]["responses"]["504"]["$ref"],
        "#/components/responses/GatewayTimeout"
    );
}

#[test]
fn an_old_client_reads_a_code_run_without_new_enum_values() {
    let openapi = openapi();
    let run = schema(&openapi, "Run");

    // What a client of 0.2 requires of a run is what it still gets; the code run's view is one optional object.
    assert_eq!(
        strings(&run["required"], "Run.required"),
        set(&[
            "runId",
            "workflowId",
            "versionId",
            "status",
            "version",
            "createdAt"
        ])
    );
    let code = &run["properties"]["codeWorkflow"];
    assert_eq!(code["type"], "object", "codeWorkflow is an object");
    assert_eq!(code["additionalProperties"], false);
    assert_eq!(
        strings(&code["required"], "codeWorkflow.required"),
        set(&["state", "cause", "currentVersion"])
    );
    assert_eq!(
        code["properties"]["state"]["$ref"],
        "#/components/schemas/CodeWorkflowTaskState"
    );
    assert_eq!(
        strings(
            &schema(&openapi, "CodeWorkflowTaskState")["enum"],
            "CodeWorkflowTaskState"
        ),
        set(&["READY", "LEASED", "WAITING", "CLOSED", "UNDER_REVIEW"])
    );
    assert_eq!(code["properties"]["currentVersion"]["minimum"], 1);
    assert!(
        code["properties"]["cause"]["enum"].is_null(),
        "the review cause is open text, so a new cause never breaks a client"
    );

    // The enumerations getRun and listResetPoints already answered gain no value: the workflow-task step is not a
    // RuntimeNode, and a code run's reset point is STEP_COMPLETED with the node `~workflow`.
    assert_eq!(
        strings(&run["properties"]["status"]["enum"], "Run.status"),
        set(&[
            "RUNNING",
            "WAITING",
            "PAUSED",
            "CANCELLING",
            "COMPENSATING",
            "MANUAL_REVIEW",
            "SUCCEEDED",
            "FAILED",
            "CANCELLED",
            "TIMED_OUT",
            "COMPENSATED",
            "PARTIAL",
            "COMPENSATION_FAILED",
        ])
    );
    let node = schema(&openapi, "RuntimeNode");
    assert_eq!(
        strings(
            &node["properties"]["waitingOn"]["enum"],
            "RuntimeNode.waitingOn"
        ),
        set(&[
            "timer",
            "event",
            "branches",
            "child",
            "saga",
            "reconciliation",
            "operator",
            "loop",
            "approval",
            "collection",
        ])
    );
    assert_eq!(
        strings(&node["properties"]["status"]["enum"], "RuntimeNode.status"),
        set(&[
            "NOT_STARTED",
            "PENDING",
            "READY",
            "LEASED",
            "RUNNING",
            "WAITING",
            "RETRY_WAIT",
            "SUCCEEDED",
            "FAILED",
            "CANCELLED",
            "SKIPPED",
        ])
    );
    assert_eq!(
        strings(
            &schema(&openapi, "ResetPoint")["properties"]["kind"]["enum"],
            "ResetPoint.kind"
        ),
        set(&[
            "RUN_STARTED",
            "STEP_COMPLETED",
            "CHILD_FINISHED",
            "COLLECTION_FINISHED",
            "LOOP_FINISHED",
        ])
    );

    // The request fields design 044 adds to existing operations are optional, so an old request stays valid.
    for (name, field) in [
        ("DeclaredUpdateRequest", "waitMs"),
        ("DeclaredUpdateRequest", "acceptPending"),
        ("OperationRequest", "graceMs"),
        ("SetWorkerBindingRequest", "workflowTypes"),
        ("BatchCreateRequest", "moveCodeVersion"),
    ] {
        let request = schema(&openapi, name);
        assert!(
            request["properties"][field].is_object(),
            "{name}.{field} exists"
        );
        let required = request["required"]
            .as_array()
            .is_some_and(|list| list.iter().any(|item| item == field));
        assert!(!required, "{name}.{field} is optional");
    }
    // A graph run's update answer is still the receipt an old client reads.
    let answers = &openapi["paths"]["/runs/{runId}/updates/{operationName}"]["post"]["responses"]["200"]
        ["content"]["application/json"]["schema"]["oneOf"];
    assert!(
        answers
            .as_array()
            .expect("the update answer is one of two shapes")
            .iter()
            .any(|shape| shape["$ref"] == "#/components/schemas/DeclaredUpdateResponse"),
        "a graph run's update still answers DeclaredUpdateResponse"
    );
}

#[test]
fn batch_item_has_first_difference_and_the_new_reasons() {
    let openapi = openapi();

    // The action grows only.
    let actions = strings(&schema(&openapi, "BatchAction")["enum"], "BatchAction");
    for action in [
        "CANCEL",
        "SIGNAL",
        "UPDATE",
        "RERUN",
        "RETRY_STEP",
        "PAUSE",
        "RESUME",
        "MARK_MANUAL_REVIEW",
        "REPIN",
        "MOVE_CODE_VERSION",
    ] {
        assert!(actions.contains(action), "BatchAction has {action}");
    }
    let parameters = &schema(&openapi, "BatchCreateRequest")["properties"]["moveCodeVersion"];
    assert_eq!(parameters["additionalProperties"], false);
    assert_eq!(
        strings(&parameters["required"], "moveCodeVersion.required"),
        set(&["toVersion"])
    );
    assert_eq!(parameters["properties"]["toVersion"]["minimum"], 1);
    assert_eq!(parameters["properties"]["dryRun"]["type"], "boolean");
    assert_eq!(parameters["properties"]["dryRun"]["default"], false);

    // The item: an optional first difference, and the closed reason lists grown, the old values kept.
    let item = schema(&openapi, "BatchItem");
    let difference = &item["properties"]["firstDifference"];
    assert!(
        strings(&difference["type"], "BatchItem.firstDifference.type").contains("integer"),
        "firstDifference is an integer"
    );
    assert_eq!(difference["minimum"], 1, "a command number starts at 1");
    assert!(
        !strings(&item["required"], "BatchItem.required").contains("firstDifference"),
        "firstDifference is optional"
    );
    let reason = item["properties"]["reason"]["description"]
        .as_str()
        .expect("BatchItem.reason lists its codes");
    for code in [
        // Feature 018's, kept.
        "ALREADY_TERMINAL",
        "NOT_APPLICABLE",
        "NOT_WAITING",
        "RUN_CONTINUED",
        "HIDDEN_FROM_CREATOR",
        "QUOTA_EXCEEDED",
        "RATE_LIMITED",
        "CONCURRENCY_QUEUE_FULL",
        "LOCK_WAIT",
        // Design 044's skip reasons, success reason and defer causes.
        "REPLAY_MISMATCH",
        "ENVELOPE_MISMATCH",
        "RUN_NOT_MUTABLE",
        "MATCHED",
        "REPLAY_PENDING",
        "RUN_BUSY",
    ] {
        assert!(reason.contains(code), "BatchItem.reason names {code}");
    }
    let error = item["properties"]["errorCode"]["description"]
        .as_str()
        .expect("BatchItem.errorCode lists its codes");
    for code in [
        "RUN_BUSY",
        "NO_TARGET_WORKER",
        "FORBIDDEN",
        "REPLAY_CHECK_ABANDONED",
    ] {
        assert!(error.contains(code), "BatchItem.errorCode names {code}");
    }
    // The item codes are not Error.code values; the move's own failures stay batch item codes.
    let codes = strings(
        &schema(&openapi, "Error")["properties"]["code"]["enum"],
        "Error.code",
    );
    for code in ["RUN_BUSY", "NO_TARGET_WORKER", "REPLAY_CHECK_ABANDONED"] {
        assert!(!codes.contains(code), "{code} is not an Error.code");
    }
}
