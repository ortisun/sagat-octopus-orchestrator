# Feature Specification: Flow control and step policies

**Feature Branch**: `feature/022-flow-control-step-policies`

**Created**: 2026-10-26

**Status**: Draft — specification, plan and tasks proposed; not implemented; pending owner review

**Release 0.3.0 slice (2026-11-13)**: the owner put throughput control in release 0.3.0 (OD-2026-11-13-11,
quoted in [research.md](research.md#owner-decision-of-2026-11-13-throughput-control-in-release-030)): US02
(concurrency limits by key, FR-022-017..022) as specified, plus rate limits per time window with buffering,
added below as US07 (FR-022-039..050, NFR-022-007..008, SC-022-017..023). US01 and US03–US06 are not in the
slice. The slice, its tasks and what it leaves out are in [Release 0.3.0 slice](#release-030-slice). Nothing
of it is implemented.

**Input**: Bring Windmill/Temporal flow control to the platform without changing its execution model: a
declared **step result cache** (key = canonical input digest + pinned identity + scope, TTL, retained and
classified entries, journaled hits, explicit invalidation); **early return** of a workflow's declared
result to waiting callers before trailing steps finish; a workflow **`onFailure` handler** (node region or
child workflow, with failure context) and a **namespace default failure handler** started through the
outbox, after saga compensation and defined against cancellation; **concurrency limits by key** (key
expression, maximum concurrent runs or steps per key, durable semaphores, queued admission with timeout,
fairness, quotas); **standalone steps** (one activity, script or managed operation started durably without
a workflow, generalizing 017 ad hoc admission); and a **draft test mode** (pinned/mocked step outputs and
run-from-node in draft test runs, never in production, same engine semantics).

## Context and delivery boundary

The engine interprets a declarative, versioned IR (ADR-0003) with PostgreSQL as authority (ADR-0002):
journal, state, next tasks, timers, subscriptions and outbox commit together; a run pins one immutable
version. This feature adds policies on that model and adds no code replay, worker protocol minor, run
status or `Error.code`. Temporal mechanisms that exist only for code replay (local activities, sticky
queues, `patched`/`getVersion`, side-effect markers) do not apply: a cache hit, an early result and a
mock are journal facts.

Reused, not rebuilt:

- **Ad hoc admission (017)**: private immutable definitions per tenant, namespace, actor and digest, then
  an ordinary run (`crates/application/src/runs/ad_hoc.rs`, ADR-0035 decision 9). Standalone steps and
  draft test runs generalize it.
- **Script test runs (005)**: drafts run without namespace secrets, with their own quota (FR-005-005).
- **Run result (002 G21, 016)**: `end.outputExpr` is stored as `runs.result_digest` and disclosed by
  `POST /runs/{runId}/payload-disclosures` `{"source":"result"}` under the pinned carrier (FR-016-006).
  Early return sets that same result earlier.
- **Per-activity concurrency (010)**: `activity_quota_limits` counts leases at claim time (migration
  `20261019000000_activity_quotas`). Concurrency by key generalizes the key and adds run permits.
- **Sagas (003, ADR-0010)**: a stopping saga winds down, compensates and ends before the run finishes.
  Failure handlers run after that, never instead of it.
- **Obligations (ADR-0012, ADR-0025)**: rows written with the fact, processed by a leased worker with
  retry, `DEAD` and redrive. The namespace failure handler is such an obligation.

Boundaries: **reset/restart from a point of a production run is 018**; "run from a chosen node" here exists
only for draft test runs. Draft test runs use their own semaphore pool and are refused as sources of 018
reset, rerun and batch operations (018 refusal `RESET_SOURCE_TEST_RUN`). Every run creator, including 018
schedule firing, batch reruns, reset and Signal/Update-with-Start, admits through this feature's permit path
(FR-022-018); a reset run acquires its permit at reset admission and cannot queue, and every journal event
type added here declares its reset rule in 018's registry (FR-022-038). Semaphores take slot 7 of
ADR-0050's merged admission lock order. Erasure (020) deletes cache entries by source run through 020's
content-store registry (FR-022-037). Business-ID
policies for standalone executions are 018; priority/fairness keys are 019 (grants are oldest first until
then); run-and-wait and Streams are 026 (they consume the early result); the time-skipping clock is 028;
encrypted fields (031) are opaque to keys (FR-022-036).

Rate limits (US07, added on 2026-11-13) reuse the same pieces as concurrency keys: the key expression, the
keyed digest and its label, groups, the one admission port, the queue ceilings and the shard-fenced grant
(ADR-0056). What differs is that a rate slot is freed by time, not by a holder finishing, so the count is a
durable window of grants on the database clock and every grant to a buffered waiter is made by a scheduler
pass (proposed [ADR-0084](../../docs/adr/ADR-0084-rate-limits-are-a-durable-window-of-grants-on-the-database-clock-and-the-scheduler-grants-never-the-claim.md)).
They are distinct from the tenant rate quotas of feature 009 (`api_requests_per_second`,
`events_per_second`), which refuse a request and never buffer, and from 018's schedule overlap policies
(`BUFFER_ONE`, `BUFFER_ALL`), which buffer a schedule's actions before a run exists.

## User Scenarios & Testing *(mandatory)*

### US01 — Reuse a step's recent result (Priority: P1)

An author marks an expensive, side-effect-free lookup as cacheable for a period; runs that reach it with
the same key reuse the stored result instead of dispatching work.

**Why this priority**: repeated deterministic lookups dominate cost and latency; worker-side caches have no
tenant boundary and no audit.

**Independent Test**: run a workflow whose activity declares `cache` (TTL 60 s) twice with one key — the
second run leases nothing; run after expiry and after invalidation — both lease again; replay both journals.

**Acceptance Scenarios**:

1. **Given** `cache: {key: "input.sku", ttlSeconds: 60}`, **when** a second run reaches the step with the
   same `sku` within 60 s, **then** the step completes with the stored output, without lease or attempt,
   and `step.cache_hit` is journaled in the completing transaction.
2. **Given** an entry, **when** it has expired on the database clock, was invalidated, or the publisher can
   no longer use a secret the step declares, **then** the lookup misses and the step is dispatched.
3. **Given** a node with `compensateWith`, `reconcileWith`, `irreversible`, `highRisk`, or a managed
   database node, **when** it declares `cache`, **then** publication fails with `CACHE_WITH_EFFECT`.
4. **Given** a completed miss and a full content quota, **when** the entry would be stored, **then** step and
   run are unaffected and the skipped store is only counted.

---

### US02 — Limit concurrent work per business key (Priority: P1)

An author allows at most N active runs (or N attempts of a step) per business key, e.g. one run per
account. Excess work waits durably in admission order without a lease and gives up after a timeout.

**Why this priority**: per-key serialization is otherwise built with external locks that lose state.

**Independent Test**: five same-key runs at `limit: 1`; kill the engine while two wait; restart; permits
are granted one at a time in admission order, semaphore rows equal the journals' open acquisitions, and a
waiter whose timeout races a grant ends in exactly one outcome.

**Acceptance Scenarios**:

1. **Given** workflow-level `concurrency: {key: "input.accountId", limit: 1}`, **when** a second same-key
   run is admitted, **then** it is `WAITING` (wait kind `CONCURRENCY`) with no ready step and counts as an
   active run.
2. **Given** a waiting run, **when** the holder becomes terminal, **then** the permit is released in that
   transaction and granted to the oldest waiter its own limit admits, in a later fenced transaction that
   creates its first tasks.
3. **Given** a grant racing the queue timeout, **when** both commit attempts happen, **then** the first commit
   wins (run starts, or ends `FAILED`/`CONCURRENCY_WAIT_TIMEOUT`) and the other is an audited no-op.
4. **Given** a step-level limit, **when** more attempts are ready than permits, **then** extra steps stay
   `READY` without spending attempts and fail `CONCURRENCY_WAIT_TIMEOUT` through their failure route
   after their queue timeout.
5. **Given** a waiting run, **when** it is cancelled, **then** it leaves the queue in that transaction.

---

### US03 — Handle a failed workflow (Priority: P1)

An author declares what happens when a run fails for good: a handler region or a child workflow receiving
the failure context (notify, open a ticket). An administrator sets a namespace default handler for runs
that declare none.

**Why this priority**: failed runs are only found by looking; customers build polling notifiers.

**Independent Test**: a saga whose second step fails and whose definition declares an `onFailure` child:
journal order is compensated → handler started → handler finished → run `COMPENSATED`; an operator-cancelled
run starts no handler. With a namespace handler, kill the dispatcher between commit and admission: exactly
one handler run with provenance exists after restart.

**Acceptance Scenarios**:

1. **Given** a permanent failure inside a saga, **when** the saga stops, **then** compensation reaches its
   outcome before the handler starts, and the final run status is the no-handler status.
2. **Given** a handler start, **then** it receives a closed, payload-free failure context (pending status,
   code, node, attempt, kind, time).
3. **Given** a handler that fails or times out, **when** it ends, **then** the run finishes with its pending
   status, the handler outcome is recorded and the handler is not invoked for its own failure.
4. **Given** a namespace default handler and a run without `onFailure` ending `FAILED`, **when** the terminal
   transaction commits, **then** it contains a dispatch obligation, and exactly one handler run with
   `handlerFor` is admitted whatever the dispatcher crashes.
5. **Given** a cancelled run, a handler run or a test-tree run, **when** it ends, **then** nothing is
   dispatched.

---

### US04 — Return a result early to waiting callers (Priority: P2)

An author returns the business answer as soon as it is known while trailing steps continue in the run.

**Why this priority**: callers waiting for the full run pay for work they do not need.

**Independent Test**: `validate → earlyReturn → notify (slow) → end`: the result is disclosed while the run
is `RUNNING`; the run then ends `SUCCEEDED` with the same result; if `notify` fails it ends `FAILED` and
keeps the result.

**Acceptance Scenarios**:

1. **Given** an `earlyReturn` node, **when** reached, **then** its `result` is evaluated, stored as the run
   result, journaled as `result.returned`, announced through the outbox, and `next` is scheduled, in one
   transaction.
2. **Given** an early result, **when** an authorized caller asks for the run result, **then** the existing
   disclosure returns it under the pinned `outputSchema` carrier while the run is running.
3. **Given** a returned result, **when** a later step fails or the run is cancelled, **then** the run ends
   with that status and keeps the early result.
4. **Given** an `earlyReturn` in a saga body, parallel branch, body or handler region, or one from which an
   `end` with `outputExpr` is reachable, **when** published, **then** it fails with
   `EARLY_RETURN_PLACEMENT`.

---

### US05 — Execute one step without authoring a workflow (Priority: P2)

A developer starts one activity, script or managed database operation durably (retries, timeout,
idempotency, audit) and inspects or cancels it like any run.

**Why this priority**: one-off jobs do not justify a published workflow; 017 proved the pattern for
database operations only.

**Independent Test**: start a standalone activity, lose the response, retry with the same key and body
(same run) and with another body (conflict), cancel a second execution while leased, disclose the first
result.

**Acceptance Scenarios**:

1. **Given** `POST /namespaces/{namespaceId}/step-executions` with an activity node, **when** admitted,
   **then** a private one-node definition is retained or reused and a run with origin `STANDALONE_STEP`
   commits with its task, journal, audit, receipt and outbox.
2. **Given** a lost response, **when** the same key and body are resent, **then** the same run is returned
   after reauthorization; a changed body is `CONFLICT`.
3. **Given** a node with `compensateWith`, **when** submitted, **then** it fails with `VALIDATION_FAILED`.
4. **Given** a standalone run, **then** listing, inspection, cancellation and result disclosure use the
   existing run routes and rules.

---

### US06 — Test a draft with pinned or mocked outputs (Priority: P2)

An author runs a draft from a chosen node, supplying upstream outputs (by hand or pinned from a previous
run) and mocking steps they do not want executed.

**Why this priority**: otherwise an author publishes to test a late step and re-executes every earlier,
possibly effectful, step.

**Independent Test**: in a `dev` namespace, test from the fourth node with three upstream pins (one from a
previous run) and one downstream mock: no lease for pinned nodes, each journaled `step.mocked`, ordinary
reducer elsewhere; the same request in a `prod` namespace is refused.

**Acceptance Scenarios**:

1. **Given** a draft at revision R, **when** a test run names `draftRevision: R`, **then** a private
   definition is retained and a `DRAFT_TEST` run admitted; a moved draft is `CONFLICT`; nothing is
   published.
2. **Given** `startAt: X`, **when** a node reachable from X references `steps.<id>.output` of an unpinned
   node not reachable from X, **then** the request fails with `MISSING_PIN` before any run exists.
3. **Given** a mocked node, **when** reached, **then** it completes without lease, and a later saga
   compensation does not dispatch a compensation for it.
4. **Given** a `prod` namespace, **when** a test run is requested, **then** it fails `FORBIDDEN`
   (`TEST_RUN_PRODUCTION_NAMESPACE`) and nothing is retained.
5. **Given** a test run, **when** it ends, **then** it created no outbound delivery, no namespace handler
   dispatch and no shared cache entry.

---

### US07 — Limit how often work starts per business key, buffering the excess (Priority: P1)

An author allows at most N run starts (or N dispatches of a step) per second, minute or hour for a business
key, for example 100 calls a minute to one provider account. Work beyond the limit is not refused and does
not fail: it waits durably, in arrival order, without a lease or a worker, and starts as the window frees.

**Why this priority**: a downstream system with a quota is otherwise protected by sleeps in worker code,
which lose their state on restart and are invisible to the operator; the owner asked for it in release
0.3.0 (OD-2026-11-13-11).

**Independent Test**: 30 same-key runs at `rateLimit: {limit: 10, per: "second"}` started at once; kill the
engine while 20 wait; restart. From the journals alone: no window of one second on the database clock holds
more than 10 `rate_limit.granted` events of the key, grants follow admission order, every run was granted
exactly once, and a second key started in the middle of the flood is not delayed.

**Acceptance Scenarios**:

1. **Given** workflow-level `rateLimit: {key: "input.accountId", limit: 10, per: "second"}` and no grant
   of the key in the last second, **when** 10 same-key runs are admitted, **then** each is granted in its
   admission transaction (`rate_limit.granted`) and starts at once.
2. **Given** 10 grants of the key in the last second, **when** an eleventh run is admitted, **then** it is
   `WAITING` (wait kind `RATE_LIMIT`) with `rate_limit.queued`, no ready step, a timer at its queue
   deadline, and it counts as an active and as a queued run.
3. **Given** buffered runs of one key, **when** grants age out of the window, **then** a scheduler pass
   grants the oldest waiters first, each in its own fenced transaction that journals the grant and creates
   the run's first tasks; no replica's own clock is read.
4. **Given** a grant racing the queue timeout or a cancellation, **when** both try to commit, **then** the
   first commit wins (the run starts, or ends `FAILED`/`RATE_LIMIT_WAIT_TIMEOUT`, or `CANCELLED`) and the
   other changes nothing; a waiter that leaves the queue consumed no slot.
5. **Given** a node-level `rateLimit`, **when** more attempts would become ready than the window admits,
   **then** the extra steps are not claimable, spend no attempt and hold no lease until granted, and at
   most `limit` attempts of the key are leased in any window; a step waiting past its queue timeout fails
   `RATE_LIMIT_WAIT_TIMEOUT` through its failure route.
6. **Given** both `concurrency` and `rateLimit` at workflow level, **when** a run cannot have both, **then**
   it holds neither: it is granted the permit and the slot in one transaction, or it waits.
7. **Given** a full per-key buffer or a full namespace `queued_runs`, **when** a synchronous start arrives,
   **then** it is refused with `QUOTA_EXCEEDED`/`RATE_LIMIT_QUEUE_FULL` and no run exists; an event or
   cron start stays pending under its retry.
8. **Given** a buffered run, **when** it is inspected, **then** it shows that it waits for a rate slot,
   since when, its queue deadline, its position in the key's queue and an estimated grant time marked as an
   estimate, with the key shown only as a label.

### Edge Cases

- A cache key expression errors or exceeds its bound: the lookup is `bypassed`; never a run failure.
- Two concurrent misses on one key both execute; the first store wins, the second is a counted duplicate.
- A wrong cached value is removed by invalidation; runs that already hit keep their journaled completion.
- Retries of a cached step are attempts of a miss; failures are never cached.
- An early-return expression fails: the run ends `FAILED`/`EXPRESSION_ERROR` without a result; handlers
  apply. Pause, timeout or compensation after an early result never change the result.
- A step failing with an `onError` route inside a handler region follows the route; the region's failure
  ends the handler `FAILED`, never changes the run status.
- The namespace handler workflow has no active version: the dispatch retries, then becomes `DEAD` with an
  alert and can be redriven; the failed run is unaffected. Disabling the handler does not cancel
  obligations already committed.
- A permit holder in `MANUAL_REVIEW` keeps its permit (not terminal); waiters time out on their own.
- A parent holding a run permit starts a same-key child with `limit: 1`: the child waits until its queue
  timeout; publication warns `CONCURRENCY_SELF_CHILD`.
- A lowered limit in a new version does not revoke holders; acquisitions compare with the acquirer's limit,
  and an older waiter whose own lower limit is reached does not block a younger waiter whose limit admits
  it (no head-of-line blocking).
- A waiter that times out or whose key cannot be evaluated never held a permit: it finishes without its
  workflow failure handler; only the namespace handler may be dispatched.
- Pins referencing another namespace, a blob output or a value with `secret` fields are refused.
- A draft reaching an unpinned 021, 024, 025 or 030 effect node from `startAt` is refused
  (`TEST_RUN_UNMOCKED_EFFECT`); 017 database nodes still execute against the namespace's connections.
- `MISSING_PIN` checks expression references only: worker code that reads an unpinned upstream output from
  its run data document sees it absent. The Studio warns about unpinned upstream nodes.
- A standalone script with an unknown artifact digest is refused as at publication (`UNKNOWN_SCRIPT`).

Rate limits (US07):

- The limit is lowered while runs wait (a new version, or a lower namespace policy ceiling): no grant is
  revoked; each waiter is evaluated at its next grant against its own pinned `limit` and `per` clamped by
  the current policy, so a lowered policy applies to waiters at once and a lowered declaration to new runs.
- Two definitions share an explicit group with different `limit` or `per`: each acquirer counts the shared
  grants over its own `per` against its own `limit`; publication warns `RATE_LIMIT_MISMATCH`; an older
  waiter its own limit refuses does not block a younger one (no head-of-line blocking, as D1196).
- The engine is down longer than the window: on restart every waiter is still buffered, the window has
  aged on the database clock, and the first pass grants at most `limit` per key; nothing is granted twice.
- The database clock moves backwards (a failover to a server whose clock is behind): grants counted "in
  the future" keep counting, so the limit is under-delivered until the clock catches up and never exceeded.
- Workers stop while steps hold granted slots: a granted step that no worker leases keeps its slot, so at
  most `limit` granted steps of a key are ever unleased and a returning worker cannot lease a backlog
  larger than the limit.
- A retry of a rate-limited step asks for a new slot when its backoff ends; the wait for the slot is not an
  attempt and does not consume the retry budget.
- A granted step is cancelled or its run ends before any worker leases it: its slot is returned in that
  transaction. A leased attempt never returns its slot, whatever its outcome.
- A flood of starts on one key fills that key's buffer and is refused beyond it; other keys, other
  workflows and other tenants keep being granted in the same passes.
- A buffered run whose key cannot be evaluated never existed as a waiter: it ends
  `FAILED`/`EXPRESSION_ERROR` in its admission transaction, as for concurrency (D1180).
- A schedule (018) starts a rate-limited workflow: the buffered run is an open run for the overlap policy,
  so `SKIP` skips and `BUFFER_ONE`/`BUFFER_ALL` buffer the next action in 018; the two buffers never hold
  the same start.
- A collection body wants a rate limit per item: `rateLimit` on a body node is refused in this slice
  (`SCOPED_RATE_LIMIT`, G1175); a body `child` node that starts a workflow declaring a top-level
  `rateLimit` is buffered through the admission port like any other start.

## Requirements *(mandatory)*

### Functional Requirements

#### Step result cache (US01)

- **FR-022-001 — Cache declaration**: A root `activity` or `script` node MUST be able to declare
  `cache: {key, ttlSeconds, scope?, name?, version?}`: `key` a required bounded expression, `ttlSeconds`
  required, `scope` `version` (default) | `workflow` | `namespace`, `name` required for `namespace`,
  `version` an author separator. Publication MUST refuse `cache` with `compensateWith`, `reconcileWith`,
  `irreversible` or `highRisk`, on database or other node kinds (`CACHE_WITH_EFFECT`,
  `CACHE_POLICY_INVALID`), and in bodies (`UNSUPPORTED_NODE`, dependency `SCOPED_CACHE`).
- **FR-022-002 — Cache identity**: An entry MUST be identified by tenant, namespace, scope identity (pinned
  version, workflow or cache name), node fingerprint and key digest. The fingerprint MUST cover the
  executable identity (activity name or artifact digest), the canonical key expression, the author
  `version`, the `outputSchema` carrier digest and the classification rules version. The key digest MUST
  be a keyed digest of the canonical key value; the raw key MUST NOT be persisted outside the run's data.
- **FR-022-003 — Hit semantics**: When a cacheable step becomes ready, the engine MUST evaluate the key over
  committed run data and, on a hit, in the same transaction copy the entry's output into the run's
  retained payloads (charged like any output), journal `step.cache_hit` (entry, source run/node/attempt,
  payload digest, expiry) and the ordinary completion, and continue scheduling, with no lease or attempt.
  Replay MUST reach the same state from the journal and the run's payloads without reading the cache.
- **FR-022-004 — Liveness and authorization of a hit**: A hit MUST require a matching entry whose expiry is
  later than the database clock in that transaction and whose scope was not invalidated after its
  creation. For a step declaring secrets, the publisher MUST currently be allowed to use them (metadata
  check, no value resolved); otherwise it is a miss. A key error or oversized key MUST bypass the cache.
- **FR-022-005 — Storing an entry**: A successful completion of a miss with an inline output MUST write a
  store obligation in its completion transaction; a leased writer MUST insert the entry from the run's
  immutable retained output, insert-if-absent, with expiry from the database clock. Storing MUST NOT fail,
  delay or roll back the step or run: quota exhaustion, blob output, a full backlog and an existing entry
  are counted outcomes. Failures, cancelled attempts and outputs refused by secret scanning are never stored.
- **FR-022-006 — Invalidation and retention**: An authorized principal MUST be able to invalidate by
  workflow and node (optionally one version), by key value (sent in the body, digested server-side), or by
  shared cache name. Invalidation and expiry purge MUST release bytes in the deleting transaction and MUST
  NOT change any run that already hit. Listings MUST NOT return values or raw keys.
- **FR-022-007 — Classification and retained content**: Entries MUST be retained content charged to
  `tenant_content_bytes` (ADR-0027) and in the storage classification manifest. A consuming run discloses
  a copied output only through its own pinned carrier, identical to the producer's by FR-022-002. Cache APIs
  MUST NOT disclose a value.

#### Early return (US04)

- **FR-022-008 — Early-return node**: The IR MUST gain an additive `earlyReturn` kind with required
  `result` and `next`. Publication MUST refuse it outside the root graph, in a saga body, parallel branch,
  body or handler region, reachable from another `earlyReturn`, with `continueAsNew`, or when an `end` with
  `outputExpr` is reachable from it (`EARLY_RETURN_PLACEMENT`).
- **FR-022-009 — Early-return transition**: Reaching it MUST, in one transaction, evaluate `result`, store it
  under the run-result bounds (256 KiB, content quota), set `runs.result_digest`, journal `result.returned`,
  write the `run.result_returned` outbox and delivery records, and schedule `next`. Evaluation failure MUST
  fail the run with `EXPRESSION_ERROR`, an oversized value with `RUN_DATA_LIMIT`. The status MUST NOT change.
- **FR-022-010 — Visibility of an early result**: The run resource MUST expose `resultSource`
  (`EARLY_RETURN` | `END`) and `resultAvailableAt`. The existing result disclosure MUST serve an early
  result before the run is terminal, with its existing permissions. The SSE stream and journal MUST show
  `result.returned` without payload. Subscriptions MUST be able to select the payload-free event type
  `run.result_returned`.
- **FR-022-011 — Outcome after an early result**: The run MUST finish with the status its later progress
  produces and keep the early result; a later `end` MUST record that the result was already returned. A
  parent's `child` node MUST keep waiting for the child's terminal status (G1162).

#### Failure handlers (US03)

- **FR-022-012 — Workflow handler declaration**: A definition MUST be able to declare top-level
  `onFailure` with exactly one of `node` (handler region entry) or `workflow` (+ `version`, `input`), plus
  optional `on` (subset of `FAILED`, `TIMED_OUT`, `COMPENSATED`, `PARTIAL`, `COMPENSATION_FAILED`; all by
  default) and `timeoutMs`. A region MUST be disjoint from the graph reachable from the entrypoint, hold
  only activity, script, condition, switch, timer, parallel, child and end nodes, and its ends MUST NOT
  declare `outputExpr` (`FAILURE_HANDLER_REGION`). `onFailure` with `continueAsNew` MUST be refused.
- **FR-022-013 — Handler ordering and context**: When a run would finish with a status in `on`, the engine
  MUST first complete the existing stop (cancel unstarted work, settle in-flight attempts, finish
  compensation and reconciliation), then journal `failure_handler.started` with the pending status and
  enter the handler with the closed context `failure: {status, code, node, attempt, kind, occurredAt}`
  (no payload, message text or secret). A run in `MANUAL_REVIEW` is not terminal and MUST wait. A run that
  ends before it ever acquired its run permit (queue timeout, key error at admission) MUST finish directly,
  without its workflow handler, because the handler would run outside the key limit; the namespace handler
  still applies. A run holding a permit keeps it through its handler phase.
- **FR-022-014 — Handler outcome**: A handler MUST NOT change the outcome: when it ends (succeeded, failed,
  timed out, cancelled) the engine MUST journal `failure_handler.finished` and finish the run with the
  pending status in the same transaction. A handler's failure MUST NOT re-invoke it. Cancellation during the
  handler MUST stop its work and finish with the pending status. `CANCELLED` runs MUST NOT start a handler.
- **FR-022-015 — Namespace default handler**: An administrator MUST be able to set, read and clear one
  default handler per namespace (workflow, optional pinned version, statuses, enabled flag; optimistic
  revision, justification, audit). It applies to runs whose version declares no `onFailure` or whose
  handler did not succeed.
- **FR-022-016 — Dispatch through the outbox**: A qualifying terminal transaction MUST insert one dispatch
  obligation keyed by the failed run, using the configuration read under its policy lock there. A
  shard-fenced dispatcher MUST admit the handler run (closed failure context and source identity as input;
  provenance `handlerFor`; initiator `system:engine`; authority of the handler version's publisher) and mark
  the obligation in the same transaction, so exactly one handler run exists per failure. Refused admissions
  MUST retry with bounded backoff, then become `DEAD`, alerted and redrivable. Handler runs (dispatched or
  `onFailure` children), test-tree runs and cancelled runs MUST NOT be dispatched; a failed `onFailure`
  child is covered by its parent's dispatch.

#### Concurrency limits by key (US02)

- **FR-022-017 — Concurrency declaration**: A definition MUST be able to declare top-level `concurrency`
  (run permits) and root `activity`/`script` nodes node-level `concurrency` (step permits), each with a
  positive `limit`, optional `key` (run level over `input`, step level over run data), optional `group`
  (default derived from the workflow, or workflow and node; an explicit group is shared in the namespace at
  the same level) and optional `queueTimeoutMs`. Run and step permits never share a semaphore. Body nodes
  (`UNSUPPORTED_NODE`, `SCOPED_CONCURRENCY`) and combination with `continueAsNew` MUST be refused.
- **FR-022-018 — Run permits**: Admission MUST evaluate the key and, in its transaction, acquire a permit
  (`concurrency.acquired`) or enqueue the run (`concurrency.queued`, `WAITING`/`CONCURRENCY`, timer at the
  queue deadline, no task). Every run creator MUST use this one admission port (plan, Run creators): start
  API, event and cron triggers, child nodes, reruns, 017 ad hoc and probes, 022 standalone, draft-test and
  handler dispatch, 018 schedule firing, batch reruns, Signal/Update-with-Start; a run with an 018 start
  delay acquires or enqueues when the delay elapses. A **reset** run (018) MUST acquire a fresh permit in
  its reset admission transaction and MUST NOT queue: without a free permit the reset is refused
  (`CONCURRENCY_PERMIT_UNAVAILABLE`, 018), and `concurrency.*` events copied from the source prefix are
  history (FR-022-038). A key error MUST end the new
  run `FAILED`/`EXPRESSION_ERROR` in that transaction. A holder's terminal transaction MUST release its
  permit. Grants MUST be made by the waiter's shard owner under its fence to the oldest unpaused waiter
  whose own declared limit admits one more holder (an older waiter at its own lower limit does not block
  it); when 019 is present, by (019 priority 1 first, enqueue time ascending) among those waiters; creating
  the waiter's first tasks and outbox; a grant and the queue timeout MUST be decided by first
  commit (ADR-0056).
- **FR-022-019 — Step permits**: Claiming a step with node-level `concurrency` MUST acquire a permit after
  the activity-quota checks of the claim transaction; the transaction ending the attempt (completion,
  failure, cancellation, lease expiry) MUST release it. Without a permit the step MUST stay `READY` without
  spending an attempt; among steps a claiming worker may serve, same-key steps MUST be claimed oldest first
  (no strict cross-pool order); a step ready beyond its queue timeout MUST fail `CONCURRENCY_WAIT_TIMEOUT`
  through its failure route.
- **FR-022-020 — Durable semaphores**: Holders, waiters and counters MUST be PostgreSQL rows keyed by
  tenant, namespace, pool (ordinary or draft-test), permit scope, group and keyed digest, changed only in
  the transactions above, after each path's own locks and its `runs` write (per-path orders in ADR-0056
  decision 2). Holders of a key MUST equal the open, non-`restored` acquisitions in its runs' journals,
  verified after fault tests. Empty rows MUST be removed. A new digest key version MUST NOT be activated
  while any holder or waiter exists under the previous one.
- **FR-022-021 — Quotas and lifecycle**: Queued runs MUST count against `active_runs` and
  `tenant_active_runs` and against a separate namespace `queued_runs` ceiling, so waiting can never hold
  more than that share of the namespace; a full `queued_runs` refuses like a full per-key queue. Step permits MUST add to `activity_concurrency`; a full per-key queue MUST refuse
  a synchronous start (`QUOTA_EXCEEDED`, `CONCURRENCY_QUEUE_FULL`) and leave event/cron starts pending under
  their retry. Any terminal transition of a queued run (queue timeout, cancellation, run deadline) MUST
  dequeue it in that transaction. Pausing a holder MUST NOT release its permit; a paused waiter is skipped
  by grants until resumed. Lower limits MUST NOT revoke holders.
- **FR-022-022 — Visibility**: A run MUST show its concurrency state (holding or waiting, since, queue
  deadline, group, key label). A namespace view MUST list keys with holders, waiters and oldest wait, by key
  label only. Raw key values MUST NOT appear in any response, log, span, metric or audit.

#### Standalone steps (US05)

- **FR-022-023 — Standalone admission**: `POST /namespaces/{namespaceId}/step-executions` (idempotency key
  required) MUST accept exactly one `activity`, `script` or `database` node, input and an optional timeout;
  build a private one-node definition whose result is the node's output and whose `outputSchema` is the
  node's; retain or reuse it per tenant, namespace, actor and digest; and admit a run with origin
  `STANDALONE_STEP` in the existing admission transaction. Same key and body MUST replay after
  reauthorization; changed content MUST be `CONFLICT`.
- **FR-022-024 — Standalone semantics**: The node's retry, timeout, secrets, idempotency template,
  `reconcileWith`, `cache` and node `concurrency` MUST behave as in a workflow; `compensateWith` MUST be
  refused. A database node MUST keep every 017 rule; when feature 021 exists, its typed managed operation
  nodes MUST be accepted under their own rules. `database-executions` MUST remain and share the path.
- **FR-022-025 — Standalone operations**: Standalone runs MUST be listed (`GET /runs?origin=STANDALONE_STEP`),
  inspected, cancelled, retried and disclosed through existing routes; the five SDKs MUST offer start and
  execute-and-wait; the Studio MUST offer a start form and a list filter.

#### Test mode for drafts (US06)

- **FR-022-026 — Draft test run**: `POST /workflows/{workflowId}/test-runs` (idempotency key required) MUST
  take `draftRevision`, `input`, optional `startAt`, `pins` and deadline; validate the draft at that
  revision with publication rules and enabled capabilities; retain a private definition (origin
  `DRAFT_TEST`, publisher the actor); and admit a run. A moved draft MUST be `CONFLICT`; nothing is
  published. A `prod` namespace MUST refuse (`FORBIDDEN`, `TEST_RUN_PRODUCTION_NAMESPACE`).
- **FR-022-027 — Pins and mocks**: A pin MUST be `{mock}` or `{fromRun, fromNode?}` on an activity, script,
  child or waitEvent node, or, when those features exist, on a 021 `resourceOperation`, 024 data, 025 agent
  or 030 `serviceCall` node; it is held only in the request and the run's data, never in a definition. A
  draft whose graph reachable from `startAt` contains an unpinned node of one of those four kinds MUST be
  refused before admission (`VALIDATION_FAILED`, `TEST_RUN_UNMOCKED_EFFECT`, naming the node). A pinned node
  MUST complete (at admission for upstream pins, else when reached) with `step.mocked` (source, payload
  digest), without lease, attempt, cache lookup, child run or wait, and MUST be excluded from compensation
  and reconciliation planning.
- **FR-022-028 — Start from a chosen node**: `startAt` MUST be a root node outside saga bodies, parallel
  branches, bodies and handler regions. Each node reachable from it that references an output of a node not
  reachable from it MUST have that node pinned (`MISSING_PIN`); upstream pins MUST commit in the admission
  transaction; unreachable unpinned nodes MUST NOT run.
- **FR-022-029 — Pin from a previous run**: `fromRun` MUST be a run of the same tenant, namespace and
  workflow whose node succeeded with an inline output. The caller MUST be able to disclose the whole value
  under the source carrier at pin time (`secret` refuses; `pii` needs the reveal grant and is audited). The
  pin MUST record the strictest level found as a floor; the test run MUST disclose that output at the
  stricter of floor and draft carrier.
- **FR-022-030 — Isolation of test runs**: A test run and its child runs (test tree) MUST NOT create
  outbound deliveries, dispatch namespace handlers, fire triggers, or read/write `workflow`/`namespace`
  cache scopes; they MUST acquire permits only in the draft-test semaphore pool, never blocking or being
  blocked by ordinary runs; they MUST count against test-run quotas and ordinary run quotas; and they
  MUST NOT be accepted as sources of 018 reset, rerun or batch operations (`RESET_SOURCE_TEST_RUN`). Unmocked steps run with
  ordinary semantics; declared secrets resolve under the actor's current `secret:use` and access lists
  (O1162). Features 026 (no approval notifications) and 020 (no export of `DRAFT_TEST` runs) apply their
  own rules to test trees.

#### Rate limits with buffering (US07)

- **FR-022-039 — Rate-limit declaration**: A definition MUST be able to declare top-level `rateLimit` (run
  starts) and root `activity`/`script` nodes node-level `rateLimit` (attempt dispatches), each with a
  positive `limit`, a required `per` (`second` | `minute` | `hour`), an optional `key` (run level over
  `input`, step level over run data, same bounds, keyed digest and label as FR-022-017), an optional
  `group` (same default and sharing rule as FR-022-017) and an optional `queueTimeoutMs`. Run-level and
  step-level limits never share a window, and a rate limit never shares a row with a concurrency
  semaphore, whatever the group names. Publication MUST refuse an invalid policy
  (`RATE_LIMIT_POLICY_INVALID`), `rateLimit` on a body node (`UNSUPPORTED_NODE`, dependency
  `SCOPED_RATE_LIMIT`) and `rateLimit` together with `continueAsNew` (`FLOW_CONTROL_COMBINATION`). The
  declaration is beside `concurrency`, not inside it; a definition may declare either or both.
- **FR-022-040 — The window**: For every key, the number of grants an acquirer may see in any interval of
  length `per`, measured on the database clock, MUST NOT exceed that acquirer's `limit` (O1163). A
  run-level grant counts from the instant of its grant. A step-level grant counts from the instant its
  attempt is leased, and counts without end while it is granted and not yet leased, so at most `limit`
  attempts of a key are leased in any window. The window MUST be PostgreSQL rows changed only in the
  transactions of the plan; process memory, NATS, Redis and a replica's own clock MUST NOT decide or
  remember a grant. A slot MUST be returned only when a granted step ends without ever being leased.
- **FR-022-041 — Run admission and grant**: Admission MUST evaluate the key and, in its transaction, grant
  the slot (`rate_limit.granted`) when the window admits the run and no waiter of the key is ahead of it,
  or buffer the run (`rate_limit.queued`, `WAITING`/`RATE_LIMIT`, timer at the queue deadline, no task).
  Every run creator MUST use the one admission port of FR-022-018; a run with an 018 start delay is
  evaluated when the delay elapses. A key error MUST end the new run `FAILED`/`EXPRESSION_ERROR` in that
  transaction. A grant to a buffered run MUST be made by the scheduler replica that owns the waiter's
  shard, under its fence, in a transaction that journals the grant, removes the waiter, cancels its timer
  and creates the run's first tasks and outbox (ADR-0084). A **reset** run (018) MUST take a slot in its
  reset admission transaction and MUST NOT be buffered: without a free slot the reset is refused
  (`RATE_LIMIT_SLOT_UNAVAILABLE`, to be registered in 018's contract), and `rate_limit.*` events copied
  from the source prefix are history (FR-022-038).
- **FR-022-042 — What buffering guarantees**: Buffered work MUST survive a restart of every process and
  hold no lease, worker, attempt or concurrency permit. Within one key, grants MUST follow enqueue order
  (database clock of the enqueuing commit, ties by run id and node) among the waiters that are not paused
  and that their own limit and their other gate admit; there is no order between keys (O1164). The buffer
  MUST be bounded: the per-key waiter ceiling and the namespace `queued_runs` ceiling of FR-022-021 apply
  to rate waiters too; a full buffer MUST refuse a synchronous start (`QUOTA_EXCEEDED`,
  `RATE_LIMIT_QUEUE_FULL`) before a run exists and leave event and cron starts pending under their retry
  (O1165). A waiter past its queue deadline MUST end `RATE_LIMIT_WAIT_TIMEOUT` (a run `FAILED`; a step
  through its failure route); a grant and the timeout MUST be decided by first commit.
- **FR-022-043 — Step slots**: A step with node-level `rateLimit` MUST ask for a slot in the transaction
  that would make it ready, for its first attempt and for every retry after its backoff (O1166). With a
  free slot and no waiter ahead it is granted there and becomes `READY`; otherwise it waits (step
  `WAITING`, wait kind `RATE_LIMIT`, no attempt spent) and a scheduler pass grants it later and makes it
  `READY`. A claim MUST NOT read or write any rate-limit row: a step is claimable only after its grant,
  and the claim of a rate-limited step runs the same statements as the claim of any other step
  (NFR-022-007). The pass MUST record the lease instant of a granted attempt from the journal fact the
  claim already writes; until it has, the grant counts as unleased, so the pass may be late and never
  early.
- **FR-022-044 — Concurrency and rate together**: When a definition declares both at run level, a run MUST
  be given the permit and the slot in one transaction or neither: it MUST NOT hold a permit while it waits
  for a slot, nor consume a slot while it waits for a permit. It waits once, with one queue deadline (the
  earlier of the two), and a timeout names the gate that refused at the deadline. Every transaction that
  touches both MUST lock the semaphore (slot 7 of ADR-0050's order) before the rate-limit rows, which are
  rate rows of slot 8b and are locked in `(pool, scope, group, key)` order before the usage rows; no path
  reverses it. At step level the rate grant comes first (it makes the step claimable) and the step permit
  is taken at the claim (FR-022-019); a granted step waiting for a permit keeps its slot.
- **FR-022-045 — Replay**: `rate_limit.queued`, `rate_limit.granted`, `rate_limit.wait_timed_out` and
  `rate_limit.dequeued` MUST be journal events committed with the transition they cause, carrying the
  facts of the decision (`limit`, `period_ms`, `granted_at_us`, key label). Replay and `verify_run` MUST
  reach the persisted state from the journal alone and MUST NOT read the clock, the window or the queue.
- **FR-022-046 — Lifecycle**: Cancellation, the run deadline and any other terminal transition of a
  buffered run or step MUST remove it from the buffer in that transaction (`rate_limit.dequeued`) without
  consuming a slot. A paused waiter MUST be skipped by grants until resumed; pausing a granted run changes
  nothing. Lowering a limit MUST NOT revoke a grant. A run that ends before it was ever granted finishes
  without its workflow failure handler (as D1197). Reruns, 017 ad hoc starts, child runs, and 018 schedule
  firings, batch reruns and Signal/Update-with-Start are ordinary admissions. Draft test trees, when US06
  exists, use the draft-test pool and never share a window with ordinary runs.
- **FR-022-047 — Quotas, ceilings and fairness**: Buffered runs MUST count against `active_runs`,
  `tenant_active_runs` and `queued_runs`. `limit`, `per`, the queue timeout, waiters per key and live
  rate-limit keys per namespace MUST have the defaults and hard ceilings of the plan; the effective value
  is the minimum of declaration, namespace policy and deployment; zero refuses. A grant pass MUST bound
  the work it does for one key and for one tenant, so that a flood on one key cannot delay an eligible
  grant of another key or tenant by more than one further pass.
- **FR-022-048 — Visibility**: A run MUST show its rate-limit state (waiting or granted, since, queue
  deadline, group, key label, position in the key's queue, and an estimated grant time that is absent when
  it cannot be computed and is never a promise). A step waiting for a slot MUST be shown as such in the
  run's steps. A namespace view MUST list rate-limit keys with their grants in the current window,
  waiters, oldest wait and the instant the next slot frees, by key label only. Raw key values MUST NOT
  appear in any response, log, span, metric or audit (FR-022-022).
- **FR-022-049 — Observability and alerts**: The capability MUST emit the plan's spans and fixed-label
  metrics (decisions by outcome, wait time, waiters, grant lag behind a free slot) and raise per-namespace
  alerts when grants lag behind free slots (`rate_limit_grant_lagging`) and when a key's buffer nears its
  ceiling (`rate_limit_queue_saturated`), each with a runbook section. Labels follow FR-022-033.
- **FR-022-050 — Capability, SDKs and Studio**: `rateLimits` MUST be a capability enabled separately from
  `concurrencyKeys` under the rules of FR-022-031. The six SDKs (TypeScript, Go, Python, C#, Java, Rust;
  FR-022-035 predates the Rust SDK and reads "five") MUST author identical canonical IR for `concurrency`
  and `rateLimit` and pass the shared conformance scenarios `flow-concurrency` and `flow-rate-limit`. The
  Studio MUST author the same IR, show a run or step waiting for a rate slot with its position and
  estimate from the run resource, and list the keys. The worker protocol MUST NOT change.

#### Cross-cutting

- **FR-022-031 — Additive IR and capability gates**: New fields and kind MUST be additive in IR v1
  (ADR-0032). `stepCache`, `earlyReturn`, `failureHandler` and `concurrencyKeys` MUST be separately enabled;
  using a disabled one MUST fail before a version exists; disabling MUST stop new publications and starts
  while existing runs drain with compatible readers.
- **FR-022-032 — Authorization and tenant boundary**: Operations MUST follow the plan's matrix (new
  `cache:invalidate`, `failure-handler:manage`, `step:standalone`; test runs need `definition:write` and
  `run:start`). Tenant and namespace come from the authenticated context; foreign resources MUST be opaque
  `NOT_FOUND`; replays MUST reauthorize; authorization MUST fail closed.
- **FR-022-033 — Observability and audit**: Each capability MUST emit the plan's spans and fixed-label
  metrics, journal events in the run transaction, and audit in the decision transaction (invalidation,
  handler configuration and redrive, standalone and test admission, pin disclosure). Metric labels MUST
  NOT carry tenant, namespace, run, workflow, group or key; logs, spans and audit MUST NOT carry a payload,
  raw key or secret.
- **FR-022-034 — Finite ceilings and typed errors**: Every new resource MUST have a default and hard ceiling
  (plan); effective = min(declaration or request, namespace, deployment); zero refuses; admission fails
  before work. Errors MUST use the existing `Error.code` set with the reasons and codes of
  `contracts/errors.md`.
- **FR-022-035 — SDK and Studio parity**: The five SDKs MUST author identical canonical IR for the new
  fields and offer standalone, test-run, result-wait and invalidation calls, passing shared conformance.
  The Studio MUST author the same IR, show hits, mocks, early results, handler phases and waits from the
  journal, and show unknown capabilities as unsupported, never with sample values (FR-007-010).
- **FR-022-036 — Encrypted payloads**: With feature 031, this feature's positions and write paths are
  entries of 031's encoded-field registry (`specs/031-client-side-encryption/contracts/workflow-encoding.md`,
  shared check §5): publication MUST refuse a cache key over an encoded location with
  `ENCODING_READ_BY_CACHE_KEY` and a concurrency key with `ENCODING_READ_BY_CONCURRENCY_KEY` (registered by
  031); `earlyReturn.result` is a carriage position checked against `outputSchema` like `end.outputExpr`
  (`ENCODING_CARRIAGE_MISMATCH`); mocks, `fromRun` pins, standalone inputs and cache copies are write paths
  validated by 031. Encoded values are cached, mocked or returned early only as opaque bytes; this feature
  never decodes them and invents no encrypted-field code of its own.
- **FR-022-037 — Erasure and payload references**: Cache entries (inline copies of a run's output) and
  cache hit provenance MUST be registered in 020's content-store registry
  (`crates/infra-postgres/run-content-references.json`, 020 data model "Reference discovery") with the
  erasure hook "invalidate by `source_run_id`"; store obligations and `test_run_pins.payload_digest` reference
  run payloads by foreign key so 020's catalog-driven discovery covers them; every column is also classified
  in the storage classification manifest. When
  020 erases a run, every cache entry whose `source_run_id` is that run and every pending store obligation
  of it MUST be deleted (no later hit may copy it), and the runs that already received a copy (hit
  provenance, `fromRun` pins) MUST be discoverable by source run for 020's erasure to act on.
- **FR-022-038 — Reset rules**: Every journal event type this feature adds MUST declare its reset rule in
  018's reset-rule registry (`COPY`, `REMATERIALIZE`, `EXCLUDE`, `REFUSE`), as listed in
  `contracts/journal-events.md`: permit events and `result.returned` are re-materialized by the reset
  transaction, cache hits re-create their provenance row, and every reset point inside a failure-handler
  phase (after `failure_handler.started`) or after a queue timeout or dequeue is refused; a reset run never
  inherits `handler_for`.

### Non-functional requirements

- **NFR-022-001 (durability)**: Every acknowledged hit, early result, handler phase, acquisition, grant,
  release, dispatch and pin survives a crash at any commit; no duplicate or restart creates a second
  handler run, permit for one holder or early result.
- **NFR-022-002 (security)**: Zero cross-tenant/namespace reads, hits, invalidations, grants or dispatches
  in the tenant battery; zero raw-key, payload or secret canaries in logs, spans, metrics, audit, outbound
  bodies or listings.
- **NFR-022-003 (compatibility)**: Definitions without the new fields publish and run unchanged; stored runs
  replay unchanged; older SDKs and Studio builds show new events and kinds as unknown, never as failures.
- **NFR-022-004 (performance, benchmark criteria)**: Measure on a stated environment lookup overhead, hit
  versus dispatch time, acquisition with 1/16/256 same-key contenders, grant delay and dispatch delay;
  report failures; no figure is a commitment.
- **NFR-022-005 (operability)**: Stuck queues, dead dispatches and a store backlog are found by the engine
  monitor per namespace and alerted with a runbook; every operator action has an audited API.
- **NFR-022-006 (privacy)**: Keys are stored and shown only as keyed digests or labels that cannot be
  confirmed offline without the deployment's digest key.
- **NFR-022-007 (hot path)**: A definition that declares neither `concurrency` nor `rateLimit` adds no
  statement, lock or row to a claim, a completion or an admission (the recorded budget of design 038,
  FR-038-001, is unchanged). A node-level `rateLimit` adds none to any claim. What a declaring definition
  adds to its own admission and completions is recorded as a budget line in the change that adds it.
- **NFR-022-008 (time)**: Every instant a rate decision uses is read from the database clock inside the
  deciding transaction. Replicas whose own clocks disagree grant the same waiters; a database clock that
  moves backwards delays grants and never adds one.

### Key Entities

- **Cache entry**: immutable cached output with scope identity, fingerprint, keyed key digest, carrier
  digest, provenance, expiry and charged size; its store obligation.
- **Early result**: the run result set by `earlyReturn` (`runs.result_digest`, `resultSource`).
- **Failure handler**: workflow declaration and run phase; namespace default configuration; one dispatch
  obligation per failed run and the handler run it admits.
- **Concurrency semaphore**: per tenant, namespace, pool, permit scope, group and key digest: holders,
  waiters, counters.
- **Rate-limit window**: per tenant, namespace, pool, scope, group and key digest: the grants that still
  count, the waiters in enqueue order, counters.
- **Standalone execution**: a private one-node definition and its ordinary run.
- **Draft test run**: a private definition of a draft revision, its pins, start node and test tree.

## Success Criteria *(mandatory)*

### Measurable Outcomes

All criteria are future evidence; none is satisfied by this document.

- **SC-022-001**: In the cache suite, every hit creates no lease and journals `step.cache_hit`; expiry,
  invalidation and revoked secret usability each turn a hit into a miss; replay reproduces the persisted
  state; listings never contain a value or raw key. (FR-022-001..006)
- **SC-022-002**: A full content quota, a blob output and a full backlog never change a step or run outcome;
  every skipped store is counted. (FR-022-005, FR-022-007)
- **SC-022-003**: In 20 rounds of grant-versus-timeout races and 20 rounds of crash-during-release, every
  waiter ends in one outcome and holders equal the journals' open acquisitions after each round.
  (FR-022-018, FR-022-020)
- **SC-022-004**: With `limit: 1` and five same-key runs, simultaneous holders never exceed one, grants
  follow admission order, an older `limit: 1` waiter does not delay a younger `limit: 5` waiter, a full
  `queued_runs` ceiling refuses new waiters, and each refused declaration of the fixture corpus fails with
  its code. (FR-022-017..021)
- **SC-022-005**: For each status in `on`, the handler starts after compensation ends, receives the closed
  context, and the final status equals the no-handler status; none starts for `CANCELLED` or for a waiter
  that timed out before acquiring its permit. (FR-022-012..014)
- **SC-022-006**: Killing the dispatcher at each commit point yields exactly one handler run per failure; an
  unadmittable handler becomes `DEAD`, alerts and can be redriven. (FR-022-015, FR-022-016)
- **SC-022-007**: In all five SDKs an early result is disclosed while the run runs and remains the result
  after a later failure or cancellation. (FR-022-008..011)
- **SC-022-008**: Standalone activity, script and database steps survive a lost response, duplicate request,
  worker crash and cancellation, and their results equal a one-node published workflow's. (FR-022-023..025)
- **SC-022-009**: A draft test from a chosen node creates zero leases for pinned nodes, refuses missing pins,
  `prod` namespaces and unpinned effect nodes (`TEST_RUN_UNMOCKED_EFFECT`), and produces no outbound
  delivery or handler run. (FR-022-026..030)
- **SC-022-010**: The tenant battery denies or hides every new operation across tenants and namespaces; no
  raw-key, payload or secret canary appears anywhere. (FR-022-022, FR-022-032, FR-022-033)
- **SC-022-011**: Every ceiling has a test at and over its bound; zero refuses. (FR-022-034)
- **SC-022-012**: The five SDKs emit byte-identical canonical IR for the shared fixtures and identical
  normalized observations for the six conformance scenarios. (FR-022-035)
- **SC-022-013**: Old fixture corpora publish and replay unchanged with each capability enabled and disabled;
  disabling refuses new publication while active runs finish. (FR-022-031, FR-022-036)
- **SC-022-014**: The benchmark report states environment, measured latencies and failures, with no number
  presented as a commitment. (NFR-022-004)
- **SC-022-015**: For every run creator listed in the plan, a test proves admission goes through the permit
  port (acquire or queue); a reset of a permit-holding source acquires a fresh permit or is refused and
  never queues; restored acquisitions are ignored by the consistency check; draft test trees only use the
  draft-test pool; with 018 present, the reset corpus covers every event type of the reset-rule table,
  refuses every point inside a handler phase, and never copies `handler_for`. (FR-022-018, FR-022-020,
  FR-022-030, FR-022-038)
- **SC-022-016**: After 020 erases a source run, no cache entry or pending store obligation from it
  remains, the next lookup misses, the manifest-driven reference check reports no unregistered
  referencing column, and every consumer copy is listed by source run. (FR-022-037)

- **SC-022-017**: With `limit: 10, per: second` and 1,000 same-key starts, across an engine restart, no
  one-second interval of the journals holds more than 10 grants of the key, grants follow admission order,
  each run is granted once, starts beyond the buffer ceiling are refused with their code, and a run of
  another key and of another tenant admitted during the flood is granted within two passes.
  (FR-022-040..042, FR-022-047)
- **SC-022-018**: In 20 rounds of grant-versus-timeout, 20 rounds of grant-versus-cancel, a crash at every
  commit point of admission, grant, timeout and lease recording, and a duplicate grant attempt across a
  shard takeover, every waiter ends in exactly one outcome, the window's grant counter equals the journals'
  grants, and every waiter row has an open `rate_limit.queued`. (FR-022-041, FR-022-042, FR-022-045)
- **SC-022-019**: With both `concurrency` and `rateLimit` at run level, no committed state ever shows a
  run holding a permit while waiting for a slot or the reverse, and the lock-order barrier test for each
  path ends without a deadlock while its reversed-order control waits. (FR-022-044)
- **SC-022-020**: The budget test of design 038 passes unchanged for a definition without the new fields,
  and the claim of a rate-limited step has the same statements, transactions and rows as the claim of a
  plain step. (NFR-022-007, FR-022-043)
- **SC-022-021**: With a step-level limit and no worker for three windows, at most `limit` steps of the
  key are granted and unleased, a returning worker leases at most `limit` in its first window, a retry
  waits for a new slot without spending an attempt, and a granted step cancelled before its lease returns
  its slot. (FR-022-040, FR-022-043)
- **SC-022-022**: Lowering the policy ceiling while runs wait slows later grants and revokes none;
  cancellation, run deadline and pause of a waiter behave as specified; a reset takes a slot or is
  refused and never waits; every run creator of the plan is buffered through the port. (FR-022-041,
  FR-022-046)
- **SC-022-023**: Two engine replicas whose process clocks differ by ten minutes produce grants that
  satisfy SC-022-017, and a database clock set back by one window produces no grant until it catches up.
  The six SDKs emit byte-identical canonical IR for the rate-limit fixture and identical normalized
  observations for `flow-rate-limit`; no raw-key canary appears in any surface. (NFR-022-008, FR-022-048,
  FR-022-050)

## Assumptions

- Cache, concurrency and early return target the root graph first; bodies follow with per-instance
  evidence (G1160). Only inline outputs (≤ 256 KiB) are cached, mocked or pinned (G1161).
- A failure handler never changes the outcome (O1161) and does not run for `CANCELLED`.
- Draft test runs never run in `prod` namespaces (O1160); unmocked steps are real executions with real
  effects in the chosen non-production namespace.
- The idempotency key identifies a standalone execution until 018 adds business-ID policies; grants are
  oldest first until 019 adds priority.
- A rate limit bounds when work becomes eligible (a run's start, a step's lease), not how long the work
  takes or how many calls an activity makes once it runs. Rate limits target the root graph first
  (G1175); a limit rate is a ceiling, never a promised throughput (G1176).

## Dependencies

- Existing: 001, 002, 003, 004, 005 (test-run precedent), 006, 007, 008, 009, 010, 011, 014 (additive IR,
  gates), 016 (carriers), 017 (ad hoc admission, managed database).
- Parity features: 018 (business-ID policies; owns production reset, whose admission acquires a permit
  through FR-022-018 and refuses test-tree sources with `RESET_SOURCE_TEST_RUN`), 019 (priority order; heartbeat
  details), 020 (search attributes for listing; erasure and the manifest-driven reference check, FR-022-037), 021 (typed managed operations as standalone operations),
  026 (run-and-wait and Streams consume the early result), 028 (test clock), 031 (encrypted fields).

## Out of Scope

- Reset or restart of a production run (018); time skipping (028).
- Recovering a failed run to success from a handler; handlers for `CANCELLED`; waits or approvals in
  handler regions.
- Caching blob outputs, database results, child results or compensations; deduplicating concurrent misses;
  cross-namespace caches.
- ~~Rate limits per time window (Windmill "N executions per window"); only concurrency is limited.~~
  Brought into scope by the owner on 2026-11-13 (OD-2026-11-13-11) as US07.
- For rate limits: a lossy mode that drops or refuses instead of buffering; a `burst` allowance or evenly
  spaced grants (O1163); limits on body nodes (G1175) or with `continueAsNew` (G1177); a limit shared
  across namespaces or tenants; priority between waiters (019); limiting the calls an activity makes
  inside one attempt.
- Completing a parent's `child` node on the child's early result.
- Test runs in `prod`; mocks in published versions; partial execution of published versions.

## Owner questions

See [research.md](research.md#owner-questions): O1160 (test runs in `prod`), O1161 (handler recovery),
O1162 (secrets in draft test runs), each with a recommended default. For the 0.3.0 slice: O1163 (what "N per
window" means), O1164 (what the buffer guarantees), O1165 (a full buffer), O1166 (whether a retry takes a
slot), O1167 (rate limits inside collections), O1168 (whether a reset may wait), each with options and a
recommendation.

## Release 0.3.0 slice

Decided by the owner on 2026-11-13 (OD-2026-11-13-11): throughput control for flows with execution limits,
as "Concorrência + taxa".

| In the slice | Requirements | Success criteria |
|---|---|---|
| US02 — concurrency limits by key with a durable queue, as specified | FR-022-017..022 | SC-022-003, SC-022-004, SC-022-015 (creators that exist) |
| US07 — rate limits per window with buffering | FR-022-039..050 | SC-022-017..023 |
| The cross-cutting rules both need | FR-022-031 (gates `concurrencyKeys`, `rateLimits`), FR-022-032..035, FR-022-038; NFR-022-001..008 | SC-022-010..014 for these two capabilities |

Not in the slice, and unchanged by it: US01 (cache, FR-022-001..007), US03 (failure handlers,
FR-022-012..016), US04 (early return, FR-022-008..011), US05 (standalone steps, FR-022-023..025), US06
(draft test mode, FR-022-026..030), and FR-022-036..037 (features 031 and 020). Consequences for the
slice: the run creators "022 standalone, draft-test and handler dispatch" of FR-022-018 do not exist yet
and join the port when their stories are built; the draft-test pool exists as a column value with no
writer; the rule "no workflow handler without a permit" (D1197) has nothing to act on until US03; the
permissions `cache:invalidate`, `failure-handler:manage` and `step:standalone` are not added. The tasks of
the slice are listed in [tasks.md](tasks.md#release-030-slice-us02-and-us07) and its plan in
[plan.md](plan.md#release-030-slice).
