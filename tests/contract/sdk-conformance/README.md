# SDK conformance suite

One suite, five languages (`FR-006-006`, `SC-006-001`). The same scenarios are executed by the
TypeScript, Go, Python, C# and Java SDKs against a real platform (REST API, worker service, scheduler,
PostgreSQL, identity provider — the stack of `tests/support`), and the **normalized** results must be
identical across the five **and** equal to the golden files in `expected/`. A scenario therefore says what
an SDK does, not how it is written. `sdks/SPEC.md` is the behaviour; this file is the test contract.

```text
cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture
SAGAT_SDK_LANGS=go,python cargo test -p sagat-tests --test sdk_conformance -- --ignored        # some languages
SAGAT_SDK_SCENARIO=run-lifecycle cargo test -p sagat-tests --test sdk_conformance -- --ignored # one scenario
SAGAT_SDK_BLESS=1 …                                                              # write expected/ (review the diff!)
SAGAT_SDK_REUSE=1 …                                                              # do not rebuild the SDKs
```

`--ignored` because the suite needs the five toolchains (Node, Python, Go, .NET and a JDK) and Docker, which `cargo test --workspace` cannot assume; the `sdks` job of CI sets them up and runs the suite with `--ignored`. `SAGAT_SDK_REUSE=1` also skips the build of the runners, so run once without it after changing an SDK or a runner.

## Protocol compatibility

`protocol-negotiation.json` keeps the production worker ceiling and old-minor path executable in
all five SDKs. It probes the accepted 1.x minors, malformed input and incompatible majors, then
runs the same root activity through protocol 1, a retained 1.4 worker and the current 1.5 worker.
The production adapter accepts 1.5 while a worker that offers 1.4 remains at 1.4. This scenario
exercises additive negotiation and root compatibility; scoped identity has its own runtime scenario.

## Workflow-primitives scenarios

`switch-builders.json` is the reviewed WP-07 contract. Its build phase verifies that all five public
builders preserve the ordered cases, mandatory default and v1 document shape, and reject empty cases,
duplicate case ids and dangling targets. Its runtime phase starts overlapping-true, no-match and
predicate-error runs through every SDK. The golden requires only the first case activity and the default
activity to execute, leaves every non-selected node `NOT_STARTED`, and records `EXPRESSION_ERROR` without
a `switch.evaluated` event for the invalid predicate. The two golden files were written after inspecting
the builder documents and equal normalized runtime outputs from all five SDKs; they were not auto-blessed.

`collection-builders.json` checks additive v1 collection authoring in the five SDKs. Its
`buildOnly: true` marker makes the harness compare the reviewed workflow golden, canonical text
and validation errors without publishing or executing a collection. The fixture includes two
named bodies that reuse local node IDs and covers `forEach`, its `map` alias and bounded options.
It does not count as collection runtime conformance.

`loop-builders.json` checks additive v1 `while` and `repeatUntil` authoring in the five SDKs.
It is build-only while durable loop execution remains disabled. The reviewed fixture covers pre-test
and post-test shapes, the maximum iteration and duration bounds, optional durable delay/error routing,
named body references and the required successor. It does not count as loop runtime conformance.

`approval-builders.json` checks additive v1 native approval authoring in the five SDKs. It remains
build-only so the authoring contract is reviewed independently of runtime behavior.
The reviewed fixture covers subject and built-in role policies, the default initiator exclusion,
bounded deadlines, all three routes and local validation failures. It does not count as approval
runtime conformance.

`continuation-builders.json` checks additive v1 `continueAsNew` authoring and the required finite
`chainPolicy` in the five SDKs. It remains build-only while atomic handoff persistence, chain command
serialization and parent-child finalization are disabled. The reviewed fixture covers explicit input
and optional state transfer, the 10,000-segment and duration ceilings, inherited policy requirements
and the top-level-only boundary. It does not count as continuation runtime conformance.

`state-operation-builders.json` checks additive v1 typed state plus finite Query/Update declarations
in the five SDKs. It remains build-only and checks authoring separately from runtime calls. The
reviewed fixture covers explicit result disclosure, closed state/result schemas,
bounded strings and arrays, operation-count/name limits and the 1,024-byte expression boundary.

`state-operations.json` runs the public Query/Update calls in all five SDKs against the real API and
PostgreSQL store. It checks the observed revisions, exact update replay, stale-revision conflict,
durable validator rejection, authentication and opaque cross-tenant lookup. Every retry preserves
the body `updateKey`; no idempotency header is synthesized. Its lost-response fault proves that the
same update is replayed instead of applied twice. Decimal values and instants stay exact JSON strings,
including the UTC `Z`, under the harness's non-UTC process timezone. The language unit suites cancel an
in-flight update and prove that cancellation does not trigger a hidden retry.

`approval-decisions.json` publishes and starts native approval workflows, discovers each request through
the public run approval page, and calls the public decision route from all five SDKs. It covers
authentication, opaque tenant isolation, approve and reject routing, exact replay and conflicting reuse
of the body decision key. No private store is seeded; the scheduler creates every request through the
published workflow path. The SDK unit suites separately cover multi-page cursor traversal and strict
response parsing.

`chain-inspect-cancel.json` starts a public chain with an initial activity waiting for a worker,
then inspects and cancels it through all five SDKs. The reviewed runtime golden requires opaque
cross-tenant lookup, exact cancel replay and a terminal `CANCELLED` chain and run. It does not
exercise continuation handoff or chain-target signals.

`scoped-identity.json` provisions the tenant iteration quota through the public API, then runs the
restricted collection body through the served scheduler and worker API. A retained 1.4 worker
completes root work but cannot claim scoped work. A 1.5 worker observes two equal items at distinct
indices plus a third distinct item, preserves each typed identity, captured `{item,index}` and
idempotency key across a retry, and reaches a terminal graph with all three logical instances. No
private store or control tick advances the collection.

```text
SAGAT_SDK_SCENARIO=switch-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=collection-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=loop-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=approval-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=continuation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=state-operation-builders cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=state-operations cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=approval-decisions cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=chain-inspect-cancel cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=scoped-identity cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

## Pending scenarios

Files ending in `.json.pending` are reviewed contracts for behavior that the server and all five SDKs
cannot execute yet. The harness deliberately ignores them, so they are never counted as passing evidence
and have no golden result.

`chain-commands.json.pending` exercises typed chain inspection, root-signal and cancellation calls
implemented by all five SDKs, including expected-head fencing, exact command replay and tenant opacity.
It remains pending because its signal-controlled `waitEvent` continuation graph is outside the
published chain subset. The runner now resolves the namespace UUID and current head from public
responses. Do not seed the private chain store or substitute the namespace slug.

`chain-query.json.pending` exercises the typed chain Query call and expected-head fence in all five
SDK runners. It has no runtime golden because the public workflow subset still rejects a definition
that combines declared state with continuation. Run-addressed Query/Update remains the executable
shared scenario.

## What each SDK ships

`sdks/<language>/conformance/` with two executables (shell scripts, `chmod +x`):

| Script | Does |
|---|---|
| `prepare.sh` | installs dependencies and builds the SDK and its runner. Idempotent; exit 0 |
| `run.sh build <scenario.json>` | prints `{"workflows": {"<key>": <IR>}, "canonical": {"<key>": "<text>"}}` on stdout: every workflow of the scenario built with **the SDK's builders**, and the **canonical text** (`toJson()`, RFC 8785, SPEC §7) of each, which the harness compares **byte for byte** across the five and with the golden file |
| `run.sh run <scenario.json> <config.json> <out.json>` | plays the scenario against the platform and writes the observations to `<out.json>` |

The runner is a small program in the language that uses **only the public API of the SDK** (plus a plain
HTTP call to the control endpoint, below). Anything the runner needs from the SDK that is not public API is
a defect of the SDK.

## The harness (Rust, `main.rs`)

For each scenario: for each language run `build` and require the five IR documents to be equal (canonical
JSON) and equal to `expected/<scenario>.workflows.json`. Then, **for each language, on a platform of its
own** (booted with the scenario's `server` settings, so no run, lease, event or quota of one language is
there for the next to meet): publish the IR with the REST API, run `run` with a `config.json`, normalize
the result. A scenario is `ok` when all five are equal to each other and to `expected/<scenario>.json`.
For `buildOnly: true`, the harness stops after the common workflow golden and error comparison; no
server run or runtime golden is implied.

**config.json** (written by the harness for every run):

```json
{
  "apiUrl": "http://127.0.0.1:41000/v1", "grpcUrl": "http://127.0.0.1:41001", "namespace": "payments",
  "clientToken": "<jwt of an admin>", "workerToken": "<jwt of a worker bound to every activity>",
  "otherTenantToken": "<jwt of an admin of another tenant>", "badToken": "not-a-jwt",
  "unboundWorkerToken": "<jwt of a worker identity with no binding>",
  "controlUrl": "http://127.0.0.1:41002", "unique": "go-7f3a9c",
  "workflows": { "<key>": { "workflowId": "<uuid>", "versionId": "<uuid>", "name": "<name>" } },
  "leaseTtlSeconds": 3
}
```

`unique` differs per language and run. The runner replaces the literal text `{{unique}}` in **every string
value of the scenario** with it (idempotency keys, correlation keys, event ids: no two languages touch each
other's data). The harness puts `{{unique}}` back in the results before it compares.

**Control endpoint.** `POST <controlUrl>/control` with `{"action": …}` (JSON) is how a scenario step acts on
the server without an SDK API: `expireLease {run, node}`, `elapseRetryDelay {run, node}`,
`elapseTimer {run, node}`. It answers `{"ok": true}`. A run is addressed by the **id the step returned**.

## Scenario files (`scenarios/<name>.json`)

```jsonc
{
  "name": "run-lifecycle",
  "description": "…",
  "server": { "leaseTtlSeconds": 3, "iterationConcurrency": 2 }, // optional; quota is set through the public API
  "workflows": {                                   // key → builder program
    "linear": { "name": "linear-flow", "description": "…", "program": [ /* ops */ ],
                "triggers": [], "labels": {}, "inputSchema": {}, "outputSchema": {} }
  },
  "workers": {                                     // optional: name → worker host
    "w1": { "workerName": "conf-w1", "maxInflight": 4, "protocolVersion": "1.2", "autostart": true,
            "activities": { "payments.charge": { "as": "activity", "behavior": { /* … */ } } } }
  },
  "faults": [ { "channel": "http", "match": "POST /runs", "action": "dropResponse", "times": 1 } ],
  "steps": [ { "id": "s1", "op": "start", … } ]
}
```

### Builder program ops (`workflows.*.program`, and nested flows)

| `op` | Fields | Builder call |
|---|---|---|
| `step` | `id`, `activity`, `options?` | `step` (options named as in `workflow.schema.json`: `retry`, `timeoutMs`, `compensateWith`, `reconcileWith`, `idempotencyTemplate`, `secrets`, `compensationRetry`, `compensationTimeoutMs`, `irreversible`, `highRisk`, `onError`) |
| `sleep` | `id`, `durationMs` | `sleep` |
| `waitForEvent` | `id`, `options` (`eventType`, `correlationExpr`, `timeoutMs`, `onTimeout?`, `lookbackMs?`) | `waitForEvent` |
| `invokeChild` | `id`, `workflow`, `options?` | `invokeChild` |
| `condition` | `id`, `expression`, `ifTrue`, `ifFalse` | `condition` |
| `switch` | `id`, ordered `cases` (`id`, `when`, `target`), `default` | `switch` (`switchCase` in Java) |
| `forEach`, `map` | `id`, `items`, `body`, `result`, `maxItems`, `maxConcurrency` | collection builder |
| `while`, `repeatUntil` | `id`, `condition`, `body`, `initial`, `carry`, `maxIterations`, `timeoutMs`, `delayMs?`, `onError?` | sequential loop builder |
| `continueAsNew` | `id`, `input`, `state?` | terminal continuation builder; the workflow also declares `chainPolicy: {maxSegments,timeoutMs}` |
| top-level `state` | closed `schema`, `initial`, up to 32 named `queries` and 32 named `updates` | typed state/declaration builder; every operation explicitly declares `declared-business-result-v1` disclosure |
| `return` | `id`, `result` inside a named body | body return builder |
| `parallel` | `id`, `join`, `branches` (array of op arrays) | `parallel` |
| `saga` | `id`, `options?`, `body` (op array) | `saga` |
| `end` | `id`, `options?` | `end` |

A scenario may also hold `"invalidWorkflows": { "<key>": {"workflow": {"name": …, "program": […]}, "contains": "…"} }`:
the runner builds each with the builders **and writes its canonical text** (`toJson()`), expects one of the two to fail, and records under `"errors"` (in `build`
mode) the `SagatError` code, whether the message contains the text, and that it was raised.

### Worker activities

A worker definition may carry `"token": "unbound"` to use `unboundWorkerToken` instead of `workerToken`.

`activities.<name>` = `{ "as": "activity" | "compensation" | "reconciler", "behavior": {…} }`, registered with
`registerActivity` / `registerCompensation` / `registerReconciler`. A behavior is evaluated per attempt:

| Behavior | Effect |
|---|---|
| `{"returns": V}` | returns `V` |
| `{"echo": true}` | returns `{"input": <the input>, "steps": <ctx.steps>}` |
| `{"echoInput": true}` | returns the input itself, unchanged |
| `{"returnsText": N}` | returns a string of `N` times the letter `x` |
| `{"fails": {"code","message","retryable"}}` | raises an `ActivityFailure` |
| `{"failsUntilAttempt": N, "fails": {…}, "returns": V}` | fails while `attempt < N`, then returns `V` |
| `{"throws": "text"}` | raises an ordinary error of the language (reported as `ACTIVITY_ERROR`) |
| `{"outcomeUnknown": "text"}` | raises `ActivityFailure.outcomeUnknown` |
| `{"sleepMs": N, "returns": V, "onCancel": {"returns": W}}` | waits `N` ms (or until cancelled: then returns `W`, or raises the SDK's cancellation error if `onCancel` is `{"raises": true}`) |
| `{"hang": true}` | waits until cancelled or the host stops, then returns nothing useful |
| `{"hang": true, "onCancel": {"raises": true}}` | the same, then ends by raising the SDK's cancellation error (what a handler does that is cancelled and says so) |
| `{"sleepMs": N, "ignoresCancel": true, "returns": V}` | a handler that **never looks at its token**: it waits `N` ms whatever happens and returns `V`; the host must not interrupt it and what it returned is reported |
| `{"verdict": "applied"\|"not_applied"\|"unknown", "output": V}` | for a reconciler |
| `{"readsSecret": "name", "returns": V}` | resolves the step's secrets and returns `V` plus `secretLength`, the length of the named one (never the value) |
| `{"readsSecret": "name", "reportsSecretError": true, "returns": V}` | the same, but if resolving raises a canonical error the output is `V` plus `secretError`, its code (a protocol-1 worker has no `ResolveSecrets`: `UNSUPPORTED_PROTOCOL`) |
| `{"blobRoundTripBytes": N}` | streams `N` deterministic bytes with size/SHA preconditions, reads the descriptor back to verified EOF, and returns only the verified size/hash |
| `{"blobOutputBytes": N}` | streams `N` deterministic bytes and returns the SDK's explicit native `BlobOutput` wrapper |
| `{"blobInlineBytes": N}` | streams `N` deterministic bytes and returns ordinary JSON shaped as `{ "$blob": descriptor }`; it must remain inline |
| `{"blobInputBytes": N}` | reads the run's lease-bound input descriptor through verified EOF and returns only the verified size/hash |

Every attempt appends an entry to the worker's `workerLog` **when the handler ends**:
`{activity, kind, node, attempt, input, taskKind, targetNode, targetIdempotencyKey, idempotencyKey,
traceparentPresent, traceId, steps (the outputs of earlier steps, by node), deadlineAhead, abortedReason (or null), outcome:
"returned"|"failed"|"threw"|"cancelled"}`.

### Steps (`steps[]`)

Each step has a unique `id` and an `op`; `run` fields name the **step** whose result holds the run
(`"run": "s1"` is the `runId` of the result of step `s1`). A step that raises records the error, it does not
stop the scenario.

| `op` | Fields | Calls | Result |
|---|---|---|---|
| `start` | `workflow` (key), `input`, `idempotencyKey?`, `correlationId?`, `traceparent?`, `deadlineInMs?`, `token?` (`bad`\|`other`) | `start` | the Run |
| `startWithBlob` | `workflow` (key), `inputBlobBytes`, `idempotencyKey?`, `correlationId?`, `traceparent?`, `deadlineInMs?` | `uploadRunInput` then `startWithBlob` | the Run; a successful upload response is refused unless the one-pass source reached EOF |
| `get` | `run`, `token?` | `get` | the Run |
| `list` | `status?`, `limit?`, `all?` | `list` / `listAll` | `{items: [runId…]}` in the order the server gave (newest first), also across pages |
| `listApprovals` | `run`, `cursor?`, `limit?`, `waitForItem?`, `traceparent?`, `token?` | `listApprovals` | the typed page in immutable opening order |
| `cancel` | `run`, `reason`, `expectedRunVersion?` | `cancel` | `{accepted, runStatus}` |
| `operate` | `run`, `action`, `reason`, … | `operate` | `{accepted, runStatus}` |
| `decideApproval` | `approvalId` or `approval` (a list step), `decisionKey`, `decision`, `reasonCode?`, `traceparent?`, `token?` | `decideApproval` | the normalized durable receipt |
| `query` | `run`, `name`, `args`, `traceparent?`, `token?` | `query` | `{runVersion, stateRevision, result}` |
| `queryChain` | `namespaceId`, `chainId`, `name`, `args`, `expectedRunId?`, `traceparent?`, `token?` | `queryChain` | `{runId, runVersion, stateRevision, result}` |
| `update` | `run`, `name`, `args`, `updateKey`, `expectedStateRevision?`, `traceparent?`, `token?` | `update` | the normalized durable receipt |
| `inspectChain` | `namespaceId`, `chainId`, `traceparent?`, `token?` | `inspectChain` | current head and ordered segments |
| `signalChain` | `namespaceId`, `chainId`, `expectedRunId`, `name`, `payload`, `idempotencyKey?`, `traceparent?`, `token?` | `signalChain` | `{deliveredTo}` |
| `cancelChain` | `namespaceId`, `chainId`, `expectedRunId`, `reason`, `idempotencyKey?`, `traceparent?`, `token?` | `cancelChain` | `{accepted: true}` |
| `signal` | `run`, `name`, `payload` | `signal` | `{deliveredTo}` |
| `publishEvent` | `type`, `data`, `eventId?`, `correlationKey?` | `publishEvent` | `{eventId, status}` |
| `waitForCompletion` | `run`, `timeoutMs` | `waitForCompletion` | the Run |
| `events` | `run` | `events` | `{events: [{type, stepId, attempt}…]}` in order |
| `sleep` | `ms` | — | none |
| `control` | `action`, `run?`, `node?` | the control endpoint | none |
| `startWorker` / `stopWorker` | `worker` (stop: `timeoutMs?`) | `start()` / `stop(timeout)` | `{protocolVersion}` for start |
| `restartWorker` | `worker`, `timeoutMs?` | `stop()` then `start()` **on the same host object** (the host is restartable, SPEC §6.8) | `{protocolVersion}` |
| `probeWorker` | `protocolVersion`, `activities?` | a throw-away host registered with that offer | `{accepted}` or the error |
| `awaitJournal` | `run`, `eventType`, `stepId?`, `timeoutMs` | polls `events` until the journal has an event of that type (for that step) | none |
| `awaitWorkerLog` | `worker`, `count`, `timeoutMs` | waits until the worker's log has `count` entries | none |
| `workflowByName` | `name`, `input` | `start` with the workflow **name** | the Run |

A result is a plain JSON object with the fields named here. A **Run** is exactly
`{runId, workflowId, versionId, status, version, paused, startedBy, createdAt, updatedAt, finishedAt,
deadline, traceId, correlationId, depth, parent}`.

### Faults

`faults[]` are applied by the runner at the SDK's transport hooks (a custom `fetch`/`RoundTripper`/
`HttpClient` handler; a gRPC interceptor): `channel` (`http` | `grpc`), `match` (`"POST /runs"` — method and
path prefix — or the RPC name `CompleteStep`), `action`, `times`, `after` (skip this many matching calls
first). Actions: `dropResponse` (the call is made and **applied by the server**, the answer is thrown
away and the SDK sees a transport failure), `failBefore` (the call never leaves: connection refused),
`status` + `code` (an HTTP status or gRPC code answered in place of the server's; with optional
`retryAfterMs`, and for `http` an optional `body`, the JSON the answer carries).

**Counting.** One plan serves every client and worker host of the run. A call is counted against **every**
fault that matches it, whether or not another fault answered it; the first fault in file order that is due
(`after` ≤ its count < `after` + `times`) answers the call. The runner reports in `faultsApplied`, in file
order, how many calls each fault answered: a fault the scenario never reached, or an SDK that repeated a
call it must not (a refused request), shows in the golden file.

### Observations (`out.json`)

```json
{ "scenario": "…", "language": "go", "sdkVersion": "0.1.0", "faultsApplied": [1, 1, 1, 2, 1],
  "observations": [ {"id": "s1", "op": "start", "ok": true, "result": { … } },
                    {"id": "s2", "op": "get", "ok": false, "error": {"code","status","retryable","traceId","reason","retryAfterMs","remediation","message"} } ],
  "workerLog": { "w1": [ … ] } }
```

The harness runs every script under `TZ=Asia/Kolkata` (UTC+5:30, so that local time cannot pass for UTC) and
requires the instants of a Run (`createdAt`, `updatedAt`, `finishedAt`, `deadline`) to be strings ending in `Z`.
It replaces ids, instants and trace ids by placeholders, masks `message` **of an error** (the SDK's own words;
it must be non-empty and free of tokens) and `version` **of a Run** (a journal counter) — a `message` or a
`version` in a payload the scenario sent is compared like any other value — and turns the `deadline` of a Run
into `<createdAt+Ns>`, the seconds between its creation and its deadline (an SDK that sends the deadline in
the wrong unit shows). It sorts nothing: **order is part of the result**. Where a scenario would be racy it is written so that it is not (a worker started after the
run, a wait that nothing wakes, `control` to move time).

## Optional modes

`run.sh <mode> <config.json> <out.json>` with a mode other than `build` and `run` is an SDK's own acceptance
test against the real platform. There is one, for `US01-AC01`:

**`aspnet`** (C# only; `tests/e2e/sdk-multilanguage/scenarios/us01-aspnet.json` is its scenario, whose
workflows are published as for any scenario and are in `config.json`). The runner starts a minimal ASP.NET Core
service on a loopback port that registers the SDK with `AddSagatOctopusOrchestrator` (`apiUrl`, `namespace` and
`clientToken` of the config) and has `POST /orders`, a handler that takes `ISagatClient` from dependency
injection and calls `StartAsync` for `workflows.order`, with the request's `Idempotency-Key` header as the
idempotency key and **no explicit trace**: the trace reaches the platform through the ambient `Activity` that
ASP.NET Core made from the request's `traceparent`. The runner then calls its own service twice with the same
`Idempotency-Key` and one `traceparent`, and writes

```json
{ "sent": {"idempotencyKey": "…", "traceparent": "00-…-…-01", "traceId": "…32 hex…"},
  "first": {"status": 202, "run": <Run>}, "second": {"status": 202, "run": <Run>} }
```

where `run` is the Run of the SDK (the exact fields above) as the service returned it. The harness holds it to
the acceptance scenario: both calls are the same run, and the run's `traceId` is the one sent.

## Golden files

`expected/<scenario>.json` is the normalized result. It is reviewed by a person: a blessed file that is
wrong makes five SDKs wrong the same way.

## Protocol 1.4 streaming completion

Workers accept `transport: "auto" | "poll" | "stream"` (auto by default), as specified by
[ADR-0029](../../../docs/adr/ADR-0029-worker-task-streams-spend-finite-receive-credit.md).
Existing scenarios that specifically inject PollTasks faults explicitly select poll. Other
default hosts use the negotiated transport. `worker-stream-faults` requires streaming and
injects faults into StreamTasks itself.

The gRPC-only `dropStreamAfterFirst` fault opens the real server stream, forwards its first
nonempty task frame intact to the SDK, then cancels only that receive stream and makes the next
read fail with UNAVAILABLE. It must not cancel task handlers or silently discard the first frame.
Its plan is counted once when the matching stream is opened, with the same `times`/`after` rules
as unary faults. A scenario must contain queued work so the nonempty frame is actually reached.
The runner must not manufacture a task or substitute a unary poll for this operation.
