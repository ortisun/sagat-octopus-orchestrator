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
documents. The server accepts a restricted collection graph made from one or more root `forEach`
nodes ending at `end`; every referenced body must contain exactly one activity followed by one
`return`. Before starting work, an administrator must set the tenant-wide active iteration budget:

```sh
curl -X PUT "$API/v1/quotas/iteration_concurrency" \
  -H "authorization: Bearer $ADMIN" -H 'content-type: application/json' \
  -d '{"limit": 32}'
```

Without this setting, the scheduler fails closed before opening a scope or dispatching body work.
The setting is audited, ceiling checked and shared by collection and loop iterations across every
namespace in the tenant. Its default operator ceiling is 1,024 active iterations; raising that
ceiling is a separate operator action.

The server also accepts a bounded root approval with basic nodes and two closed continuation graphs: `activity -> continueAsNew`, and
`activity -> condition -> (continueAsNew | end)`. The direct form may transfer the exact declared
`state` snapshot with its input. The conditional form remains input-only; bodies, top-level limits,
triggers and wider graphs fail publication. Run the current builder
parity checks with:

```sh
SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=collection-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-runtime cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=chain-query cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

The `collection-runtime` and `loop-runtime` scenarios execute their restricted public shapes
through PostgreSQL, the scheduler and worker protocol 1.5 in every language. `chain-query` starts
a public stateful chain and queries its initial head in every language. The builder commands
compare authored documents only.

The real PostgreSQL continuation checks publish through the normal codec, start through the public
run service, complete activities through the worker boundary and let a fenced scheduler tick move
the chain head. They cover the direct stateful snapshot, a successful conditional final outcome and terminal `CHAIN_LIMIT`
without rediscovery:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked chain_handoff -- --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked chains_http -- --nocapture
```

The five-SDK chain command scenario remains pending because its signal-controlled wait graph is
outside the published continuation subset. Typed inspect and cancel can address chains created by
the reviewed server shapes; signal has only its negative no-matching-wait path there. The pending
fixture is not runtime proof.

The chain Query REST route resolves declared state on an existing chain. The five-SDK `chain-query`
scenario exercises this on a public initial head; Query after public handoff and chain Update
runtime parity still need coverage.
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
concurrent-run expiry semantics are verified. A declared state document combined with a
collection, loop, approval or continuation node is still rejected at publication.

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
does not admit another iteration, and no worker lease is retained during the delay. The full
zero-worker production scenario remains a release gate until planner and worker wiring are enabled.

The private durable drain boundary can be checked independently while the public loop gate remains
closed:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loop_progress::expression_failure_drains_once_restarts_and_finishes_from_the_journal -- --exact --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loop_progress::deadline_cancels_a_suspended_timer_and_fences_the_stale_fire -- --exact --nocapture
cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  loop_progress::cancellation_drains_the_active_iteration_and_finishes_after_restart -- --exact --nocapture
```

These checks prove the journal/projection, timer, capacity, audit, outbox and replay boundaries of
the private adapter. They are not public runtime evidence. Carry and result payload retention must
join the same transaction before loop publication can be enabled; otherwise a revision or
cancellation race can retain content without committing the corresponding loop transition.

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

## Evidence boundary

The report lists only commands actually executed. US01 success does not close SC-014-002–006,
prove crash recovery for new primitives, accept the proposed ADR or make its contracts available.
Future implementation tasks stay unchecked until their named tests and documentation exist.
