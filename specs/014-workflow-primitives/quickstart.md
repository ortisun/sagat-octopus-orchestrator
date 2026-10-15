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

## Proposed primitives — implementation required

Do not paste the proposed node names or endpoints from `contracts/README.md` into a v1 workflow.
They are not available. T014-003 will register the future test targets below; until then these
commands intentionally describe future validation, not commands reported as runnable or passing:

```sh
cargo test -p sagat-tests --test contract_workflow_primitives --locked
cargo test -p sagat-tests --test integration_workflow_primitives --locked
cargo test -p sagat-tests --test fault_workflow_primitives --locked
cargo test -p sagat-tests --test e2e_workflow_primitives --locked
```

The release operator follows these scenarios after implementing the relevant unchecked tasks:

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
5. **Query/Update** (only after the value-disclosure decision is accepted): read state revision R,
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
