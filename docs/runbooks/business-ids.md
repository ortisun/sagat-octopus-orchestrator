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

Each `409` of the first two rows is audited as `business_id.conflict` (`GET /audit?action=business_id.conflict`): the person who asked,
the workflow they asked to start (`resourceType` `workflow`) and the closed reason, `BUSINESS_ID_IN_USE` or `BUSINESS_ID_REUSE_REJECTED`.
**The identifier is not in the record** (it is customer text). The record is written after the refused start rolled back, in a
transaction of its own, so nothing of the start commits with it; a failure to write it is logged (`failed to record audit entry`) and
the answer is the same `409`. The decision of a schedule that finds its id held is audited the same way, by the schedule
(`system:schedule:<id>`, `resourceType` `schedule`), in the transaction of its decision. `USE_EXISTING`, which answers with the holder,
is no conflict and writes no such record (G2086, D2218).

## 6. The identifiers a schedule makes, and selecting runs by prefix

**A schedule's action may name a prefix** (`businessIdPrefix`, 1 to 160 characters, with `businessIdConflictPolicy` and
`businessIdReusePolicy`). Every run it starts is bound to `<prefix>-<nominal time>` (RFC 3339, UTC, to the second, ending in `Z`:
`order-2026-11-10T12:00:00Z`), in the transaction that makes the run, with the policies of the action. What an action does when
the id is taken (a recorded decision, never a run, never a retry) is in
[`schedules.md`](schedules.md#11-the-business-id-of-the-runs-of-a-schedule-fr-018-016). The prefix is customer content like the
id: it is kept in the schedule's revision (charged as content) and in the ids it makes, in no log, label, span, audit reason or
event. Two schedules with the same prefix compete for the same ids at the same instant: one of them gets the run and the other
the decision `BUSINESS_ID_CONFLICT` (or, with `USE_EXISTING`, `ALREADY_ACTED` naming the run that has it); give each schedule
its own prefix.

**A batch job can select runs by the prefix of their id** (`selector.filter.businessIdPrefix`, 1 to 200 visible ASCII
characters; the Studio's run list filter carries into *Batch operation…*; `sagat batch create --business-id-prefix`):

- **A literal prefix.** `%`, `_` and `\` match themselves: the match is a range over the ids (`id >= prefix AND id < prefix with
  its last character one step up`), not `LIKE`, and `%` alone selects nothing. It is case-sensitive, like the id.
- **Runs, not bindings.** It matches the id **the run carries** (`runs.business_id`), so a run whose binding was purged after its
  window (the binding exists only for an open holder and a recently closed one) is still selected, and a run whose id was erased by
  feature 020 (the column set to null) is not. It is the namespace's runs only, whatever the tenant: another namespace's and another tenant's
  runs are never reached.
- **Counted like every other filter.** The preview counts up to `maxItems + 1`; a prefix that matches nothing counts zero and a job
  over it completes with no item (it is not an error). It composes with the other filters (status, workflow, creation range).
- **Cost.** The selection reads `runs_business_id_idx (tenant, namespace, business_id text_pattern_ops)` as a range, a page of
  500 runs by run id at a time, so a selective prefix does not read every run of the namespace; a prefix that matches most of the
  namespace's runs (`o`, for ids that all begin with `o`) reads most of them, as an unfiltered job does, and sorts each page by run id.
  The preview is cut off at five seconds.
- **The prefix is customer text**: it is kept in the job (`batch_jobs.selector`, charged as content) and nowhere else. A read of a job never
  returns it (the Studio's job page does not show it either, G2084).

What is not built yet: nothing here; the `020` query selector is refused with `QUERY_UNSUPPORTED_OPERATOR` until that feature is
approved.
