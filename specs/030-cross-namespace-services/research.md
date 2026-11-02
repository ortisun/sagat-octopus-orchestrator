# Feature 030 — Baseline, sources, decisions, gaps and owner questions

Baseline investigation: 2026-10-26, on branch `feature/018-032-parity-specs` cut from `origin/develop`
at `70f09435` (release 0.2.0). Nothing of this feature is implemented. Identifier ranges allocated to
this feature: D1480–D1519, G1480–G1519, O1480–O1519; ADR-0065.

## Repository evidence (verified paths)

| Observation | Where | Consequence for 030 |
|---|---|---|
| The `child` node starts a workflow "of the same namespace", resolved in the parent's namespace | `crates/domain/src/workflow.rs` (`NodeKind::Child`), `specs/001-platform-foundation/contracts/workflow.schema.json` (`$defs.child`), D36 in `specs/002-durable-execution/research.md` | No cross-namespace start exists; the child protocol (reserve, create under the parent's lock, flags written by the parent) is the precedent but not reusable as is across namespaces |
| A child is created by the scheduler under the **parent's** shard; "a child that ended is reported to its parent, so its shard is the parent's" | `crates/scheduler/src/passes.rs` (`start_child`), ADR-0020 §1 | Notifications to the caller are processed under the caller run's shard; admission under the handler run's shard (D1501) |
| No transaction holds two run locks; the parent writes child flags under its own lock | `specs/002-durable-execution/data-model.md` (lock order), ADR-0030 §18–19, ADR-0037 | Two-sided records with messages (D1482, ADR-0065) |
| Memberships are tenant-wide: one role per subject per tenant | `crates/infra-postgres/migrations/20261008000000_identity.up.sql` (`memberships` PK `(tenant_id, subject)`), `crates/domain/src/identity.rs` (`Principal`) | Roles cannot separate namespaces; the endpoint allow-list of namespaces is the crossing authority (D1486) |
| The permission catalog has 48 entries and seven built-in roles | `crates/domain/src/identity.rs` (`Permission::ALL`, `Role::permissions`), ADR-0036 §5 | Four additive permissions; roles unchanged (D1499) |
| Errors use a closed `Error.code` of ten values refined by `details.reason`; step failures use `FailureCode` strings such as `CHILD_FAILED`, `CHILD_QUOTA_EXCEEDED` | `specs/001-platform-foundation/contracts/openapi.yaml` (`Error`), `sdks/error-catalog.json`, `crates/domain/src/execution/event.rs` | New reasons and `OPERATION_*` step codes only (D1506) |
| Declared Query/Update results use a closed `resultSchema` and `disclosure: declared-business-result-v1`; classification annotations are refused there | `workflow.schema.json` (`$defs.query`, `$defs.resultSchema`), ADR-0031, FR-016-016 | Service results need their own declaration that *is* a classification carrier (D1489) |
| Classification carriers are a closed set (workflow input/output, `outputSchema` of activity, script, waitEvent, child) | FR-016-002 in `specs/016-payload-classification/spec.md` | The `serviceCall` node must be added as a carrier, which is 016's to accept (G1493) |
| A run result is stored at `end` and handed to a parent as `steps.<child>.output` | G21 in `specs/002-durable-execution/research.md`, `runs.result_digest` | The handler run result is the operation result |
| Payloads are content-addressed per tenant (`payloads` PK `(tenant_id, digest)`, ≤ 256 KiB) | `crates/infra-postgres/migrations/20261009000100_execution.up.sql` | Input and result are shared by digest inside the tenant, charged once (ADR-0027) |
| Durable rate buckets exist for `api_requests_per_second` and `events_per_second`, with a source kind | `quota_rate_buckets` in `crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql` | An additive resource `service_starts_per_second` with source kind `SERVICE_ENDPOINT` (D1500) |
| "Endpoint" already names webhook endpoints (`/namespaces/{namespaceId}/event-endpoints`) and an audit resource type for API routes | `openapi.yaml`, `crates/domain/src/audit.rs` (`ResourceType::Endpoint`) | Use `service-endpoints` in paths and `ServiceEndpoint` in types (D1498) |
| `/runs/{runId}/operations` (`operateRun`, `OperationRequest`) and "declared operations" (Query/Update) already use the word "operation" | `openapi.yaml`, `crates/application/src/declared_operations.rs` | Prefix every new type and route with `service` (D1498) |
| A compensation circuit breaker exists per tenant, namespace and activity | `crates/infra-postgres/src/breaker.rs` (FR-003-003) | Not reused: start refusals here are local admission decisions, not remote faults (D1493) |
| Internal NATS hints are AsyncAPI channels such as `sagat.v1.chain.finished.{tenant}` | `specs/001-platform-foundation/contracts/events.asyncapi.yaml`, `docs/event-schema-evolution.md` | Two additive hint channels; messages only grow |
| The SDK builder surface is in one shared spec and five builders | `sdks/SPEC.md` §7, `sdks/typescript/src/workflow.ts`, `sdks/go/workflow/workflow.go`, `sdks/python/sagat_octopus_orchestrator/workflow.py`, `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/Flow.cs`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/WorkflowBuilder.java` | `.callService(...)` and a `service(...)` contract builder in all five |
| One conformance suite with scenario and golden files | `tests/contract/sdk-conformance/README.md`, `scenarios/`, `expected/` | New `service-*` scenarios (contracts/conformance.md) |
| Managed database nodes are root-only until per-instance pins are proven | ADR-0036 §3 | Same restriction for `serviceCall` in the first increment (D1496) |
| Unknown outcomes already have a vocabulary: failure code `OUTCOME_UNKNOWN`, `MARKED_FOR_REVIEW`, journal `step.needs_review`, operator `RETRY_STEP` and `RESOLVE_STEP` (`saga:override`) | `crates/domain/src/execution/event.rs`, `operateRun` in `openapi.yaml` | Effect-possible call outcomes reuse manual review and these actions (D1512) |

## External semantics consulted (2026-10-26)

- Temporal Nexus overview, <https://docs.temporal.io/nexus>: a service is a named collection of
  operations; async operations start a workflow and return an operation token; sync operations must
  finish within a **10-second handler deadline**; an endpoint is a reverse proxy routing to a target
  namespace and task queue, managed in a registry; async schedule-to-close maximum **60 days** in
  Temporal Cloud; runtime access is controlled by an **endpoint allowlist of caller namespaces**.
- Nexus operations, <https://docs.temporal.io/nexus/operations>: lifecycle Scheduled → Started →
  Completed/Failed/Canceled/TimedOut; sync operations have no Started event; completion callbacks are
  attached to the handler workflow and delivered to the caller; cancelling the caller propagates to
  pending operations; schedule-to-close, schedule-to-start and start-to-close timeouts; at-least-once
  handler execution with retries; a per (caller namespace, endpoint) circuit breaker opening after 5
  consecutive retryable errors for 60 s.
- Nexus security, <https://docs.temporal.io/nexus/security>: the caller namespace is verified against
  the allowlist before routing; endpoints are reachable only inside one account; payload codecs apply.
- Nexus execution debugging, <https://docs.temporal.io/nexus/execution-debugging>: bi-directional links
  between caller and handler histories; pending operations and callbacks visible; tracing via
  interceptors.
- Cancellation types, <https://python.temporal.io/temporalio.workflow.NexusOperationCancellationType.html>:
  `ABANDON`, `TRY_CANCEL`, `WAIT_REQUESTED`, `WAIT_COMPLETED`.
- Windmill was not consulted for this feature: no equivalent of cross-workspace service contracts was
  identified, and none is claimed.

Mapping: Nexus endpoint → service endpoint; handler task queue → handler workflow binding; operation
token → derived operation id; completion callback → `COMPLETED` message; `ABANDON` → `abandon`;
`TRY_CANCEL` → `request_cancel`; `WAIT_COMPLETED` → `wait_cancelled`; `WAIT_REQUESTED` → not offered
(G1494); schedule-to-close → `timeoutMs`; schedule-to-start → `startTimeoutMs`; start-to-close → not
separate (the handler run's own timeout); circuit breaker → deferral without attempts (D1493).

## Decisions

- **D1480 — One tenant.** Calls never cross tenants; the schema makes it unrepresentable
  (ADR-0065 §1). A Nexus-like cross-account path is a different product decision.
- **D1481 — Every handler is a workflow run.** Both modes are backed by a handler run of the handler
  namespace. `sync` is a contract class: pinned flat handler, timeout ≤ 60 s (default 10 s, after
  Nexus), fixed `request_cancel`. Activity, Query/Update or signal handlers would need 022 standalone
  steps or 018 business ids (O1480, G1483).
- **D1482 — Two-sided records and durable messages.** Caller-side and handler-side records each change
  only under their own run; messages are the shared rows (ADR-0065 §2–3).
- **D1483 — Derived ids.** Operation id from (tenant, caller run, node instance, call attempt); handler
  run id from the operation id; uniqueness enforced by primary keys. The call attempt (D1513) lets an
  operator retry start a new operation without colliding with the first.
- **D1484 — Pin the contract at publication, resolve routing at admission.** The caller's downstream
  expressions are typed by the contract; routing may change under the binding policy. An endpoint
  serves at most four revisions so that a handler team can publish a new revision while callers move.
- **D1485 — Schema compatibility is canonical equality.** The operation's input and output schemas must
  equal the handler version's (classification annotations included). Equality is checkable and makes
  "no looser classification across the boundary" trivially true; subtyping is G1485.
- **D1486 — Explicit allow-list.** Namespace ids of the same tenant, no wildcard, empty denies, checked
  at publication, admission and completion (ADR-0065 §6). Same-namespace calls need the namespace
  listed (D1507).
- **D1487 — Fail closed after revocation.** Removing a caller namespace or deleting the endpoint makes
  completions of operations in flight carry `OPERATION_ACCESS_REVOKED` and no value; pausing does not.
  The handler's effect class still crosses (it is metadata, not a value), so an `applied` effect goes to
  compensation or manual review (D1512). Recommended default of O1481.
- **D1488 — Authority.** The caller version's publisher must hold `service:call` at publication and
  when the call is scheduled (current membership, read under a share lock as in feature 017). The run
  initiator's `run:start` already authorizes running the published calls and is not rechecked. The
  handler run acts with its own version publisher's authority and the system actor
  `system:service-operation`, as event-started runs do; it gains nothing from the caller.
- **D1489 — Declared, classified result.** `disclosure: "declared-service-result-v1"` on every
  operation; the output schema is closed and bounded (ADR-0031 §4) and is a classification carrier,
  unlike a Query/Update result; `secret` is refused in it.
- **D1490 — Failure codes.** A declared code crosses verbatim as the caller step's failure code;
  undeclared codes become `OPERATION_FAILED`; `OPERATION_*`, `CHILD_*` and platform codes cannot be
  declared.
- **D1491 — Cancellation policies.** `abandon` (like the child `detach`), `request_cancel` (default, like
  the child `propagate`), `wait_cancelled` (like the child `request_cancel`). A pending `START` is
  withdrawn whatever the policy, so no orphan handler run starts after a cancellation. The IR spells
  the values in snake_case, as the child and saga policies do.
- **D1492 — Deadlines.** Optional `timeoutMs` (the operation's `defaultMs`, itself 10 s for sync and
  24 h for async when the contract omits it); optional `startTimeoutMs`; ceilings 60 s (sync) and
  60 days (async); the handler run's deadline equals the operation deadline; database clock.
- **D1493 — Deferral, no circuit breaker.** Refusals for room (paused endpoint, quotas, rate,
  concurrency) defer without consuming attempts, as ADR-0012 does for events waiting for room, with a
  1 s doubling backoff capped at 60 s; non-retryable refusals end the call. A breaker protects remote
  providers; here every refusal is a local, already-known database decision.
- **D1494 — Dead notifications.** Twelve failed applications (the ADR-0025 attempt ceiling) make a
  notification or `CANCEL` `DEAD`, visible, alerted and redrivable. A `START` never goes dead: the
  caller's deadline withdraws it.
- **D1495 — Call depth.** A `service_call_depth` on runs, inherited by children; default 4, hard 8,
  mirroring the child depth limit of D37 (feature 002), because handler runs start new trees and would
  otherwise escape the 1,024-run tree budget.
- **D1496 — First-increment shape.** Root-path `serviceCall` only (`UNSUPPORTED_NODE` with dependency
  `SCOPED_SERVICE_CALL` inside bodies); handler versions with `continueAsNew` are refused for binding;
  a caller run with a call in flight cannot continue (ADR-0030 §16 quiescence).
- **D1497 — Trace.** Continue the caller's trace (as a child run does, ADR-0017 §3) and add reciprocal
  OTel links between the computed service-call span and the handler run span; resolving a link in the
  Studio needs the viewer's own authorization in the other namespace.
- **D1498 — Names.** `service`, `service endpoint`, `service call`, `operation execution`, `operation
  message`; routes under `service-endpoints`, `service-operations`; schema and type names prefixed
  `Service`.
- **D1499 — Permissions.** `service:read` (admin, developer, operator, viewer), `service:publish` (admin,
  developer), `service:endpoint:manage` (admin), `service:call` (admin, developer); catalog 48 → 52;
  roles stay seven. Endpoint and allow-list changes open a path into a namespace, so they are admin-only
  like database catalog management in feature 017.
- **D1500 — Quotas reuse 009/ADR-0027 machinery.** New count resources in `quota_limits` and one rate
  resource in `quota_rate_buckets`; operator ceilings bound tenant and namespace settings.
- **D1501 — Shards.** `START` and `CANCEL` are processed under the shard of the handler run id;
  `STARTED`, `START_REFUSED` and `COMPLETED` under the caller run's shard (ADR-0020 §1, §3).
- **D1502 — Retention.** Settled messages are purged 7 days after settlement (configurable up to 30);
  records follow the retention of their runs; tombstones for services and endpoints.
- **D1503 — Paused caller.** A completion is applied to a paused caller run (the step settles, its
  output is retained) but no next work is scheduled until resume, following existing pause semantics.
- **D1504 — Binding policy and activation.** `active` bindings are rechecked at every admission; an
  incompatible active version refuses new starts with `OPERATION_HANDLER_UNAVAILABLE` and the endpoint
  reports the binding as incompatible. Feature 011's activation path is not changed, so no lock edge is
  added between activation and endpoints. Production endpoints are advised to pin.
- **D1505 — Compensation (P3).** `compensateWith: {operation, input}` on the same pinned contract; the
  compensation is a derived `serviceCall` under ADR-0010; `wait_cancelled` is required inside a saga so
  that a cancellation learns the handler outcome before the saga plans. It does not cover deadlines,
  invalid results or revocations: those settle with an effect class, and an effect-possible class is
  compensated (D1512). (Corrected after the 2026-10-27 review, which found the earlier wording claimed
  more than `wait_cancelled` gives.)
- **D1506 — Errors.** No new `Error.code`; new `details.reason` values and `OPERATION_*` step failure
  codes ([contracts/errors.md](contracts/errors.md)).
- **D1507 — Same-namespace calls.** Allowed when the namespace allow-lists itself; useful for
  decoupling teams that share an environment.
- **D1508 — No worker protocol change.** The caller node is executed by the engine; the handler is an
  ordinary workflow. No protobuf minor is allocated; SDK workers are unchanged.
- **D1509 — Contract document.** A separate `kind: Service` document under the same `apiVersion`, with
  its own canonical schema file; publication is idempotent by digest.
- **D1510 — Input is explicit.** `input` is required on `serviceCall` (a child defaults to the parent's
  input; crossing a boundary should not).

Decisions added after the independent review of 2026-10-27 and the coordinator decisions C1, C4, C6,
C7, C8, C10 and C13:

- **D1511 — Notifications in any order.** A `COMPLETED` can be applied before its `STARTED` (same pass,
  concurrent candidates, backoff or a dead `STARTED`). CP6 therefore applies from `SCHEDULED`, deriving
  the handler run id, and a later or redriven `STARTED` is a no-op `APPLIED`. Only `COMPLETED` and
  `START_REFUSED` can be `DISCARDED`. Fixes review blocking finding 1.
- **D1512 — Effect classes (constitution III).** Every settlement records `none`, `failed`, `applied`,
  `partial` or `unknown`. A caller deadline after admission first looks the outcome up: it sends
  `CANCEL` and waits at most the outcome lookup window (default 60 s, hard 15 min; the handler run's
  deadline is the same instant, so its outcome normally follows at once). An effect-possible failure is
  compensated in a saga or enters the existing manual review outside one; it never follows `onError`.
  A discarded late `COMPLETED` writes its code and class once to the call record for lookup; the record's
  state stays terminal. Fixes review should-fix 2.
- **D1513 — Call attempts.** `RETRY_STEP` on a call in manual review starts call attempt n+1 with a new
  operation id; the earlier operation keeps its records and outcome. Without it a retry would collide
  with the first operation's unique handler run id.
- **D1514 — Input disclosure.** `inputDisclosure: "declared-service-input-v1"` is required on the node;
  for a direct path input, publication compares each operation input field's level with its caller
  source and refuses a looser one (`INPUT_CLASSIFICATION_LOOSENED`); a source outside any carrier counts
  as `pii`; a computed expression is not compared (G1496). Fixes review should-fix 3.
- **D1515 — Feature 031 interplay (C7).** Registered in 031's encoded-field registry with 031's codes
  only. Superseded in substance by D1661 after 031's final text: nothing encoded crosses.
- **D1661 — Encoded values are refused, not carried (031 D1556, D1667).** A v1 envelope binds tenant,
  namespace and location in its authenticated `ctx`, so bytes made in the caller namespace cannot be
  valid in the handler namespace, and no platform component may re-encode. The operation schemas refuse
  `x-sagat-encoding` (`ENCODING_NOT_APPLICABLE`); no handler version declaring encoding can be bound;
  `serviceCall.input`/`compensateWith.input` from an encoded source are refused at publication
  (`ENCODING_CARRIAGE_MISMATCH`); an envelope at an undeclared location is refused at admission and
  withheld from a result at runtime (`ENCODING_CARRIAGE_MISMATCH` or `ENVELOPE_CONTEXT_MISMATCH`; a
  withheld result has effect class `applied`). This replaces the carriage design of the first review
  round. Fixes review should-fix 4 and aligns with 031's registry rows for 030.
- **D1516 — Reset rules (C1).** `service_call.*` of finished calls `COPY` (operation id kept as
  provenance, no side rows re-created because the reset run never calls), of open calls `REFUSE`;
  `service_operation.cancel_requested` and `.completed` `EXCLUDE`; the handler `run.started` cause is
  copied as provenance and the reset run backs no operation (FR-030-018). 018's eligibility rule (no
  waiting step) already excludes an open call; `REFUSE` makes it explicit.
- **D1517 — Lock-order slots (C4).** Aligned on 2026-10-27 to ADR-0050 decision 7 as written by 018:
  handler admission takes its shard fence and the `START` row in slot 3, the handler version's
  namespace fences and memberships in 4a, the handler version's pins and then the service-endpoint
  head and allow-list entries at the end of 4b (binding resolved from an unlocked read and rechecked
  under the locks), secrets in 5, the new run and its rows in 6, usage in 8b. Caller publication takes
  memberships (4a) before the endpoint head (4b). Transitions start at 3R. The earlier text put the
  endpoint head before memberships and the `START` row in slot 2; both are corrected in the plan and
  ADR-0065 decision 3.
- **D1660 — One exception to decision 7.** The caller's deadline or cancellation (CP7) locks its own
  existing `START` row (slot 3) after its run (3R). Safe because the `START` row's order-(a)
  transaction (admission) never locks an existing run; listed for ADR-0050 decision 7 / 018's G1605.
- **D1518 — Registries of 020 and 028 (C8, C13).** Operation messages and call records are a content
  store in 020's registry (`crates/infra-postgres/run-content-references.json`, purge 7 days after
  settlement, erasure hook below in data-model.md); the payload digests in messages get foreign keys
  to `payloads`. Call deadlines, start deadlines and history instants are workflow time; delivery
  backoff, settlement and purge instants are infrastructure time in `crates/infra-postgres/time-classification.json`.
- **D1519 — Amendments of 016 and 029 (C6, C10).** The FR-016-002 amendment is stated in the spec's
  "Amendment to feature 016" subsection and joins the single consolidated amendment; the four
  permissions are additive grants that 029 lists, and 029's evaluator decides visibility when enabled.

## Gaps

| Id | Gap | Status |
|---|---|---|
| G1480 | Handler workflows that use `continueAsNew` are refused; following a handler chain to its FINAL segment (the ADR-0030 §19 parent-link analogue) needs its own design and tests | Open |
| G1481 | `serviceCall` inside `forEach`, `while`, `repeatUntil` bodies needs scoped operation ids and per-instance pins | Open |
| G1482 | Attaching several callers to one handler run (Nexus Use-Existing) depends on 018 business workflow id policies | Open, depends on 018 |
| G1483 | Sync handler kinds other than workflows (activity, Query/Update, signal) depend on 022 and 018 | Open, O1480 |
| G1484 | Calls between namespaces placed in different regions (032) are out of scope; registration must refuse them once placement exists | Open, depends on 032 |
| G1485 | No compatible evolution of schemas (subtyping); a new schema means a new revision and republished callers | Accepted for the first increment |
| G1486 | Contract and endpoint changes are not routed through publication approval in protected namespaces | Open, O1482 |
| G1487 | Handler expressions cannot read operation metadata (`operation.*`); the deterministic handler run id gives stable activity keys | Accepted |
| G1488 | Per-service or per-endpoint ACLs and folders belong to 029 | Open, depends on 029 |
| G1489 | Services and endpoints are not part of 011 GitOps manifests | Open |
| G1490 | The 028 time-skipping test environment must cover service calls when it exists | Open, depends on 028 |
| G1491 | No public-API (non-workflow) callers and no HTTP callback transport between cells | Accepted (out of scope) |
| G1492 | All latency and throughput figures are unmeasured; only ceilings are specified | Open (SC-030-014) |
| G1493 | The `serviceCall` node is not yet in 016's closed carrier set; until 016 accepts it, results are `structure-only` for disclosure | Open, needs 016 owner |
| G1494 | The `WAIT_REQUESTED` cancellation type is not offered | Accepted |
| G1495 | Blob descriptors cannot cross namespaces; inputs and results are inline ≤ 256 KiB | Accepted for the first increment |
| G1496 | A computed `serviceCall.input` expression is not compared field by field with its sources (no taint tracking, D283 of 016); the publisher's `inputDisclosure` is the only control. Owner gap: revisit with 016 rules v2 | Open, owner gap |
| G1497 | Encoded values cannot cross namespaces in v1 (D1661, 031 D1667): a team that encrypts a field cannot pass it to a service operation. Needs 031's delegation design (customer key policy for a pair of namespaces plus re-encoding outside the platform), 031 G1536 | Open, depends on 031 |

## Owner questions

- **O1480 — Handler kinds for sync operations.** Options: (a) short flat handler workflows only, now
  (recommended default: one durable path, already testable, no dependency); (b) wait for 022
  standalone steps and offer activity-backed sync handlers; (c) also offer declared Query/Update of a
  run addressed by business id after 018. The specification applies (a).
- **O1481 — In-flight results after revocation.** When a caller namespace is removed from the
  allow-list or the endpoint is deleted while operations run: (a) fail closed — deliver
  `OPERATION_ACCESS_REVOKED` and no value (recommended default, constitution V); (b) deliver results of
  operations admitted before the change. Pause never affects operations in flight. The specification
  applies (a).
- **O1482 — Approval for contract and endpoint changes.** In namespaces whose publication policy asks
  for approvals (FR-011-003): (a) not in the first increment; changes are admin-only and audited
  (recommended default, G1486); (b) route contract revisions and endpoint revisions through the
  publication-request flow. The specification applies (a).
