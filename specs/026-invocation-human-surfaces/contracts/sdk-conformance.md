# Five-SDK surface and conformance scenarios (draft)

DRAFT, not implemented. Added to `sdks/SPEC.md` (§3 client, §6 worker, §7 builders) and to
`tests/contract/sdk-conformance/scenarios/` by T026-055..058. Every scenario runs in TypeScript, Go,
Python, C# and Java against the real platform (`tests/support`), and the normalized results must equal
each other and the reviewed golden files; goldens are written only after reading the diff
(`SAGAT_SDK_BLESS=1`, then rerun without it).

## Client operations

| Operation | Request | Success |
|---|---|---|
| `execute(workflowId, input, options)` | `POST /runs/execute` with `StartRunRequest` + `waitMs`, `Idempotency-Key` (generated per call, kept across retries) | `RunOutcome` for `200` and `202` alike; `completion` says which |
| `awaitOutcome(runId, options)` | `GET /runs/{runId}/outcome?waitMs=`; loops until terminal or the caller's overall timeout, each wait ≤ the server ceiling | final `RunOutcome` |
| `streamEvents(runId, options)` | `GET /runs/{runId}/stream-events` (SSE via the language's streaming HTTP, never browser `EventSource`); resumes with the last `<epoch>.<offset>` cursor after a disconnection; stops at `end` | an iterator of typed `StreamEvent` with raw JSON kept, plus the control events |
| `listStreamEvents(runId, after, options)` | the JSON page | `StreamEventPage` |
| `getStartForm(workflowId, version)` | `GET …/form` | typed `FormModel` |
| `getWorkflowOpenApi(workflowId, version)` | `GET …/openapi` with `If-None-Match` | the document as raw JSON + `ETag` |
| `decideApproval(approvalId, decision, options)` | existing, gains optional `input` and `via` | `ApprovalDecisionReceipt` |

`waitForCompletion` is kept and documented as the polling fallback for servers without `getRunOutcome`
(an older server answers `NOT_FOUND` with `ROUTE_NOT_FOUND`).

## Stream consumer rule (all five SDKs and the Studio)

1. Keep, per node instance, the current attempt from the `attempts` snapshot sent on open and on every
   resume, and update it on each `superseded` control event.
2. Deliver to the application only events whose attempt equals the current attempt of their node
   instance (an option exposes superseded events for diagnostics, marked as such); on `superseded`,
   tell the application that the partial output of the older attempt is void.
3. On `settled`, mark the node instance complete; later events of that attempt cannot arrive.
4. On `reset` (cursor of another cell epoch, feature 032), drop all state built from the stream and
   rebuild it from the events that follow; never compare offsets across epochs.
5. Resume with the last cursor received, never a cursor computed by the client.

## Worker

`ctx.stream(channel).publish(data)` and `flush()` per [worker-protocol.md](./worker-protocol.md); the
worker negotiates the new minor and refuses locally to publish when the server did not accept it
(`UNSUPPORTED_PROTOCOL`), without failing the step.

## Builders

`streams` (channel name → schema, display, description), node `emits`, approval `notify`, `summary`,
`form` and the `x-sagat-form` annotation as a key of the free-form schema map (no helper, as feature 016
did for classification). Byte-identical canonical documents across the five builders.

## Scenarios

| File | Covers |
|---|---|
| `start-form.json` | form model of a schema with every supported keyword, one unsupported optional field, one unsupported required field (UNAVAILABLE); hint validation errors at publication |
| `invocation-execute.json` | `execute` that ends within the wait (200, result per audience), one that does not (202), replay with the same key (same run), changed body (409), `waitMs` out of range |
| `invocation-outcome-wait.json` | `awaitOutcome` over a run that ends after two server waits; terminal failure with closed code |
| `stream-builders.json` | build-only: channels, `emits`, progress display shape, invalid declarations |
| `workflow-streams.json` | worker publishes 500 events in batches; lost-response replay; lease expiry and second attempt with the `superseded` control event and the consumer rule; subscriber disconnects and resumes from the `attempts` snapshot; `end`; a budget refusal does not fail the step |
| `stream-budgets.json` | one author channel exhausting its own budget while a second author channel and a reserved channel (a test-only reserved channel standing in for feature 025's `sagat.agent.tokens`) keep appending; author pool exhaustion; the per-run hard ceiling; the scope named in each refusal; a multi-attempt sequence that stays within the reserved pool |
| `stream-epoch-reset.json` | a cursor with another epoch receives `reset` and the full stream; the consumer rebuilds its state (epoch injected by the test harness; 032 itself is not required) |
| `approval-channel-builders.json` | build-only: `notify`, `summary`, `form`, `requiredFor` |
| `approval-form-decision.json` | decision with valid input; missing required input; replay; changed input under the same key (409); later step reads `steps.<node>.output.input` |
| `workflow-openapi.json` | document retrieval, `ETag`/`304`, agreement of a valid and an invalid request with the server |
