# SDK conformance suite

One suite, five languages (`FR-006-006`, `SC-006-001`). The same scenarios are executed by the
TypeScript, Go, Python, C# and Java SDKs against a real platform (REST API, worker service, scheduler,
PostgreSQL, identity provider — the stack of `tests/support`), and the **normalized** results must be
identical across the five **and** equal to the golden files in `expected/`. A scenario therefore says what
an SDK does, not how it is written. `sdks/SPEC.md` is the behaviour; this file is the test contract.

```text
cargo test -p sagat-tests --test sdk_conformance -- --nocapture
SAGAT_SDK_LANGS=go,python cargo test -p sagat-tests --test sdk_conformance        # some languages
SAGAT_SDK_SCENARIO=run-lifecycle cargo test -p sagat-tests --test sdk_conformance # one scenario
SAGAT_SDK_BLESS=1 …                                                              # write expected/ (review the diff!)
SAGAT_SDK_REUSE=1 …                                                              # do not rebuild the SDKs
```

## What each SDK ships

`sdks/<language>/conformance/` with two executables (shell scripts, `chmod +x`):

| Script | Does |
|---|---|
| `prepare.sh` | installs dependencies and builds the SDK and its runner. Idempotent; exit 0 |
| `run.sh build <scenario.json>` | prints `{"workflows": {"<key>": <IR>}}` on stdout: every workflow of the scenario built with **the SDK's builders** |
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
  "server": { "leaseTtlSeconds": 3 },            // optional
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
| `parallel` | `id`, `join`, `branches` (array of op arrays) | `parallel` |
| `saga` | `id`, `options?`, `body` (op array) | `saga` |
| `end` | `id`, `options?` | `end` |

A scenario may also hold `"invalidWorkflows": { "<key>": {"workflow": {"name": …, "program": […]}, "contains": "…"} }`:
the runner builds each with the builders, expects the build to fail, and records under `"errors"` (in `build`
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
| `{"verdict": "applied"\|"not_applied"\|"unknown", "output": V}` | for a reconciler |
| `{"readsSecret": "name", "returns": V}` | resolves the step's secrets and checks the named one exists (length goes in the log) |

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
| `get` | `run`, `token?` | `get` | the Run |
| `list` | `status?`, `limit?`, `all?` | `list` / `listAll` | `{items: [runId…]}` in the order the server gave (newest first), also across pages |
| `cancel` | `run`, `reason`, `expectedRunVersion?` | `cancel` | `{accepted, runStatus}` |
| `operate` | `run`, `action`, `reason`, … | `operate` | `{accepted, runStatus}` |
| `signal` | `run`, `name`, `payload` | `signal` | `{deliveredTo}` |
| `publishEvent` | `type`, `data`, `eventId?`, `correlationKey?` | `publishEvent` | `{eventId, status}` |
| `waitForCompletion` | `run`, `timeoutMs` | `waitForCompletion` | the Run |
| `events` | `run` | `events` | `{events: [{type, stepId, attempt}…]}` in order |
| `sleep` | `ms` | — | none |
| `control` | `action`, `run?`, `node?` | the control endpoint | none |
| `startWorker` / `stopWorker` | `worker` | `start()` / `stop()` | `{protocolVersion}` for start |
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
It replaces ids, instants and trace ids by placeholders, drops `message` (the SDK's own words;
it must be non-empty and free of tokens) and `version` (a journal counter), sorts nothing: **order is part of
the result**. Where a scenario would be racy it is written so that it is not (a worker started after the
run, a wait that nothing wakes, `control` to move time).

## Golden files

`expected/<scenario>.json` is the normalized result. It is reviewed by a person: a blessed file that is
wrong makes five SDKs wrong the same way.
