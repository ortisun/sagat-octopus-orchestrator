# egm-review Stage 9 codebase-details lens, round 3 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 2249e90100039f5c012b6217e4e5bb454ace1296
Reviewer: egm-codebase-details, implementation-readiness standard, DetailedRange 1526-1826 and every code claim
Note: condensed by the author from the reviewer's output; every finding and severity is kept.

1. Blocker: `steps.kind` allows only `FORWARD`, `COMPENSATION`, `RECONCILIATION` and a check ties the kind to the node's
   suffix, so `~workflow` cannot be stored; `is_derived`, `forward` and `parse_stored` mishandle it; the claim skips
   `FORWARD` steps once a stop reason is set, so a cancelled code run could not lease its workflow task. Add a kind,
   rewrite the constraints, change the domain functions, state the claim's treatment.
2. Blocker: `operation_update_receipts` is immutable (an `ENABLE ALWAYS` trigger, `SELECT, INSERT` only), so a receipt
   cannot move through the lifecycle; choose a model.
3. Major: `security_workflow_as_code` breaks the test layout; no `[[test]]` entries or paths; red tests without targets.
4. Major: no file actions per task.
5. Major: `worker_bindings` is one row per pattern, so a `workflow_types` column copies per row and a workflow-only
   identity has no row; `SetWorkerBindingRequest.activities` has `minItems: 1`; use a side table like
   `worker_binding_pools`.
6. Major: capabilities carry a version suffix and live in an enum and three closed checks; name `code-workflows-v1`, its
   kinds, and change them; register `CAPABILITY_NOT_ACTIVE`.
7. Major: `infra-ir`'s `canonical_json` is crate-private and not RFC 8785 (UTF-8 key order, serde number printing);
   add a separate public function with golden vectors; `infra-ir` already hashes with `sha2`.
8. Major: the audit misses `WorkflowDefinition::node`, `entrypoint` and `scope_body`; `ensure_activity_bound` returns
   `Ok(())` for a missing node and would skip the binding check; name the type that carries the overlay.
9. Major: `required_worker_protocol_minor` is a per-run `RunTx` method with two implementations; the candidate scan has
   minor gates in its SQL and applies activity quotas, pools and capacity windows to every activity name; decide the
   reserved names' treatment.
10. Major: the code-profile table names fields that do not exist (`delayMs`, `until`, `artifact`, approval `policy` by
    name, `subject`, activity `input`, child `businessId`) and omits required ones; rewrite against the schema; say how
    the lease input of a `c-<n>` step is built.
11. Major: the example document fails the schema (`state.schema` must be a closed schema, `initial` an expression,
    updates require `argsSchema`, `result`, `resultSchema`, `disclosure`); specify the code variants.
12. Major: route parameter names differ from today's; `GET /runs/{runId}/updates/{updateKey}` clashes with the existing
    template; chain-addressed updates not covered.
13. Major: the generated reference docs, the cross-tenant battery and `x-planned` markers are not in the tasks.
14. Major: telemetry has no task (metric enums in `crates/telemetry/src/metrics.rs`, `AuditAction`).
15. Major: the inbox of Architecture decision 0111 does not exist at this commit; Task 044-21 must depend on its builder
    or specify it.
16. Major: the move function would write the journal in SQL, duplicating the Rust append path; limit it to the pin.
17. Major: replay-check rows keyed by `job_id` and `item_id` do not fit `batch_items` (`ordinal`) or a single-run move;
    verdicts in deletable rows; reuse the item retry columns.
18. Major: `publishCode` is a new SDK surface; publication today publishes and activates in one call; specify the route.
19. Major: protocol minors are cumulative; an ordinary SDK on the new minor would imply 1.6 to 1.9; decide.
20. Major: the pause setting has no table or column; `steps.input_digest` has no purpose stated.
21. Major: the saga command lacks the saga's `cancelPolicy`.
22. Major: the conformance harness and runner changes are not planned; relation of "not offered" to `.json.pending`.
23. Nit: the candidate scan is in `crates/infra-postgres/src/workers.rs`.
24. Nit: name the lease-expiry paths and functions.
25. Nit: `AppendStepLogs` is not in `worker.proto` yet.
26. Nit: `metadata.name` and `workflowType` patterns differ; forbid `bodies` in a code document.
27. Nit: the error catalog is `sdks/error-catalog.json` and its six copies.
28. Nit: the runbook is `docs/runbooks/instance-registry.md`; no Rust quickstart exists.
29. Nit: `REPIN` against `MOVE_CODE_VERSION`.
30. Nit: storage classification for each new variable-size column.
31. Nit: reset rules that differ for code runs in the per-kind table.

Counts: Blocker 2, Major 20, Nit 9
