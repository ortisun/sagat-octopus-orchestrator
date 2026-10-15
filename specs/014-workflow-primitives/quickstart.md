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
documents. These builder fixtures check authoring only. The server accepts a bounded root
approval with basic nodes; collection, loop and continuation nodes still fail publication. Run
the current builder parity checks with:

```sh
SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored
```

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
idempotency and recovery checks. A declared state document combined with a collection, loop,
approval or continuation node is still rejected at publication.

The PostgreSQL integration target now verifies atomic collection checkpoint/finish,
addressed journal restoration, declared-operation receipts, loop delay checkpoints and chain
membership constraints. Scheduler ticks now discover open collection scopes and advance their
admission/checkpoint under a real fence, but no published workflow can create those scopes yet;
production scoped workers also remain disabled. To
inspect a delayed loop checkpoint after a restart, run:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives \
  loop_progress::delay_checkpoint_restart_and_duplicate_timer_fire_are_atomic --locked -- --nocapture
```

The test asserts that the timer and journal checkpoint survive reload, a duplicate timer fire
does not admit another iteration, and no worker lease is retained during the delay. The full
zero-worker production scenario remains a release gate until planner and worker wiring are enabled.

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
