# Feature 022 — five-SDK conformance scenarios (draft)

Added to `tests/contract/sdk-conformance/scenarios/` (one JSON scenario each, normalized observations in
`expected/`), executed by TypeScript, Go, Python, C# and Java against the real stack (`tests/support`).
Independent assertions check the values below before any golden is blessed (`SAGAT_SDK_BLESS=1` diffs are
reviewed). Each SDK must also emit byte-identical canonical IR for the authoring fixture
`tests/contract/flow-control-step-policies/fixtures/order-intake.workflow.json` (the example in
`workflow-ir-fragment.schema.json`).

| Scenario | Steps | Normalized observations |
|---|---|---|
| `flow-cache` | publish activity with `cache` (ttl 60 s); run twice with same key; run with another key; invalidate by key; run again | leases per run `[1,0,1,1]`; journal of run 2 has `step.cache_hit` then `step.completed`; results equal |
| `flow-early-return` | `ack (earlyReturn) → slow → end`; SDK `awaitResult` with timeout; then wait for terminal | result disclosed while `status=RUNNING`; `resultSource=EARLY_RETURN`; final status `SUCCEEDED`; variant where `slow` fails → `FAILED` with same result |
| `flow-on-failure` | saga with failing second step and `onFailure` child; plus operator cancel of a second run | journal order `compensation.planned` < `failure_handler.started` < `failure_handler.finished` < `run.finished`; child input `failure.kind=STEP_FAILED`; final `COMPENSATED`; cancelled run has no handler |
| `flow-concurrency` | `limit: 1`, three runs same key, one other key; cancel the third while queued | max simultaneous holders 1; grant order = admission order; other key not delayed; third run `CANCELLED` with `concurrency.dequeued` |
| `flow-standalone` | start activity step with key K; repeat with K and same body; repeat with K and other body; cancel a second leased execution | same run id; `CONFLICT`/`IDEMPOTENCY_KEY_REUSED`; result via disclosure equals worker output; cancelled run `CANCELLED` |
| `flow-test-run` | draft test from node 3 with two upstream mocks and one downstream mock; same request in a `prod` namespace | zero leases for pinned nodes; `step.mocked` ×3; executed nodes leased once; `prod` → `FORBIDDEN`/`TEST_RUN_PRODUCTION_NAMESPACE` |

Release 0.3.0 slice (US02 and US07, D022-016): `flow-concurrency` above and the scenario below run in
**six** languages (TypeScript, Go, Python, C#, Java, Rust); the other five scenarios wait for their stories.

| Scenario | Steps | Normalized observations |
|---|---|---|
| `flow-rate-limit` | publish a workflow with top-level `rateLimit: {key, limit: 2, per: "second"}` and one activity with node-level `rateLimit: {limit: 1, per: "second"}`; start five runs of one key and one of another key at once; cancel the fifth while buffered; wait for the rest | runs 1–2 have `rate_limit.granted` with `from_queue: false` and never waited; runs 3–4 have `rate_limit.queued` then `rate_limit.granted` with `from_queue: true`, in admission order; no one-second interval holds more than 2 run-scope `granted_at_us` of the key nor more than 1 `step.leased` of the activity's key; the other key's run never waited for the run-level limit; run 5 is `CANCELLED` with `rate_limit.dequeued` and no grant; a buffered run reports `rateLimit.state=WAITING` with a `position` |

The assertions on intervals are computed by the harness from the journals' instants, not compared with
recorded timestamps; the golden holds only the normalized event order and states.

Builders added by the slice: `rateLimit({key, limit, per, group, queueTimeoutMs})` on activity/script
steps and at workflow level, beside `concurrency(...)`, in the six SDKs. No client call is added: waiting
runs are read through the existing run resource.

SDK surface added (names per language idiom; semantics identical):

- Builders: `cache({key, ttlSeconds, scope, name, version})`, `concurrency({key, limit, group,
  queueTimeoutMs})` on activity/script steps and at workflow level, `onFailure(node | workflow, {on,
  timeoutMs, input})`, `earlyReturn(id, result)`.
- Client: `startStepExecution(step, input, options)` and `executeStep(...)` (start then await result),
  `startTestRun(workflowId, {draftRevision, input, startAt, pins})`, `awaitResult(runId, {timeout})`
  (follows run state until `resultAvailableAt` or a terminal status, then discloses the result; a client
  timeout stops waiting, never the run), `invalidateStepCache(target, reason)`.
- Errors: the new `details.reason` values surface through the existing typed error of each SDK; no new
  error class.

The `database` kind of a standalone step reuses the `database-connections` scenario fixtures (prebuilt
`database-runner`, OpenBao, target PostgreSQL) and is asserted equal to the 017 ad hoc result.
