# SDK test environment, clock and activity test context (draft for `sdks/SPEC.md` §13)

Draft normative text. The test environment is a thin client of the test server's REST API
([testing.openapi.yaml](testing.openapi.yaml)) plus the ordinary client and worker host: it holds no engine
semantics. Every **MUST** is held to the conformance scenarios listed at the end.

## 13.1 Clock

```text
Clock.now() -> instant (UTC)
ClientConfig.clock: Clock | none      // default: the system clock
```

The client uses `clock` only to turn a duration option into an instant (today: the run `deadline` given as a
duration, §3). Heartbeat intervals, poll waits and retry backoff stay on the SDK's monotonic clock.

## 13.2 Test environment

```text
TestEnvironment.connect({apiUrl, grpcUrl, namespace, adminToken, workerToken}) -> env
    GET /version; requires capabilities ∋ "virtual-clock", else NOT_FOUND / UNSUPPORTED_BY_SERVER
env.clock                       // Clock: system now + offset of the last clock answer
env.client(config?)             // a client with clock = env.clock (interceptors and, with feature 031, a data
                                //  converter allowed; the environment adds neither)
env.worker({activities, compensations, reconcilers, ...}) -> WorkerHost
    binds the names through PUT /namespaces/{id}/worker-bindings/{subject} with adminToken (never a pattern),
    then returns an ordinary host using workerToken
env.now() -> TestClock          // GET /testing/clock; refreshes env.clock
env.advance(by | to, {mode = STEP, requireQuiescence = true, quiescenceTimeoutMs, maxStops, expectedEpoch?}) -> SkipReport
env.skip({instant | duration | nextDue | runTerminal | chainTerminal}, {quiescenceTimeoutMs, maxStops, wallTimeMs}) -> SkipReport
env.waitForCompletion(runId, {skipTime = true, timeoutMs}) -> Run
    loop: skip({runTerminal: runId}); on RUN_TERMINAL return get(runId);
          on WAITING_EXTERNAL raise CONFLICT with reason RUN_WAITING_EXTERNAL and the waiting kinds;
          on STOP_LIMIT / WALL_TIME_LIMIT continue until timeoutMs; with skipTime = false: the client's waitForCompletion
env.createNamespace(slug) -> namespace   // through the public namespace API, for data isolation between tests
env.replayHistory(document) -> ReplayReport   // POST /testing/history-replays
env.close()                     // stops hosts it created; does not stop the server
```

- `advance` and `skip` return the server's report unchanged (typed, raw JSON kept); a refusal is the canonical error
  (`CONFLICT` with `TIME_ADVANCE_BLOCKED` or `CLOCK_EPOCH_CHANGED`, `VALIDATION_FAILED` with `CLOCK_CEILING`).
- None of them is retried after a `CONFLICT`; transport retries follow §5 (an advance carries an `Idempotency-Key`).
- Time in every returned instant is UTC with `Z`, also under a non-UTC process time zone.
- Tokens are read from the files the test server writes (`0600`), or given; never printed (§10).
- The environment never changes the clock behind the caller's back: only `advance`, `skip` and `waitForCompletion`
  with `skipTime` move it.

## 13.3 Activity test context

```text
ActivityTestContext.create({activityName, taskKind = FORWARD, attempt = 1, node, runId?, deadline?, input,
                            steps = {}, item?, index?, targetNode?, targetIdempotencyKey?, secrets = {}}) -> tctx
tctx.run(handler) -> Outcome
    Outcome = Completed{output} | Failed{code, retryable, messageClass} | Verdict{effect, output?}
tctx.cancel(reason: RUN_CANCELLED | LEASE_LOST | SHUTDOWN)   // cancels the handler's token with that reason
tctx.heartbeats() -> [instant]                                // calls the handler made
tctx.logs() -> [{level, message}]                             // the handler's logger, redacted as the host's
```

- `run` maps results with the host's own functions (§6.5–6.6): `ActivityFailure` keeps its code and retryability,
  another error is `ACTIVITY_ERROR` retryable with its type name, an input that cannot build the handler's type is
  `INPUT_INVALID`, an output over 256 KiB is `PAYLOAD_TOO_LARGE`, a reconciler's answer of another shape is
  `RECONCILIATION_ANSWER_INVALID`.
- `secrets` is a fixture: `ctx.secrets()` returns copies of it once per attempt and raises after the handler returns,
  as the host does. No secret is resolved from anywhere.
- It does not emulate leases, retries, timeouts, heartbeat expiry or authorization (G1403).
- The shared fixture `tests/contract/sdk-conformance/fixtures/activity-context.json` lists handler behaviours and the
  expected outcome; each SDK embeds a copy, a test proves it equal to the file, and each SDK's unit suite runs it.

## Conformance scenarios (sketch)

- `time-skipping`: a workflow with a 30-day timer, a step failing twice (retryable) and an approval that expires after 7
  days; every SDK starts a run with a deadline given as a duration, calls `waitForCompletion(skipTime)` and asserts the
  terminal status; the golden holds the normalized journal (types, order, workflow-time intervals between events) and
  the skip reports (outcome, number of stops).
- `time-skipping-blocked`: a step served by nobody; `advance` returns `TIME_ADVANCE_BLOCKED` with `readySteps = 1` and the
  clock is unchanged; then a worker starts and the advance succeeds.
- `time-skipping-cron`: a daily trigger; `STEP` over 30 days starts 30 runs, `JUMP` over 30 days on a fresh server starts
  one (two platforms; both goldens).
- `time-skipping-race`: a timer and a signal due at one stop; first commit wins; the loser is audited.
- `history-replay`: replay of three exported corpus histories (expected `compatible`) and one corrupted (expected
  divergence of kind `STEP_STATUS` at a named sequence) — gated on feature 020.
