# Quickstart (validation guide): flow control and step policies

**Nothing in this guide runs today.** It is the reproducible demo the Definition of Done requires once the
tasks are implemented: one happy path and one failure scenario per capability, on the local stack. Every
command that does not exist yet is marked *(proposed)*; the stack, CLI and worker commands are the
existing ones from `docs/quickstarts/README.md`.

## Prerequisites

- The repository's pinned toolchains, Docker, and the local stack:

  ```sh
  docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
  docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
  ```

- The four capabilities enabled for the local deployment *(proposed configuration, task T022-005)* and the
  flow-control digest key generated into the ignored local env file *(proposed, task T022-007)*.
- A `dev` namespace (seeded) and a `prod` namespace for the refusal checks.
- The demo workflow `order-intake` from
  [contracts/workflow-ir-fragment.schema.json](contracts/workflow-ir-fragment.schema.json) (`examples`),
  saved as `order-intake.workflow.json`, and a child handler workflow `notify-failure` whose only step is an
  activity `ops.notify`.

```sh
sagat workflow validate order-intake.workflow.json
sagat workflow publish order-intake.workflow.json
sagat worker dev --mocks order-intake.mocks.yaml --workflow order-intake.workflow.json --bind
```

`order-intake.mocks.yaml` answers `catalog.price` after 2 s with a fixed price, `crm.notify` after 5 s, and
fails `crm.notify` permanently when `input.failNotify` is true. (These are worker canned answers, the
existing `sagat worker dev --mocks` feature, not test-mode pins.)

## 1. Step cache (US01)

Happy path:

```sh
sagat workflow run order-intake --input '{"sku":"A-1","accountId":"acc-1"}' --wait
sagat workflow run order-intake --input '{"sku":"A-1","accountId":"acc-2"}' --wait
sagat workflow inspect <second-run>
```

Expected: the first run leases `price` (one attempt, about 2 s); the second has `step.cache_hit` followed by
`step.completed` for `price`, no attempt, and the same price. `GET /v1/namespaces/{id}/step-cache` lists one
entry with a 12-hex `keyLabel` and no value.

Failure scenario: invalidate and rerun.

```sh
sagat cache invalidate --workflow order-intake --node price --key '"A-1"' --reason "price list changed"  # (proposed)
sagat workflow run order-intake --input '{"sku":"A-1","accountId":"acc-3"}' --wait
```

Expected: the third run leases `price` again; the audit log has one `cache.invalidated` record without the
key value.

## 2. Early return (US04)

```sh
sagat workflow run order-intake --input '{"sku":"B-2","accountId":"acc-4"}' --await-result   # (proposed flag)
```

Expected: the command prints the result about 2 s after start while `sagat workflow inspect` still shows
`RUNNING` (`resultSource: EARLY_RETURN`); the run ends `SUCCEEDED` after `notify` with the same result.

Failure scenario: `--input '{"sku":"B-2","accountId":"acc-5","failNotify":true}'`. Expected: the early
result is returned, the run ends `FAILED`, and the result disclosure still returns the early value.

## 3. Failure handler (US03)

Using the failure run of section 2: the definition declares `onFailure: {workflow: notify-failure}`.
Expected journal order: `step.failed (notify)` → `failure_handler.started` (detail `pending_status: FAILED`) →
`child.started` (derived `onFailure~h`) → `child.finished` → `failure_handler.finished` (`outcome: SUCCEEDED`) →
`run.finished{FAILED}`. The child's input `failure` has `kind: STEP_FAILED`, `node: notify` and no message.

Namespace default handler, failure scenario with a crash:

```sh
sagat namespace failure-handler set --workflow notify-failure --reason "page the on-call team"   # (proposed)
# publish a copy of order-intake without onFailure as order-intake-plain, then:
sagat workflow run order-intake-plain --input '{"sku":"C-3","accountId":"acc-6","failNotify":true}'
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env restart execution-engine
```

Expected: exactly one `notify-failure` run with `handlerFor` = the failed run, whatever the restart timing;
`GET /v1/namespaces/{id}/failure-handler/dispatches` shows it `ADMITTED`. Unpublishing the handler's active
version before the failure instead leaves the dispatch retrying, then `DEAD` with a `failure_handler_dead`
alert; `redrive` admits it after the version is restored.

## 4. Concurrency by key (US02)

```sh
for i in 1 2 3; do sagat workflow run order-intake --input '{"sku":"D-4","accountId":"acc-7"}'; done
sagat workflow run order-intake --input '{"sku":"D-4","accountId":"acc-8"}'
```

Expected: one `acc-7` run holds the permit; two are `WAITING` with `concurrency.state = WAITING`; the `acc-8`
run is not delayed. `GET /v1/namespaces/{id}/concurrency-keys` shows `holders: 1, waiters: 2` for one label.
The waiters start one at a time in admission order.

Failure scenario: set `queueTimeoutMs: 3000` in a new version, start three same-key runs and observe the
third end `FAILED` with `CONCURRENCY_WAIT_TIMEOUT` while the second starts; restart the engine during the
wait and observe the same outcome.

## 5. Standalone step (US05)

```sh
sagat step run --activity catalog.price --input '{"sku":"E-5"}' --idempotency-key demo-e5-0001 --wait   # (proposed)
sagat step run --activity catalog.price --input '{"sku":"E-5"}' --idempotency-key demo-e5-0001          # same run
sagat step run --activity catalog.price --input '{"sku":"E-6"}' --idempotency-key demo-e5-0001          # CONFLICT
sagat workflow runs --origin STANDALONE_STEP                                                          # (proposed filter flag)
```

## 6. Draft test run (US06)

Edit the draft of `order-intake` (do not publish), then:

```sh
sagat workflow test order-intake --start-at ack \
  --pin price='{"mock":{"amount":10}}' --input '{"accountId":"acc-9"}'                          # (proposed)
sagat workflow test order-intake --start-at ack \
  --pin price='{"mock":{"amount":10}}' --pin notify='{"mock":{"sent":true}}' --input '{"accountId":"acc-9"}'
```

Expected for the first: run origin `DRAFT_TEST`; `step.mocked` for `price` in the admission transaction;
`ack` returns `{"amount":10}` early; only `notify` is leased; no webhook delivery is created for the test
run. The second creates no lease at all (`notify` is mocked when reached). The same command against the `prod`
namespace fails with `FORBIDDEN` / `TEST_RUN_PRODUCTION_NAMESPACE`; omitting the `price` pin fails with
`MISSING_PIN` naming `price`.

## Automated equivalents (proposed targets, task T022-003)

```sh
cargo test -p sagat-tests --test integration_flow_control_step_policies --locked
cargo test -p sagat-tests --test fault_flow_control_step_policies --locked
cargo test -p sagat-tests --test e2e_flow_control_step_policies --locked -- --ignored
SAGAT_SDK_SCENARIO=flow-cache cargo test -p sagat-tests --test sdk_conformance -- --ignored
```

Record the actual commands, exits and environment in the feature's verification record; a skipped or
failing scenario is reported as such.
