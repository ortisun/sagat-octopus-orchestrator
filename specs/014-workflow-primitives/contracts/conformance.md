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

`workflow-instance-limits` exercises WP-12 instance admission at3/excess2 in all five, with persisted
INSTANCE_LIMIT and cancelled unadmitted end, plus instance0/100001 and depth0/9 builder refusal.
Depth1/8 is an authoring check here, not execution of nested depth8. Each root is created before
its dedicated worker starts so start responses cannot race completion.

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

### Exact-path maintenance transaction prerequisites

`PathExecutionTx` reads both live lease clocks only for its addressed path. Its scheduler fence
checks the run shard, owner, generation and current database expiry and holds the shard's shared
row lock until commit. Run-lock contention must not backdate mutation time: both full and bounded
transactions sample the database clock after acquiring that lock. Real competing-lock assertions
live in `integration/workflow-primitives/byte_ceilings.rs`; the 19-case runtime-role checkpoint
and the failed PostgreSQL infrastructure run are recorded in
`tasks/project-completion/integration-notes.md`. These prerequisites alone do not establish that
all production consumers use bounded restoration or satisfy the large-workload resource gate.

### Current exact-path heartbeat

Scoped heartbeat opens its current path after acquiring the run lock, reads on that same database
connection, and samples mutation time after restoration. It does not acquire a second pool
connection or repeatedly prepare stale revisions while holding the lock. Prepared scheduler
transitions retain the separate snapshot/revision comparison contract. Both readers share the
complete bounded-path restoration implementation; an unadmitted path remains a stale attempt.

The live heartbeat atomically records the first started event and renews the exact lease. A repeat
cannot duplicate that event. Late heartbeat for the same reported lease returns no new expiry,
including after its collection/run closes, but still requires current binding and worker membership.
Root heartbeat also rechecks current membership after lock contention. The scoped trace is
`worker.scoped.heartbeat` with tenant/run identity and no token/input; late acknowledgements retain
`worker_heartbeats_after_report_total`. No wire or SDK representation changes.

`bounded_worker.rs` tests revoked whole-history SELECT, forged identity/generation, tenant opacity,
late authority loss, terminal acknowledgement, current role after a real lock wait, and twelve
concurrent renewals using two runtime-role connections. Claims, completion/failure, capability
disclosure and large workload acceptance remain separate open gates.

### Exact-path worker claims

Discovery retains `(run, path)` from the existing finite quota/pool/protocol/breaker-aware candidate
window. The run-only discovery API and the path projection share that SQL, preventing a second,
less selective path window from hiding eligible work behind a capped or restricted activity.
Candidates are hints: each current path is rechecked under its run lock with `SKIP LOCKED`.

A scoped claim holds drain authority, then binding authority before the session row, checks current
membership, and retains activity/breaker/script-retention admission through its atomic lease commit.
Its full task input is measured before leasing; receive bytes include only committed returned tasks.
Every path transaction shares that session's inflight count across paths/runs/replicas. A failure in
a later path returns tokens already committed by earlier paths rather than silently losing them.
Task/forward-effect identity and idempotency templates share the existing root construction rules.
Managed database work remains root-only; reports and capability delivery retain their current gates.

A completed loop has no active iteration. Its exact-path ancestor fence records `active=false`,
while durable saga authority separately permits the derived work needed to drain that loop.
`worker.scoped.claim` records tenant/run identity without payloads or tokens and reuses existing
quota/refusal/unreadable-definition counters. Tests revoke unrelated history SELECT, inject a later
lease failure and use actual binding-lock waits to check current role/session revocation and lock
order. These checks alone do not close the full compatibility or large resource acceptance tasks.

### Parent authority during child creation

Normal full and bounded scheduler mutations require the lease of their own run's shard. Starting
a child is an action admitted by its parent's shard owner. The new child's creation transaction
therefore uses an explicit parent-shard fence, deriving the parent from immutable admission
metadata. It is available only before the new child's first event in its uncommitted creation
transaction. Reopened children, ordinary Root transactions and already-mutated transactions cannot
use it. Owner, generation, shard identity and expiry are still checked under the shared shard row
lock, with database time sampled after lock contention. Losing that fence rolls back child creation.

The continuation child-start test checks creation and rejection after reopening; the ordinary-run
negative test retains a successful own-shard check. The two-replica child/parent suite verifies that
parent ownership chooses the creator even when child shard ownership differs. These controls do
not authorize subsequent child mutations using a parent's lease.

### Bounded inline worker reports and receipt authority

Inline scoped completion/failure reads only its current path. The existing pure path commands also
settle compensation/reconciliation, preserving forward-effect identity and reconciliation output.
Run-wide logical output bytes come from the admission ledger under the run lock, not a restoration
of historical values. Lease outcome, exact output references, journal/projection/outbox, applicable
audit and the original report receipt share the transaction. A failed retained-content write rolls
back before a newly fenced metadata-only failure is committed under that same request identity.
Secret-denial auditing survives that recovery.

Secret output scanning takes an immutable version snapshot, closes its database transaction,
performs provider I/O, then reads the current path again. Check current binding/membership, the
exact lease and applicable cancellation/expiry fences again; refuse changed secret versions. Only
the explicit non-retryable `CANCELLED` report can acknowledge cancellation through that scan fence.
Receipts are looked up only after current authority and echoed step identity checks, for both Root
and scoped tasks. A stale caller cannot regain an acknowledgement through an old receipt.

`bounded_reports.rs` first reproduces revoked-history completion/failure and Root replay after role/
binding withdrawal or a forged step id. `bounded_report_provider.rs` uses actual OpenBao, an explicit
legacy-secret migration and a wrapper that pauses real reads to exercise authority withdrawal
during the scan. Traces retain `worker.scoped.report` and `secret.output.scan`, without payloads or
tokens; existing quota and secret-denial counters/audits remain. Blob completion and secret/blob
capability reads are separate boundaries, as are larger graph/nesting/resource release gates.

The automatic scoped lease-expiry and retry passes now use this port: load one exact path outside
the write lock, compare the run revision under lock, hold the current shard fence and recheck live
expiry clocks before persisting the addressed command. A revision race is skipped for later
rediscovery. Journal, task/timer projection and outbox effects still commit together. Only the
exceptional retained-content quota drain uses the guarded full-run path. The PostgreSQL test
`scoped_maintenance_does_not_read_whole_run_iteration_history` revokes iteration-history SELECT
in its isolated database, proves full restoration fails, then requires both maintenance passes
and journal replay to succeed. Worker claim/report consumers and wider scale gates remain separate.

Scheduler lease expiry must be checked **after** the shard row's shared lock is acquired. A
predicate evaluated before a lock wait cannot authorize a lease that expired during that wait.
The full and exact-path transactions share that post-lock check and refresh their mutation clock
when the fence succeeds. A bounded binding read likewise holds the shared binding-authority key
that revocation takes exclusively, until commit. `bounded_fences.rs` exercises both expiry waits
against an unchanged locked row and a real competing binding-revocation lock. Its runtime-role
profile also verifies that the waiting execution connection is `sagat_app`.


### Scoped forward blob completion

Ordinary forward collection/loop activities may return the native BlobOutput descriptor through
protocol1.5. No new wire fields or SDK semantics are introduced. Current membership, binding and
echoed exact instance identity precede receipt lookup. Only an exact receipt may bypass lease
expiry; a miss verifies the live token, attempt and generation before object/catalog lookup.
A produced result can still be reported during cancellation, matching Root effect accounting.
The path writer validates descriptor ID/size/digest/content type against its BlobCommit and commits catalog,
payload, journal, quota, outbox and receipt together. Derived and declared-secret blob outputs
remain denied. The `collection-blob-output` case requires all five SDK builders and runtime
results to agree on two ordered300KB descriptors; native tests separately check actual catalog
state, rollback, foreign-iteration refusal, current authority withdrawal and pre-catalog fencing.

Preparation returns either the exact receipt or the authorized report scope in one transaction;
consumers must not split receipt lookup from live scope resolution. A duplicate that commits
between two caller invocations remains replayable instead of becoming a spurious stale attempt.
Blob quota-failure recovery reopens current authority and rechecks its live fence before writing
any failure/receipt; rollback does not grandfather an expired lease.

### Shared switch case boundary

`switch-builders` authors and actually publishes/executes 32 cases in every SDK. Its otherwise
valid 33-case fixture must fail through every public builder with `VALIDATION_FAILED`.
Canonical expectations preserve every case and ordering; live first-match/default/predicate-error
observations retain the pre-existing complete golden. This covers the WP-12 case-count boundary;
it does not replace the other limit observations.

### Shared expression limit phases

`expression-limits` executes exact 1,024 UTF-8 source bytes, a 64-item literal and a constructed
4,096-item list through all five public query clients. Constructing 4,097 fails with the existing
`VALIDATION_FAILED`/`OPERATION_EVALUATION_FAILED`, followed by a successful query with unchanged
Run/state revisions. The same five builders author reviewed documents containing 1,025-byte
source, a 65-item literal and an unsupported object literal; shared HTTP publication rejects each
with `EXPRESSION`, leaving no active or published version. Those compile-time refusals never run.
Complete normalized `publicationEvidence` distinguishes this server phase from SDK query results.

### Remaining shared public observations

WP-04 `collection-fail-fast-drain` retains concurrency2 while one real polling host holds owned
work; only after journal step.started does a second host fail the other iteration. The complete
graph/logs plus summary/source evidence require FAILED/NO_STOCK, sibling cancellation, admitted
indices0/1, no index2/checkpoint and no successful join. No global journal or compensation order
is added. WP-08 expression-limits additionally exercises query result-schema refusal with
INVALID_OPERATION_STATE, raw full/prefix marker absence and unchanged revisions after failure.
WP-12 approval-builders accepts32principals (authoring-only); expression-limits actually publishes
32queries/32updates. Existing shared33-negative fixtures remain required. These observations do
not replace general nested/tree/retention/resource gates.


## Complete current 60-case SDK acceptance — 2026-10-18

T014-041 closes on the complete, unfiltered current five-SDK aggregate: 60 scenarios, 300
language/scenario plays, 30 authoring-only, 5 publication-only and 265 live workflow/protocol.
All 270 complete normalized outputs independently equal the reviewed golden; all 1,672 frozen
inputs are unchanged. The aggregate passed in 1,067.94 s (harness 1,067.57 s), without blessing,
prepared-runner reuse or archived source. Eight actual native compatibility tests pass on
identical executable inputs, covering active-old-run and old-worker-minor compatibility.

The first full attempt failed during managed-secret PUT with an unexplained 503, before the
five database scenario outputs. Its same-source focused retry and this subsequent full pass
are retained separately; no inferred provider cause or speculative fix is claimed. The third
and final supported-public mapping review found no further missing observation after the
last focused fixtures; stale matrix wording was corrected. Actual tagged v0.1.0 lifecycle
coverage remains separately scoped, with other historical features/versions untested.

Evidence: [complete SDK60 aggregate](../../../tasks/project-completion/evidence/sdk60-current-20261018/reproduce.md).
This evidence does not accept general nested runtime, C0, production ceilings or final release gates.
