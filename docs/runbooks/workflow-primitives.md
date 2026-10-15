# Workflow primitives: operating and validating the current build

The public workflow document remains additive `sagat-octopus-orchestrator.io/v1`. Bounded
`switch`, run-addressed declared Query/Update and a bounded root human approval are available
with the existing basic root nodes. A public collection workflow is a chain of `forEach` nodes that
ends at `end`; every referenced body contains exactly one activity followed by one `return`.
The restricted public loop shape uses an activity followed by a return in its body. Nested
collections, triggers, structured-primitive combinations and broader body graphs remain closed.
The public continuation subset accepts an activity followed by `continueAsNew`, or an
activity and condition that select `continueAsNew` or `end`. Approval documents cannot combine
approval with parallel execution, declared state or another structured primitive yet.

Collection admission has no implicit capacity. An administrator must configure the tenant-wide
`iteration_concurrency` quota before the scheduler can open the first scope:

```sh
curl -X PUT "$API/v1/quotas/iteration_concurrency" \
  -H "authorization: Bearer $ADMIN" -H 'content-type: application/json' \
  -d '{"limit": 32}'
```

The value is 1..1,000,000,000, is ceiling checked and audited, and applies to collection and loop
iterations across all namespaces of the tenant. The default operator ceiling is 1,024; a higher
setting requires an explicit operator grant. Clearing the tenant setting disables new structured
iteration admission. Reducing it below current use preserves admitted work and blocks further
admission until use falls below the new value.

The scheduler discovers and advances published collections through the durable collection port.
The worker protocol negotiates minor 1.5 for scoped work while workers requesting 1.4 retain their
root-only behavior. PostgreSQL tests cover missing policy, audited provisioning, ordered admission,
checkpoint, finish, replay, cancellation, stale fences and lost broker hints. The public round trip
publishes and starts through HTTP, provisions capacity and worker binding through their authorized
routes, executes each body activity through worker protocol 1.5, then verifies `collection.finished`,
the terminal `end`, `run.finished` and the reconstructed projection.
For a `map` result expression such as `bodyResult.amount`, the scheduler evaluates each member
against its checkpointed body result. The finish transaction verifies the checkpoint source
digests and retains the mapped aggregate with the journal, projection and outbox.

Sequential-loop persistence now records timeout, cancellation and expression-failure drain in one
PostgreSQL transaction with the journal, projection, timer cancellation, capacity release, audit,
outbox and telemetry wake. Its focused tests also reopen the run between drain stages and reject a
stale timer fire. Evaluated carry and result bytes now commit in the same progress transaction after
the scheduler fence, replay, revision and quota checks; rollback tests prove that outbox and audit
failures leave no orphan payload. A focused public PostgreSQL test covers the restricted loop
shape through worker completion, scheduler restart, `repeatUntil` and rollback retry. Broader
bodies, five-SDK runtime parity and the full failure matrix remain release gates.

## Check the running system

Run the contract and PostgreSQL suites from the repository root with the pinned toolchain:

```sh
cargo test -p sagat-tests --test contract_workflow_primitives --locked
cargo test -p sagat-tests --test integration_workflow_primitives --locked
cargo test -p sagat-tests --test fault_workflow_primitives --locked
```

The PostgreSQL test harness starts real infrastructure or fails if it cannot. A passing private
transaction test shows the asserted journal/projection/outbox boundary only; it is not evidence of
a public workflow round trip. See
[`validation-report.md`](../../specs/014-workflow-primitives/validation-report.md) for commands,
counts and remaining gates from the latest run.

For authoring parity, run the shared five-language SDK scenarios individually:

```sh
SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=collection-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=state-operations cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-decisions cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

The four builder scenarios compare normalized documents with reviewed goldens and do not call
the server's runtime APIs. `collection-runtime` publishes and executes a mapped collection result
in every SDK using worker polling. `loop-runtime` publishes the restricted body shape and
completes both loop types with worker protocol 1.5 in every SDK. `state-operations` publishes a
supported document and exercises
Query/Update through the real server in all five languages. `approval-decisions` publishes a
root approval workflow, lists its pending request and submits approve/reject decisions through
the real server in all five languages. Do not replace a golden merely
because an output changed: inspect the contract and intended compatibility behavior first.

## Diagnose a stuck or duplicated transition

Identify the tenant, namespace, run, workflow version, instance path and last journal sequence.
Inspect the run journal and projection under the same tenant scope; keep the trace ID and request
ID, but omit payloads, credentials and decision policy values from logs. The journal is the
authority. A mismatch with a step, timer, subscription, collection, loop or approval projection
means the transition must not be retried blindly. Preserve the run and investigate the failed
transaction or projection verifier.

The runtime graph uses the full logical instance identity. A repeated body node therefore appears
as `items[0]/work`, `items[1]/work`, and so on, while `GET /runs/{runId}/graph/changes` returns only
the instances touched after the requested journal sequence. A response with `reset: true` means the
client must replace its snapshot. Persisted nodes that this build cannot match to the pinned
definition remain visible with `type: unsupported`; do not hide them or infer a newer node kind.
Selected switch routes and continuation links come from the same durable journal/projection read.
Trace spans carry `sagat.instance.path` and `sagat.scope.depth`, and continuation spans link to the
other run's stable run span rather than copying payload data into telemetry.

For a worker task, compare the attempt and lease fence as well as the instance path. An old
worker minor must never receive a scoped task. A stale completion should leave journal, quota and
outbox unchanged. For a delayed loop or approval, verify that the due record is durable and no
worker lease is retained while waiting; duplicate delivery must produce one committed result.
Broker delivery is a hint, so use reconciliation of PostgreSQL state rather than assuming the
broker delivered exactly once. External business effects require stable idempotency keys or a
reconciliation procedure; neither a journal event nor an SDK retry makes an external effect
exactly once.

For declared Update, check the operation name, immutable definition version, expected state
revision, request hash and update key. `POST /runs/{runId}/updates/{operationName}` rechecks
current authorization even on receipt replay. A durable rejected receipt replays the same
business result. `POST /runs/{runId}/queries/{operationName}` reads one committed snapshot and
adds an access audit without appending a workflow event or scheduling work. Both responses must
carry `Cache-Control: no-store`; a foreign tenant's run remains opaque. A stale state revision
is a conflict, while a schema or declared-business rejection has a typed reason. If an outbox
write fails, the state, receipt and journal must all roll back before retrying the same update
key.

For a pending approval, list `GET /runs/{runId}/approvals` under current run and approval read
permissions, then submit the decision with a stable decision key. The decision transaction must
commit its route, receipt, audit and outbox together. A replay with the same key and body returns
the original receipt; a changed body conflicts. Cancellation and deadline expiry race the decision
under the run lock, so inspect the committed approval status after a conflict. A raw signal cannot
stand in for a decision. Keep identity, policy details and reason text out of logs.

For chain commands, use `GET /namespaces/{namespaceId}/chains/{chainId}` to inspect the current
head. A declared Query may address that head with
`POST /namespaces/{namespaceId}/chains/{chainId}/queries/{operationName}` and body
`{"args": ..., "expectedRunId": "..."}`. `expectedRunId` is optional; when present it makes a
replaced head fail with `409` and `details.reason=CHAIN_HEAD_CHANGED`. The caller needs both
`run:read` and `run:query`. A successful response identifies the actual `runId`, `runVersion` and
`stateRevision` that produced `result`, and carries `Cache-Control: no-store`.

Chain Query takes a shared chain lock before the shared run/snapshot lock. A handoff race therefore
reads either the complete old-head snapshot or the complete successor snapshot, never a mixture.
It appends no workflow event and schedules no task. Operation arguments and results remain limited
to 256 KiB each, retained run data to 4 MiB and task context to 8 MiB. The immutable declaration's
closed argument/result schemas and disclosure mode still apply; Query cannot read secrets or make
network calls. The focused Query tests seed chains with declared state directly; public publication
still rejects declared state combined with continuation, so there is no public end-to-end chain
Query workflow yet.

A declared Update may address the chain with
`POST /namespaces/{namespaceId}/chains/{chainId}/updates/{operationName}`. Keep one stable
`updateKey`; `expectedRunId` is optional and `expectedStateRevision` remains the business-state CAS.
The server locks the chain before the resolved run. A new key with a stale expected head fails with
`CHAIN_HEAD_CHANGED`. An exact replay uses the same presence and value of `expectedRunId`, returns
the original applied or rejected receipt after rollover, and reports the run that originally
committed it. Reusing the key with changed arguments, operation, revision or expected run fails with
`IDEMPOTENCY_KEY_REUSED`. Authorization is checked again on every replay, the resolved-run audit
commits with the decision, and every response carries `Cache-Control: no-store`. The focused Update
test uses the same seeded compatibility boundary as chain Query; a publicly authored workflow still
cannot combine declared state and continuation. Use the exact-run Update route when the caller
deliberately accepts segment-bound, non-forwarding semantics.

`POST` to the chain's `signals/{signalName}` or `cancel` path requires `expectedRunId` in the body
and `Idempotency-Key` in the header. For a stale Query or command, inspect the chain again before
choosing whether to retry against the new head. Replaying the same command key and body returns the
same receipt; a different body is a conflict. A published workflow in the restricted continuation
subset creates the chain on start and hands off under a scheduler fence. At the segment limit, the
run fails with `CHAIN_LIMIT` and the chain records `FINAL` once. Declared state transfer, body-local
continuation, triggers and broader graphs remain disabled. Exact-run commands remain bound to their
segment and do not forward silently.

## Rollback and compatibility

Disable publication and starts of any newly enabled primitive before rolling back its writer.
Keep readers, journal codecs, projections and worker capability gates for already active runs
until they drain. Do not remove migrations or delete chain/receipt rows while an active run or
retained idempotency receipt references them. Revert only after validating old active runs, old
worker minors and all five SDKs against the target build. The worker adapter advertises minor 1.5
for scoped tasks. Workers using 1.4 or earlier receive only root tasks; preserve that capability
gate and the 1.5 journal/lease recovery path during rollback.
