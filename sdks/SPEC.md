# SDK specification (v1)

The normative description of what every Sagat Octopus Orchestrator SDK does — TypeScript, Go, Python,
C#/.NET, Java and Rust — so that the six behave the same where it matters and differ only in idiom
(`FR-006-001..008`, ADR-0015). A statement with **MUST** is checked by the conformance suite
(`tests/contract/sdk-conformance/`) unless it says otherwise. Naming below is language neutral; each
SDK uses its own conventions (`startRun` / `StartRun` / `start_run` / `StartAsync`).

An SDK is these things in one package and **nothing else**:

1. a **client** (REST): start, read, list, cancel and signal runs, decide approvals, publish events;
2. a **worker host** (gRPC, `worker.proto`): register activities, take leases, heartbeat, complete or
   fail, with cancellation and the saga/reconciliation task kinds;
3. **workflow builders**: typed, side-effect-free constructors of the workflow IR
   (`workflow.schema.json`) with the primitives `step`, `sleep`, `waitForEvent`, `invokeChild`,
   `parallel`, `saga`, and the `compensateWith` / `reconcileWith` of a step;
4. from release 0.3.0, a **determinism runtime** for workflow as code (design 044, being built; §12).

A workflow reaches the server in one of three modes:

- a **graph drawn in the Studio**;
- a **graph built by an SDK builder** (item 3): a document the server validates and versions;
- **workflow as code**: a function the SDK's determinism runtime runs in the tenant's worker (item 4), which proposes
  commands under a lease and a fencing token, and whose memory is a cache that replay against the run's journal
  rebuilds (§12).

In all three the server decides every transition and every retry and keeps the journal (Architecture decisions 0001
and 0002); an SDK never commits a state, and in the third mode the workflow function never runs an effect itself:
effects are activities.

## 1. Configuration and credentials

| Setting | Meaning | Default |
|---|---|---|
| `apiUrl` | Base URL of the REST API including `/v1`, e.g. `https://api.example/v1` | required |
| `grpcUrl` | URL of the worker service, `https://host:port` (`http://` only for loopback or development; any other scheme is `VALIDATION_FAILED`, never plaintext) | required for a worker |
| `namespace` | Slug of the namespace every call is made in | required |
| `token` | A bearer token, **or** a provider called before every request that returns a fresh one | required |
| `timeout` | Deadline of one REST call or one unary RPC | 30 s |
| `retry` | `maxAttempts`, `initialDelay`, `maxDelay` (see §5) | 4, 200 ms, 5 s |
| `userAgent` suffix | appended to `sagat-sdk-<language>/<version>` | none |
| TLS | trust roots; client certificate for mTLS | system roots |

- A **token provider** (a function) is called for every request so that a rotated token is used at
  once; a static token is the simple case. The SDK never writes a token to a log, an error message or a
  metric label.
- The tenant is **never** configured: it is the token's (`tenant` claim). A tenant is never read from a
  header or a body (ADR-0005).
- Plain `http://` and `grpc` without TLS outside loopback is accepted but logs one warning at start. The
  warning names the host and nothing else (a URL can carry a credential in its userinfo or query).
- **An HTTP redirect is never followed.** A `Location` can point anywhere, and the request carries a bearer
  token, the namespace and an idempotency key; a `3xx` is raised as `PROTOCOL` ("the server redirected the
  call"), whatever its target. A transport that follows redirects by default is configured not to.
- The SDK has no setting of its own, and no code, that turns certificate or host-name verification off. Where an
  ecosystem lets the caller hand the SDK its own transport object (Go's `*http.Client` and `*tls.Config`, .NET's
  `HttpMessageHandler`, Java's `HttpTransport`), what that object does is the caller's choice and the README says
  so; the SDK never builds such an object with verification off. A server name
  other than the URL's host can be given for verification (`tls.serverName`); it is verified, not skipped.

## 2. Canonical errors

Every failure an SDK raises to its caller is **one** type, `SagatError`, with these fields (the
language's native shape: exception, error value, result):

| Field | Meaning |
|---|---|
| `code` | A stable machine code from the catalog below |
| `message` | Safe, human-readable. Never a payload, a token, a secret or another tenant's data |
| `status` | The HTTP status, or the gRPC status name; `null` for a failure before any answer |
| `retryable` | Whether the same call may be made again as it is. From the server's own `retryable` when it sent one |
| `traceId` | 32 lowercase hex characters from `X-Trace-Id` / the error envelope; empty when the server did not answer |
| `reason` | The server's stable sub-reason (`details.reason`, or the gRPC `x-sagat-error` value), if any |
| `field` | The server's `details.field`, if any: a JSON pointer into the request (or, for a refused run input, into the input) whose segments are only names the version's `inputSchema` declares and array indices, never a value (design 043) |
| `rule` | The server's `details.rule`, if any: the closed keyword of the input schema that failed (`type`, `required`, `pattern`, ..., `workLimit`, `maxDepth`, `other`; design 043). An SDK passes a value it does not know through unchanged |
| `retryAfterMs` | From `Retry-After` / `retry-after-ms`, if any |
| `remediation` | What to do about it: the fixed sentence of the catalog for `code` |

The catalog is `sdks/error-catalog.json`; each SDK embeds a copy and a test proves it is equal to the
file. Codes:

- from the REST envelope: `BAD_REQUEST`, `UNAUTHENTICATED`, `FORBIDDEN`, `NOT_FOUND`, `CONFLICT`,
  `VALIDATION_FAILED`, `RATE_LIMITED`, `QUOTA_EXCEEDED`, `UNAVAILABLE`, `INTERNAL`;
- from the worker protocol: `STALE_ATTEMPT` (gRPC `ABORTED`), `IDEMPOTENCY_KEY_REUSED`, `NOT_BOUND`,
  `SECRET_NOT_USABLE` (both gRPC `PERMISSION_DENIED`, told apart by `x-sagat-error`),
  `UNSUPPORTED_PROTOCOL` (registration refused the version);
- raised by the SDK itself: `TRANSPORT` (no answer: connection refused, reset, timed out),
  `CANCELLED` (the caller's own cancellation), `PROTOCOL` (an answer that does not match the
  contract), `PAYLOAD_TOO_LARGE` (an output over 256 KiB, before it is sent);
- reported by the database runner for a managed step (feature 035, `contracts/errors.md`): `ICEBERG_*`,
  `CATALOG_UNAVAILABLE`, `STORAGE_UNAVAILABLE`, `DESTINATION_*`, `LAKE_HOST_PROTOCOL`, `LAKE_SANDBOX_UNAVAILABLE`,
  `CLASSIFICATION_EGRESS_REFUSED` and `OUTCOME_UNKNOWN` (25 codes, origin `worker`, no HTTP status). They arrive as
  the code of a failed step; `ICEBERG_COMMIT_CONFLICT`, `CATALOG_UNAVAILABLE`, `STORAGE_UNAVAILABLE`,
  `DESTINATION_UNAVAILABLE`, `LAKE_HOST_PROTOCOL` and `LAKE_SANDBOX_UNAVAILABLE` are retryable by default.

gRPC mapping (the trailer `x-sagat-error` wins when present): `UNAUTHENTICATED` → `UNAUTHENTICATED`;
`PERMISSION_DENIED` → `NOT_BOUND` / `SECRET_NOT_USABLE` / `FORBIDDEN`; `INVALID_ARGUMENT` →
`VALIDATION_FAILED` (`UNSUPPORTED_PROTOCOL` when `x-sagat-error` says so: `RegisterWorker` refused the
version); `ABORTED` → `STALE_ATTEMPT` or `IDEMPOTENCY_KEY_REUSED`;
`RESOURCE_EXHAUSTED` → `RATE_LIMITED` (or `QUOTA_EXCEEDED` when the trailer says so); `UNAVAILABLE`
and `DEADLINE_EXCEEDED` → `UNAVAILABLE` / `TRANSPORT`; `NOT_FOUND` → `NOT_FOUND`; anything else →
`INTERNAL`.

## 3. Client

All calls are `<apiUrl><path>` with `Authorization: Bearer <token>`, `Accept: application/json`,
`X-Octopus-Namespace: <namespace>`, `User-Agent`, and — when there is one — `traceparent`. A body is
`application/json`. Unknown fields in an answer **MUST** be ignored (a newer server).

| Operation | Request | Success |
|---|---|---|
| `start(workflowId, input, options)` | `POST /runs` with `{workflowId, versionId?, input, correlationId?, deadline?}`, header `Idempotency-Key` | `202` Run |
| `uploadRunInput(onePassSource, options)` | `POST /run-input-blobs`, raw streaming body; optional `X-Octopus-Blob-Size` and `X-Octopus-Blob-Sha256` preconditions | `201` BlobDescriptor |
| `startWithBlob(workflowId, descriptor, options)` | `POST /runs` with `{workflowId, versionId?, inputBlobRef: descriptor.id, correlationId?, deadline?}`, header `Idempotency-Key` | `202` Run |
| `get(runId)` | `GET /runs/{runId}` | `200` Run |
| `list(options)` | `GET /runs?workflowId=&status=&limit=&cursor=` | `200` `{items, nextCursor}`; `listAll` pages for the caller |
| `listApprovals(runId, options)` | `GET /runs/{runId}/approvals?limit=&cursor=` | `200` ApprovalPage; `listAllApprovals` follows opaque cursors |
| `cancel(runId, reason, options)` | `POST /runs/{runId}/operations` with `{action: "CANCEL", reason, expectedRunVersion?}`, `Idempotency-Key` | `202` OperationResult |
| `signal(runId, name, payload, options)` | `POST /runs/{runId}/signals/{name}` with the payload object, `Idempotency-Key` | `202` `{deliveredTo}` |
| `decideApproval(approvalId, decision, options)` | `POST /approvals/{approvalId}/decisions` with `{decisionKey, decision: APPROVE \| REJECT, reasonCode?}` | `200` ApprovalDecisionReceipt |
| `query(runId, name, args, options)` | `POST /runs/{runId}/queries/{name}` with `{args}` | `200` QueryResult with the observed run and state revisions |
| `queryChain(namespaceId, chainId, name, args, options)` | `POST /namespaces/{namespaceId}/chains/{chainId}/queries/{name}` with `{args, expectedRunId?}` | `200` QueryResult for the head resolved under the chain lock |
| `update(runId, name, args, options)` | `POST /runs/{runId}/updates/{name}` with `{updateKey, args, expectedStateRevision?}` | `200` UpdateReceipt |
| `inspectChain(namespaceId, chainId)` | `GET /namespaces/{namespaceId}/chains/{chainId}` | `200` ChainInspection |
| `listChainRuns(namespaceId, chainId, options)` | `GET /namespaces/{namespaceId}/chains/{chainId}/runs?limit=&cursor=` | `200` ChainRunPage `{items, nextCursor}` in segment order; `listAllChainRuns` follows opaque cursors |
| `signalChain(namespaceId, chainId, name, payload, options)` | `POST /namespaces/{namespaceId}/chains/{chainId}/signals/{name}` with `{expectedRunId, payload}`, `Idempotency-Key` | `202` `{deliveredTo}` |
| `cancelChain(namespaceId, chainId, reason, options)` | `POST /namespaces/{namespaceId}/chains/{chainId}/cancel` with `{expectedRunId, reason}`, `Idempotency-Key` | `202` empty |
| `publishEvent(event)` | `POST /events` with `{eventId, type, data, subject?, occurredAt?, correlationKey?, dedupeTtlSeconds?}` | `202` `{eventId, source, status: ACCEPTED \| DUPLICATE}` |
| `waitForCompletion(runId, options)` | polls `get` every `pollInterval` (default 500 ms, ±20 % jitter) until the status is terminal or `timeout` | the final Run |

Also exposed, thin: `operate(runId, action, reason, options)` for `PAUSE`, `RESUME`, `RERUN_AS_NEW` and the saga
decisions, `events(runId, cursor)` for the journal. `rerunAsNew(runId, reason, options)` is `operate` with
`RERUN_AS_NEW`: it starts a new run of the same version, with the same input and correlation id, from a run that
ended (a run that has not ended is `CONFLICT`). The `OperationResult` names it in `newRunId` (absent for every other
action) and the new run names its origin in `Run.rerunOf` (absent for a run that is not a rerun). A rerun of a
segment of a continuation chain is an unrelated run with a chain, a deadline and a segment 0 of its own. Terminal statuses: `SUCCEEDED`, `FAILED`,
`CANCELLED`, `TIMED_OUT`, `COMPENSATED`, `PARTIAL`, `COMPENSATION_FAILED`.

Rules:

- `Idempotency-Key` is 8 to 160 visible ASCII characters (0x21 to 0x7e: no space, no control character). The caller may give one; when it does not,
  the SDK generates a random one **per call** (UUID) and reuses it for every retry of that call. Two
  separate calls with no key are two runs.
- `publishEvent`'s `eventId` works the same way: the caller's, or generated per call and kept across
  retries, so an event is admitted once per window and a retry is answered `DUPLICATE`.
- An approval decision key is 1 to 128 visible ASCII characters and lives in the request body. The SDK
  requires it, sends no `Idempotency-Key` header, and may safely retry the exact same body. A replay
  returns the original receipt with `replayed: true`; changing the body under the same key is a
  `CONFLICT` with reason `IDEMPOTENCY_KEY_REUSED`. The receipt is typed and retains the raw JSON.
- Approval pages preserve immutable opening order and expose only the request identity, run/node instance,
  closed status, deadline and settlement time returned by the server. They do not infer eligibility,
  policy or actor data. A page is bounded to 200 entries and `listAllApprovals` treats `nextCursor` as opaque.
- A declared operation name is 1 to 128 characters and matches `[A-Za-z0-9][A-Za-z0-9_.:-]*`.
  An update key is 1 to 128 visible ASCII characters. It lives in the body, is required, and is
  preserved with the exact arguments and expected revision across retries. Query and update results
  are typed, retain raw JSON and expose the exact committed revisions returned by the server.
- Chain commands require the namespace UUID and chain UUID because the chain is a namespace-scoped
  resource. Every mutating command carries the caller's observed `expectedRunId`; a delayed command
  fails with `CHAIN_HEAD_CHANGED` instead of reaching a replacement head. The SDK generates or accepts
  an `Idempotency-Key` and preserves the exact key and body across retries.
- A `Run` read inside a continuation chain carries its `chain` segment (`chainId`, `segmentIndex`,
  `completionKind` `CONTINUED` | `FINAL` | none while open, `successorRunId`); a run outside any chain has none.
  A segment that is `SUCCEEDED` with `CONTINUED` ended into its successor and is **not** the process outcome:
  `waitForCompletion` is a segment wait, `waitForChainCompletion` waits for the chain's final outcome. A chain run
  page is bounded to 200 entries, in segment order, carries lineage and lifecycle only (no input, output or
  state), and treats `nextCursor` as opaque. An unknown `completionKind` or an index above 9,999 is a
  `PROTOCOL` error.
- `start` with a `deadline` sends an RFC 3339 instant in UTC (`...Z`); the SDK **MUST NOT** send a local
  offset it computed. A `deadline` option as a duration is converted by the SDK from the clock at the
  call, once, before the first attempt.
- Run, Page and the other answers are returned as typed values **and** keep the raw JSON (`raw`) so a
  field added by a newer server is reachable.
- Pagination: `list` returns `nextCursor` as given; `listAll` follows it until it is null.
- A run-input upload consumes its source once while measuring size and SHA-256. It has no automatic
  retry, even after authentication failure: no second stream exists to send. A successful answer is
  accepted only after the source reached EOF and its descriptor exactly matches the locally measured
  bytes. The descriptor is opaque and scoped by the server to its tenant, namespace and uploader.
  `startWithBlob` sends only its id and uses the normal run-start idempotency key; `input` and
  `inputBlobRef` are never sent together. Uploading alone does not start a run.

### 3.1 Schedules, business identifiers, batch jobs, the with-start calls and reset (feature 018)

Names are given in TypeScript style; each SDK uses its own casing. The operations are groups on the client
(`schedules`, `batch`, `businessIds`) and methods of it. The request/answer shapes are those of the REST contract
(`openapi.yaml`, tags Schedules, Batch operations, Runs, Operations); the scenarios are
`tests/contract/sdk-conformance/scenarios/{schedules-crud,schedules-operate,schedules-overlap,business-ids,start-delay,signal-with-start,update-with-start,batch-operations,reset}.json`.

| Operation | Request | Success |
|---|---|---|
| `schedules.create(namespaceId, write, options)` | `POST /namespaces/{ns}/schedules`, `Idempotency-Key` | `201` Schedule |
| `schedules.get(namespaceId, id, {nextCount, recentCount})` | `GET …/schedules/{id}?nextCount=&recentCount=` | `200` ScheduleDescription |
| `schedules.list(namespaceId, filter)`, `listAll` | `GET …/schedules?state=&origin=&workflowId=&name=&limit=&cursor=` | page; `listAll` follows opaque cursors |
| `schedules.update(namespaceId, id, write, expectedRevision, options)` | `PUT …/schedules/{id}` with the write and `expectedRevision`, `Idempotency-Key` | `200` Schedule |
| `schedules.delete(namespaceId, id, expectedRevision, options)` | `DELETE …/schedules/{id}?expectedRevision=`, `Idempotency-Key` | `204` empty |
| `schedules.pause / resume(namespaceId, id, note, options)` | `POST …/schedules/{id}/pause` / `…/resume` with `{note}`, `Idempotency-Key` | `200` Schedule |
| `schedules.trigger(namespaceId, id, {overlap}, options)` | `POST …/schedules/{id}/trigger` with `{}` or `{overlap}`, `Idempotency-Key` | `202` ScheduleActionRecord |
| `schedules.backfill(namespaceId, id, start, end, {overlap}, options)`, `cancelBackfill`, `listBackfills` | `POST …/backfills` with `{start, end, overlap?}`; `POST …/backfills/{b}/cancel`; `GET …/backfills` | `202` / `200` ScheduleBackfill |
| `schedules.listActions(namespaceId, id, {outcome, limit, cursor})` | `GET …/schedules/{id}/actions` | page of ScheduleActionRecord |
| `businessIds.get(namespaceId, businessId)` | `GET /namespaces/{ns}/business-ids/{businessId}` | `200` BusinessIdBinding |
| `start(…, {businessId, businessIdConflictPolicy, businessIdReusePolicy, startDelay})` | `POST /runs` as before plus the new fields | `202` Run, or `200` the run that already held the identifier |
| `signalWithStart(workflow, businessId, input, signalName, payload, options)` | `POST /runs/signal-with-start` with `{workflowId, businessId, input, signal: {name, payload}, …}`, `Idempotency-Key` | `202` `{runId, started, deliveredTo}` |
| `updateWithStart(workflow, businessId, input, updateName, updateKey, args, options)` | `POST /runs/update-with-start` with `{…, update: {name, updateKey, args, expectedStateRevision?}}`, `Idempotency-Key` | `200` `{runId, started, update: {outcome, receipt \| reason}}` |
| `batch.preview(namespaceId, selector, maxItems)` | `POST …/batch-operations/preview` | `200` BatchPreview |
| `batch.create / get / list / cancel / listItems` | `…/batch-operations…`, `Idempotency-Key` on the writes | BatchJob / page |
| `resetPoints(runId)`, `previewReset(runId, sequence, reapply)`, `reset(runId, request, options)`, `listResets`, `getReset` | `GET /runs/{id}/reset-points`, `POST …/reset-previews`, `POST …/resets` (`Idempotency-Key`), `GET …/resets`, `GET …/resets/{resetId}` | ResetPoint page / ResetPlan / Reset |

Rules:

- Every call that changes something carries one `Idempotency-Key` (8 to 160 visible ASCII characters), given or
  generated once per call and kept by every retry. `updateWithStart` also requires the caller's `updateKey` (1 to 128).
- A business identifier is sent **as given** in `start`, `signalWithStart` and `updateWithStart`: no trimming, no case
  folding and no local pattern check, so the server's `BUSINESS_ID_INVALID` is what every language reads.
  `businessIds.get` percent-encodes it in the path and refuses locally (`VALIDATION_FAILED`, nothing sent) only what is
  not 1 to 200 visible ASCII characters and an identifier that is exactly `.` or `..`: every URL resolver (also for
  `%2E`) takes a whole dot segment as a step in the path, so the request would reach another route. An identifier that
  merely contains dots is valid.
- `start` says whether it created the run: `started` is true for `202` and false for the `200` of `USE_EXISTING`. An
  SDK that never sends a conflict policy never sees the `200`. Where the language cannot widen the type `start` already
  returns (Java: `Run` is a final record), `start` keeps answering the `Run` and `startResult` answers the pair. A `Run` carries `businessId` and `startDelayedUntil` when
  the server sent them.
- A `Run` says where it came from, when the server says so: `schedule` (`{scheduleId, nominalTime, kind, revision}`: the schedule action
  that started it, `kind` being the kind of the action, `SCHEDULED`, `TRIGGERED` or `BACKFILL`, or another string kept as sent) and
  `resetFrom` (`{runId, sequence}`: the run a reset made it from and the journal sequence it keeps). Both are absent/null/None/nil for a run
  that has none, read strictly (a `schedule` that is not an object, lacks `scheduleId` or `kind`, has an unreadable `nominalTime` or a
  `revision` below 1, and a `resetFrom` without `runId` or with a `sequence` below 1, is the SDK's `PROTOCOL` error, never a run with half of
  it), and an instant keeps the language's native precision (the server writes microseconds). The conformance observation of a `Run` carries the
  two objects only when the run has them, so the observations of every other run are what they were.
- A `Run` carries `failureCode` (design 043, FR-043-015), a string, when its stop carries a failure code (the code of the step whose failure
  failed it, a ceiling such as `STORAGE_QUOTA_EXCEEDED`, or `CONTINUATION_INPUT_INVALID`) and is absent/null/None/nil/empty otherwise; the
  conformance observation of a `Run` carries the key only when it is present, never as `null`.
- The action of a schedule may name a business-id prefix (FR-018-016): `businessIdPrefix` (a string), `businessIdConflictPolicy` and
  `businessIdReusePolicy` (the two policies of `start`), optional on the schedule write (create and update) and on the `action` of what is
  read back. **Absent means absent**: a write that sets none of them sends none of the three keys (not `null`, not empty), and a schedule whose
  action has none reads back with none (null, None, nil). The prefix is customer text and is sent **as given**, like a business id: no
  trimming, no case folding, no local pattern or length check (the platform answers `VALIDATION_FAILED`, reason `SCHEDULE_SPEC_INVALID`, for
  one that is not 1 to 160 visible ASCII characters and for a policy without a prefix), and it never appears in a log line, a span, a metric
  label or an error text an SDK writes, nor in the debug text of the types that hold it. The values of the two policies on a read are kept as the
  string the platform sent. The outcomes of a schedule action record that a prefix can make (`BUSINESS_ID_CONFLICT`; `ALREADY_ACTED` with the
  `runId` of the run that held the id) and their reasons (`BUSINESS_ID_IN_USE`, `BUSINESS_ID_REUSE_REJECTED`) are kept as raw strings like
  every other. The observation of a schedule's action carries the three keys **only when the value is present**, so the observations of
  every schedule without a prefix are what they were.
- The `businessIdPrefix` of a batch selector's filter (a literal prefix of the id the runs carry, 1 to 200 visible ASCII characters) is sent
  as given in the same way; the platform counts and selects the runs it matches, and a prefix that matches nothing counts zero.
- A start delay is a duration in the language's idiom, converted **once**, before the first attempt, to integer
  milliseconds by truncation toward zero and sent as `startDelayMs`. Only a value that is not a finite number is
  refused locally; the range (1 ms to 366 days) is the server's (`START_DELAY_INVALID`).
- Instants sent (a schedule's `startAt` and `endAt`, a backfill's range, a batch filter's bounds) are written in UTC
  (`…Z`); text that is not an instant is refused locally. Instants read are date-times.
- A value of an enumeration this SDK does not know (`ScheduleActionOutcome`, a batch item state or reason, a reset point
  kind) is kept as the raw string and is never an error.
- Ids that become path segments (namespace, schedule, backfill, batch, reset, run) are UUIDs; `expectedRevision` is an
  integer of 1 or more; a pause or resume note is not empty; a reset sequence is an integer of 1 or more. A refusal made
  by the SDK is `VALIDATION_FAILED` and sends nothing.
- A `REJECTED` outcome of `updateWithStart` (the validator said no, or the expected revision is stale) is an answer,
  not an error: the run was started and the receipt is absent.
- The input of a schedule is customer content: the SDK sends it and the server never returns it, only its digest.

## 4. Trace context, correlation, deadline

- **traceparent.** An explicit `traceparent` option wins; otherwise the SDK uses the ambient trace
  context of its ecosystem when it has one (C#: `Activity.Current`; Java/Go/Python/TypeScript: OpenTelemetry
  context if the application installed the API, through an optional hook `traceContext()` the caller
  can set); otherwise none is sent and the server makes one. The value is sent as the `traceparent`
  header, validated as `00-<32 hex>-<16 hex>-<2 hex>` first (a malformed one is a `BAD_REQUEST` raised by the
  SDK, not sent).
- The run started that way has `traceId` equal to the trace id of that header; the worker's `TaskLease`
  carries `traceparent` of the run, and the SDK exposes it on the activity context and sends it back on
  `Heartbeat`, `CompleteStep`, `FailStep` and `ResolveSecrets`.
- **correlationId** is the `start` option of that name, sent as `correlationId` in the body.
- **idempotency key** — §3 for the client; for a worker the key of a report is `done-<taskId>` /
  `failed-<taskId>`, stable across retries of the report (8–160 ASCII).
- **deadline** — a run's `deadline` is the client option above. An activity's deadline is the expiry of
  its lease, `expiresAtEpochMs`, extended by every heartbeat (§6); the context exposes it as an instant
  and cancels the activity's cancellation token when the lease is lost.

## 5. Retries — only when safe

An SDK retries a call **only** when repeating it cannot do harm, with exponential backoff and full
jitter (`min(maxDelay, initialDelay * 2^n)` scaled by a random factor in `[0.5, 1.0]`), up to
`maxAttempts` total attempts (default 4), honouring `Retry-After` / `retry-after-ms` when larger:

| Call | Retried on |
|---|---|
| `GET` (REST) | `TRANSPORT`, `UNAVAILABLE`, `RATE_LIMITED`, `INTERNAL` with `retryable` true |
| `POST` that carries an idempotency key or event id | the same; **never** after any other answer |
| approval decision with its body `decisionKey` | the same, with the exact same body |
| declared query, and declared update with its body `updateKey` | the same, with the exact same body |
| `uploadRunInput` | never: its one-pass body is not replayable |
| worker `PollTasks`, `Heartbeat` | `UNAVAILABLE`, `RESOURCE_EXHAUSTED`, transport failures |
| worker `AcknowledgeTasks` (1.8) | the same (the server's own deadline is `UNAVAILABLE`), and `DEADLINE_EXCEEDED`, `CANCELLED`, `UNKNOWN`, `ABORTED`, a retryable `INTERNAL`, an unusable answer; once on `UNAUTHENTICATED` with a fresh token; with no limit on attempts, each bounded to 6 s, until a verdict or the give-up of §6.2; after a failure with the server's envelope, a call of more than eight leases is split in two; the leases answered `RETRY` are asked for again together after 200 ms doubling to 1 s, outside this backoff and the user's retry options |
| worker `CompleteStep`, `FailStep` | the same, with the **same** idempotency key and body; at most 5 times |
| worker `ResolveSecrets`, `ResolveDatabase`, blob transfers, `AppendStepLogs` (1.8 hosts) | `UNAVAILABLE` with the reason `REGISTRY_SILENT`, after its retry hint, within the handler's deadline (the attempt's time left from `attempt_time_left_ms`, or else the lease's expiry); otherwise as before |
| `RegisterWorker` | the same; on `UNAUTHENTICATED` once, with a fresh token |

The rule is by **code**, the same in every SDK: a call is repeated when the error's code is `TRANSPORT`,
`UNAVAILABLE`, `RATE_LIMITED` or `INTERNAL` *and* it says it may be. The server's `408` is `UNAVAILABLE` (retryable),
its `429` is `RATE_LIMITED` or, when a quota is exhausted, `QUOTA_EXCEEDED` (never repeated). A body that claims
`retryable: true` on any other `4xx` is not believed.

**A token provider that fails is repeated too** — before anything is sent, so repeating cannot do harm — in every call
the SDK repeats and in the report of a result (a result that was computed is not lost to a hiccup of the identity
provider): the error is `UNAUTHENTICATED` with the reason `TOKEN_PROVIDER_FAILED` once the attempts are used up. That
it is the provider that failed is something the SDK knows and keeps privately; a `reason` in a server's answer never
makes a call repeatable.

**Never** retried: any `4xx` other than `408` and `429`; `STALE_ATTEMPT` (stop: the lease is gone, do not retry
the report); `IDEMPOTENCY_KEY_REUSED`; `VALIDATION_FAILED`; a call with no key and a non-idempotent
effect. The SDK never retries a failure of *the user's activity code*; that is the server's retry
policy (the node's `retry`).

## 6. Worker host

### 6.1 Registration and negotiation

`register` sends `RegisterWorkerRequest{worker_name, namespace, capabilities{languages, activities,
pools, max_inflight, protocol_version}}`:

- `languages`: one entry naming the SDK's language (`typescript`, `go`, `python`, `csharp`, `java`, `rust`).
- `activities`: every activity name with a handler (forward, compensation and reconciliation alike),
  1 to 256.
- `max_inflight`: the host's concurrency limit, 1 to 1000 (default 10).
- `protocol_version`: the newest minor version of protocol 1 the SDK speaks — a constant of the SDK,
  `1.5` for SDK 0.1 (`1` is `1.0`). The server answers `accepted_protocol_version`: the newest both
  speak (the worker's own if it is not newer than the server's, the server's otherwise). The host **MUST**
  use only the features of the accepted version: against `1` it sends `Heartbeat` without the step
  identity (`run_id`, `step_id`, `attempt`) and does not call `ResolveSecrets`; against `1.1` it does
  not read `task_kind` at all (every task is a forward task). A major version other than `1` is refused by
  the server (`UNSUPPORTED_PROTOCOL`); the host does not retry it.
- The host exposes the accepted version (`host.protocolVersion`).
- There is one live session per worker name and identity: registering again replaces it (§6.2).
- Protocol `1.5` adds the execution-instance identity to a lease. When `1.5` is accepted, the host
  echoes the identity supplied by the server on heartbeat, completion, failure, secret resolution,
  blob upload and blob read. It does not construct or alter this identity. When `1.4` or older is
  accepted, the host omits it from every request, even if an additive field is present on the lease.
  The server uses the echoed identity with the lease fence and tenant-bound worker session; an SDK
  never treats the identity alone as authorization.
- Protocol `1.8` adds delivery acknowledgement (Architecture decision 0099): a lease the server marked provisional
  carries `acknowledge_before_start = true`, and the host acknowledges it with `AcknowledgeTasks` and calls or runs
  nothing else for it but the hand-back before the server answers (§6.2); an unflagged lease is dispatched at once,
  as before. A host that cannot acknowledge registers again at its own minor with
  `capabilities.declines_acknowledgement = true`, and the server then flags none of its leases. `1.8` is also the
  minor at which `TASK_NOT_STARTED` is reserved for the host (§6.2). `1.8` includes `1.7`. A server offers it on the
  proofs of its build alone; whether any lease is flagged is the operator's activation of `delivery-acknowledgement-v1`,
  which a host does not need to know.
- The activity context exposes a copied, typed `ExecutionInstanceIdentity` only when protocol `1.5`
  was accepted and the lease supplied it. Its ordered path contains `{scopeNodeId, iterationIndex}`
  segments and its `definitionNodeId` identifies the node activated at that path. The field is absent
  at protocol `1.4` or older. Reports still echo the original lease field, so handler code cannot
  change the identity used for authorization or fencing.

### 6.2 The loop

```text
register → loop:
  free = max_inflight - in flight
  if free > 0: PollTasks(max_tasks = min(free, 64))
     tasks → dispatch each (§6.3), concurrently
     none → wait retryAfterMs from the answer (default 1000, minimum 50), with ±20 % jitter, or
            until a task finishes, whichever comes first
  UNAUTHENTICATED (session gone or expired) → register again, then continue; if the next poll is
            UNAUTHENTICATED again, wait with the backoff of §5 before the next registration (reset by a
            successful poll): a server that registers a worker and refuses it at once must not be hammered
  RESOURCE_EXHAUSTED → wait retry-after-ms
  UNAVAILABLE / transport → wait with the backoff of §5 (no limit on attempts, cap 30 s), reconnect
```

The host runs at most as many tasks as it asked for (`max_tasks`) and has free slots for: if a poll answer or a stream
frame carries more, it runs the first ones and, from protocol 1.8, hands the rest back (below); below 1.8 it logs the rest
and leaves their leases to lapse. A wait derived from the server
(`retry_after_ms`, `Retry-After`) is capped at 60 s, whatever it says.

**At 1.8 a flagged lease is acknowledged before anything runs or is called for it** (Architecture decision 0099; the
procedure, the verdicts and the failure table are [`stream-completion.md`](../specs/006-sdk-multilanguage/stream-completion.md),
"Delivery acknowledgement (protocol 1.8)"). A `PollTasks` call is bounded to 6 s or the shorter configured call timeout, as a
stream window is. An unflagged lease is dispatched at once. As soon as a frame or a poll answer is read, the host sends one
`AcknowledgeTasks` with its flagged leases that it will run, concurrently with its next receive, on a transport connection of its own
(grpc-js and grpcio with `grpc.use_local_subchannel_pool = 1`, Go with a `ClientConn` of its own, Java with a `ManagedChannel` of its own, .NET with a `GrpcChannel` on a handler of its own, Rust with an `Endpoint` connected apart) built by the same function as the main connection (the same target, transport credentials, TLS settings, server name,
dial options and interceptors), with at most 4 calls in flight held by the SDK's own permit, taken per attempt (an acknowledgement
waiting for that permit has not been sent and is not an attempt; retries take a free permit before new frames; frames waiting for
it are merged into a new call, up to 64 leases, which is one call only for the fallback's ordering and counting, each lease keeping
its own window, evidence and give-up); when an attempt on the current acknowledgement connection ends with no answer (a transport
failure, or the host's own 6 s deadline with no status received) while a call on the main connection, sent after that attempt was sent, was answered, or
after two `ACKNOWLEDGEMENT_STORE_UNREACHABLE` in a row while a call on the main connection was answered `OK` after them, the host
dials a new acknowledgement connection for its next attempts and closes the old one once its attempts in flight have ended; at
most one old connection drains at a time (a redial while one drains closes the older), after a failed dial the host waits one step
of the §5 backoff before dialling again, and an attempt ended by a redial's close is not an attempt, is sent again and counts for
nothing; each acknowledgement attempt is bounded to 6 s whatever the configured call timeout, which bounds only the receive; and makes no call for them but
the hand-back, and starts no handler or heartbeat, before the answer. It never dispatches a flagged lease without `ACKNOWLEDGED`,
whatever it has declined: one received while it declines is handed back.

- `ACKNOWLEDGED`: dispatch (§6.3) with the lease's own expiry as the deadline. When `lease_time_left_ms` less the attempt's
  duration is under 3 s, the host sends the first heartbeat before it starts the handler and hands the lease back if that
  heartbeat fails; when `attempt_time_left_ms` less the attempt's duration is under 2 s, it treats the answer as `CANCELLED`.
- `RETRY`: ask again for those leases (§5), which is not a failed call.
- `CANCELLED`: run nothing and hand the lease back, which is also the answer when the lease is too close to its end to start.
- `NOT_HELD`: drop the lease, run and report nothing, free its slot, emit `dropped` with that code.

A call that fails is repeated as §5 says. The host gives a lease up when its time left at receipt (the earlier of its expiry and,
when the lease carries it in `attempt_deadline_epoch_ms`, its attempt deadline, minus the host's clock at receipt) minus 6 s has passed on its monotonic
clock, clamped to between 6 s and 120 s, never while an attempt is in flight
(it waits for that attempt's outcome, at most 6 s, and runs a late `ACKNOWLEDGED`), and never compares its clock with the server's
again; a lease given up is not run and is handed back. The early give-up is decided by an attempt sent after
the evidence. The evidence: after an attempt of a lease ended with no verdict, another of the host's calls that reaches the database
(a heartbeat, a report, a hand-back), sent after that attempt ended, is answered `OK`, or a frame with leases arrives after it. The
host gives the lease up at the end of an attempt sent after the evidence only when that attempt ended with no verdict and with
neither a shed (`RESOURCE_EXHAUSTED` with the reason `ACKNOWLEDGEMENT_SHED`) nor an unreachable store (`UNAVAILABLE` with the
reason `ACKNOWLEDGEMENT_STORE_UNREACHABLE`, which the server gives when it has not reached a writable database), no attempt of the lease was shed, at least 6 s have passed since its first attempt was sent, and no try of
another of its calls that reaches the database (a heartbeat, a report, a hand-back, a receive; never an `AcknowledgeTasks`) ended
with a transport failure, `DEADLINE_EXCEEDED`, `UNAVAILABLE`, `RESOURCE_EXHAUSTED` or a retryable `INTERNAL` between the latest
evidence before that attempt was sent and its end; a refusal the server answered (stale, not held) is not such an end, nor is a
stream window ended by the host's own bound. An attempt sent before the first evidence that follows the previous failed attempt does not decide,
whatever evidence arrives while it is in flight; when it ends with no verdict, the next attempt may go early. Attempts keep the backoff of §5 and any retry hint; only once 6 s
have passed and no attempt of the lease was shed does the evidence send its next attempt early, after a random delay of up to
200 ms, at most once per lease in 6 s. A `RETRY` restarts the 6 s. A failover gives no evidence before its promotion; an attempt
sent after it is answered by a working primary, and an engine whose own connections have not recovered answers
`ACKNOWLEDGEMENT_STORE_UNREACHABLE`, which decides nothing; a busy engine and a host with no other call and no frame wait for the
ordinary give-up. The server flags a lease at most once
per attempt after a hand-back, so a failing path costs a lease at most one hand-back, and a hand-back of a lease not yet `RUNNING` to
which no capability was served is withdrawn while its mark counts, whether or not it was acknowledged.

The host declines acknowledgement only on `UNIMPLEMENTED`, after three calls in a row refused with `PERMISSION_DENIED`,
`INVALID_ARGUMENT`, `FAILED_PRECONDITION` or `NOT_FOUND`, or after three counted calls in the run since the last `ACKNOWLEDGED`. A
call counts when every lease of it was given up with no verdict, every attempt ended with no answer or with a status without the
server's envelope (`x-sagat-error`, which the server's own transient answers carry and which never count), and its probe failed
the same way: once the hand-back of one of its
leases is answered `OK`, or refused stale with the server's envelope, the host sends one `AcknowledgeTasks` for that lease, one
attempt and evidence only, which a working path answers `NOT_HELD`; a verdict other than `NOT_HELD` or `RETRY` means the hand-back
did not land, and the host hands back once more, with no second probe, without counting the call. Calls are ordered by the give-up of the last of their leases,
the split halves of a call are one call, and a call that does not qualify neither counts nor breaks the run. On fallback the host waits a random delay of up to 6 s, closes its receive, hands back what waits, waits
for the answers of those hand-backs (their retries of §5), logs once, and registers again at its own minor with
`declines_acknowledgement = true` for 10 minutes (a random factor in `[0.8, 1.2]`, doubling for each fallback in a row up to an
hour, back to 10 minutes after an `ACKNOWLEDGED`), then registers again without declining.

**A lease the host will not run is handed back, from protocol 1.8.** A host whose session negotiated protocol 1.8 or later
(the minor of delivery acknowledgement, Architecture decision 0099, reserved by OD-2026-11-21-6, which reserves the code) and that drops a lease it received reports `FailStep` for it with the
code `TASK_NOT_STARTED`, retryable, with the lease's fence, before any handler of that task runs, and never for a task whose
handler started. **Below 1.8 the host does not hand back** (decision D006-080 of
`specs/006-sdk-multilanguage/research.md`): it drops the same leases, logs each one (the warning below, with `lapses`
instead of a report) and leaves it to lapse, as before this rule. The reason: a server of 1.6 or 1.7 records
`TASK_NOT_STARTED` as an ordinary failure, which spends the attempt, so a stopping host would fail a step of one attempt
for good where the lapse sends it to a review (feature 037), which an operator can still retry. The minor is the one
the registration accepted (§6.1), read per lease from the session that received it, never the newest the SDK offers. The server then gives the step back to `READY` without consuming the attempt when it can (decision D006-005
of `specs/006-sdk-multilanguage/research.md`): only for a lease held by a session of protocol 1.8 or later, which reserves the
code, with the requirement `lease-withdrawal-v1` recorded, the step still `LEASED` and at most four withdrawals of the attempt
(decisions D006-030 and D006-031). On a fleet that does not withdraw (the requirement not recorded, or the engine's
switch `SAGAT_LEASE_WITHDRAWAL=off`) it records nothing and answers `STALE_ATTEMPT`, so the lease lapses as before
(decision D006-089). At `1.8` (Architecture decision 0099 decision 9) the reservation is read from the lease, not
from the session, and honoured only while the operator has activated `delivery-acknowledgement-v1` (and, before it,
`lease-withdrawal-v2`), for a lease claimed after that
activation on the current database server; a lease not yet `RUNNING` (no heartbeat) to which no capability was served is then
withdrawn on its hand-back, whether or not it was acknowledged, also after its expiry until the server's pass has ended it. A
hand-back that the server refuses at 1.8 for a reason of the fleet (those activations not recorded, the lease claimed before the
current activation or on another database server, the engine's switch off for a lease that was not flagged, a lease that an engine of
the older build claimed without the reservation while the session is stored at 1.8) is answered `STALE_ATTEMPT` with nothing
recorded, so the lease lapses (decisions D006-067 and D006-068, Architecture decision 0145). Otherwise (a step no
longer `LEASED` to the lease or `RUNNING`, a capability served, the cap reached, a session below 1.8) the server records it as an
ordinary failure, which spends the attempt. The rules (decisions D006-020 to D006-023;
from 1.8 only, decision D006-080):

- *Which leases.* Every lease of a poll answer or a stream frame that the host received and does not dispatch:
  1. the whole answer or frame when it arrives after `stop` began (§6.8), on either transport: a host that is stopping
     starts nothing new;
  2. the leases beyond what the host may run: beyond the credit it asked for in this poll or receive window, or beyond
     its free slots;
  3. the rest of a frame the host stops reading because the frame breaks the receive contract (§10: the 8 MiB input
     budget of the window), the lease that broke it included. A window whose credit is used up exactly is read until it
     ends, so a frame that still comes is handed back under item 2; a frame that breaks the credit or the budget ends
     the window: the host hands back the rest of that frame and cancels the receive, and the frames it does not read
     are the server's (it withdraws what it did not hand to the transport; what it did is the residue G006-001, which
     only a server that broke the credit can cause);
  4. a lease the host could not start for a reason of its own (its runtime refused to schedule the task, its in-flight
     table could not be updated).
  5. at 1.8, a flagged lease given up, answered `CANCELLED`, received while the host declines acknowledgement, waiting for its
     acknowledgement when the host stops or falls back, or acknowledged with too little time left to start (Architecture decision
     0099).
- *Never handed back.* A lease whose in-flight key (the task id and, from protocol 1.5, the instance path; never the task
  id alone, which sibling iterations of a collection share) is already in flight on this host (a task running, or a
  hand-back on its way), or already dispatched or handed back in the same answer or window: it is a duplicate, it is
  ignored and logged, and it is neither run nor reported, because its fence may be the one of the running handler or
  of the hand-back. A lease is handed back at most once.
- *What is sent.* `FailStep{task_id, run_id, step_id, attempt, generation, lease_token, idempotency_key = "failed-<task_id>",
  error_code = "TASK_NOT_STARTED", safe_message = "the host did not start this task", retryable = true, traceparent}`, with
  the execution-instance identity under protocol 1.5 as every report (§6.1). No heartbeat, no `ResolveSecrets`, no handler,
  no "task taken" event. It is retried as a report is (§6.5); `STALE_ATTEMPT` ends it without an error (the lease is no
  longer this host's, or the server left it to lapse, decision D006-067). It is sent only on a session of 1.8 or later, whose server reserves the code. `TASK_NOT_STARTED` is a step failure code, like `NO_HANDLER`; it is never raised to a caller and is
  not in `sdks/error-catalog.json`. **The code is reserved for the host**: a handler that fails with it (an
  `ActivityFailure` whose code is `TASK_NOT_STARTED`, of a forward, compensation or reconciliation handler) is reported as
  `WORKER_ERROR`, with its own message and `retryable` (§6.5), because only the host can know that no handler ran. At most eight hand-backs are on their way at once (the
  derivation is in §6.5: seven to ten); the others wait their turn in the order they were received, so a server that floods one answer
  cannot make the host open a call per lease.
- *What is observed.* A warning log line with the task id and the reason (`stopping`, `credit` or `malformed`, or
  `not_started` for item 4), never the lease token; below 1.8 the same line says the lease lapses, and nothing else is
  emitted; the host's failure event or counter with the code `TASK_NOT_STARTED`
  (or, when the report cannot be made, its dropped event, as for any report).
- *The connection stays open for it.* `stop` waits for the hand-backs on their way, within the same grace as the tasks
  in flight (§6.8), before it closes the connection. A hand-back that cannot be made (the connection is gone) is logged
  and the lease lapses.

The host never busy-polls: an empty answer is always followed by a wait. It opens its connection when
it starts, not before. A connection that was idle may be closed by the server; `UNAVAILABLE` on it is
"reconnect and retry", not an outage.

### 6.3 Running a task

For a `TaskLease` the host (at 1.8, for a flagged lease, only once it was answered `ACKNOWLEDGED`, §6.2):

1. finds the handler by `activity_name`; none → `FailStep(error_code = "NO_HANDLER", retryable = false)`
   (the server would not have leased it to a worker that did not advertise it, so this is a bug report);
2. parses `input_json` — the **step document** `{run: {id}, node, input, steps: {<node>: {output}}}`,
   with `{item,index}` for an admitted collection instance —
   and hands the handler `input` (the run's input) typed by the handler's declared type, plus the context
   (`ctx`): `activityName`, `taskId`, `runId`, `node` (the node id of the workflow definition, `node` of the step document;
   the lease's `step_id` when the document has none), `stepId` (the lease's `step_id`, a UUID, which the reports carry), `attempt`, `generation`,
   `idempotencyKey`, `traceparent`, `deadline`, `taskKind` (`FORWARD` when empty, `COMPENSATION`,
   `RECONCILIATION`), `targetNode`, `targetIdempotencyKey`, `item`/`index` (the immutable captured value
   and index for collection work; absent otherwise), `steps` (the outputs of earlier steps,
   by node), a cancellation token, a logger and `secrets()`;
3. starts the **heartbeat** before the handler and keeps it until the report is made (§6.4);
4. runs the handler with the lease as its deadline; its result is the output;
5. reports (§6.5). If the lease was lost or the call answered `STALE_ATTEMPT`, it stops, drops the
   result, records a metric, and reports nothing.

A handler's input type that cannot be built from the document is not a crash: the host reports
`FailStep(error_code = "INPUT_INVALID", retryable = false)`.

### 6.4 Heartbeat and cancellation

- Interval: one third of the time left on the lease when the task is taken (the lease's `expires_at` minus the
  SDK's clock at receipt: the lease carries no TTL), never under 500 ms and never over 10 s (a third of the
  platform's default 30 s lease). A clock skewed by more than the lease's TTL distorts the interval, which is
  why it is clamped; a host whose clock is not synchronized sets `heartbeatIntervalMs` explicitly. The
  interval is then counted on the SDK's monotonic clock. At 1.8 the heartbeat of a flagged lease starts when it is
  answered `ACKNOWLEDGED`, and its first beat comes at the earlier of the interval and a third of the answer's
  `lease_time_left_ms` less the duration of its acknowledgement attempt (before the handler starts when that time is under 3 s, §6.2), whatever interval
  is configured (the acknowledgement renews nothing).
- `Heartbeat` carries `task_id, worker_id, generation, lease_token, run_id, step_id, attempt` (the last
  three only from protocol 1.1). The answer's `lease_expires_at_epoch_ms` becomes the context's new deadline.
- A beat that was already on its way when **the same lease** reported (`CompleteStep` or `FailStep`) is answered
  without an error: `lease_expires_at_epoch_ms = 0` and `cancel_requested = false` (G141). A live lease never
  has `0`, which is why a host only takes the deadline when it is positive; there is nothing more to do, the
  host has already stopped beating for that task. Any other lease that is no longer current, lost or forged,
  is still `ABORTED` / `STALE_ATTEMPT`.
- `cancel_requested = true` → the cancellation token of the activity is cancelled with the reason
  `run-cancelled`. The handler decides what to do; when it ends, the host reports whatever it returned
  or raised (a handler that raises its cancellation error is reported as `FailStep(error_code =
  "CANCELLED", retryable = false)`). A compensation or reconciliation task is never cancelled by the
  server (it answers `false`).
- **The host cancels the token and nothing else.** It never interrupts, aborts or kills a handler (in a runtime
  where cancellation is delivered into the running code, such as Python's `asyncio`, the handler's own awaits are
  not cancelled either: the SDK exposes the token and an awaitable on it). A handler that ignores the token runs
  to its end and what it returned is reported as usual, unless the lease was lost.
- **The lease is not fenced locally.** The SDK does not cancel a handler because the lease ran out by its own
  clock (the clocks differ); only a `STALE_ATTEMPT` to a heartbeat or a report does. A worker cut off from the
  platform keeps running its handler and its report is refused as stale; external effects stay at-least-once.
- **What a cancelled handler's end means depends on why it was cancelled.** `run-cancelled`: the platform asked
  for it, so a handler that raises its cancellation error is reported as `CANCELLED` (above) and one that
  returns is reported as what it returned. `lease-lost`: nothing is reported. `shutdown` (the host is
  stopping, §6.8): a handler that then raises its cancellation error is **not reported** — the lease lapses and
  the platform applies its unknown-outcome recovery rules. The SDK does not promise a replacement lease;
  failing a step for good because a deploy stopped its worker would be wrong. A handler that returns a value,
  or fails with another error, after `shutdown` is reported as usual.
- `ABORTED` (`STALE_ATTEMPT`) or `FAILED_PRECONDITION` from a heartbeat → the lease is lost: the token is
  cancelled with the reason `lease-lost` and the eventual result is dropped.
- A heartbeat that fails with a transport error is retried on the next beat; the lease is still ours until
  it runs out.

### 6.5 Reporting

- Success: `CompleteStep{task_id, run_id, step_id, attempt, generation, lease_token, idempotency_key =
  "done-<task_id>", output_json, traceparent}`. `output_json` is the JSON of the handler's result (`null` →
  `"null"`; no result → empty string means "no value"). More than 256 KiB: not sent;
  `FailStep(error_code = "PAYLOAD_TOO_LARGE", retryable = false)` instead.
  `CompleteStep` is the handler's final-domain-result assertion, not proof that the SDK, platform or
  provider independently established finality. Accepted, queued or pending provider responses are
  reported as `OUTCOME_UNKNOWN`, including for compensation; `retryable = true` cannot make such a
  result a known failure.
- Protocol 1.3 exposes `ctx.writeBlob(onePassSource, options)` and `ctx.readBlob(descriptor)` only while
  the handler is active. Upload sends one header followed by chunks no larger than 256 KiB, measures the
  source once, and accepts the returned descriptor only when its size and SHA-256 match locally. Read requires
  the complete descriptor, validates the response header, size and SHA-256 incrementally, and claims success
  only at clean gRPC EOF; partial or closed streams are unverified. An explicit native `BlobOutput(descriptor)`
  result sends `output_blob_ref` plus `output_sha256` with empty `output_json`. A JSON object that merely looks
  like a descriptor remains ordinary inline JSON. Protocol 1.2 and older raise `UNSUPPORTED_PROTOCOL` locally.
- Failure: `FailStep{..., idempotency_key = "failed-<task_id>", error_code, safe_message, retryable,
  traceparent}`. A handler fails by raising an **`ActivityFailure(code, message, retryable)`**; helpers:
  `retryable(code, message)`, `permanent(code, message)` (non-retryable), `outcomeUnknown(message)`
  (`OUTCOME_UNKNOWN`, non-retryable, reserved: the platform reconciles or reviews instead of retrying blindly).
  Any other raised error is reported as `error_code = "ACTIVITY_ERROR"`, `retryable = true`, and the
  `safe_message` is only its type name — never its text, which may hold the data it was handed.
  `safe_message` is cut to 512 Unicode code points (never inside a surrogate pair, and a lone surrogate becomes
  U+FFFD, so what is sent is always valid UTF-8); a code that is not `[A-Z0-9_]{1,64}`, or that is `TASK_NOT_STARTED`
  (reserved for the host, §6.2), is replaced by `WORKER_ERROR`.
- The report is retried per §5. `already_applied = true` in the answer is a success. The host never
  reports one attempt twice with different content.
- **Why at most eight hand-backs at once (§6.2).** The number has to be large enough to hand a full frame back before its
  leases expire, and small enough that a host that is stopping opens no more calls than a busy host does. The bounds:
  1. *The largest legitimate batch.* A host has one receive at a time, and one poll answer or stream frame carries at most
     64 leases (`max_tasks` is 1 to 64, and the server's claim stops at 64), so one batch is at most B = 64 hand-backs.
  2. *Fast enough.* The batch goes in ⌈B / H⌉ rounds of one report each. It must land within half the lease's
     time-to-live, the other half being what delivery may already have used. The shortest time-to-live the platform runs
     is 3 s (some conformance scenarios run with it; the default is 30 s). With H = 8 that is 8 rounds in 1.5 s, so one
     report must be answered within 187 ms; at the default 30 s, within 1.87 s. A `FailStep` is one run transaction. With
     H = 6 the budget at 3 s would be 136 ms (11 rounds); H ≥ 7 keeps at least 150 ms (10 rounds).
  3. *Not more than a busy host.* A host reports each running task once, so a host of the default size (`max_inflight` 10)
     has up to 10 reports open. H ≤ 10 keeps a stopping host, whose running tasks still report, within twice that. Each call
     takes one slot of the tenant's share of the server's in-flight cap (a quarter of it: 128 with the defaults) and one
     unit of the per-identity budget (200 a second, a burst of 400). Eight is 6% of the tenant's slots, and a full batch of
     64 fits in the burst.
  So 7 ≤ H ≤ 10, and the SDKs use 8. A hand-back that waits for its turn holds no connection; a slower database delays
  the batch, and a lease whose hand-back comes too late lapses as before.

### 6.6 Compensation and reconciliation

- `registerCompensation(name, handler)` and `registerReconciler(name, handler)` are `registerActivity`
  with the same lease handling; they differ in the context (`taskKind`) and in what the host does with
  the result.
- A **compensation** handler attempts to undo the known-applied effect of `ctx.targetNode`; its
  `idempotencyKey` is the key of *this* compensation and `targetIdempotencyKey` the key that identifies
  the original effect at the provider (send the latter to the provider). It **MUST** be idempotent.
  Delivery 1 does not defensively compensate an unknown original, and declarations/stable keys do not
  prove absent-effect tolerance or safe repetition. Its output is discarded by the server.
- A **reconciler** answers a verdict: `applied(output?)`, `notApplied()` or `unknown()`. The host
  completes the task with exactly `{"effect": "applied" | "not_applied" | "unknown", "output": …}`
  (`output` only for `applied`) and nothing else. A handler of another shape is a programming error
  reported as `FailStep(error_code = "RECONCILIATION_ANSWER_INVALID", retryable = false)`.
  `applied` is the handler's final-result assertion. `notApplied` is an observation, not proof of
  permanent absence, and the server holds the original for review as `RECONCILIATION_NOT_FINAL`;
  `unknown` also reviews. Neither verdict authorizes the SDK to repeat the original itself.
- The three kinds are told apart at run time by `task_kind` (protocol 1.2); a server at 1.0 or 1.1 never
  sends the others.

### 6.7 Secrets

`ctx.secrets()` calls `ResolveSecrets` once per attempt, lazily, with the task's identity, and returns a
read-only map of name → value (bytes; text on request). The values are credentials: the SDK never logs
them, never includes them in an error, never hands out its own buffer (the caller gets copies), wipes and drops
them when the handler returns (a later `secrets()` raises), and does not cache a failed resolution for the rest of
the attempt. Against protocol 1 it
raises `UNSUPPORTED_PROTOCOL` instead of calling.

### 6.8 Lifecycle

`start()` registers and begins the loop and returns once registered; `stop(timeout)` stops polling (it cancels the
receive call in progress, never the code that reads an answer or a frame already received, which is handed back, §6.2),
at 1.8 waits at most 6 s, within `timeout`, for the verdict of an acknowledgement in flight, runs what is `ACKNOWLEDGED`
as any task in flight and hands back the rest, cancels nothing else, waits for the tasks in flight and the hand-backs on their way up to
`timeout` (default 30 s), then
cancels their tokens
with the reason `shutdown` and returns. A task that did not finish keeps its lease until it lapses; the
server then reconciles or holds review according to task kind and available evidence. The host is
restartable. Metrics hooks (counters for tasks taken, completed,
failed, stale, heartbeats, polls, reconnects; a histogram of activity duration) are optional callbacks
or the ecosystem's standard (C#: `System.Diagnostics.Metrics`).

## 7. Workflow builders

Pure, deterministic constructors of the IR. `build()` returns the document (the language's JSON value)
and `toJson()` the canonical text: **RFC 8785** (JSON Canonicalization Scheme) — UTF-8, no insignificant
whitespace, the keys of every object sorted by their **UTF-16 code units**, numbers as ECMAScript prints them
(`1.0` is `1`, `600000` stays `600000`, never `6e5`). The same program in the six languages **MUST** produce
byte-identical canonical text. (Keys that differ only above U+FFFF sort differently by code point than by UTF-16
code unit; the rule is the UTF-16 one.)

```text
workflow(name).description(text).label(key, value)
  .step(id, activity, {retry, timeoutMs, compensateWith, reconcileWith, idempotencyTemplate, secrets,
                       compensationRetry, compensationTimeoutMs, irreversible, highRisk, onError})
  .database(id, {connectionRef, operation, statement, parameters?, limits?, timeoutMs?, onError?})
  .icebergWrite(id, {connectionRef, table, records, ref?, mapping?, ignoreUnknownFields?, limits?, compensation?, onError?})
  .sleep(id, durationMs)
  .waitForEvent(id, {eventType, correlationExpr, timeoutMs, onTimeout?, lookbackMs?})
  .invokeChild(id, workflow, {version?, input?, cancelPolicy?})
  .condition(id, expression, ifTrue, ifFalse)
  .parallel(id, join, [branch, branch, ...])          // a branch is a flow built the same way
  .saga(id, {onCompensationFailure?, cancelPolicy?}, body)   // body is a flow
  .end(id, {outputExpr?})
  .trigger(kind, {...}) .inputSchema(schema) .outputSchema(schema)
  .build()
```

`.inputSchema(schema)` must stay within the supported subset of JSON Schema (design 043, ADR-0090: the keyword list is
in `specs/001-platform-foundation/contracts/workflow.schema.json`): from release 0.3.0 the platform checks every run
input against it before the run is created, refuses a publication whose schema is outside it in `enforce` mode, and
answers a refused start `VALIDATION_FAILED` with `reason` `RUN_INPUT_INVALID` and the error's `field` and `rule`, or
`RUN_INPUT_SCHEMA_UNUSABLE`. An SDK checks nothing of it locally. `CHILD_INPUT_INVALID`, `CONTINUATION_INPUT_INVALID`
and `RUN_INPUT_SCHEMA_UNUSABLE` reach a run view as failure codes, unchanged, and a schedule action's `INPUT_INVALID`
as an outcome.

A `Flow` (the body of a saga, a branch of a parallel) is **read when `build()` is called**, not when it is joined: a
step added to it after it was joined is part of the document. Ids are checked when a node is added to its flow and again
by `build()` over the whole document.

Linking (the whole algorithm — it is why the output is the same everywhere):

- A flow is an ordered list of nodes. Node *i* gets `next = id of node i+1`, except a `condition` (it names
  its two branches) and an `end` (it ends), which never get one; the last node of a flow has no `next`.
- `entrypoint` is the id of the first node of the top flow.
- `parallel(id, join, branches)`: `branches` lists the id of the first node of each branch flow (at least
  two, in the order given); the last node of every branch has no `next`; the `parallel` node's own `next`
  is the next node of the parent flow.
- `saga(id, options, body)`: `begin` is the id of the first node of the body; the last node of the body has
  no `next`; the `saga` node's `next` is the next node of the parent flow. The body holds no `end`.
- `compensateWith` and `reconcileWith` are the names of activities (strings) on the step.
- Node ids are unique across the whole document, including nested flows; a duplicate, an empty flow, a
  parallel with fewer than two branches, a `saga` with an empty body, a bad name (`^[a-z][a-z0-9-]{2,63}$`)
  or an `irreversible` step with `compensateWith` is raised when the node is added (`VALIDATION_FAILED`),
  not when the server refuses the document.
- Options that are absent are omitted from the document (never `null`).

### 7.1 Managed database operations (feature 017)

All six SDKs emit the same canonical `type: database` node. `connectionRef` is
`{name, revision?}` within the workflow namespace, never a DSN, password or provider
path. `operation` is `query` or `execute`. Parameters are either
`{literal: {type, value}}` or `{type, expression}`. Tags are `null`, `boolean`,
`integer`, `decimal`, `float`, `text`, `timestamp`, `date`, `binary` and `json`;
integer/decimal values use exact strings, binary uses base64, timestamp/date use
RFC3339/ISO text. JSON literals obey the same canonical fidelity rules as other IR.

The builder snapshots a database node's supplied values when it is added, validates
basic shapes and finite contract ceilings, and raises `VALIDATION_FAILED` with a
fixed message that contains no SQL or values. It adds neither retry policy nor
secret references. SQL parsing, permission checks, connection/credential pinning,
execution and unknown-write handling are server responsibilities. Writes have no
automatic retry. Ordinary SDK workers keep negotiating the scoped-identity minor 1.5 (or
older); only the dedicated managed executor offers 1.6, which carries `ResolveDatabase` and is a
superset of 1.5. The server rejects unsupported IR before execution.

Inline results retain `{columns, rows, affectedRows, truncated}` with ordered tagged cells,
exact numeric strings and explicit nulls. They use the ordinary retained steps map
for downstream execution. The expression environment wraps each output, so a
binding may read `steps.query.output.rows[0][0].value`. Results over 256 KiB use the
existing `$blob` descriptor and explicit lease-bound blob reading; contexts do not
auto-expand them, and database expressions cannot read blob contents. Interactive result reading
requires the dedicated result permission. See
[the five-language guide](../docs/guides/managed-database-sdk.md) and the shared
`database-connections` conformance scenario.

### 7.2 Iceberg append (feature 035)

All six SDKs emit the same canonical `type: icebergWrite` node, always with `mode: append`: overwrite and
upsert are not built yet, so no SDK accepts a mode. `connectionRef` is the same `{name, revision?}` as
for a database node and never carries a catalog, bucket, credential or secret. `table` is
`{namespace, name}` with 1 to 8 namespace levels. `records` holds exactly one of `inline` (an expression
that evaluates to a list of objects), `blobs` (one that evaluates to 1 to 32 blob descriptors) or
`databaseResult` (the id of an earlier root-level database step; the lake runner does not read that source
yet, gap G035-067). `mapping` is a list of `{field, column}` rows whose field is a JSON pointer
(`/customer/name`, at most 16 segments, escapes `~0` and `~1`); a field or a column used twice is refused.
`limits` may tighten `maxRecordBytes` (1024 to 2147483648), `maxRecords` (1 to 100000000) and `timeoutMs`
(1000 to 3600000). `compensation` is `revert` or absent.

The builder snapshots the supplied values when the node is added, validates the shapes and the contract
bounds, and raises `VALIDATION_FAILED` with the fixed message `invalid iceberg write`, which names no
value. A supplied empty `mapping` or `limits` is kept as `[]` or `{}`; an option that is absent is omitted.
Whether the connection exists, the table is writable and the classification allows the write are server
responsibilities. The shared scenario holds no refusal for an unknown member, because the typed builders cannot
express one uniformly (the conformance runners of Go, Java and C# ignore it when they read the options; the Rust
options type refuses it when decoded).
The shared scenario is `iceberg-write-builders` (build only: it claims neither publication nor execution).

### 7.3 Collection operators (feature 042)

A `forEach` (and `map`, which writes the same node) may carry `operators`, the stages applied once to the list
`items` yields when the collection is entered. Each SDK builds a stage with one factory and the same names, in its own
casing: `filter(where)`, `map(select)`, `flatMap(select)`, `skip(count)`, `take(count)`, `distinct(by?)` and
`window(count)`. A stage is an object with exactly one key, the operator's name: `{"filter":{"where":"…"}}`, and
`distinct` without `by` is `{"distinct":{}}`. The chain is carried without breaking a present call:

| SDK | Chain | Factory |
|---|---|---|
| TypeScript | optional `operators` of `CollectionOptions` | `ops.filter(…)`, `ops.flatMap(…)`, … (type `Operator`) |
| Go | variadic `operators ...ops.Operator` after `maxConcurrency` in `ForEach` and `Map` | package `workflow/ops`: `ops.Filter`, `ops.FlatMap`, `ops.Distinct()`, `ops.DistinctBy(by)`, … |
| Python | keyword-only `operators=None` of `for_each` and `map` | module `ops`: `ops.filter`, `ops.flat_map`, … (type `Operator`) |
| C# | optional init property `Operators` of `CollectionOptions` | static class `Ops` (type `OperatorStage`) |
| Java | overloads `forEach(…, CollectionOptions, Operators)` and `map(…)`; the record keeps its two components | class `Ops`; `Operators.of(…)` gathers the stages |
| Rust | `for_each_with_operators` and `map_with_operators`; `CollectionOptions` is unchanged | module `workflow::ops`: `ops::filter`, `ops::flat_map`, `ops::distinct()`, `ops::distinct_by(by)`, … |

An absent or empty chain writes no `operators`, so every document built before the field existed is unchanged. The
builders only assemble the list, in the order given: the chain's length (one to eight), the ranges of the counts, the
operators allowed on a collection and the variables a stage expression reads (`element`, `position`, `input`, `steps`,
`run`, `state`) are checked by the server at publication. A stage the factory did not build is `VALIDATION_FAILED`
where the language can express one. The shared scenario is `collection-operators` (build only).

## 8. Serialization

- JSON only. Integers up to 2^63-1 round-trip where the language can hold them; TypeScript holds up to
  2^53-1 and raises `PRECISION_LOSS`-style `VALIDATION_FAILED` on an unsafe integer instead of rounding.
  For a worker, an unsafe integer in the handler's own `input` fails the step (`INPUT_INVALID`); one in the output of
  an earlier step (`ctx.steps`) is raised when the handler *reads* `steps`, not before: a step that never reads it is
  not failed by another step's 64-bit id.
- Other numbers are not reformatted: `0.1` stays `0.1`. Amounts of money are strings (`"19.99"`) by
  convention of the platform's examples; the SDK has no decimal type of its own and never converts a
  string to a number.
- Instants the SDK writes are RFC 3339 in UTC with a `Z`; instants it reads may carry any offset and are
  returned as the language's instant type, in UTC.
- `NaN`, infinities, duplicate keys and invalid UTF-8 (a lone surrogate included) are errors on the way out and
  are not produced.
- The canonical text of a workflow (`toJson()`, §7) refuses an integer beyond 2^53 - 1 with `VALIDATION_FAILED`, in
  every SDK: the text must be the same bytes everywhere and no language but TypeScript's can print the nearest
  double and call it exact. The rule is on the **value**, not on how it was written: a number that is a whole number
  beyond 2^53 - 1 and that ECMAScript prints in digits (anything below 1e21: `9007199254740993`, `9007199254740992.0`,
  `1e20`, `2.5e19`) is refused; one it prints with an exponent (`1e21` and above, and every number with a fraction)
  is a number and is written as RFC 8785 writes it. A number is written as RFC 8785 writes it; an exponent is never expanded in memory
  (`1e999999999` is a number, not a billion digits).
- A `Retry-After` is read in seconds only; an HTTP date is ignored (no wait).

## 9. Versioning and compatibility

- The SDK follows **SemVer**. Before 1.0 a minor version may change an API; the **wire** contract does
  not move with it: REST is additive within `/v1`, the worker protocol is additive within `1.x`.
- **Newer SDK, older server.** The registration negotiation (§6.1) runs the host at the server's version;
  REST answers missing a field the SDK knows are accepted (fields are optional unless the OpenAPI marks
  them required); a call the older server does not have is raised as
  `NOT_FOUND` with the reason `UNSUPPORTED_BY_SERVER`: a `501`, a `404` whose envelope says
  `details.reason = ROUTE_NOT_FOUND`, a `405` that says `METHOD_NOT_ALLOWED`, or a `404` that is not the
  platform's envelope at all. (A `404` for a run that does not exist carries the envelope without that reason.)
- **Older SDK, newer server.** Unknown fields are ignored; the worker is served at its own version.
- Runtime matrix and package metadata: `sdks/README.md`.
- **Register of worker-protocol minors.** A minor is taken by one feature, recorded here when its contract merges;
  a feature whose draft names a minor that is taken uses the next free one (OD-2026-11-15-8).

  | Minor | What it adds | Feature or source |
  |---|---|---|
  | 1.1 | The step identity on `Heartbeat` | `worker.proto` |
  | 1.2 | `task_kind` (compensation, reconciliation) and `ResolveSecrets` | features 003 and 009 |
  | 1.3 | Blobs (`WriteBlob`, `ReadBlob`, `input_blob_ref`) | FR-002-008 |
  | 1.4 | Bounded task streams (`StreamTasks`) | ADR-0029 |
  | 1.5 | Scoped execution identity | `ExecutionInstanceIdentity` |
  | 1.6 | Managed database capabilities (`ResolveDatabase`) | feature 017, D600 |
  | 1.7 | Step logs (`AppendStepLogs`, `StepLogPolicy`, the `StepLogCarry` of `CompleteStep` and `FailStep`) | feature 041, OD-2026-11-15-8, OD-2026-11-19-1; served from its tasks T041-7 and T041-36 |
  | 1.8 | Delivery acknowledgement (`TaskLease.acknowledge_before_start` field 23 and `attempt_deadline_epoch_ms` field 24, `WorkerCapabilities.declines_acknowledgement` field 10, `AcknowledgeTasks`; a flagged lease is provisional until acknowledged); the minor at which `TASK_NOT_STARTED` is reserved for the host | feature 006, Architecture decision 0099, Owner decision 2026-11-20-12; reserved by its design on 2026-11-20 at the coordinator's instruction, an exception to this register's rule that the owner approved (Owner decision 2026-11-21-6, which answered Owner question 006-002); offered from Task 006-057, after its gate, and only after 1.7 is served by the server and spoken by the SDK |
  | next | Drafts 019, 020, 021, 023, 024, 025, 026, 031, 032 and 035 each name a minor; each takes the next free one (1.9 or later) when its contract merges, and its SDKs carry the 1.8 host procedure first | |

## 10. Observability and safety

- Logs are structured and never contain a payload, a token, a lease token, a session token or a secret.
  Identifiers (run id, task id, node, attempt, trace id) are fine.
- Every request and RPC propagates `traceparent`; the C# SDK emits `Activity` spans from an
  `ActivitySource` named `SagatOctopusOrchestrator.Sdk`, and the others expose a tracing hook.
- Lease tokens and session tokens live in memory only and are dropped with the task / session.
- **A config, a client or a host never prints a credential**: its string form, its JSON and its debugger/inspect
  form show no bearer token, session token, lease token or TLS private key (a test builds one with a canary token
  and looks for it).
- **What the server says is bounded.** A REST body is read up to 32 MiB, then the call fails as `PROTOCOL`; the
  `message` of a server error is cut to 512 characters; a `traceId` that is not 32 hexadecimal characters is not
  repeated; the heartbeat, the poll loop and a task in flight survive an exception of the caller's hooks and of the
  runtime's observers (a token provider, a logger, an event or metrics callback, a trace listener) instead of dying
  with it or dropping a leased task; a token provider that throws is `UNAUTHENTICATED` with a reason, never the
  provider's own exception.
- **Ids and names in a URL path** (run id, signal name, event type...) are percent-encoded; a run id is a UUID and
  a name of `.`, `..` or the empty string is refused as `VALIDATION_FAILED` before anything is sent.
- Errors follow §2: a message that could carry a payload is replaced by its class.

## 11. What each SDK ships

Client, worker host, builders, errors (catalog), a README with a minimal example per capability, unit
tests, the conformance runner (`sdks/<language>/conformance`), a package manifest with the runtime
matrix, and nothing that needs a network to be tested except the conformance suite itself.

## 10. Task streaming (worker protocol 1.4)

The normative receive contract is
[`stream-completion.md`](../specs/006-sdk-multilanguage/stream-completion.md), ADR-0029.
Each public WorkerHost MUST support auto (default), poll and stream transport selection.
Auto MUST use StreamTasks for a negotiated minor >= 1.4 and PollTasks for older accepted minors.
An UNIMPLEMENTED stream permits auto fallback for that registration; permission/authentication
errors do not. Explicit stream on an unsupported minor fails with UNSUPPORTED_PROTOCOL.

A host MUST request only available execution slots, at most 64 total leases per receive window,
dispatch each frame immediately, keep partial deliveries alive across receive failures and
cancel pending receiving promptly on stop while preserving its existing handler drain.
The stream has a five-second server lifetime; clients additionally bound receiving to six
seconds or their shorter configured call timeout. A normal underfilled window ends with a
positive empty retry hint. An unexpected underfilled EOF MUST wait before reconnecting.
A lease the server committed and did not hand to the transport is withdrawn by the server, once the fleet honours it; a
lease the host drops is handed back with `TASK_NOT_STARTED` (§6.2). A frame lost on the wire, and a lost `PollTasks`
answer, lapse for an unflagged lease; a flagged lease of 1.8 is provisional and is withdrawn at its own earlier deadline instead
([`stream-completion.md`](../specs/006-sdk-multilanguage/stream-completion.md), "Leases that never reach a host" and
"Delivery acknowledgement (protocol 1.8)").
Protocol version changes do not themselves change package SemVer.

## 12. Workflow as code

**Status.** Approved in [design 044](../docs/designs/044-workflow-as-code.md) (Architecture decisions 0103 to 0109,
Proposed until their build-evidence conditions are met) and being built; nothing in this section is served or shipped
before its task lands: the server lane by Tasks 044-2 to 044-13 of that design, each SDK's runtime by its own task
(TypeScript 044-10, Python 044-15, Go 044-16, C# 044-17, Java 044-18, Rust 044-19), patches and the abandoned-activation
cap by Task 044-20, the six-language conformance scenarios by Task 044-26. "Technical Plan" followed by a number is a
section of that design. The server side is in
[`docs/architecture/execution-semantics.md`](../docs/architecture/execution-semantics.md#workflow-as-code-design-044)
and the wire in
[`docs/architecture/worker-runtime-protocol.md`](../docs/architecture/worker-runtime-protocol.md#workflow-as-code-design-044).

A workflow function runs in the tenant's worker under the SDK's **determinism runtime**. It never touches the outside
world itself: each call below is a *command* the SDK records, and in each *workflow task* the SDK runs the function on
the events of the activation window until it yields, then sends the commands (`CompleteWorkflowTask`) or a cause
(`FailWorkflowTask`). On a worker that does not hold the run in its cache, the SDK replays the function from the start
against the run's history and issues only the commands the history does not hold. Every command that does work is a node
of the workflow IR in its code profile, so its limits and checks are the graph node's.

### 12.1 The API

The reference signatures, in TypeScript (Technical Plan 1.12). The other languages follow their own naming (§12.2) and
take the same options.

```typescript
activity<T>(name: string, input: unknown, options?: { retry?: ActivityRetryPolicy; timeoutMs?: number;
  concurrencyKey?: string; rateLimitKey?: string }): Promise<T>          // fragment: activity profile
script<T>(artifactDigest: string, input: unknown, options?: { retry?: ActivityRetryPolicy; timeoutMs?: number }): Promise<T>
sleep(durationMs: number): Promise<void>                                  // timer
waitForEvent<T>(eventType: string, correlationKey: string, options?: { timeoutMs?: number; lookbackMs?: number }): Promise<T | undefined>
child<T>(workflowId: string, input: unknown, options?: { cancelPolicy?: 'propagate' | 'request_cancel' | 'detach'; businessId?: string }): Promise<T>
approval(policyName: string, subject: unknown): Promise<'approved' | 'rejected' | 'expired'>
saga<T>(options: { onCompensationFailure?: 'manual_review' | 'fail'; cancelPolicy?: 'compensate_completed'
  | 'wait_in_flight_then_compensate' | 'abort_no_compensation' }, body: (scope: SagaScope) => Promise<T>): Promise<T>
  // throws SagaCompensated | SagaPartial | SagaCompensationFailed after an abort
onSignal(name: string, handler: (payload: unknown) => void | Promise<void>): void
signal<T>(name: string): Promise<T>
onUpdate(name: string, validator: (args: unknown) => void, handler: (args: unknown) => Promise<unknown>): void
setState(state: unknown): void
patched(id: string): boolean; deprecatePatch(id: string): void
sideEffect<T>(compute: () => T): T                                        // a recorded marker, at most 64 KiB
now(): Date; random(): number; randomInt(n: number): number; uuid(): string
continueAsNew(input: unknown, options?: { version?: 'same' | 'active'; state?: unknown }): never
cancellation: { requested: boolean; promise: Promise<void> }; acknowledgeCancellation(): never
class WorkflowFailure extends Error { constructor(code: string, safeMessage: string) }
continueAsNewSuggested(): boolean; historyLimitReached(): boolean
Replayer.replay(history: AsyncIterable<string>, options: { bundlePath: string; workflowType: string }): Promise<ReplayVerdict>
// worker side: WorkerConfig gains workflowBundlePath and workflowManifestPath; the host loads the bundle into its vm
// context and registers the manifest's (workflow type, code digest) pairs
```

The calls are made on the workflow context the function receives (`ctx.activity(...)`, `ctx.saga(...)`,
`ctx.onSignal(...)`). `ActivityRetryPolicy` is the IR's `retry` of an activity node (`maxAttempts`, `initialDelayMs`,
`maxDelayMs`, `strategy`, `jitter`) in the code profile, distinct from the client's HTTP retry policy (§5). The
TypeScript replayer takes the bundle, not host functions, because no host function enters the run's `vm` context.

The client gains `definitions.publishCode(manifest, envelope, declarations?)` (§12.8), `runs.moveCodeVersion`,
`runs.exportHistory`, `runs.update(..., {waitMs, acceptPending})` and `runs.workerQuery` (the last deferrable, Task
044-29).

### 12.2 The API in each language

| Language | Package | Workflow API | Client additions |
|---|---|---|---|
| TypeScript | entry point `@sagat-octopus-orchestrator/sdk/workflows` (`./workflows` in the package's `exports`), which checks at load that Node is 22.8 or later; the bundler `@sagat-octopus-orchestrator/workflows-bundler` (`sdks/typescript-workflows-bundler/`, command `sagat-workflows`) | §12.1 | §12.1 |
| Python | `sagat_octopus_orchestrator.workflows`, which refuses to import below Python 3.11 | The same names in snake case: `activity`, `script`, `sleep`, `wait_for_event`, `child`, `approval`, `saga`, `on_signal`, `signal`, `on_update`, `set_state`, `patched`, `deprecate_patch`, `side_effect`, `now`, `random`, `random_int`, `uuid`, `continue_as_new`, `cancellation`, `acknowledge_cancellation`, `WorkflowFailure`, `continue_as_new_suggested`, `history_limit_reached`; the waiting calls are awaitables of the SDK's deterministic event loop, under which `asyncio.sleep`, `asyncio.gather`, `asyncio.wait_for` and `asyncio.timeout` also work durably | `definitions.publish_code`, `runs.move_code_version`, `runs.export_history`, `runs.update(..., wait_ms=, accept_pending=)`, `runs.worker_query` |
| Go | package `github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflows` (plural, because `sdks/go/workflow` is the builder of the workflow document; Decision 044-239) and the analyzer `cmd/sagatvet` | The same names, exported, each taking `workflows.Context` first: `workflows.Activity`, `Script`, `Sleep`, `WaitForEvent`, `Child`, `Approval`, `Saga`, `OnSignal`, `Signal`, `OnUpdate`, `SetState`, `Patched`, `DeprecatePatch`, `SideEffect`, `Now`, `Random`, `RandomInt`, `UUID`, `ContinueAsNew`, `Cancellation`, `AcknowledgeCancellation`, `WorkflowFailure`, `ContinueAsNewSuggested`, `HistoryLimitReached`; with `workflows.Go`, `workflows.Channel`, `workflows.Selector` and `workflows.SortedKeys` for branches, channels and ordered map keys | The same names, exported (`Definitions.PublishCode`, ...) |
| C# | packages `SagatOctopusOrchestrator.Workflows` and `SagatOctopusOrchestrator.Workflows.Analyzers` | The same names in Pascal case, with the `Async` suffix on every call that returns a task: `ActivityAsync`, `ScriptAsync`, `SleepAsync`, `WaitForEventAsync`, `ChildAsync`, `ApprovalAsync`, `SagaAsync`, `SignalAsync`; and `OnSignal`, `OnUpdate`, `SetState`, `Patched`, `DeprecatePatch`, `SideEffect`, `Now`, `Random`, `RandomInt`, `Uuid`, `ContinueAsNew`, `Cancellation`, `AcknowledgeCancellation`, `WorkflowFailure`, `ContinueAsNewSuggested`, `HistoryLimitReached` | The same names with `Async` suffixes (`Definitions.PublishCodeAsync`, ...) |
| Java | artifact `sagat-workflows` (Java release 21; the core SDK stays on release 17) | The same names as TypeScript; the SDK also provides `Async.function`, `Promise` and `WorkflowQueue` for branches and queues | The same names, in the core SDK |
| Rust | the SDK crate's feature `workflows` | The same names in snake case, `ctx` first (methods of the workflow context: `ctx.activity`, `ctx.wait_for_event`, `ctx.set_state`, `ctx.side_effect`, `ctx.random_int`, `ctx.continue_as_new`, ...), with `ctx.spawn` for branches, which are not `Send` | The same names in snake case (`publish_code`, ...) |

A language's waiting calls return its own asynchronous shape (a promise, an awaitable, a future, a task); the shapes and
the exact option types are fixed by that SDK's runtime task, under these names. Each SDK documents the same rules for
what workflow code may not do (§12.4).

### 12.3 Deterministic primitives

The same in the six languages, so that one conformance golden holds all six and a history is readable by any SDK's
replayer (Requirement 044-010, Technical Plan 1.8, scenario `code-deterministic-primitives`):

- **Time.** `now()` is the time, in microseconds, recorded in the `workflow_task.started` that opens the activation (the
  last start of its window): the later of the previous start's time and the database's clock in the lease transaction,
  so it never goes backwards. It does not advance during the activation run. A clock type with milliseconds (TypeScript
  `Date`) truncates.
- **Seed.** Each run has a 32-byte seed, drawn at its creation and delivered in every workflow task. A successor of
  continue-as-new and a child get new seeds; a reset keeps its source run's seed.
- **Generator.** xoshiro256\*\* whose four 64-bit state words are the seed read as four little-endian unsigned integers;
  an all-zero seed is replaced by the four words of SplitMix64 started at 0.
- **`random()`** is `(next() >> 11) × 2^-53`.
- **`randomInt(n)`**, for 0 < n ≤ 2^32, draws `x = next()` until `x ≥ (2^64 − n) mod n` and returns `x mod n`, so the
  result has no bias.
- **`uuid()`** takes 16 bytes from two `next()` values, little-endian, sets the version nibble to 4 and the variant bits
  to `10`.
- **Not for secrets.** The values are predictable to every holder of the seed (the workers bound to the workflow type,
  the holders of `history:export`); every SDK says so, and the analyzers of Go and C# flag a value of these primitives
  named like a token, key or secret.

The golden vectors are in the corpus of Task 044-30 (`specs/044-workflow-as-code/contracts/goldens/`).

### 12.4 The runtime per language and its precondition

Each SDK runs workflow code under the determinism runtime of Technical Plan 1 for its language and **refuses to register
workflow types when the runtime's precondition fails** (Requirement 044-011, Technical Plan 1.11):

| Language | Runtime | Detection beyond the command comparison and the server's chain | Precondition |
|---|---|---|---|
| TypeScript | A `node:vm` context per run without a host global, the bundled code, drained microtasks | The bundler refuses imports of built-ins and activity modules | Node 22.8 or later (the package's `engines` stays Node 20); process time zone UTC |
| Python | A deterministic `asyncio` loop, a private module namespace with the SDK's importer, an audit hook | The importer and the audit hook | Python 3.11 or later (the activity worker keeps 3.10); process time zone UTC; the operator's own fixed `PYTHONHASHSEED` (not `random`, not `0`), kept private: it is a 32-bit value, so a reader of a Python version's digest who knows its modules' sources could search it (Gap 044-016); the SDK caps decoded objects at 10,000 keys, which bounds the harm whatever the seed |
| Go | An `iter.Pull` coroutine dispatcher | The `sagatvet` analyzer, the activation deadline | Go 1.25 |
| C# | A single-threaded `TaskScheduler` per run, the invariant culture | A scheduler check on every SDK call, the analyzer package | .NET 8; `TimeZoneInfo.Local` of UTC |
| Java | Virtual threads under one permit | A thread check on every SDK call | Java 21; default time zone UTC and default locale `Locale.ROOT` |
| Rust | A single-threaded executor on workflow threads with no Tokio runtime | A caught panic for a Tokio call, the clippy configuration | Rust 1.98 |

The determinism runtime isolates for determinism, not for security; workflow code is the tenant's own, in the tenant's
worker.

**The activation deadline** (Technical Plan 1.10). Every SDK stops an activation run that has not yielded within the
smaller of 2 s and the version's workflow task timeout, of wall time (configurable from 100 ms to the workflow task
timeout), and fails the workflow task with cause `ACTIVATION_DEADLINE` (scenario `code-activation-deadline`).
TypeScript's `vm` script timeout stops the code. In Python, Go, C#, Java and Rust a stuck coroutine or thread cannot be
killed: the dispatcher stops waiting, abandons it and the run's cache entry, and counts it. A worker that holds 4
abandoned activation runs stops taking workflow tasks, withdraws its workflow types and reports itself unhealthy so
that its process is restarted; in Rust the stuck pool thread is replaced first.

**Failures.** An uncaught exception that is not the SDK's `WorkflowFailure` fails the workflow task (`FailWorkflowTask`
with its type name, never its message), not the run. A `WorkflowFailure` ends the run `FAILED` with its code, which
must match `^[A-Z][A-Z0-9_]{0,63}$` (another becomes `WORKFLOW_FAILED`); its message is stored only as a `pii` payload
(Requirement 044-009, scenario `code-failures`).

**Nondeterminism.** During replay the SDK compares each command with the record at the same command number (its kind
and target), and reports a command where the history has none, a recorded command the function no longer makes, a
`patched(id)` that disagrees with the markers and a deferred command not re-emitted last as a failure with cause
`NONDETERMINISM`, the command number, the actual kind and target and the code digest, never a payload (Requirement
044-006, scenario `code-nondeterminism`).

### 12.5 Registration and the capability declaration

A worker host that registers workflow types sends `WorkerCapabilities.workflow_types` (its `(workflow type, code
digest)` pairs, at most 64) and **`offers_workflow_as_code`**, and offers the workflow-as-code minor (the next free minor
of §9's register when the contract of Task 044-2 merges, Architecture decision 0109); a host with activities only keeps
its minor. Minors are cumulative: a host on that minor implements the host procedures of 1.7, 1.8 and 1.9. An SDK whose
runtime task has not landed declares `offers_workflow_as_code = false`, and the conformance suite reports it as **not
offered**, never as passed; the release needs all six offered (Success criterion 044-001).

### 12.6 Bounds the function meets

The SDK enforces them at the call that would cross them, at the same point live and in replay, so the function takes the
same branch both times (Requirement 044-012, Non-functional requirement 044-006):

- **`CommandBatchLimitExceeded`.** The function's own calls make at most 256 commands with at most 48 KiB of command
  metadata per activation, besides at most 147 commands the SDK issues itself (update decisions and results, the
  consumption of signals, one set-state, scope closes and aborts, one re-emitted deferral). The SDK throws the typed
  `CommandBatchLimitExceeded` into the function at the call that would cross its share; code can catch it, or move the
  fan-out to a graph child (scenario `code-command-batch-limit`).
- **`InlineValuesExceeded`.** A completion fits the worker service's decode limit of 320 KiB: at most 64 KiB of command
  metadata and at most 192 KiB of inline values. The declared state (at most the version's `state.maxBytes`, at most
  128 KiB), a continue-as-new input and its carried state are always inline, together at most 192 KiB; a gated
  activity's input and an approval's subject are always inline too, the SDK reserving the declared state's maximum from
  the activation's start. Every other value (a work command's input, an update result, a side-effect value, a terminal
  output) is written by the SDK as a blob, largest first, when the message would not fit, at most 16 per completion.
  The SDK throws the typed `InlineValuesExceeded` at the call that would cross one of these bounds, or after which the
  activation's values could no longer be carried, and for a terminal output over the 4 MiB validation bound.
- A side-effect value is at most 64 KiB; `setState` at most once per completion (the last call of an activation wins).

### 12.7 Patches

`patched(id)` returns true and records a `patch` marker the first time a run reaches it; on replay it returns true if
the marker is at that point of the history and false if the history has another command there, which is where the old
code went. `deprecatePatch(id)` records the marker and always takes the new path, for code from which the old path was
removed after every run that could take it ended. A marker identifier matches `^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`. A
code change is always a new code version, so a patch takes effect on a running run only through a verified move, which
proves per run that the patched code replays (Requirement 044-024, Technical Plan 7.2, scenario `code-patch-and-move`).

### 12.8 The manifest and publication

The SDK computes the code digest, the SHA-256 of the RFC 8785 canonical JSON of `{"language", "runtime", "sdkName",
"sdkVersion", "workflowType", "content"}` (`runtime` names the runtime with its full version and, where it has one, its
locale data), and writes it with the workflow type, the language and the runtime into a *manifest*; the worker computes
the same digest at start and registers it (Technical Plan 3.2):

| Language | What `content` hashes | How the manifest is produced |
|---|---|---|
| TypeScript | The bundle file the worker loads | `sagat-workflows bundle` writes the bundle and the manifest |
| Python | The modules the SDK's importer resolved for the workflow type, the standard library by version, and the hash-seed fingerprint | `python -m sagat_octopus_orchestrator.workflows manifest <module>`, with the deployment's `PYTHONHASHSEED` |
| Go | The executable file | `<worker binary> --sagat-manifest` |
| C# | The workflow assembly and its dependency closure from `deps.json` | `dotnet <worker>.dll --sagat-manifest` |
| Java | Every jar and class directory of the worker's class path | `java -jar <worker>.jar --sagat-manifest` |
| Rust | The executable file | `<worker binary> --sagat-manifest` |

For the compiled languages the digest hashes the whole executable, so a change of an activity in the same binary makes a
new digest that open runs take only through a move. The recommended topology is one worker binary (assembly, jar) for
workflow code and another for activities.

`definitions.publishCode(manifest, envelope, declarations?)` assembles the code document from the manifest (produced by
the build), the envelope (reviewed by the publisher) and the declarations, which hold the other parts of the document of
Technical Plan 3.1: `signals`, `state` with its queries and code updates, `inputSchema`, `outputSchema`, `triggers`,
`limits`, `chainPolicy`, and the `code` object's `workflowTaskTimeoutMs`, `cancellationGraceMs` and `workerQueries`
(Decision 044-238). For the order workflow of the quickstarts, which waits for a `paid` signal:

```typescript
await api.definitions.publishCode(manifest, envelope, {
  signals: [{ name: 'paid', buffered: true, maxPending: 10,
    schema: { type: 'object', properties: {}, additionalProperties: false } }],
});
```

The command line's `sagat workflow publish-code --manifest <file> --envelope <file>` takes `--declarations <file>` as
well. The call saves the code document as the workflow's draft and publishes it through today's route
(`POST /workflows/{workflowId}/publish`, `definition:publish`), which makes it the active version; it is refused
`422 CAPABILITY_NOT_ACTIVE` until the operator has recorded the instance registry requirement `code-workflows-v1` (and
`signal-inbox-v1` for a version that declares signals). Nothing runs code on the server.

### 12.9 The replayer

Each SDK ships a replayer that replays exported histories against the local code and reports the first mismatch, usable
in a test of the language's own framework (Requirement 044-025):

- The history comes from `GET /runs/{runId}/history` (`run:read` and `history:export`, held by the admin role only by
  default), **export format version 1**: JSON lines of `HistoryEvent` under a first line `{"exportFormat": 1,
  "gatedEntries": [...], "stateMaxBytes": n, ...}`, every payload redacted by the rules of feature 016 for the reader
  and every blob a reference without its content.
- The replayer **refuses a history of an unknown format version**.
- A replay that reads a placeholder (a redacted field, a blob-carried value) stops with the verdict
  **`INCONCLUSIVE_REDACTED`** at that command, never with a false mismatch.
- A mismatch names the command number and the expected and actual kind and target.

### 12.10 Errors

`CommandBatchLimitExceeded` and `InlineValuesExceeded` are errors the SDK throws into the function, with no HTTP or gRPC
status; `INCONCLUSIVE_REDACTED` is a replayer verdict. The codes of Technical Plan 13.3 (`CAPABILITY_NOT_ACTIVE`,
`SIGNAL_NOT_DECLARED`, `UPDATE_PENDING`, `MOVE_TARGET_INVALID`, `QUERY_UNAVAILABLE`, ...) join the catalog
`sdks/error-catalog.json` and its six copies with the contracts of Task 044-2.

## Approved Rust extension (release 0.3)

[Feature 034](../specs/034-rust-sdk/spec.md) adds Rust as the sixth official SDK under this same
contract. Historical five-language results remain historical; Rust/six-language acceptance needs
new named conformance, fault and packaging results. Rust uses Tokio/tonic, native typed values,
lossless JSON and idiomatic Result/cancellation APIs. Its public crate is independent of backend
workspaces and contains its own checked canonical protocol/catalog for clean consumers.

## Feature 037 delivery 1 — implementation in verification

The delivery uses the existing wire shapes and treats every leased `Activity` as external-capable,
regardless of `highRisk`. A lease that never reached a handler is not an attempt: a lease the server withdrew, or one a host
handed back with `TASK_NOT_STARTED` while it was still `LEASED`, has a known outcome (nothing ran) and is never reviewed; a
lease that lapses keeps its unknown outcome (D006-007). Unknown outcomes take precedence over retry, cancellation and explicit
failure. The server will not directly retry an unknown original or compensation, even for a caller
with `SagaOverride`; an authorized outcome-bearing resolution must atomically retain actor and
reason first, and no new in-place attempt after resolution is promised. Refused conflicts create no
success audit, scheduling outbox or idempotency success. A known failed, retry-eligible operation
continues to use the existing retry command.

Human success resolution can allow a saga to report `COMPENSATED`, but that result retains human
provenance and is not provider confirmation. No provider/capability, SDK handler or plugin is
qualified by delivery 1, and no qualification registry or exception path exists; US04 remains open.
The historical behavior and rationale remain in features
[003](../specs/003-sagas-compensations/spec.md) and
[014](../specs/014-workflow-primitives/spec.md). The proposed direction is recorded in
[ADR-0076](../docs/adr/ADR-0076-compensation-failure-and-uncertain-effects.md), whose status remains
**PROPOSED**. Consult the feature-037
[validation report](../specs/037-compensation-safety/validation-report.md) before claiming
implementation acceptance; SDK conformance proves protocol behavior, not a real provider guarantee.
