# Validation guide: workflow primitives and examples

## Runnable in this delivery

The order examples use the current v1 engine and SDKs. Follow
[`examples/order-processing/README.md`](../../examples/order-processing/README.md) for connection
variables, local token, publishing, activity binding and per-language preparation. It includes
three workflows and five cases: accepted routing (amount 100), rejected routing (amount -1),
fulfillment, reminder and reminder with NOTIFICATION_REJECTED. Rejected routing is business rejection
with a SUCCEEDED run; injected notification failure is a FAILED run and process exit 1.

Prerequisites: the repository's pinned Rust toolchain, Docker and current supported TypeScript/Go/
Python/.NET/Java toolchains; dependency downloads for first preparation. Use environment credentials,
not source literals. Activity implementations simulate inventory, quotation and notification; they
are not provider integrations and do not prove external-effect idempotency.

From the repository root:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
cargo test -p sagat-tests --test contract_developer_experience --locked
SPECIFY_FEATURE_DIRECTORY=specs/014-workflow-primitives SPECIFY_FEATURE_NO_PERSIST=1 \
  .specify/scripts/bash/check-prerequisites.sh --json --require-spec --require-tasks --include-tasks
```

The first command executes all five languages unless `SAGAT_EXAMPLE_LANGS` deliberately selects a
subset. A subset cannot close five-language parity. Expected evidence is the real engine's status,
activity/journal order and traces for all five cases; inspect the harness's assertions and
[validation report](./validation-report.md) for actual execution counts and environment. A CLI
schema validation or language compilation alone is insufficient. The second command checks the
existing developer-experience contracts/docs; it does not prove future runtime primitives.

## Additive v1 capabilities

The workflow document stays at `sagat-octopus-orchestrator.io/v1`. A bounded `switch` has ordered
`cases` and a mandatory `default`; predicate errors fail the run with `EXPRESSION_ERROR`. A case
selection is recorded in the journal and projected into the runtime graph. For example:

```json
{
  "apiVersion": "sagat-octopus-orchestrator.io/v1",
  "kind": "Workflow",
  "metadata": {"name": "route-order"},
  "entrypoint": "choose",
  "nodes": [
    {"id": "choose", "type": "switch", "cases": [
      {"id": "priority", "when": "input.amount >= 100", "target": "priority"},
      {"id": "positive", "when": "input.amount > 0", "target": "accepted"}
    ], "default": "rejected"},
    {"id": "priority", "type": "end"},
    {"id": "accepted", "type": "end"},
    {"id": "rejected", "type": "end"}
  ]
}
```

With `amount: 100`, both predicates are true and the `priority` case wins. With `amount: -1`, the
default route wins. A missing `amount` fails evaluation; no later case or default is tried. The
switch integration target exercises these outcomes against PostgreSQL and checks journal/replay.

The five SDKs can construct additive v1 collection, bounded-loop, approval and continuation
documents. The server accepts a restricted collection graph with an optional ordinary source
activity prefix followed by one or more root `forEach` nodes ending at `end`; each body contains a
bounded acyclic graph with an activity entry, one to eight executable nodes (activities,
conditions, switches) and one `return`. The 0.3 increment adds local `onError` recovery and
recorded local decisions. Control-node entry, timers, child workflows and nesting remain
separately gated. Before starting work, an administrator must set the tenant-wide active iteration budget:

```sh
curl -X PUT "$API/v1/quotas/iteration_concurrency" \
  -H "authorization: Bearer $ADMIN" -H 'content-type: application/json' \
  -d '{"limit": 32}'
```

Without this setting, the scheduler fails closed before opening a scope or dispatching body work.
The setting is audited, ceiling checked and shared by collection and loop iterations across every
namespace in the tenant. Its default operator ceiling is 1,024 active iterations; raising that
ceiling is a separate operator action.

The server also accepts a bounded root approval with basic nodes and three closed continuation graphs: `activity -> continueAsNew`,
`activity -> condition -> (continueAsNew | end)`, and `activity -> waitEvent -> continueAsNew`.
The direct and conditional forms may transfer the exact declared `state` snapshot with their input.
The wait form transfers input only; bodies, triggers and wider graphs fail publication. A top-level
`limits` block may lower `maxInstances` and `maxDepth` in any of these shapes, never raise them. Run the current builder
parity checks with:

```sh
SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=collection-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-cancel cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-error-activity cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-query cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-query-handoff cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-final-success cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-state-final-success cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-commands cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-three-segments cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-child cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

The `collection-runtime` and `loop-runtime` scenarios execute their restricted public shapes
through PostgreSQL, the scheduler and worker protocol 1.5 in every language. `chain-query` starts
a public stateful chain and queries its initial head in every language. `chain-query-handoff`
completes the predecessor through worker protocol 1.5, queries the successor's transferred state,
updates that state, replays the update key and rejects stale predecessor fences for both commands
in every language. It distinguishes the predecessor's segment wait from a chain wait, verifies
the latter times out without mutating state, then cancels the chain and receives its final outcome.
`chain-final-success` demonstrates a normal two-segment chain that ends `SUCCEEDED` without a
cancel command or another continuation.
`chain-three-segments` runs three segments in every language: one pinned version and one original
deadline, a segment wait that reads `SUCCEEDED` with `CONTINUED` while the chain wait reads the final
outcome, a paged list of the chain's runs, and refusals for writes aimed at a continued segment.
`chain-child` runs a chain as the child of another run: the parent waits for the final outcome only.
`chain-state-final-success` adds declared state to that conditional shape. The successor input
holds `0.00`, while its final Query returns the transferred `19.99` in all five SDKs.
The builder commands compare authored documents only.

The real PostgreSQL continuation checks publish through the normal codec, start through the public
run service, complete activities through the worker boundary and let a fenced scheduler tick move
the chain head. They cover the direct stateful snapshot, a successful conditional final outcome and terminal `CHAIN_LIMIT`
without rediscovery:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked chain_handoff -- --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked chains_http -- --nocapture
```

`chain-commands` runs a worker activity, waits for a signal, replays the same signal receipt,
observes the committed successor head, rejects commands fenced to the predecessor, and cancels
the successor. Its runtime golden passes in all five SDKs. `chain-root-wait` publishes a continuation whose
entrypoint is itself `waitEvent` (`waitEvent -> continueAsNew`, or one condition choosing between that
continuation and an `end`): the hand-off creates each successor already waiting, so the correlation key, the
wait deadline and the event subscription commit with it and a signal sent to the new head is delivered at
once. `chain-cancel-parent` cancels a parent while its child chain is mid-chain (and shows a detached chain
keep going) and `chain-rerun` starts a continued segment again with `RERUN_AS_NEW`; their goldens pass in all
five SDKs. The PostgreSQL proof is `chain_wait` (integration) and the `wait`-chain tests of `chains` (fault):
```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked chain_wait -- --nocapture
cargo test -p sagat-tests --test fault_workflow_primitives --locked chains:: -- --nocapture
```

The chain Query REST route resolves declared state on an existing chain. The five-SDK `chain-query`
scenario exercises this on a public initial head. The five-SDK `chain-query-handoff` scenario
exercises Query and Update on the public successor after stateful handoff, including exact Update
replay and a stale expected-head Query. Broader race and fault combinations remain open.
For a chain with declared state, the request shape is:

```sh
curl -X POST \
  "$API/v1/namespaces/$NAMESPACE_ID/chains/$CHAIN_ID/queries/balance" \
  -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d "{\"args\":{},\"expectedRunId\":\"$CURRENT_RUN_ID\"}"
```

The token needs `run:read` and `run:query`. `expectedRunId` is optional; supplying it prevents an
unnoticed handoff and returns `409` with `details.reason=CHAIN_HEAD_CHANGED` when the committed head
has changed. Inspect the chain again before deciding whether to retry. A successful response names
the actual run, run version and state revision used for the result and carries
`Cache-Control: no-store`. The server locks the chain before the run snapshot, so a concurrent
handoff returns one complete old-head or successor snapshot. It never combines the two and does not
append a workflow event or schedule work.

Chain Update uses the same declared evaluator and addresses the current head under an exclusive
chain-before-run lock. `expectedRunId` is optional. A new key with an obsolete expected head returns
`409 CHAIN_HEAD_CHANGED`; an exact retry uses the same presence/value and returns its original
applied or rejected receipt even after rollover:

```sh
curl -X POST \
  "$API/v1/namespaces/$NAMESPACE_ID/chains/$CHAIN_ID/updates/balance.replace" \
  -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"updateKey":"replace-27","args":{"allowed":true,"nextState":{"balance":27}},"expectedRunId":"'$CURRENT_RUN_ID'"}'
```

The response names the run that actually committed the receipt and carries `Cache-Control:
no-store`. A run-addressed Update remains bound to that exact run and never forwards. Operation
arguments and results are each limited to 256 KiB; the existing 4 MiB retained run-data and 8 MiB
task-context limits still apply. Evaluation uses immutable closed schemas and receives no secret or
network capability. Run the focused public and handoff/replay checks against real PostgreSQL with:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives \
  chain_query_ --locked -- --nocapture --test-threads=1
cargo test -p sagat-tests --test integration_workflow_primitives \
  chain_update_ --locked -- --nocapture --test-threads=1
```

These chain operation checks use a retained compatibility fixture. Direct public continuation now
transfers declared state, but the combined public Query/Update path remains to be exercised.

Run-addressed declared Query/Update is available for workflows made from the existing basic
root nodes. The shared runtime scenario publishes one such workflow, starts a run, applies and
replays an update, queries the resulting revision, and checks rejection and tenant opacity in all
five SDKs:

```sh
SAGAT_SDK_SCENARIO=state-operations cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-decisions cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

The `approval-decisions` scenario also publishes and starts a root approval workflow, discovers
the pending request and checks authorized approve/reject, replay, conflicting keys and tenant
opacity in all five SDKs. See
[`workflow-primitives.md`](../../docs/runbooks/workflow-primitives.md) for the current API paths,
idempotency and recovery checks. Approval combined with parallel execution is rejected until
concurrent-run expiry semantics are verified. A declared state document combines with a native
approval and, with the shapes listed below, with an approval inside a continuation chain (decision
D295, next section). A declared state document combined with a collection or loop node is still
rejected at publication, and so is an approval inside a collection or loop body.

The PostgreSQL integration target verifies public collection publication/start, missing-policy
fail-closed behavior, audited capacity provisioning, scoped worker leasing, ordered checkpoint,
terminal finish and journal/projection replay. It also verifies declared-operation receipts, loop
delay checkpoints and chain membership constraints. Worker protocol 1.5 carries the complete
collection instance identity; protocol 1.4 remains root-only. To
inspect a delayed loop checkpoint after a restart, run:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives \
  loop_progress::delay_checkpoint_restart_and_duplicate_timer_fire_are_atomic --locked -- --nocapture
```

The test asserts that the timer and journal checkpoint survive reload, a duplicate timer fire
does not admit another iteration, and no worker lease is retained during the delay. The public
round trip of a `delayMs` loop with no worker connected while it waits, and its failure cases, are
in the section "A loop that waits, is cancelled, or recovers" below.

The restricted public loop shape is published (a loop chain ending at `end`, with a body of one to eight
activities and one `return`; see `contracts/README.md`). A loop failure drains the active iteration
first and then follows the loop's `onError` edge; without an `onError` edge it fails the run after
the drain. A cancelled run still ends `CANCELLED` and never takes the edge. Publication accepts an
`end` or a chain of recovery activities ending at an `end` as the `onError` target, and an
expression error while entering the loop takes the edge too. The drain and edge boundaries can be
checked independently:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loop_progress::expression_failure_drains_once_restarts_and_finishes_from_the_journal -- --exact --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loop_progress::deadline_cancels_a_suspended_timer_and_fences_the_stale_fire -- --exact --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loop_progress::cancellation_drains_the_active_iteration_and_finishes_after_restart -- --exact --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loop_progress::permanent_body_failure_with_an_error_edge_routes_the_recovery_activity -- --exact --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loops_http::public_loop_body_failure_routes_the_error_edge_and_the_run_completes -- --exact --nocapture
cargo test -p sagat-tests --test fault_workflow_primitives --locked loops:: -- --test-threads=2
```

The storage checks prove the journal/projection, timer, capacity, audit, outbox and replay
boundaries, including restarts and duplicate deliveries at every drain step and the rollback of
the edge when its outbox or audit record cannot be stored. The `loops_http` checks run the same
failures through publication, the scheduler and a worker over HTTP and gRPC. Carry and result
payloads are retained in the same transaction as the transition that records them.

The other proposed node names and endpoints in `contracts/README.md` remain unavailable until their
runtime, fault and security gates pass. These targets are registered, but their presence does not
mean every primitive is implemented:

```sh
cargo test -p sagat-tests --test contract_workflow_primitives --locked
cargo test -p sagat-tests --test integration_workflow_primitives --locked
cargo test -p sagat-tests --test fault_workflow_primitives --locked
cargo test -p sagat-tests --test e2e_workflow_primitives --locked
```

The release operator follows the remaining scenarios after implementing the relevant unchecked tasks:

1. **Collection**: process 17 items with concurrency 3; release worker completions in reverse order;
   expect ordered results, exactly 17 logical iteration records and never more than 3 active slots.
   Repeat with zero items, 10,001 items, one permanently failing body and two competing tenant runs.
2. **Loop**: pre-test false yields zero iterations, post-test true yields one, three-step carry yields
   three. Restart after checkpoint two and verify only iteration three remains. Exhaust iteration,
   run-data and deadline budgets. No worker stays leased for between-iteration delay.
3. **Approval**: open, inspect and restart while pending; unauthorized/revoked/self decisions fail;
   valid approve/reject routes work; response/expiry/cancel races have one outcome. A raw signal cannot
   approve. Read the durable audit and absence of sensitive policy details in denied responses.
4. **Switch**: two true cases pick the first; no match picks default; malformed predicate fails;
   restart after chosen-case commit preserves that exact choice.
5. **Query/Update**: read state revision R,
   apply an update with expected R, then duplicate it after a lost response. State changes once and
   returned receipt is stable. Two updates expecting R produce one application and one
   `CONFLICT`/`REVISION_CONFLICT`; an activity completing in between does not cause a conflict. Query
   sees one consistent snapshot, including on a retained terminal segment, without changing the
   workflow journal, and an update schedules no task.
6. **Continue**: run three segments; inspect predecessors and current head; explicit chain wait stops
   only at FINAL, while a segment wait returns SUCCEEDED/CONTINUED. Race old-run and chain-target
   commands against handoff and verify their different addressing/deduplication contracts. Parent of
   a continuing child waits for FINAL; chain cancellation prevents further handoffs.
7. **Failure gate**: execute every barrier in `contracts/conformance.md` with a real database, worker
   and scheduler restart; check persisted records independently and confirm no acknowledged work
   vanished when broker hints were dropped.
8. **Compatibility/security gate**: run all five shared conformance runners plus old-minor/old-IR
   cases; run cross-tenant, role-revocation, secret-marker, unsupported-handler and resource-limit cases.
9. **Measurements**: run the three workloads in `plan.md`, retaining environment, concurrency,
   memory/journal growth, schedule latency, transaction/lock wait and failures. Zero per-wait worker
   ownership and enforced limits are assertions; capacity/latency numbers remain measurements.

Complete standard formatting, Clippy, locked workspace, architecture, contract compilation, Compose
smoke and dependency advisory gates before merge of engine changes. Rollback first disables new
primitive publication/start; keep readers for in-flight runs using the new capabilities until they drain.

## A loop that waits, is cancelled, or recovers (T014-019..021)

A `while` or `repeatUntil` loop may wait between iterations with `delayMs`. While it waits the
engine holds one durable timer and nothing else: no worker is leased, no task is ready, and the
scheduler wakes the loop when the timer is due. These demos run against a deployment whose
scheduler is running; `$API`, `$ADMIN` (an admin token) and a namespace `payments` are assumed, and
the worker is any SDK worker on protocol 1.5 that serves `counter.increment` (returns the number
`0` the first time and `1` after) and, for the recovery demo, `recover.cleanup`.

Admit iterations once per tenant, then publish the happy-path loop (two iterations, 30 s apart):

```sh
curl -X PUT "$API/v1/quotas/iteration_concurrency" -H "authorization: Bearer $ADMIN" \
  -H 'content-type: application/json' -d '{"limit": 4}'

cat > delay-loop.json <<'JSON'
{"name": "delay-loop", "ir": {
  "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
  "metadata": {"name": "delay-loop"}, "entrypoint": "poll",
  "bodies": {"iteration": {"entrypoint": "increment", "nodes": [
    {"id": "increment", "type": "activity", "activity": "counter.increment", "next": "return-value"},
    {"id": "return-value", "type": "return", "result": "steps.increment.output"}]}},
  "nodes": [
    {"id": "poll", "type": "while", "condition": "carry < 1", "body": "iteration",
     "initial": "0", "carry": "bodyResult", "maxIterations": 3,
     "delayMs": 30000, "timeoutMs": 600000, "next": "done"},
    {"id": "done", "type": "end", "outputExpr": "steps.poll.output"}]}}
JSON
WF=$(curl -s -X POST "$API/v1/workflows" -H "authorization: Bearer $ADMIN" \
  -H 'x-octopus-namespace: payments' -H "idempotency-key: $(uuidgen)" \
  -H 'content-type: application/json' -d @delay-loop.json | jq -r .id)
curl -X POST "$API/v1/workflows/$WF/publish" -H "authorization: Bearer $ADMIN" \
  -H 'x-octopus-namespace: payments' -H "idempotency-key: $(uuidgen)" \
  -H 'content-type: application/json' -d '{"draftRevision": 1}'
RUN=$(curl -s -X POST "$API/v1/runs" -H "authorization: Bearer $ADMIN" \
  -H 'x-octopus-namespace: payments' -H "idempotency-key: $(uuidgen)" \
  -H 'content-type: application/json' -d "{\"workflowId\": \"$WF\", \"input\": {}}" | jq -r .runId)
```

**Happy path, no worker during the wait.** Let the worker complete the first body with `0`, then
stop the worker (`SIGTERM`). Within a few seconds the run shows only a timer:

```sh
curl -s "$API/v1/runs/$RUN" -H "authorization: Bearer $ADMIN" | jq .status     # "WAITING"
curl -s "$API/v1/runs/$RUN/events" -H "authorization: Bearer $ADMIN" \
  | jq -r '.items[].type'     # ... loop.iteration_checkpointed, and no step.leased after it
```

Wait out the 30 s with no worker running. The scheduler fires the timer by itself: the journal gains
exactly one `loop.delay_elapsed` and the next iteration becomes ready (`READY`, nothing leased). Start a
worker again; it takes iteration 1 (`instancePath` `[{"scopeNodeId":"poll","iterationIndex":1}]`),
completes it with `1`, the condition `carry < 1` turns false and the run ends `SUCCEEDED`. Expected: one
`loop.delay_elapsed`, two admitted iterations, tenant iteration usage back to zero.

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loops_http::a_while_loop_waits_out_its_delay_with_no_worker_connected -- --exact --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loops_http::a_repeat_until_loop_waits_out_its_delay_with_no_worker_connected -- --exact --nocapture
SAGAT_SDK_SCENARIO=loop-semantics cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

**Failure path 1, the budget runs out during the delay.** Publish the same document with
`"delayMs": 5000, "timeoutMs": 9000` and `"onError": "failed"` plus an `end` node `failed`, complete
the first body, stop the worker and wait 10 s. The timer is cancelled, the loop ends `FAILED` with
`RUN_TIMED_OUT` and the run follows the edge to `failed` (`SUCCEEDED`). With `delayMs` larger than
the remaining budget (for example `30000` against `timeoutMs` `6000`) no timer is created at all: the
iteration waits and the loop times out at its deadline (D402). Without `onError` the run ends `FAILED`.

**Failure path 2, cancel while it waits or while a body runs.**

```sh
curl -X POST "$API/v1/runs/$RUN/operations" -H "authorization: Bearer $ADMIN" \
  -H 'x-octopus-namespace: payments' -H "idempotency-key: $(uuidgen)" \
  -H 'content-type: application/json' \
  -d '{"action":"CANCEL","reason":"stop the loop while it waits"}'      # 202, runStatus CANCELLING
```

The scheduler cancels the timer (or asks the leased body attempt to stop: its next heartbeat answers
`cancelRequested`), drains the iteration and ends the loop and the run `CANCELLED`; no further
iteration is admitted and tenant iteration usage returns to zero. `PAUSE` holds the run: a delay that
falls due while paused admits nothing until `RESUME`.

**Failure path 3, a loop that fails routes to an activity.** Add
`"onError": "recover"` to the loop and two nodes
`{"id":"recover","type":"activity","activity":"recover.cleanup","next":"recovered"}` and
`{"id":"recovered","type":"end"}`. When a body fails for good (or the loop times out, or its initial
expression cannot be computed) the active iteration drains, the loop ends `FAILED` with the code and
a worker takes `recover.cleanup` as an ordinary root task; the run ends `SUCCEEDED` through `recovered`.
A recovery activity declares no `onError`, `compensateWith`, `reconcileWith`, `irreversible` or
`highRisk`, and an activity after the loop's success edge is still refused at publication.

Faults the suites run against real PostgreSQL (restart of the scheduler during the delay, a duplicate
fire by two replicas under one fence, a replica that lost its shards, a cancel racing the timeout in
both orders, a signal during the delay):

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked loops_http:: -- --test-threads=2
cargo test -p sagat-tests --test fault_workflow_primitives --locked loops:: -- --test-threads=2
SAGAT_SDK_SCENARIO=loop-cancel cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-error-activity cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored

## An approval with declared state, and an approval inside a chain

Decision D295: an approval node combines with declared run state and with a continuation chain. It does
not combine with a collection or loop body; publication refuses that with `UNSUPPORTED_NODE` and
`details.dependency: SCOPED_COMPENSATION`, naming what has to exist first.

**Approval and state.** The state belongs to the declared Query and Update calls. While the request is
open the run is `WAITING` and still mutable; the decision, an expiry or a cancel then end the run with
the state exactly as the last Update left it, readable but no longer mutable (`RUN_NOT_MUTABLE`). The
approval's routes are the decision alone: no node expression reads the state.

```sh
# 1. Publish a workflow with `state` and an `approval` node (see tests/support/approval_flow.rs for the
#    document: review -> approved | rejected | expired, state {balance}, balance.read, balance.replace).
WF=$(curl -s -X POST "$API/v1/workflows" -H "authorization: Bearer $ADMIN" -H "x-octopus-namespace: payments" \
  -H 'content-type: application/json' -H "idempotency-key: $(uuidgen)" \
  -d "{\"name\":\"review-with-state\",\"ir\":$(cat review-with-state.json)}" | jq -r .id)
curl -s -X POST "$API/v1/workflows/$WF/publish" -H "authorization: Bearer $ADMIN" -H "x-octopus-namespace: payments" \
  -H 'content-type: application/json' -H "idempotency-key: $(uuidgen)" -d '{"draftRevision":1}'
# 2. Start a run as the initiator, then read the request the scheduler opened.
RUN=$(curl -s -X POST "$API/v1/runs" -H "authorization: Bearer $ADMIN" -H "x-octopus-namespace: payments" \
  -H 'content-type: application/json' -H "idempotency-key: $(uuidgen)" \
  -d "{\"workflowId\":\"$WF\",\"input\":{\"account\":{\"balance\":100}}}" | jq -r .runId)
APPROVAL=$(curl -s "$API/v1/runs/$RUN/approvals" -H "authorization: Bearer $OPERATOR" | jq -r '.items[0].approvalId')
# 3. Change the state while the request is open (the revision fences concurrent callers) ...
curl -s -X POST "$API/v1/runs/$RUN/updates/balance.replace" -H "authorization: Bearer $ADMIN" \
  -H 'content-type: application/json' \
  -d '{"updateKey":"raise-0001","expectedStateRevision":0,"args":{"allowed":true,"nextState":{"balance":250}}}'
# 4. ... and decide it as an eligible person who is not the initiator.
curl -s -X POST "$API/v1/approvals/$APPROVAL/decisions" -H "authorization: Bearer $OPERATOR" \
  -H 'content-type: application/json' -d '{"decisionKey":"approve-0001","decision":"APPROVE","reasonCode":"POLICY_OK"}'
curl -s -X POST "$API/v1/runs/$RUN/queries/balance.read" -H "authorization: Bearer $ADMIN" \
  -H 'content-type: application/json' -d '{"args":0}'   # 250, stateRevision 1, after the run ended
```

**Approval and a chain.** The published shape is an entry activity, then an approval whose approval
routes to the `continueAsNew` node and whose rejection and expiry route to `end` nodes; the exact declared
state may travel with the continuation. Each segment opens an approval of its own. The state an Update
wrote while a request was open is what the successor starts with (read and update it through the chain
address); cancelling the chain, or the chain's original deadline, settles an open request as `CANCELLED`
and a later decision answers `ALREADY_DECIDED`.

```sh
# `work` (activity) -> `review` (approval) -> approve: `rollover` (continueAsNew) | reject, expire: `end`.
curl -s "$API/v1/namespaces/$NS/chains/$CHAIN" -H "authorization: Bearer $ADMIN"            # head, segments
curl -s "$API/v1/runs/$(jq -r .currentRunId head.json)/approvals" -H "authorization: Bearer $OPERATOR"
curl -s -X POST "$API/v1/namespaces/$NS/chains/$CHAIN/updates/balance.replace" -H "authorization: Bearer $ADMIN" \
  -H 'content-type: application/json' \
  -d "{\"updateKey\":\"during-review-0001\",\"expectedRunId\":\"$HEAD\",\"args\":{\"allowed\":true,\"nextState\":{\"balance\":250}}}"
curl -s -X POST "$API/v1/namespaces/$NS/chains/$CHAIN/cancel" -H "authorization: Bearer $ADMIN" \
  -H 'content-type: application/json' -H "idempotency-key: $(uuidgen)" \
  -d "{\"expectedRunId\":\"$HEAD\",\"reason\":\"stop the review chain\"}"
```

Run the evidence for both combinations (real PostgreSQL, HTTP and scheduler; the scenarios need the five
SDK toolchains):

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked approval_combinations::
cargo test -p sagat-tests --test integration_workflow_primitives --locked approval_telemetry::
cargo test -p sagat-tests --test fault_workflow_primitives --locked approval_combinations::
SAGAT_SDK_SCENARIO=approval-state cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-chain cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

## Compensating a failed collection or loop

A collection or loop body may declare `compensateWith`, `reconcileWith`, `irreversible` and `highRisk`
on its activities. Happy path: nothing fails, nothing is compensated, and the run is `SUCCEEDED` as
before (the `collection-runtime`, `collection-ordering`, `loop-runtime` and `loop-semantics` scenarios
are unchanged). Failure path, one body `reserve -> charge -> ship` where `reserve` and `charge` declare
`compensateWith` and `ship` fails for good:

1. The iteration fails; the collection stops admitting iterations and drains the ones in flight
   (`collection.drain_started`).
2. The scheduler journals `scope.compensation_planned` and the worker receives `COMPENSATION` tasks for
   `charge` and then `reserve`, each carrying the iteration's `target_instance`.
3. The run is `COMPENSATING`, then `scope.compensated` is journaled, the scope finishes and the run ends
   `COMPENSATED` (or `PARTIAL` when an `irreversible` effect completed, or `COMPENSATION_FAILED`).

```sh
SAGAT_SDK_SCENARIO=collection-compensation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-compensation cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
cargo test -p sagat-tests --test integration_workflow_primitives --locked scope_compensation_http:: -- --test-threads=2
cargo test -p sagat-tests --test fault_workflow_primitives --locked scope_compensation:: -- --test-threads=2
cargo test -p sagat-domain --test scoped_compensation --locked
```

A compensation that keeps failing leaves the run in `MANUAL_REVIEW`. List it with
`GET /runs?status=MANUAL_REVIEW`, read the waiting instance from `GET /runs/$RUN/graph` and decide with
the iteration spelled out in `instancePath`:

```sh
curl -X POST "$API/v1/runs/$RUN/operations" -H "authorization: Bearer $OPERATOR" \
  -H 'content-type: application/json' -H 'idempotency-key: release-retry-0001' \
  -d '{"action":"RETRY_COMPENSATION","stepId":"reserve~c",
       "instancePath":[{"scopeNodeId":"lines","iterationIndex":1}],
       "reason":"the provider is back, retry the release"}'
```

See `docs/runbooks/workflow-primitives.md` for the operator procedure.

## Evidence boundary

The report lists only commands actually executed. US01 success does not close SC-014-002–006,
prove crash recovery for new primitives, accept the proposed ADR or make its contracts available.
Future implementation tasks stay unchecked until their named tests and documentation exist.


## Producing collection membership in an activity

An ordinary root activity can produce the array a later collection captures. Set the owner's
`items` expression to `steps.fetch.output`, where `fetch` is the producing activity. The value is
an inline array or a descriptor of a blob the producer committed through the worker blob protocol.
Merely returning an uploaded object's descriptor as inline JSON does not commit it or authorize
its use. An unavailable/uncommitted/foreign source fails with `BLOB_UNAVAILABLE` before membership
is admitted. A failed producer starts no collection.

The producer uses the existing root run/worker quotas; iteration capacity is checked when the
collection is admitted. Source activities must precede the collection chain and currently have no
`onError`, compensation, reconciliation, irreversible/high-risk policy or managed database operation.
Activities after a collection remain outside this publication subset. These restrictions preserve
the remaining broad graph/nesting tasks and do not change external-effect idempotency obligations.

The shared [collection-step-source scenario](../../tests/contract/sdk-conformance/scenarios/collection-step-source.json)
is an executable authoring/runtime example in all five SDKs. It preserves authored limits and
returns two distinct captured members to scoped workers without exposing the root producer's
outputs as their local `steps`:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 SAGAT_SDK_SCENARIO=collection-step-source \
  cargo test --locked -p sagat-tests --test sdk_conformance -- --ignored --nocapture
```

The public blob happy path, exact source-report replay and unavailable-source failure are exercised
with real PostgreSQL, REST publication and gRPC workers:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test --locked -p sagat-tests \
  --test integration_workflow_primitives collection_pipeline -- --test-threads=2 --nocapture
```


## Local collection recovery (0.3 increment)

An activity can declare `"onError": "recover"` while its ordinary `next` names a
separate success route. Both routes must stay in the same acyclic activity-only body
and reach its single `return`. A known failed attempt follows recovery once; an unknown
effect still uses its declared reconciliation/operator policy first. A recovery failure
stops admission, drains active work and compensates accepted effects with that failure code.
The runtime graph retains the qualified error edge for the affected iteration.

With Docker and a private `CARGO_TARGET_DIR` configured, the reproducible happy/failure,
reservation, native-refusal and full/frontier cases are:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked collection_recovery::
SAGAT_SDK_SCENARIO=collection-local-recovery cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

The common SDK fixture uses existing `onError` options in all five builders. A successful
recovery yields ordered results `[1, 2]`; a failed recovery reaches `COMPENSATED` with
`RECOVERY_FAILED` after the original reservation's compensation. These simulated effects
make no external-provider undo guarantee. This increment does not accept nested capacity
or the remaining large-workload release gates.

## Local collection decisions (0.3 increment)

After an entry activity, a body can evaluate `condition` or ordered `switch` nodes.
The predicate reads its exact `item`, `index` and local `steps` outputs. A control node
never leases a worker; its recorded answer, route and selected activity commit together.
For example, the following body notifies only positive items and returns every original item:

```json
{
  "entrypoint": "reserve",
  "nodes": [
    {"id": "reserve", "type": "activity", "activity": "line.reserve", "compensateWith": "line.release", "next": "choose"},
    {"id": "choose", "type": "condition", "expression": "item > 0", "ifTrue": "notify", "ifFalse": "returned"},
    {"id": "notify", "type": "activity", "activity": "line.notify", "next": "returned"},
    {"id": "returned", "type": "return", "result": "item"}
  ]
}
```

`collection-control-routing` constructs this shape with both condition and switch in all
five SDKs. Input `[1, -1]` produces `[1, -1]` with no notification for index1. A missing
predicate field records `EXPRESSION_ERROR`; the native failure demo drains and compensates
the actual reservation. A condition may instead declare a local `onError` recovery.

```sh
RUST_TEST_THREADS=1 cargo test -p sagat-tests --test integration_workflow_primitives --locked -- collection_controls:: collection_control_transactions::
SAGAT_SDK_SCENARIO=collection-control-routing cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

Bodies still require an activity entry, at most eight executable declared nodes and one
Return. Timers, children, scoped approval and nesting have separate pending admission/runtime
gates. A loop's body capability remains the existing linear activity/Return subset.
