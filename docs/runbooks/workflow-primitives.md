# Workflow primitives: operating and validating the current build

The public workflow document remains additive `sagat-octopus-orchestrator.io/v1`. Bounded
`switch`, run-addressed declared Query/Update and a bounded root human approval are available
with the existing basic root nodes. A public collection workflow may start with ordinary source
activities followed by a chain of `forEach` nodes ending at `end`; each referenced body contains a
bounded acyclic graph with an activity/condition/switch entry, one to eight executable nodes (activities,
conditions, switches) and one `return`. In the 0.3 increment, local `onError` routes and
recorded decisions may select a branch; a recovery failure drains with its own
code. Unselected branches retain their lifetime charge as retired reservations, while actual
completed/unknown effects keep the compensation capacity they require.
The restricted public loop shape uses an activity followed by a return in its body; a loop's
`onError` edge may lead to an `end` or to recovery activities that end at an `end`. Nested
collections, control-node entry, waits inside bodies, triggers and broader structured combinations remain closed.
The public continuation subset accepts an activity followed by `continueAsNew`, including exact
declared-state transfer, or an activity and condition that select the same continuation or `end`.
An approval combines with declared state and with a continuation chain (an entry activity, then an
approval that rolls the chain over or ends it; decision D295). An approval cannot combine with parallel
execution, and an approval inside a collection or loop body is refused at publication with
`details.dependency: SCOPED_COMPENSATION` until scoped compensation can settle it.

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
bodies and the remaining failure matrix remain release gates; the public delay, cancel and
error-edge paths are described below.

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
network calls. The shared `chain-query` scenario reads the initial head through all five SDKs;
`chain-query-handoff` completes the predecessor and reads the successor's transferred snapshot
through each public SDK. It also applies and replays a chain Update, then reads revision 1.
`waitForCompletion` ends at the continued predecessor and now reads its `chain` segment
(`SUCCEEDED` with `completionKind` `CONTINUED`); `waitForChainCompletion` polls the chain's
authorized snapshot until `finalOutcome` is set. A client timeout leaves the chain running.
`GET /namespaces/{namespaceId}/chains/{chainId}/runs` lists the segments in order, 50 by default and
at most 200 per page, behind an opaque cursor that stays valid while the chain grows; it needs
`run:read`, answers a foreign or missing chain with the same 404, and returns no input, output or state.
`chain-three-segments` proves three executed segments with one pinned version and one original deadline,
and that writes aimed at a continued segment are refused (`CONFLICT`) without touching the chain.
The shared `chain-final-success` scenario also proves a conditional continuation reaches a
`SUCCEEDED` final outcome after the second segment. A transfer expression failure ends its current
head with durable `EXPRESSION_ERROR`; the scheduler does not keep retrying the same input.

A declared Update may address the chain with
`POST /namespaces/{namespaceId}/chains/{chainId}/updates/{operationName}`. Keep one stable
`updateKey`; `expectedRunId` is optional and `expectedStateRevision` remains the business-state CAS.
The server locks the chain before the resolved run. A new key with a stale expected head fails with
`CHAIN_HEAD_CHANGED`. An exact replay uses the same presence and value of `expectedRunId`, returns
the original applied or rejected receipt after rollover, and reports the run that originally
committed it. Reusing the key with changed arguments, operation, revision or expected run fails with
`IDEMPOTENCY_KEY_REUSED`. Authorization is checked again on every replay, the resolved-run audit
commits with the decision, and every response carries `Cache-Control: no-store`. The focused Update
test uses the same seeded compatibility boundary as chain Query; the combined public Update path
is not yet tested. Use the exact-run Update route when the caller
deliberately accepts segment-bound, non-forwarding semantics.

`POST` to the chain's `signals/{signalName}` or `cancel` path requires `expectedRunId` in the body
and `Idempotency-Key` in the header. For a stale Query or command, inspect the chain again before
choosing whether to retry against the new head. Replaying the same command key and body returns the
same receipt; a different body is a conflict. A published workflow in the restricted continuation
subset creates the chain on start and hands off under a scheduler fence. At the segment limit, the
run fails with `CHAIN_LIMIT` and the chain records `FINAL` once. Direct declared-state transfer is
available. The shared `chain-commands` scenario uses `activity -> waitEvent -> continueAsNew`:
wait for `wait.started` before signalling, retain one command key for exact replay, inspect the
new head after delivery, and send subsequent commands with that run ID. A direct `waitEvent`
entrypoint, body-local continuation, triggers and broader graphs remain disabled (G372 in
`research.md`). Exact-run commands remain bound to their segment and do not forward silently; a
cancel, pause or resume aimed at a continued segment is `409 CONFLICT`, as for any ended run.

A boundary that still holds open work (a running step, a timer, a subscription, an approval, an
unsettled child, an unknown effect or an unfinished compensation) **waits**: it is not a failure and
nothing is stored for it. The scheduler counts it in `scheduler_continuation_waits_total{blocker}`
(`blocker` is `sibling_work`, `child`, `lease`, `timer`, `subscription`, `approval`, `unknown_effect`,
`compensation` or `capacity`, when the namespace or tenant has no room for the successor), and
`Report.continuations_waiting`. The chain's original deadline still ends it `TIMED_OUT`. A hand-off
replaces the predecessor and never takes room it does not have; lower the active-run quota to 0 and the
boundary waits until it is raised again.

A continuation workflow can run as a `child` node. The parent waits for the chain's final outcome,
never for a segment, and is told once. Cancelling the parent latches the child chain and cancels its
current head; the latch also stops a hand-off that had not committed. The successors of a child chain
are runs of the parent's tree and spend its 1,024-run budget: a full tree ends the chain with
`CHAIN_LIMIT` (the same durable code as the segment limit). The `chain-child` scenario runs this in all
five SDKs. Nothing the chain owns is ever deleted (D298): the segment count of a chain and the run
count of a tree only grow, and the runtime role has no `DELETE` on them.

## A loop that waits, is cancelled, or recovers

A loop with `delayMs` waits on one durable timer (`loop_timers`, state `WAITING`) and nothing else.
While it waits the run is `WAITING`, no step is `READY`, `LEASED` or `RUNNING`, and a worker may be
down entirely: the scheduler fires the timer when it is due and the next iteration becomes `READY` for
whichever worker comes. Check it from the API: `GET /runs/{id}` shows `WAITING`, and
`GET /runs/{id}/events` shows `loop.iteration_checkpointed` followed by exactly one
`loop.delay_elapsed` once the delay has passed (never two, even when two scheduler replicas deliver
the timer together or the scheduler restarts during the wait).

- **A delay that cannot end inside the loop budget is never scheduled.** If `delayMs` is longer than
  what is left of `timeoutMs`, no timer exists; the returned iteration waits and the loop ends
  `FAILED` with `RUN_TIMED_OUT` when the deadline passes (then `onError`, or the run fails). An
  operator who sees a loop sitting in `RUNNING` with no timer and no leased step is looking at this
  case; it resolves itself at the deadline.
- **Cancel** (`POST /runs/{id}/operations` with `CANCEL`) of a run with an active loop answers `202` and
  `CANCELLING`. The scheduler cancels the timer or asks the leased body attempt to stop (its heartbeat
  answers `cancelRequested`; the worker's report ends that attempt), drains the iteration and ends the
  loop and the run `CANCELLED`. The `onError` edge is never taken by a cancelled run, even when the
  loop had already timed out and was draining. Tenant iteration usage and the run's reserved
  instances return to zero.
- **Pause** holds the run; a delay that falls due while paused admits nothing until `RESUME`.
- **A signal** sent to a run whose loop waits is refused with `CONFLICT` (`STATE_CONFLICT`, "not
  waiting for that signal") and writes nothing. A declared Query/Update cannot be combined with a loop
  at publication.
- **A loop that fails** (permanent body failure, loop timeout, or an initial expression that cannot
  be computed) drains the active iteration and follows its `onError` edge. When the edge leads to an
  activity, the worker takes it as an ordinary root task (any protocol); the loop owner reads
  `FAILED` with the failure code and `routed`, the success `end` stays `NOT_STARTED`.
  A scheduler action that keeps failing is counted in `failed` of the tick report and logged as
  `a loop scheduler action failed`; a stale replica fenced out of a shard reports this and changes
  nothing.

Run the proofs with `cargo test -p sagat-tests --test integration_workflow_primitives --locked
loops_http::` and `cargo test -p sagat-tests --test fault_workflow_primitives --locked loops::`; the
step-by-step demo with the exact `curl` commands is in
`specs/014-workflow-primitives/quickstart.md`.

## A collection or loop that compensates

A collection or loop whose body declares `compensateWith`, `irreversible` or `highRisk` undoes its
completed iterations when it stops (a permanent failure of an iteration, the iteration limit or a loop
timeout, the storage quota, a cancellation). The order is reverse dependency inside an iteration, none
between collection iterations (at most `maxConcurrency` are undone at once) and last iteration first for
a loop. The sagas runbook (`sagas.md`) explains the outcomes; this section is what differs inside a scope.

Read it from the journal and the graph. `scope.compensation_planned` lists the instances to undo, the
instances each waits for and the effects that cannot be undone; `scope.compensated` carries the outcome.
Between the two the run is `COMPENSATING` and the scope owner stays `WAITING` on `COLLECTION` or `LOOP`.
The graph shows the compensation of `reserve` in iteration 2 of `lines` as `lines[2]/reserve~c` with the
`instancePath` `[{"scopeNodeId":"lines","iterationIndex":2}]`, and its lookup as `lines[2]/reserve~r`.

When the run is `MANUAL_REVIEW`, find the instance with `waitingOn: operator` and its `failureCode`.
Decide with `POST /runs/{runId}/operations`, the same actions, permissions, reasons and audit records as
for a root saga (`RETRY_COMPENSATION`, `RETRY_STEP`, `SKIP_COMPENSATION`, `RESOLVE_STEP`), adding
`instancePath` with the scope ancestry from the graph, outermost first. Without it the request addresses
the root step of the same name, which does not exist or does not wait for a person, and is refused. `SKIP_COMPENSATION`
ends the scope `PARTIAL`; a retry uses the same idempotency key as the failed attempt. An operator
decision is the only way out: a cancellation or a deadline that arrives while a scoped compensation waits
for a person leaves it waiting (unlike a root saga, see D386), and a restart does not forget it.

An effect with an unknown outcome inside an iteration (`LEASE_EXPIRED`, `ATTEMPT_TIMEOUT`,
`OUTCOME_UNKNOWN`) is looked up by `<node>~r` of that iteration before anything repeats it; an iteration
that waits for a lookup or a person is not drained, so the scope cannot finish while it does. If a run
sits in `COMPENSATING` without a compensation leased, check that the scheduler is discovering
`COMPENSATING` runs and compare the projection with the journal (`verify_run`) before touching anything.
Compensations are leased while the run is stopping and are never cancelled by the stop; do not cancel
them by hand. Their idempotency keys are stable (`compensation:{id}:{run}` from the scope path and the
forward node), so a retried compensation reaches the provider with the key of the first one.

Not covered: a run deadline that expires over a collection or loop that is still open stays unsupported
(O382), and branches, timers and nested scopes inside a compensating body are not published (D388).

## An approval beside state or inside a chain

What to look at when a run waits for a person and also has declared state or a chain.

- **State.** The run is `WAITING` while the request is open and `POST /runs/{runId}/updates/{name}` still
  applies; the journal then holds `approval.opened`, `state.updated` and `approval.decided` in one gap-free
  sequence, because the decision and the Update both take the run's row lock. A stale `expectedStateRevision`
  is `CONFLICT`/`REVISION_CONFLICT` and leaves the request `PENDING`. When the run has ended the state is
  still readable and no longer mutable (`RUN_NOT_MUTABLE`). A node expression that names `state` does not read
  it (it evaluates to null, G442); read state through a declared Query.
- **Chain.** `GET /v1/namespaces/{ns}/chains/{chain}` gives the head; list the requests of the head run with
  `GET /v1/runs/{head}/approvals`. Each segment has a request of its own and a decision belongs to the segment
  that opened it. The boundary after an approval is reached only when the request is settled, so the hand-off
  never meets an open one; if some graph did, the boundary would wait (`continuations_waiting`,
  `scheduler_continuation_waits_total{blocker="approval"}`), never fail. Transferred state is read and updated
  through the chain address; the predecessor answers `RUN_CONTINUED`.
- **A chain that stops.** Cancelling the chain or reaching its original deadline settles the open request
  `CANCELLED` (audited as the engine's own act when no operator cancelled) and a later decision answers
  `ALREADY_DECIDED`. A boundary approved just before the deadline is not handed off: the deadline ends the
  chain `TIMED_OUT` and the scheduler counts a skip, not a failure (`ChainError::DeadlineElapsed`).
- **If a request stays `PENDING` on a run that ended**, that is a defect: compare the projection with the
  journal (`verify_run`) and look for a scheduler action that failed with an `approval cancellation` error
  before touching the rows.

Telemetry is the declared set only: `workflow.approval.decide` (from the journal, under the run's own
trace, with the closed `approved`/`rejected`/`expired` outcome), `workflow_primitive_transitions_total{kind="approval"}`,
`workflow_approval_wait_seconds{outcome}` and, for a refused decision,
`workflow_primitive_rejections_total{kind="approval",code}`. The earlier `workflow_approval_http_*` counters
were removed (D446).

## Local collection decisions (0.3 increment)

For a body decision, use the qualified journal address (`lines[0]/choose`, for example).
`condition.evaluated`/`switch.evaluated` and the following `step.routed` explain the chosen
branch. The decision and selected task/outbox share one commit. `EXPRESSION_ERROR` either
takes a condition's local error route or starts the ordinary collection drain and compensation.

The operator questions and their evidence are:

- Which branch ran: the qualified choice and route in the journal and runtime graph.
- Why progress waited: `scheduler_actions_total{kind="collection",outcome="skipped"}`
  versus `outcome="fenced"`; inspect the Run revision/paused state and scheduler ownership.
- Where a control transaction waited: the `scheduler.action` span with
  `kind="collection_control"`, its run/tenant/instance and expected revision, correlated to
  the Run's trace and parent span. Items, outputs and expression text are absent from it.

The real PostgreSQL cases cover a pause committed ahead of a prepared answer, native outbox
rollback, malformed controls and orphan timer/lease metadata in full/path/frontier reads and
prepared writes. An invalid projection is an integrity error; do not invent a choice or reset
its metadata manually. Preserve the journal and compare it with the projection using
`verify_run` before attempting recovery.

```sh
RUST_TEST_THREADS=1 cargo test -p sagat-tests --test integration_workflow_primitives --locked -- collection_control_transactions:: collection_controls::
SAGAT_SDK_SCENARIO=collection-control-routing cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

The published body has an activity/condition/switch entry, up to eight executable activity/condition/switch
nodes and one Return. Control entry, waits, children and nesting remain gated. The existing
wire fields and worker minor1.5 are unchanged; a full release-wide compatibility matrix
remains required before shipping.

## Rollback and compatibility

Disable publication and starts of any newly enabled primitive before rolling back its writer.
Keep readers, journal codecs, projections and worker capability gates for already active runs
until they drain. Do not remove migrations or delete chain/receipt rows while an active run or
retained idempotency receipt references them. Revert only after validating old active runs, old
worker minors and all five SDKs against the target build. The worker adapter advertises minor 1.5
for scoped tasks. Workers using 1.4 or earlier receive only root tasks; preserve that capability
gate and the 1.5 journal/lease recovery path during rollback.


## Scoped inline reports and stale receipts

Scoped claims, heartbeat and inline completion/failure use current exact-path transactions.
Completion and reconciliation retain outputs and receipts atomically with their addressed journal;
compensation outputs remain discarded. Whole-run replay remains the independent projection oracle.
A secret output scan closes its database transaction before calling OpenBao, then checks current
binding/membership, lease and secret versions again. An ordinary completion cannot acknowledge a
cancelled capability; the explicit non-retryable `CANCELLED` failure can settle cancellation.

A previously accepted receipt still requires current execution authority and the correct echoed
step identity. Investigate `FORBIDDEN`, `NOT_BOUND` or `STALE_ATTEMPT` against current membership,
bindings and the exact task identity; retaining an old receipt/token does not restore authority.
Never copy tokens or secret values into logs. Relevant traces are `worker.scoped.report` and
`secret.output.scan`; existing quota and secret-denial counters/audits remain authoritative.

Reproduce the happy report/replay and failure barriers with Docker and the Rust toolchain:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests \
  --test integration_workflow_primitives -- \
  bounded_reports:: bounded_report_provider:: --test-threads=1 --nocapture
```

The tests revoke unrelated iteration-history access, retain full journal replay as an oracle,
withdraw current membership/binding, forge a step id and pause real OpenBao reads. The provider
barriers additionally acquire the run lock independently while the scan is paused; no completion
commits after authority withdrawal. These diagnostics do not enable scoped blob output, larger
graph/nesting or the large resource acceptance gates.

## Scoped secret and blob capabilities

Scoped secret delivery and blob authorization use the current exact path. A paused OpenBao read
holds neither the run lock nor worker authority locks. A role/binding withdrawal that commits
during that read makes the resumed delivery fail; the resolved value is discarded. Publisher
access, secret versions, worker pools and live lease are also checked before handoff. Secret and
blob-read audits identify the logical scoped step UUID, so distinct iterations remain distinct
while repeats within the same attempt collapse. Secret values and tokens never enter the audit.

Blob upload for a step declaring secrets retains the existing content-scanning refusal, including
a current cancelled attempt: this only returns a denial. Other scoped blob access requires a live,
uncancelled attempt. An exact completion receipt may replay after lease expiry, after current authority and identity
checks. A receipt miss and scoped report-target lookup require the exact live lease before catalog
metadata access. Cancellation may still report a known produced result so scope drain can account
for the effect; it does not authorize fresh uploads.

Public gRPC `WriteBlob` skips the root-only managed database probe for a scoped instance. It still
requires the current exact-path upload capability before object reservation. The real RPC fixture
revokes SELECT on unrelated iteration checkpoints and verifies a successful upload plus a typed
`STALE_ATTEMPT` refusal for a self-consistent forged generation, with no second object reserved.
Upload permission is separate from completion; ordinary forward scoped completion follows the
transactional rules below.

Reproduce the bounded secret/blob checks and real-provider withdrawal barriers:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests \
  --test integration_workflow_primitives -- \
  bounded_capabilities:: bounded_report_provider:: --test-threads=2 --nocapture
```

The capability-only checks do not transfer object bytes. `scoped_blob_completion` exercises the
real public upload and completion path for300KB outputs. Blob service checks tenant/namespace,
object/catalog state and digest; the exact-path writer additionally binds the stored descriptor
ID/size/digest/content type to its catalog commit. Output, catalog state, journal, quota, outbox and receipt commit
or roll back together. Derived tasks and activities declaring secrets cannot return blob outputs.
A journal-insert fault leaves the blob UPLOADED and task leased; retrying the same request after
removing the injected fault commits once, and later calls replay its receipt.

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests \
  --test integration_workflow_primitives scoped_blob_completion:: -- --test-threads=3 --nocapture
SAGAT_TEST_AS_RUNTIME_ROLE=1 SAGAT_SDK_SCENARIO=collection-blob-output \
  cargo test --locked -p sagat-tests --test sdk_conformance \
  every_sdk_conforms_to_every_scenario -- --exact --ignored --nocapture
```

### Child admission and handoff contention

Child reservation serializes against parent cancellation with `FOR NO KEY UPDATE`, then takes
the lifetime tree lock. Creation and evaluated handoff take active-run admission locks before
retaining input/state payloads. This avoids an inversion with a successor's root foreign-key
check or a competing novel payload's uniqueness check. A database deadlock remains a failed
transaction, never an acknowledged transition; inspect existing scheduler action failures/logs
and retry from persisted state. The journal, quota deltas, audit and outbox retain their atomic
commit boundary. No payload bytes are needed in diagnostic logs.

Reproduce the two lock barriers with the repository's disposable PostgreSQL fixture:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests \
  --test fault_workflow_primitives do_not_ -- --test-threads=1 --nocapture
```

The depth-two case includes aborted admission and a real scheduler retry; the quota/payload case
commits a new root alongside continuation. Their focused results do not establish the complete
child-link lock graph or nested collection capacity; see the
[working lock inventory](../../tasks/project-completion/capacity-lock-inventory.md).

A chained child's parent updates its link for cancellation/final notification; it does not update
the first segment's legacy `parent_settled`/`parent_cancel` flags. Only children without a matching
link use legacy flags, restricted to tenant and exact parent. The scheduler follows a cancelled
link to its current child head. Diagnose failures through existing scheduler outcomes, cancellation
audits and the `child.finished` journal event without logging payloads. Reproduce the three
database trigger guards with:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests \
  --test fault_workflow_primitives chain_parent_ -- --test-threads=3 --nocapture
```

### Publication diagnostic checks

Validate an editable draft before publishing. Canonical schema errors have precedence (`SCHEMA`);
schema-valid cycle/escape/overlap/recursion failures use `INVALID_SCOPE`. Body escapes include the
body and source node. An ordinary missing root target retains `GRAPH`. Correct unsupported
object literals or expressions above 1024 UTF-8 bytes when the reason is `EXPRESSION`; the byte
limit can be exceeded even when the character count passes the schema. Rejection creates no
published version. Use the response trace ID for diagnosis; do not log expression contents.

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests \
  --test integration_workflow_primitives publication_rejections -- --test-threads=3 --nocapture
```

The repository fixture starts its own PostgreSQL and exercises both draft validation and public
publication with the application runtime role. See the exact failures and bounded review in
`tasks/project-completion/evidence/publication-scope-20261018/`.


### Stable parent identity guard

ADR-0037 retains the early handoff parent KEY SHARE guard under immutable initial/head/link
ancestry. Do not remove it or upgrade it to parent mutation authority. A reparenting or ancestry
migration must re-establish that mapping before using the guard. The focused ordered race is:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests --test fault_workflow_primitives \
  chains::parent_cancellation_and_a_child_hand_off_commit_in_either_order_and_the_latch_never_gets_lost \
  -- --exact --nocapture --test-threads=1
```

Use the repository disposable PostgreSQL fixture (unset SAGAT_TEST_DATABASE_URL). The retained
negative control failed with public Unavailable; that error alone does not diagnose a database
deadlock. Inspect database diagnostics when investigating failures. Evidence and restoration
hashes: tasks/project-completion/evidence/parent-fk-guard-20261018/. General nesting stays closed.


Exact-path mutation entry now uses the applicable chain-before-run prefix. For lock regressions,
run `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests --test integration_workflow_primitives bounded_fences -- --test-threads=3`
with the external database URL unset. The storage fixture combines scope and chain metadata only
to test locking; it does not open general publication or establish chain finality for scoped work.


A run-addressed declared Update remains pinned after continuation and acquires its membership
chain before that run. Use the `operations_http` workflow integration filter to verify successor
updates, RUN_CONTINUED, replay audits and rollback; an earlier run is never silently forwarded.
Evidence: tasks/project-completion/evidence/direct-update-chain-prefix-20261018/.


### Chain authority expiring behind a lock

Handoff and finalization reject a scheduler lease that expires while waiting for its row lock,
without durable chain/run/audit/outbox effects. The keeper supplies current authority on retry;
do not treat the stale attempt as successful. Existing scheduler continuation counters classify
SchedulerFenceLost as `scheduler_actions_total{kind="continuation",outcome="fenced"}` and
`scheduler_fenced_total{kind="continuation"}`; the private port barriers do not assert telemetry.
No payload or secret is added to those labels or diagnostics.

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests --test integration_workflow_primitives \
  post_wait_fence_ -- --nocapture --test-threads=3
```

Unset SAGAT_TEST_DATABASE_URL to use repository disposable PostgreSQL. The barrier observes
sagat_app when requested. Evidence includes setup failures separately from three reproduced
expired-authority commits and the final passing controls. Other primitive fences remain pending.


### Collection/loop authority expiry during lock wait (2026-10-18)

Refs:T014-040, FR-014-014/015. The collection and loop adapters retain their scheduler SHARE
guard in the owning transaction and validate expiry using a database clock sampled after
acquiring it. Wrong-shard and generation-conversion errors keep their existing precedence.
A rejected entry changes no projection, payload, quota, timer, journal, outbox or audit. A supplied
collection entry audit is inserted once through finish_version, atomically with the transition.
Private runtime-role entry barriers reproduce the stale commits and verify rejection, fresh
authority and replay. This is not a continuous wall-clock-until-commit guarantee or acceptance
of nested scopes. Approval's separate check and B0 ordering remain pending. Existing fenced
metrics use kind=collection/loop without tenant/run labels; the port tests do not test counters.
Evidence: tasks/project-completion/evidence/scope-post-wait-fence-20261018/.


### Approval scheduler authority after lock waits (2026-10-18)

Refs:T014-040, FR-014-008/014. Open/expire retain a scheduler SHARE guard in their owning
transaction and validate authority against a database clock sampled after acquiring it. An
expired authority returns the existing typed fence error before request/timer/projection, journal,
outbox, audit or telemetry queue effects. Oversized-generation storage errors keep precedence;
decide uses its current-user authorization. No business-deadline clock or opening replay contract
changes. Real runtime-role barriers cover both opening and genuinely due expiry; the atomic
checkpoint includes the telemetry queue and approval tables. Existing fenced metrics use
approval_open/approval_expire kinds without tenant/run labels; these port tests do not assert
counters. This does not enable nested scopes or complete B0.
Evidence: tasks/project-completion/evidence/approval-post-wait-fence-20261018/.


### Approval member-chain writer guard (2026-10-18)

Refs:T014-040, FR-014-008/014. Approval open/expire/decide acquire a tenant/namespace-scoped
applicable membership chain before the exact own run, retaining both guards through commit.
The shared transaction helper is the existing declared Run Update guard; it never forwards or
grants head authority. Initial membership commits with new-run creation and later identity is
immutable; live reparent/migration requires its own protocol. Decision authorization, typed
errors, terminal/replay and audit/outbox boundaries remain unchanged. Native wait/final-audit
barriers verify ordering and guard lifetime; no new public composition or B0 completion is
inferred. Existing outcome metrics and sensitive-data-free logs keep their current mapping.
Evidence: tasks/project-completion/evidence/approval-chain-prefix-20261018/.


### Collection/loop member-chain serialization (2026-10-18)

Refs:T014-040, FR-014-004/006/014. Collection entry/drain and loop writer helpers retain an
applicable scoped membership chain before own-run UPDATE in their owning transaction. Existing
CAS, fences, quota/deadline/drain rules, typed errors, audit/outbox and telemetry mapping stay
unchanged. Native app-role barriers prove chain wait precedes own-run acquisition; fresh/replay
controls cover persisted writes. Shared guard lifetime is separately proven by the approval
final-audit barrier. These trusted storage fixtures grant no new public composition, head
authority or nested eligibility; complete B0/group and resource gates remain pending.
Evidence: tasks/project-completion/evidence/scope-chain-prefix-20261018/.


### Production incident lock prefix (ADR-0038,2026-10-18)

Refs:T014-040, FR-009-004/005/008, FR-014-014. Production attribution now takes applicable
member chain→exact own run→binding EXCLUSIVE→own step→audit deduplication, refining ADR-0022's
reproduced opposite report/binding edge. Current tenant/namespace/fence/expiry/binding checks,
redacted closed categories, deduplication and commit-before-ACK remain unchanged. No execution
state/outbox or new capability is introduced; test-run attribution is unchanged. Native runtime
barriers cover the actual report storage cycle (PostgreSQL deadlocks0→1 before correction) and
a genuinely published/leased script chain. Completion/journal consistency are positive controls.
Reproduce with runtime option and external DB unset:
`cargo test --locked -p sagat-tests --test integration_security_multitenancy incident_lock_order_`.
Existing script_incidents_total/log mappings remain after commit; port tests do not assert counters.
Exact source, review and regression limits: tasks/project-completion/evidence/incident-lock-order-20261018/.
Complete B0/group and release gates remain open.


### Control entry admission on the 0.3 development branch

A PENDING condition/switch entry is scheduler-owned: no activity, ready hint, worker lease
or task.ready outbox belongs to its admission. Inspect the qualified admission journal,
collection membership and scheduler.action spans; durable collection candidates provide
recovery after a lost hint or cold cache. Existing collection admission/checkpoint counters
remain applicable. The current iteration_concurrency policy stays positive; a reduction to
its minimum 1 blocks new slots while accepted controls/checkpoints still progress.

Migration 20261221000000 distinguishes ACTIVITY/SCHEDULER membership. Downgrade refuses any
retained SCHEDULER admission, including terminal history and an immutable journal marker
whose projection was subsequently relabeled. Do not edit that evidence to force rollback.
Older schedulers cannot execute the new shapes: fence/drain their shard authority before
admitting new 0.3 definitions. A compatible0.3 reader is required for retained new runs.
Released-profile upgrade/active-run evidence and the complete release gates remain pending.

Reproduce `collection_control_entry::`, `collection_control_admission::` and common SDK case
`collection-control-entry`. Frozen OrdinaryLeafV1 certification still rejects both control
entry kinds, separately from public runtime support.


### 0.3 scoped timer diagnosis

A collection Timer is PENDING until its clocked scheduler transaction records timer.started.
WAITING owns waiting_kind=TIMER/due_at and no worker capability. A due timer may record firing
while paused; SUCCEEDED/unrouted is durable deferred work discovered on resume. Cancellation
clears a validated waiting timer in the member drain transaction, without compensation for
the timer itself. scheduler.action spans correlate tenant/run/qualified instance without item
data; existing finite timer action counters retain their labels. Do not repair malformed
timer projections by bypassing validation: readers/writers refuse them.

Reproduce collection_timers:: and collection_timer_transactions::, plus SDK collection-timers.
The native tests age test deadlines explicitly and order cancellation/fire with held locks.
This development capability requires compatible0.3 authority; released-profile and complete
regression/resource gates remain open. Frozen OrdinaryLeafV1 remains activity-only.
