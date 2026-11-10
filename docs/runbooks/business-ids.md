# Runbook: business identifiers, start delay and the with-start calls (feature 018)

For the people who build on the API (an order number that must never start two runs) and for the operators who answer
"which run holds order 4711". It says what an identifier is, what it promises, **what not to put in it**, how a signal or an
update can start the run it is for, and what each refusal means.

Design: decisions D1961 to D1967 and later ones about with-start in [`specs/018-schedules-bulk-operations/research.md`](../../specs/018-schedules-bulk-operations/research.md);
the contract is `startRun`, `signalWithStart`, `updateWithStart` and `getBusinessIdBinding` in
[`openapi.yaml`](../../specs/001-platform-foundation/contracts/openapi.yaml).

## 1. What an identifier is and what it promises

A **business identifier** is a name the caller gives a run (`businessId`, 1 to 200 visible ASCII characters). The platform keeps
one promise about it: **at most one open execution of a namespace holds it, at any commit.** An execution is a run, or the whole
continuation chain of runs that carries the same work; a chain is open until it has a final outcome.

- **While the holder is open**, a start with the same identifier is `409 BUSINESS_ID_IN_USE`, or, with
  `businessIdConflictPolicy: USE_EXISTING`, answers `200` with the holder's run and creates nothing (the SDKs say `started: false`).
- **When the holder has closed**, the identifier is remembered for **30 days** (the reuse window; the code allows up to 366 days).
  `businessIdReusePolicy` decides: `ALLOW_DUPLICATE` (the default) starts a new run; `REJECT_DUPLICATE` refuses while the window
  is open (`409 BUSINESS_ID_REUSE_REJECTED`, with `reusableAfter`); `ALLOW_DUPLICATE_FAILED_ONLY` refuses the same way when the holder
  succeeded.
- **The identifier is sent and kept as given**: no trimming and no case folding. `Order-1` and `order-1` are two identifiers.
- **A start delay** (`startDelayMs`, 1 ms to 366 days; `sagat workflow run --start-delay` takes seconds) holds the entry of the new run: it is created `WAITING` with
  `startDelayedUntil`, counts toward the active-run quotas, **holds its identifier meanwhile**, and a scheduler pass releases
  it once, at the instant. Cancelled, or past its deadline, while it waits, it ends `CANCELLED` or `TIMED_OUT` with no step run.
  A signal to it is refused (`409`: nothing waits for it yet).

Finding the holder: `GET /namespaces/{id}/business-ids/{businessId}` (percent-encoded) says who holds it (`executionId`, and
`currentRunId`, the run that carries the work now), whether it is still open, how it ended (`lastStatus`) and until when it is
remembered (`reusableAfter`). An identifier nobody holds, that has been free for longer than its window, that cannot be an
identifier, or that belongs to another tenant is `404`: the four look the same. The identifiers `.` and `..` cannot be looked up
this way (a URL resolver turns them into a step in the path; the SDKs refuse them locally): find those runs with
`GET /runs?businessId=…`. The run list also filters by `businessIdPrefix` (a literal: nothing in it is a wildcard).

## 2. What not to put in an identifier

**An identifier is customer content.** It is stored with the run (classified as content in the storage classification), returned
to everyone who may read the run (`Run.businessId`, the run list, the lookup) and kept with it. The platform does **not** echo it in an error, a log line or a metric label, and neither should
you: use an **opaque** reference (an order number, a UUID), never a name, an e-mail address, a document number, a token or any
text you would not want in a screenshot of the run list. Treat a support ticket that quotes one like any other customer data.

## 3. Signal-with-start and the entry-wait pattern

`POST /runs/signal-with-start` delivers a signal to the open run of an identifier, **or creates the run and delivers the signal
at its creation**, in one transaction. It needs both `run:start` and `run:signal`.

For the second case the **entry** of the workflow (its first node) must be a wait for that signal. The pattern:

```text
waitForEvent  eventType = "payment.confirmed"   <- the signal name, and the first node of the workflow
step          fulfil
end
```

- With no open holder, a run is created and the signal wakes its entry wait (`202`, `started: true`, `deliveredTo` names the node).
- If **no node of the new run waits for the signal**, the whole request is rolled back with `409 SIGNAL_NOT_ACCEPTED_AT_START`
  and **nothing is created**: a signal is never buffered. A workflow whose first node is an activity cannot use this call for
  its first signal; start it with `start`, then signal it once it waits.
- With an open holder, it is signalled exactly as `POST /runs/{id}/signals/{name}` would: `202`, `started: false`, or
  `409 NOT_WAITING` when no node of it waits for that name now (nothing changed).
- A start delay is refused with a signal (`422 START_DELAY_WITH_SIGNAL`): a held run is not waiting for anything yet.

## 4. Update-with-start

`POST /runs/update-with-start` applies a **declared Update** (the workflow's typed state and update declaration) to the open run
of the identifier, creating the run first if none is open. It needs `run:start` and `run:update`, and the caller's `updateKey`.

- The validator saying no, or a stale `expectedStateRevision`, is an **answer** (`200`, `outcome: REJECTED`, `reason:
  UPDATE_REJECTED` or `REVISION_CONFLICT`) and leaves the run started: the run exists, the state is as it was.
- An operation the workflow did not declare is `404`; arguments the declaration does not accept are `422 INVALID_ARGUMENTS`;
  the same `updateKey`, or the same `Idempotency-Key`, with another request is `409 IDEMPOTENCY_KEY_REUSED`.
- `businessIdConflictPolicy: FAIL` refuses an open holder (`409 BUSINESS_ID_IN_USE`) instead of updating it.
- With a start delay, the new run is held and still takes the Update.

## 5. Reading the refusals

| Answer | Reason | Means |
|---|---|---|
| `409` | `BUSINESS_ID_IN_USE` | an open execution holds it (`details.runId` names it when the caller may read runs) |
| `409` | `BUSINESS_ID_REUSE_REJECTED` | a closed holder is still remembered and the policy refuses (`details.reusableAfter`) |
| `422` | `BUSINESS_ID_INVALID` | not 1 to 200 visible ASCII characters, an unknown policy, or a policy without an identifier (`details.field`) |
| `422` | `START_DELAY_INVALID` | the delay is outside 1 ms to 366 days (`details.field` is `/startDelayMs`) |
| `409` | `SIGNAL_NOT_ACCEPTED_AT_START` | the new run has no node waiting for that signal; nothing was created |
| `409` | `NOT_WAITING` | the open holder has no node waiting for that name now |
| `409` | `IDEMPOTENCY_KEY_REUSED` | the same key, another request (also another delay) |
| `404` | | the identifier is unbound, forgotten, impossible, or another tenant's |

## 6. What is not built yet

- **Selecting runs by identifier prefix in a batch job**: refused with `BATCH_SELECTOR_UNSUPPORTED`; select by the other filters
  or by run ids (the run list does filter by prefix, so list first, then give the ids).
