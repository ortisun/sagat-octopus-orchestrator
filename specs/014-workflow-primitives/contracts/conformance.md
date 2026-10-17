# Conformance and failure matrix

**Status**: Required runtime tests for the accepted architecture. The US01 example suite exists separately and has its own
[validation report](../validation-report.md). None of the pending cases below is claimed to pass.

## Shared five-SDK contract

Each new scenario is added once under `tests/contract/sdk-conformance/scenarios/`, with reviewed
canonical definitions and normalized observations under `expected/`. Every language runner uses
only its public SDK API. Normalize generated IDs, instants and trace IDs consistently with ADR-0015;
do not normalize away operation ordering, instance index, failure code, attempt count or chain
membership. Goldens are reviewed against the scenario, never auto-blessed from TypeScript output.

| Case id | Requirements / story | Required observations |
| --- | --- | --- |
| WP-01 examples | FR-014-001–002, US01 | Canonical current-v1 routing/fulfillment/reminder; amounts 100/-1; NOTIFICATION_REJECTED on reminder; terminal state/activity/journal/trace checks across all five languages. |
| WP-02 scoped identity | FR-014-003,014,017, US02/03 | Equal node ids in different iterations are distinct, also in `/runs/{id}/graph`; retries preserve logical identity; old root workflow unchanged; invalid/cyclic/escaping bodies rejected with `VALIDATION_FAILED`/`INVALID_SCOPE`. |
| WP-03 collection order | FR-014-004, US02 | 17 items, concurrency 3, reversed completion; exact input-order output; duplicates by value retained as separate indices; zero items succeeds empty. |
| WP-04 collection failure | FR-014-005, US02 | Stop admission after permanent failure, cancel/drain active work, reconcile unknown effect, compensate per FR-003-002 (reverse topological within an iteration, no cross-iteration order); no false successful join. |
| WP-05 loops | FR-014-006, US03 | False while=0 bodies; repeatUntil=at least 1; three iterations/carry checkpoints; maxIterations=1 limit failure; delay is durable and owns no worker; completed iterations compensate in reverse iteration order. |
| WP-06 approval | FR-014-007–008, US04 | Approve/reject/expire routes; identical key replay, changed key body conflict, decision-vs-expiry/cancel first commit; raw signal cannot decide. |
| WP-07 switch | FR-014-009, US05 | First of multiple true cases, default, predicate error, recorded-choice replay; no non-selected node activates. |
| WP-08 queries | FR-014-010, US06 | Run version/state revision consistency with concurrent update, terminal read, schema/redaction failure; no state/journal mutation or activity scheduling; explicit disclosure declaration, closed/bounded result schema, no-store response and successful access audit under ADR-0031. |
| WP-09 updates | FR-014-011, US06 | Atomic state/result, durable rejection, `expectedStateRevision` conflict (unaffected by unrelated activity completions), identical receipt replay after lost response, changed-key-body conflict, rejection in non-mutable run states; an update schedules no task. |
| WP-10 continuation | FR-014-012–013, US07 | Three segments/one successor each; explicit transfer; same version/deadline; old segment SUCCEEDED/CONTINUED; chain wait FINAL; old run writes never forwarded. |
| WP-11 chain races | FR-014-012–014, US07 | Signal/update/cancel vs rollover; chain signal to a head that is not waiting is refused, never held; chain dedup applies once across segments; expected-head conflict; cancelled chain cannot continue; parent waits final and cancels current child chain; RERUN_AS_NEW of a segment starts an unrelated run. |
| WP-12 limits | FR-014-015, NFR-014-003 | Boundary/excess for item, concurrency, node, depth, case, operation, principal, segment and byte limits; children in bodies and child-chain successors against the 1,024-run tree budget; tenant contention across runs, descendant admission at low quotas, derived-instance reserve surviving exhaustion/policy reduction, lifetime tree/chain counts surviving retention, no quota reset on continuation. |
| WP-13 capability compatibility | FR-014-016, NFR-014-004 | Old active runs + old SDK minors; new capability shapes rejected on an unsupported server while apiVersion stays v1; scoped tasks withheld from old workers; no new `Error.code` value (new meaning only in `details.reason`); worker 1.5 scoped identity versus retained 1.0–1.4 forms; effective 1024-byte expressions, 64 literal/4096 constructed lists and unsupported object literals; decimal/UTC behavior identical across five SDKs. |
| WP-14 security/telemetry | FR-014-017, NFR-014-002 | Cross-tenant opacity, revoked role, initiator exclusion, `worker`/`runner`/`viewer` cannot decide, secret marker absent; mandatory trace/error/audit, bounded metric labels; query/update capability cannot access secrets or I/O. |

WP-04 (compensation per FR-003-002) and the compensation case of WP-05 have runnable shared scenarios,
`collection-compensation` and `loop-compensation`, whose goldens were read by hand and pass in all five
SDKs. The scenarios keep one iteration with something to undo at a time wherever the contract promises no
order between iterations, so no golden pins one; the unordered case is proven in the Rust tests named in
`research.md` ("As built").

WP-02–14 must be run for TypeScript, Go, Python, C# and Java wherever they expose a public behavior;
server-only fault injection is additionally exercised below. A subset run is diagnostic evidence,
not five-language parity. Conformance checks task/heartbeat/complete instance identity through real
worker registration and leases rather than echoing a local builder document alone.

## Server failure-injection matrix

Use deterministic barriers at the named transaction edges, a real PostgreSQL database, real
outbox/inbox and actual scheduler/worker ownership. After recovery independently inspect persisted
run journal/state/tasks/timers/subscriptions, quota ledger, receipts and chain links. Test process
restart and connection drop rather than treating in-memory reducer replay as persistence proof.

| Fault point | Before commit expectation | After commit / lost response expectation |
| --- | --- | --- |
| Scope capture/admission | No partial snapshot, instance, quota charge or ready task | Same captured collection/instance IDs; lost broker delivery found by reconciler. |
| Worker effect then completion | Unknown outcome is reconciled before retrying critical effect | Valid fenced completion applies once; duplicate/stale attempts add no result/slot. |
| Iteration checkpoint/next timer | No checkpoint without carry/decision/next work | Replay uses recorded condition; one next iteration/timer after restart. |
| Switch decision | No route/task if transaction rolls back | Recorded selected case reused; no predicate re-evaluation on replay. |
| Approval open/decide/expire | No request without timer; no chosen route without audit | One winner, stable duplicate receipt; losing expiry/signal/cancel audited appropriately. |
| Update computation/write | No state/result receipt if transaction aborts | Lost answer returns exact receipt; revoked caller cannot replay data after permission loss; failed audit returns no values. |
| Continuation predecessor/successor | Old run remains active; no stray successor/head | One linked successor and correct current head; old run terminal, parent still waiting. |
| Chain-head command resolution | No accepted command bound to a different unrecorded head | Stable resolved run and chain key; rollover cannot apply the command again. |
| Outbox dispatch/ack | Committed DB work remains authoritative | Duplicate/lost hints do not duplicate claims/transitions; reconciliation resumes READY work. |
| Scheduler lease transfer | Stale owner cannot commit any primitive transition | New owner resumes durable state; worker attempt fences remain independent. |
| Compensation/provider failure | Forward effects remain recorded, no invented rollback | Failed compensation enters manual review / partial outcome with audit and original scope identity. |

For each mutating edge inject: process death before write, before commit, after database commit
before response, and after response before outbox delivery; duplicate command/delivery; old scheduler
fence; old worker attempt where applicable. Race timeout against signal, response/update and
cancellation with both commit orders. Restart all app processes between commit and reconciliation.
Assert no invariant violations rather than asserting physical external-effect exactly-once.

## Release evidence

Each row records command, toolchain/environment, seed/barrier, failures/skips and observed persisted
outcome in the release validation report. SC-014-002–006 require this evidence and the benchmark;
writing this matrix does not complete them. Existing baseline feature suites remain mandatory to
catch changes to saga, child, waitEvent, quota, versioning and worker-minor behavior.
