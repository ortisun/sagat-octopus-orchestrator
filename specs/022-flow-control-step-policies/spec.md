# Feature Specification: Flow control and step policies

**Feature Branch**: `feature/022-flow-control-step-policies`

**Created**: 2026-10-26

**Status**: Specification, plan and tasks approved for implementation by the owner on 2026-11-11 (Owner decision 2026-11-11-1, "Aprovo os rascunhos 019–032 e 036, pode implementar"); not implemented

**Release 0.3.0 slice (2026-11-13)**: the owner put throughput control in release 0.3.0 (OD-2026-11-13-11,
quoted in [research.md](research.md#owner-decision-of-2026-11-13-throughput-control-in-release-030)): US02
(concurrency limits by key, FR-022-017..022) as specified, plus rate limits per time window with buffering,
added below as US07 (FR-022-039..050, NFR-022-007..008, SC-022-017..023). US01 and US03–US06 are not in the
slice. The slice, its tasks and what it leaves out are in [Release 0.3.0 slice](#release-030-slice). Nothing
of it is implemented.

**Revision after review (2026-11-13)**: three independent reviews of the slice at `1a21cbdc`
([reviews/2026-11-13/](reviews/2026-11-13/)) found four blockers and twenty-eight majors. This text answers
them: a gated step (node-level `concurrency` or `rateLimit`) never becomes `READY` before its grant, for
every readiness source (FR-022-019, FR-022-051); one grant routine and one timeout routine serve a waiter
whatever its gates (FR-022-052); both capabilities are activated through 018's instance-capability
registry (FR-022-053); and FR-022-054..060 close the security and counting findings. The change to ADR-0056
decisions 3 and 5 (accepted) is proposed as
[ADR-0089](../../docs/adr/ADR-0089-a-gated-step-waits-outside-ready-and-one-fenced-grant-routine-serves-every-waiter-by-key.md)
and needs the owner. ADR-0056 and ADR-0088 were accepted on 2026-11-13 (OD-2026-11-13-17).

**Release 0.3.0, the rest of 022 (2026-11-24)**: the owner put the whole feature in release 0.3.0 (OD-2026-11-24-1)
and delegated its open questions and ADR acceptances (OD-2026-11-24-4). O1160, O1161 and O1162 are answered as
recommended (decisions D022-200 to D022-202); the requirements below that the as-built slice or the new decisions change
carry dated amendment lines (decisions D022-203 to D022-221; proposed ADR-0135, ADR-0136 and ADR-0137). What the rest
takes and in which order is in [Release 0.3.0, the rest of 022](#release-030-the-rest-of-022). Nothing of it is
implemented.

**Revision after the reviews of 2026-11-25**: the three repository reviewers read the design of the rest
([reviews/2026-11-25/](reviews/2026-11-25/)); the coordinating session's rulings are decisions D022-222 to D022-240,
written below as amendment lines dated 2026-11-25, with a new proposed ADR-0146 (every end of a run goes through the
reducer). Where a line of 2026-11-25 and one of 2026-11-24 disagree, the later one wins.

**Revision after the re-reviews of 2026-11-25**: the reviewers re-read `6d02632e`; the second rulings are decisions
D022-242 to D022-259 ([reviews/2026-11-25/design-rereviews-and-rulings.md](reviews/2026-11-25/design-rereviews-and-rulings.md)),
written below as lines marked "after the re-reviews", which win over the earlier lines of the same day.

**Third reads and acceptance (2026-11-25)**: no Blocker in the third reads of `c910345b`; the third rulings are decisions
D022-260 to D022-270, written below as lines marked "after the third reads"; ADR-0135, ADR-0136, ADR-0137 and ADR-0146
and the amendments of ADR-0050, ADR-0056 and ADR-0094 are Accepted (D022-271, delegate OD-2026-11-24-4).

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

*Amended 2026-11-24 (decisions D022-211 and D022-217):* the one admission port is `Engine::start` (D022-049), not
017's `RunStore::create`: standalone steps, draft test runs, namespace handler dispatch and derived `onFailure~h` children
admit through it. Feature 020 is not on the tree: erasure acts through a 022-side port that 020 will call (gap G022-021).

Rate limits (US07, added on 2026-11-13) reuse the same pieces as concurrency keys: the key expression, the
keyed digest and its label, groups, the one admission port, the queue ceilings and the shard-fenced grant
(ADR-0056). What differs is that a rate slot is freed by time, not by a holder finishing, so the count is a
durable window of grants on the database clock and every grant to a buffered waiter is made by a scheduler
pass (accepted [ADR-0088](../../docs/adr/ADR-0088-rate-limits-are-a-durable-window-of-grants-on-the-database-clock-and-the-scheduler-grants-never-the-claim.md)).
They are distinct from the tenant rate quotas of feature 009 (`api_requests_per_second`,
`events_per_second`), which refuse a request and never buffer, and from 018's schedule overlap policies
(`BUFFER_ONE`, `BUFFER_ALL`), which buffer a schedule's actions before a run exists. Stream operators
(design 042) decide which events ask for a run; this feature decides when a requested run may start, and
no operator paces a start (D022-018).

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
   *Amended 2026-11-25 (decision D022-240):* an entry charges no content (it references the producer's payload, D022-205):
   read "a full cache-entries ceiling, or a full pending-obligations ceiling" for "a full content quota".

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
4. **Given** a step-level limit, **when** more attempts would become ready than permits, **then** the extra
   steps wait (`WAITING`, wait kind `CONCURRENCY`), are not claimable, spend no attempt and do not occupy a
   worker's poll window; the grant routine makes them `READY` oldest first, and a step waiting past its
   queue timeout fails `CONCURRENCY_WAIT_TIMEOUT` through its failure route (revised 2026-11-13, D022-019,
   proposed ADR-0089).
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

Added after the review of 2026-11-13:

- Sixty-four or more steps of one activity wait for key A; a step of key B of the same activity is
  leased by the next poll, because waiting steps are not `READY` and never enter the claim's window
  (FR-022-019, SC-022-024).
- An operator retries a gated step, a signal, approval, update, timer or finished child releases one, the
  retry pass finds its backoff over, or its run is resumed: each makes the step ask its gates again; none
  makes it `READY` directly (FR-022-051).
- A run is paused while one of its steps is granted and not leased: the step returns its slot and permit
  and waits; on resume it asks again. A step that a review marks, or that is cancelled, before its lease
  returns them too (FR-022-046).
- The head waiter of a key is refused only by its other gate (a permit): the younger waiters behind it
  are granted only while there is room for the head too, so the head is not starved (FR-022-052).
- A step's key fails to evaluate or is longer than 256 bytes: the step fails `FLOW_CONTROL_KEY_INVALID`
  through its failure route without spending an attempt (FR-022-054).
- A child node's start finds the child workflow's buffer full: the child node fails with
  `RATE_LIMIT_QUEUE_FULL` or `CONCURRENCY_QUEUE_FULL`, a retryable failure handled by its retry policy and
  failure route; no run is created (FR-022-055).
- A key reads only the run's identity (its id, business id or idempotency key): every run would be its own
  key, so publication refuses it (`FLOW_CONTROL_KEY_UNBOUNDED`, FR-022-056).
- A key reads a field classified `secret`: refused; `pii`: a warning, and its label is shown only to
  callers who may disclose that field (FR-022-057).
- An engine of an older binary is still live: the capabilities cannot be activated, and until they are,
  definitions using them are refused (`FLOW_CONTROL_ACTIVATION_PENDING`, FR-022-053).

## Requirements *(mandatory)*

### Functional Requirements

#### Step result cache (US01)

- **FR-022-001 — Cache declaration**: A root `activity` or `script` node MUST be able to declare
  `cache: {key, ttlSeconds, scope?, name?, version?}`: `key` a required bounded expression, `ttlSeconds`
  required, `scope` `version` (default) | `workflow` | `namespace`, `name` required for `namespace`,
  `version` an author separator. Publication MUST refuse `cache` with `compensateWith`, `reconcileWith`,
  `irreversible` or `highRisk`, on database or other node kinds (`CACHE_WITH_EFFECT`,
  `CACHE_POLICY_INVALID`), and in bodies (`UNSUPPORTED_NODE`, dependency `SCOPED_CACHE`).
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-207):* until the collection- and loop-finish paths are routed through
  the reducer (task T022-207), publication also refuses `cache`, `earlyReturn` and `onFailure` in a definition with a
  `forEach`, `while` or `repeatUntil` node (`UNSUPPORTED_NODE`, dependency `COLLECTION_FINISH_ROUTING`).
- **FR-022-002 — Cache identity**: An entry MUST be identified by tenant, namespace, scope identity (pinned
  version, workflow or cache name), node fingerprint and key digest. The fingerprint MUST cover the
  executable identity (activity name or artifact digest), the canonical key expression, the author
  `version`, the `outputSchema` carrier digest and the classification rules version. The key digest MUST
  be a keyed digest of the canonical key value; the raw key MUST NOT be persisted outside the run's data.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-225, D022-226, D022-229; security Blocker 1 and Major 1):* the
  fingerprint covers the node's whole canonical IR except a closed list of fields that cannot change what runs (`next`,
  `onError`, `cache.ttlSeconds`, `cache.name`, `concurrency`, `rateLimit`, `retry`, `timeoutMs`, `compensationRetry`,
  `compensationTimeoutMs`), so the producing node's sorted logical secret references, an input mapping or a binding a
  later feature adds enter it by construction; plus the classification rules version. The key digest is keyed under its
  own purpose (`KeyPurpose::StepCache`, 1 KiB by default, 4 KiB at most).
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-254):* the fingerprint and the entry carry secret ids, not names; the
  entry records the producing worker's subject; in the `namespace` scope the key's classification is the producer's.
- **FR-022-003 — Hit semantics**: When a cacheable step becomes ready, the engine MUST evaluate the key over
  committed run data and, on a hit, in the same transaction copy the entry's output into the run's
  retained payloads (charged like any output), journal `step.cache_hit` (entry, source run/node/attempt,
  payload digest, expiry) and the ordinary completion, and continue scheduling, with no lease or attempt.
  Replay MUST reach the same state from the journal and the run's payloads without reading the cache.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-203, D022-204, D022-205; ADR-0135 proposed):* a cacheable step does
  not become `READY` before its lookup: its first `step.released` leaves it `WAITING` (`CACHE_LOOKUP`) and the lookup
  is an evaluation round asked before the gates round, in the commit that released it; the outcome is journaled as
  `step.cache_hit` (with `step.completed`) or `step.cache_missed {outcome}` (`miss`, `expired`, `invalidated`,
  `unauthorized`, `bypassed`). A hit takes no permit and no slot. "Copy the entry's output" reads "reference the
  entry's payload": no payload row is written, and the consumer run is charged the output's logical bytes like any
  completion. No task-ready hint is written for a step a hit completes. While its run is paused a cacheable step waits
  for its lookup undecided and is looked up on resume. *(Revised 2026-11-25, decision D022-238: no root step is released
  while its run is paused, so that case does not arise and the sentence is withdrawn.)*
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-231):* a hit whose payload would exceed the run's retained bytes fails the
  step `RUN_DATA_LIMIT` through its route, decided in the lookup round; the hit's bytes enter through the outputs path
  of `persist` (one charge, digest and projection).
- **FR-022-004 — Liveness and authorization of a hit**: A hit MUST require a matching entry whose expiry is
  later than the database clock in that transaction and whose scope was not invalidated after its
  creation. For a step declaring secrets, the publisher MUST currently be allowed to use them (metadata
  check, no value resolved); otherwise it is a miss. A key error or oversized key MUST bypass the cache.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-203):* a step is looked up once per run; a retry is an attempt of the
  miss. An unanswered lookup fails closed (the transaction is refused and retried), never a miss.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-225, D022-226):* a hit also requires the consumer's publisher to be
  able to use now every secret reference the entry records (the producer's, equal to the node's by the fingerprint),
  checked under the shared secret-access and membership locks (`sagat_lock_secret_access`, `sagat_lock_memberships`),
  and the producing worker identity the entry records to hold now a binding that covers the activity; otherwise the
  outcome is `unauthorized`.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decisions D022-252, D022-254):* the hit also reads the key's tombstone; the binding
  check takes the binding advisory lock shared and checks the producing subject's membership (for a script, the
  producing runner's subject and its binding profile); hits staged in one round count cumulatively against the
  retained-output limit; to a run reader who may not see the producer an `unauthorized` outcome reads as `bypassed`.
  *Amended 2026-11-25 after the third reads, delegate (OD-2026-11-24-4; decision D022-265):* the producing subject is pre-read without a lock, its locks are
  taken, and it is checked again on the entry read; a change is `unauthorized`.
- **FR-022-005 — Storing an entry**: A successful completion of a miss with an inline output MUST write a
  store obligation in its completion transaction; a leased writer MUST insert the entry from the run's
  immutable retained output, insert-if-absent, with expiry from the database clock. Storing MUST NOT fail,
  delay or roll back the step or run: quota exhaustion, blob output, a full backlog and an existing entry
  are counted outcomes. Failures, cancelled attempts and outputs refused by secret scanning are never stored.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-205):* the obligation and the entry reference the completed attempt's
  payload row (`payloads (tenant_id, digest)`); the writer inserts no copy.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-230):* only the outcomes `miss`, `expired` and `invalidated` write a
  store obligation, which copies the key digest and key version the lookup kept on the step; a full pending-obligations
  ceiling skips it, counted. The entry's `created_at` is the producing completion's instant and its expiry runs from it;
  the writer skips an obligation created at or before its scope's invalidation, replaces a dead entry under its lock,
  and takes no lease or fence.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-252; supersedes the stamp above):* the entry is stamped with the
  database instant of the producing attempt's lease (`step.leased`) and expires from it; the writer skips an obligation
  whose attempt was leased at or before the scope's invalidation or the key's tombstone; two writers replacing one dead
  entry use `ON CONFLICT DO NOTHING`; one usage row holds the counts of entries and store obligations.
- **FR-022-006 — Invalidation and retention**: An authorized principal MUST be able to invalidate by
  workflow and node (optionally one version), by key value (sent in the body, digested server-side), or by
  shared cache name. Invalidation and expiry purge MUST release bytes in the deleting transaction and MUST
  NOT change any run that already hit. Listings MUST NOT return values or raw keys.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-230):* an invalidation by scope inserts the scope row when absent
  and stamps `invalidated_at` with the database clock read after its lock; an invalidation by key also deletes the
  pending store obligations of that scope and key; entries release no bytes (they hold none), only their count.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-252):* an invalidation by key writes a per-key tombstone; a scope row
  is inserted only for a scope a published version can derive, and any other target is `404`.
- **FR-022-007 — Classification and retained content**: Entries MUST be retained content charged to
  `tenant_content_bytes` (ADR-0027) and in the storage classification manifest. A consuming run discloses
  a copied output only through its own pinned carrier, identical to the producer's by FR-022-002. Cache APIs
  MUST NOT disclose a value.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-205):* an entry's value is the producer's payload row, already charged
  to `tenant_content_bytes`; an entry adds a fixed-size row, counted against the cache-entries ceiling, and is
  classified in the storage classification manifest. The capability is activated as `step-cache-v1` (FR-022-031).
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-226, D022-229):* a run of an internal definition (a standalone step,
  a test tree) uses only the `version` scope of its own private version; a cache key follows FR-022-057
  (`CACHE_KEY_SECRET` refused, `CACHE_KEY_PII` warned), and a key's label, the entry counts and a hit's source run are
  shown only to a caller who may disclose every location the key reads.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-254):* for any other caller `entry_id`, the expiry and
  `entriesDeleted` are null and listing instants are cut to the minute; spans and metrics carry no key, label or source
  run. A `namespace`-scope cache name is a declared resource of the namespace (an administrator's setting lists the
  workflows that may write it; any workflow of the namespace may read it).

#### Early return (US04)

- **FR-022-008 — Early-return node**: The IR MUST gain an additive `earlyReturn` kind with required
  `result` and `next`. Publication MUST refuse it outside the root graph, in a saga body, parallel branch,
  body or handler region, reachable from another `earlyReturn`, with `continueAsNew`, or when an `end` with
  `outputExpr` is reachable from it (`EARLY_RETURN_PLACEMENT`).
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-207):* and, until task T022-207, in a definition with a `forEach`,
  `while` or `repeatUntil` node (`UNSUPPORTED_NODE`, dependency `COLLECTION_FINISH_ROUTING`).
- **FR-022-009 — Early-return transition**: Reaching it MUST, in one transaction, evaluate `result`, store it
  under the run-result bounds (256 KiB, content quota), set `runs.result_digest`, journal `result.returned`,
  write the `run.result_returned` outbox and delivery records, and schedule `next`. Evaluation failure MUST
  fail the run with `EXPRESSION_ERROR`, an oversized value with `RUN_DATA_LIMIT`. The status MUST NOT change.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-215; ADR-0136 proposed):* `result` is evaluated by the round
  `Evaluation::EarlyResult`, and `persist` writes the rest in that commit; a test tree writes no `run.result_returned`
  delivery (FR-022-030). The result is set once, before the run is terminal, and nothing after it changes it.
  Activated as `early-return-v1`.
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
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-210):* the region is a region of the root graph, not a named body;
  its nodes are root nodes for every root-only rule (they may declare `cache`, `concurrency` and `rateLimit`).
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-224):* a region refuses `compensateWith` (`FAILURE_HANDLER_REGION`);
  a child form whose workflow's start permit shares the parent's group and a key that can coincide is refused
  (`FAILURE_HANDLER_SELF_GATE`).
- **FR-022-013 — Handler ordering and context**: When a run would finish with a status in `on`, the engine
  MUST first complete the existing stop (cancel unstarted work, settle in-flight attempts, finish
  compensation and reconciliation), then journal `failure_handler.started` with the pending status and
  enter the handler with the closed context `failure: {status, code, node, attempt, kind, occurredAt}`
  (no payload, message text or secret). A run in `MANUAL_REVIEW` is not terminal and MUST wait. A run that
  ends before it ever acquired its run permit (queue timeout, key error at admission) MUST finish directly,
  without its workflow handler, because the handler would run outside the key limit; the namespace handler
  still applies. A run holding a permit keeps it through its handler phase.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-216; ADR-0136 proposed):* `failure_handler.started` replaces
  `run.finished`; the child form is created through `Evaluation::Child` and admitted through `Engine::start`; the
  start's `concurrency.released` and `run.finished` are written in the commit of `failure_handler.finished`. "Acquired
  its run permit" reads "was granted every gate of its start".
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-224; ADR-0136):* `failure_handler.started` moves the stop into
  `pending_terminal_status` and clears it, so the run reads `RUNNING` or `WAITING` and plans only the handler's work; a
  region step failing without `onError` ends the phase `FAILED`; there is never a second phase.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decisions D022-246 to D022-248):* every stop source during the phase maps to one
  journaled `failure_handler.draining {cause}` (`CANCELLED`, `TIMEOUT`, `REGION_FAILED`, `STORAGE_QUOTA`,
  `RUN_DATA_LIMIT`, `EXPRESSION_ERROR`; the first wins), never to the run's stop; draining overrides a pause. The run
  deadline does not apply during the phase: only the handler's own timer bounds it, and the overdue pass skips a run in
  its phase. The runs of a region's `child` nodes are handler-tree runs; the `onFailure~h` node cancels and awaits its
  child.
  *Amended 2026-11-25 after the third reads, delegate (OD-2026-11-24-4; decisions D022-261, D022-262):* there is one cause: while the run drains a further
  stop request is refused `409`, and region step failures settle into the drain; an overdue pass that picked a run
  which then opened its phase is a typed no-op.
- **FR-022-014 — Handler outcome**: A handler MUST NOT change the outcome: when it ends (succeeded, failed,
  timed out, cancelled) the engine MUST journal `failure_handler.finished` and finish the run with the
  pending status in the same transaction. A handler's failure MUST NOT re-invoke it. Cancellation during the
  handler MUST stop its work and finish with the pending status. `CANCELLED` runs MUST NOT start a handler.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-201, O1161 answered):* recovery is a handler that starts a reset or a
  rerun as new under its own authority: an audited new run, never a change of the failed run's outcome.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-224, D022-228; supersedes "under its own authority" above):* a
  handler holds no authority of its own: a handler that recovers calls the public API with a machine identity it is
  given as a secret, audited as that identity. A cancellation or the run deadline during the phase is a handler
  cancellation and the handler's timer a timeout: a draining sub-state (`CANCELLING`, `TIMING_OUT`) stops the region's
  attempts and the child, then commits `failure_handler.finished`, the permit's release and `run.finished` with the
  pending status. The timeout is `FAILURE_HANDLER_TIMEOUT` in the journal and `TIMED_OUT` in the column.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-244; supersedes "a machine identity"):* the credential is of the
  built-in role `RunRecovery` (`run:read`, `run:start`, `run:operate`; no publication, approval, version or secret
  permission); a subject whose only role is `RunRecovery` may not approve a publication or decide an approval. "A
  cancellation or the run deadline during the phase is a handler cancellation" reads "a cancellation during the phase"
  (D022-247).
  *Amended 2026-11-25 after the third reads, delegate (OD-2026-11-24-4; decision D022-266):* a `RunRecovery` subject is bound to named namespaces (an
  audited administrator's setting) and answers `404` outside them, holds `definition:read`, may not redrive a dispatch
  (`RECOVERY_ROLE_MAY_NOT_REDRIVE`), and cannot be assigned until `failure-handler-v1` is activated.
- **FR-022-015 — Namespace default handler**: An administrator MUST be able to set, read and clear one
  default handler per namespace (workflow, optional pinned version, statuses, enabled flag; optimistic
  revision, justification, audit). It applies to runs whose version declares no `onFailure` or whose
  handler did not succeed.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-223):* the pending-dispatch ceiling refuses a new or re-enabled
  configuration and raises an alert; it never refuses a terminal commit.
- **FR-022-016 — Dispatch through the outbox**: A qualifying terminal transaction MUST insert one dispatch
  obligation keyed by the failed run, using the configuration read under its policy lock there. A
  shard-fenced dispatcher MUST admit the handler run (closed failure context and source identity as input;
  provenance `handlerFor`; initiator `system:engine`; authority of the handler version's publisher) and mark
  the obligation in the same transaction, so exactly one handler run exists per failure. Refused admissions
  MUST retry with bounded backoff, then become `DEAD`, alerted and redrivable. Handler runs (dispatched or
  `onFailure` children), test-tree runs and cancelled runs MUST NOT be dispatched; a failed `onFailure`
  child is covered by its parent's dispatch.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-208, D022-211, D022-214):* the configuration is read without a lock
  in the terminal commit and its `config_revision` is recorded on the dispatch row; there is no policy lock.
  *(Revised 2026-11-25, decisions D022-222, D022-223, D022-228, D022-237: the revision is read with the run's
  definition; the handler workflow's head is read without a lock; see the line below.)* The
  dispatcher admits through `Engine::start`, in ADR-0050's slots (dispatch row slot 3, the source run's identity guard
  3R, the handler workflow's head 4b, the run 6, its gates 7). Activated as `failure-handler-v1`.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-223, D022-228):* the terminal commit always inserts its dispatch
  row (a recorded deviation of ADR-0050 decision 7); a run qualifies when no workflow handler phase of it ended
  `SUCCEEDED`, it is not a handler-tree run, not a test tree and not cancelled; a namespace handler is dispatched at
  most once per lineage (the source chain of resets and reruns). Handler runs form a tree (`handler_tree`, inherited by
  children and by resets and reruns of them) and carry `handler_kind` (`DISPATCHED`, `REGION_CHILD`); `handlerFor` is a
  dispatched run's only, and a rerun or reset of a handler run inherits it not. Each dispatch attempt is counted in its
  own commit before the admission, so a crash in the admission still reaches `DEAD`.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-241, O1176 answered (b); reversed by the next line):* a reset or rerun extends its source's lineage only when its initiator is a machine identity or a handler-tree run; a reset or rerun a person (a member) starts begins a lineage of its own, so a
  person's rerun of a failed run is dispatched again when it fails, and a handler's rerun through a machine identity is
  not.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decisions D022-242, D022-243, D022-248, D022-250, D022-255; reverses D022-241,
  O1176 answered (a)):* every reset and rerun extends its source's lineage, whoever starts it; a dispatch the lineage
  suppresses is written as a `SUPPRESSED_LINEAGE` row, counted, listed and redrivable by an operator, and the terminal
  commit never fails on it. A namespace dispatch budget (per window, with a per-workflow share) defers dispatches beyond
  it and alerts. A run qualifies when no workflow handler phase ended `SUCCEEDED` and no operator asked to cancel it.
  The configuration is read by a left join in the statement that locks the run. A chain's finalization reaches the
  dispatch through `persist`'s terminal path.
  *Amended 2026-11-25 after the third reads, delegate (OD-2026-11-24-4; decisions D022-260, D022-263, D022-264):* `run.finished` is journaled after the
  dispatch statements with its `namespace_dispatch` outcome; a deferral by the budget is not an attempt; a dispatch or
  a child admission under a handler or test tree whose definition continues as new fails `CONTINUE_AS_NEW_REFUSED`.

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
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-211):* the one admission port is `Engine::start` (D022-049); the
  creators "022 standalone, draft-test and handler dispatch" and the derived `onFailure~h` child call it, each with a
  run-creator conformance case (task T022-221).
- **FR-022-019 — Step permits** *(revised 2026-11-13 after review, D022-019; changes ADR-0056 decision 3
  through the proposed ADR-0089)*: A step whose node declares `concurrency` MUST NOT become `READY` before
  it holds a permit. The transaction that would make it ready (FR-022-051) MUST acquire the permit when its
  key has room for it under FR-022-052, else make the step `WAITING` (wait kind `CONCURRENCY`, no attempt
  spent, a timer at its queue deadline), to be granted later by the grant routine (FR-022-052). The permit
  is held from its grant until the attempt ends (completion, failure, cancellation, lease expiry) or until
  the step leaves `READY` other than by a lease (FR-022-046); a granted, unleased step therefore counts as a
  holder. The claim MUST NOT read or write a semaphore. A step waiting past its queue timeout MUST fail
  `CONCURRENCY_WAIT_TIMEOUT` through its failure route. (Before the revision a permit-less step stayed
  `READY` and was refused at claim; that filled the claim's window, `crates/infra-postgres/src/workers.rs`,
  with steps nobody could lease.)
- **FR-022-020 — Durable semaphores**: Holders, waiters and counters MUST be PostgreSQL rows keyed by
  tenant, namespace, pool (ordinary or draft-test), permit scope, group and keyed digest, changed only in
  the transactions above, after each path's own locks and its `runs` write (per-path orders in ADR-0056
  decision 2). Holders of a key MUST equal the open, non-`restored` acquisitions in its runs' journals,
  verified after fault tests. Empty rows MUST be removed. A new digest key version MUST NOT be activated
  while any holder or waiter exists under the previous one.
  *Amended 2026-11-24 (decision D022-077 item 5):* the last sentence is superseded by ADR-0093, which the owner
  accepted (OD-2026-11-19-2): a new digest key version is activated while rows exist under the previous one, and each
  key moves to it when it is next decided, per key, with live rows.
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
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-209, D022-211; ADR-0137 proposed):* admission goes through
  `Engine::start`, not "the existing admission transaction". The start MAY carry 018's `businessId` with
  `businessIdConflictPolicy` and `businessIdReusePolicy` (FR-018-018..020; gap G1166 closed). The default group of the
  node's `concurrency` or `rateLimit` is derived from the activity name or the script artifact's digest, never from the
  private definition. Activated as `step-executions-v1`.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-227, D022-233):* in a namespace whose effective
  `required_approvals` is above zero, a standalone activity or script step is refused (`FORBIDDEN`,
  `STANDALONE_STEP_REVIEW_REQUIRED`) unless its publication policy enables draft executions; a script node accepts only
  the artifact of a published script version; the derived default group is one key space for every actor.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-257):* in a `prod` namespace a standalone activity or script step is
  refused (`STANDALONE_STEP_PRODUCTION_NAMESPACE`) unless the setting is enabled; the setting is read inside the
  admission transaction under the publication policy's lock; a rerun of a standalone run is checked again.
  *Amended 2026-11-25 after the third reads, delegate (OD-2026-11-24-4; decision D022-269):* the setting's lock is an advisory key per namespace, shared by
  admissions at slot 4a and exclusive in the policy writer.
- **FR-022-024 — Standalone semantics**: The node's retry, timeout, secrets, idempotency template,
  `reconcileWith`, `cache` and node `concurrency` MUST behave as in a workflow; `compensateWith` MUST be
  refused. A database node MUST keep every 017 rule; when feature 021 exists, its typed managed operation
  nodes MUST be accepted under their own rules. `database-executions` MUST remain and share the path.
- **FR-022-025 — Standalone operations**: Standalone runs MUST be listed (`GET /runs?origin=STANDALONE_STEP`),
  inspected, cancelled, retried and disclosed through existing routes; the five SDKs MUST offer start and
  execute-and-wait; the Studio MUST offer a start form and a list filter.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-220):* "the five SDKs" reads "the six SDKs" (TypeScript, Go, Python,
  C#, Java, Rust).

#### Test mode for drafts (US06)

- **FR-022-026 — Draft test run**: `POST /workflows/{workflowId}/test-runs` (idempotency key required) MUST
  take `draftRevision`, `input`, optional `startAt`, `pins` and deadline; validate the draft at that
  revision with publication rules and enabled capabilities; retain a private definition (origin
  `DRAFT_TEST`, publisher the actor); and admit a run. A moved draft MUST be `CONFLICT`; nothing is
  published. A `prod` namespace MUST refuse (`FORBIDDEN`, `TEST_RUN_PRODUCTION_NAMESPACE`).
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-200, D022-211, D022-214; O1160 answered):* `dev` and `staging`
  namespaces allow test runs; admission goes through `Engine::start`; the draft's workflow row is read `FOR SHARE` at
  slot 4b of ADR-0050's order. Activated as `draft-test-runs-v1`.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-222, D022-227, D022-233; supersedes "`FOR SHARE` at slot 4b"):* the
  draft row is read without a lock and its `draft_revision` compared (a moved draft is `CONFLICT`); what was read is
  copied into the immutable private definition. A namespace whose effective `required_approvals` is above zero refuses
  test runs (`FORBIDDEN`, `TEST_RUN_REVIEW_REQUIRED`) unless its publication policy enables draft executions. The
  checks run in this order: the namespace from the authenticated context, `404` for the workflow, the `prod` or review
  `403`, then a `fromRun` `404` before `PIN_NOT_DISCLOSABLE`.
- **FR-022-027 — Pins and mocks**: A pin MUST be `{mock}` or `{fromRun, fromNode?}` on an activity, script,
  child or waitEvent node, or, when those features exist, on a 021 `resourceOperation`, 024 data, 025 agent
  or 030 `serviceCall` node; it is held only in the request and the run's data, never in a definition. A
  draft whose graph reachable from `startAt` contains an unpinned node of one of those four kinds MUST be
  refused before admission (`VALIDATION_FAILED`, `TEST_RUN_UNMOCKED_EFFECT`, naming the node). A pinned node
  MUST complete (at admission for upstream pins, else when reached) with `step.mocked` (source, payload
  digest), without lease, attempt, cache lookup, child run or wait, and MUST be excluded from compensation
  and reconciliation planning.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-204, D022-214, D022-218):* a downstream pin completes in the commit
  that releases its node, before any lookup or gate is asked, and invariant 11 admits `step.mocked` in place of grants.
  Upstream pins (payload rows, `test_run_pins` rows, `step.mocked`) are written at slot 6 with the run. The four effect
  kinds are not on the tree: the refusal is built as an exhaustive classification of node kinds with no effect member
  yet (gap G022-023).
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-233):* an upstream pin commits `step.mocked` and a `step.routed` with no
  activation, so it releases no successor; `startAt` is recorded in `run.started` so replay reaches the same entry.
- **FR-022-028 — Start from a chosen node**: `startAt` MUST be a root node outside saga bodies, parallel
  branches, bodies and handler regions. Each node reachable from it that references an output of a node not
  reachable from it MUST have that node pinned (`MISSING_PIN`); upstream pins MUST commit in the admission
  transaction; unreachable unpinned nodes MUST NOT run.
- **FR-022-029 — Pin from a previous run**: `fromRun` MUST be a run of the same tenant, namespace and
  workflow whose node succeeded with an inline output. The caller MUST be able to disclose the whole value
  under the source carrier at pin time (`secret` refuses; `pii` needs the reveal grant and is audited). The
  pin MUST record the strictest level found as a floor; the test run MUST disclose that output at the
  stricter of floor and draft carrier.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-233):* the highest floor among a test run's pins applies to every
  disclosure of that test run.
- **FR-022-030 — Isolation of test runs**: A test run and its child runs (test tree) MUST NOT create
  outbound deliveries, dispatch namespace handlers, fire triggers, or read/write `workflow`/`namespace`
  cache scopes; they MUST acquire permits only in the draft-test semaphore pool, never blocking or being
  blocked by ordinary runs; they MUST count against test-run quotas and ordinary run quotas; and they
  MUST NOT be accepted as sources of 018 reset, rerun or batch operations (`RESET_SOURCE_TEST_RUN`). Unmocked steps run with
  ordinary semantics; declared secrets resolve under the actor's current `secret:use` and access lists
  (O1162). Features 026 (no approval notifications) and 020 (no export of `DRAFT_TEST` runs) apply their
  own rules to test trees.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-202, D022-212, D022-219; O1162 answered):* `runs.test_tree` is set at
  admission and copied by every child admission; every gate decision takes its pool from that flag through one function
  (`GatePool::of_run`), never a literal; each secret resolution of an unmocked step is audited
  (`test_run.secret_resolved`); `RESET_SOURCE_TEST_RUN` is raised by 018's reset, rerun and batch admissions in the
  slice that adds the flag.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-226, D022-237):* a test-tree run whose version is not its own private
  one bypasses the cache; `test_tree` is immutable (`BEFORE UPDATE OF test_tree`, `ENABLE ALWAYS`) and the database
  checks at insert that a child's flag equals its parent's.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decisions D022-245, D022-249):* a continue-as-new successor of a test-tree or
  handler-tree run is refused; `handler_tree`, `handler_kind` and `lineage_run_id` have the guards `test_tree` has.

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
  length `per`, measured on the database clock, MUST NOT exceed that acquirer's `limit` (O1163, decided). A
  run-level grant counts from the instant of its grant. A step-level grant counts from the instant its
  attempt is leased, and counts without end while it is granted and not yet leased, so at most `limit`
  attempts of a key are leased in any window, and at no instant are more than `limit` steps of a key
  granted and unleased. The window MUST be PostgreSQL rows changed only in the transactions of the plan;
  process memory, NATS, Redis and a replica's own clock MUST NOT decide or remember a grant. A slot MUST be
  returned only when a granted step leaves `READY` other than by its lease (pause, review, cancellation,
  run end; FR-022-046); a leased attempt never returns its slot.
- **FR-022-041 — Run admission and grant**: Admission MUST evaluate the key and, in its transaction, grant
  the slot (`rate_limit.granted`) when the window admits the run under FR-022-052, or buffer the run
  (`rate_limit.queued`, `WAITING`/`RATE_LIMIT`, timer at the queue deadline, no task). Every run creator MUST
  use the one admission port of FR-022-018, enforced by FR-022-059; a run with an 018 start delay is
  evaluated when the delay elapses. A key error MUST end the new run `FAILED`/`EXPRESSION_ERROR` in that
  transaction. A grant to a buffered run MUST be made by the grant routine of FR-022-052 (ADR-0088,
  refined by ADR-0089). A **reset** run (018) MUST take a slot in its reset admission transaction and MUST
  NOT be buffered: without a free slot the reset is refused (`RATE_LIMIT_SLOT_UNAVAILABLE`, registered in
  018's contract), and `rate_limit.*` events copied from the source prefix are history
  (FR-022-038).
- **FR-022-042 — What buffering guarantees**: Buffered work MUST survive a restart of every process and
  hold no lease, worker, attempt or concurrency permit. Within one key, grants MUST follow enqueue order
  (database clock of the enqueuing commit, ties by run id and node; with 019, priority first) under the
  reservation rule of FR-022-052; there is no order between keys (O1164). Run waiters MUST be bounded per
  key (default 100, hard 10,000, and an effective value always below the namespace's effective
  `queued_runs`) and by the namespace `queued_runs` ceiling of FR-022-021; a full buffer MUST refuse a
  synchronous start (`QUOTA_EXCEEDED`, `RATE_LIMIT_QUEUE_FULL`) before a run exists, leave event and cron
  starts pending under their retry (O1165), and fail a child node as FR-022-055 states. Step waiters are not
  bounded per key: there is at most one per run and gated node, so they are bounded by `active_runs` times
  the definition's gated nodes, and a step never fails for a full buffer. A waiter past its queue deadline
  MUST end `RATE_LIMIT_WAIT_TIMEOUT` (a run `FAILED`; a step through its failure route); a grant and the
  timeout MUST be decided by first commit, and a grant MUST skip a waiter whose deadline has passed.
- **FR-022-043 — Step slots**: A step with node-level `rateLimit` MUST ask for a slot whenever it would
  become ready, through the readiness gate of FR-022-051: its first attempt, every retry after its backoff
  (O1166), an operator retry and a resume. With room under FR-022-052 it is granted there and becomes
  `READY`; otherwise it waits (step `WAITING`, wait kind `RATE_LIMIT`, no attempt spent) and the grant
  routine grants it later. A claim MUST NOT read or write any rate-limit or semaphore row, and the claim of
  a gated step runs the same statements as the claim of any other step (NFR-022-007). A step grant names
  the attempt it is for (the step's current attempt plus one). The pass MUST record the lease instant only
  from a `step.leased` of that attempt committed after the grant event; until it has, the grant counts as
  unleased, so the pass may be late and never early.
- **FR-022-044 — Concurrency and rate together**: When a run, or a step, declares both gates, it MUST be
  given the permit and the slot in one transaction or neither: it MUST NOT hold a permit while it waits
  for a slot, nor consume a slot while it waits for a permit. It waits once, with one queue deadline (the
  earlier of the two), and is served by the one grant routine and the one timeout routine of FR-022-052.
  Every transaction that touches both MUST lock the semaphore (slot 7 of ADR-0050's order) before the
  rate-limit rows, which are rate rows of slot 8b and are locked in `(pool, scope, group, key)` order
  before the usage rows; no path reverses it. A step grant also requires room on its concurrency key, where
  holders include granted, unleased steps (FR-022-019).
- **FR-022-045 — Replay**: `rate_limit.queued`, `rate_limit.granted`, `rate_limit.wait_timed_out` and
  `rate_limit.dequeued` MUST be journal events committed with the transition they cause, carrying the
  facts of the decision (`limit`, `period_ms`, `granted_at_us`, key label). Replay and `verify_run` MUST
  reach the persisted state from the journal alone and MUST NOT read the clock, the window or the queue.
- **FR-022-046 — Lifecycle**: Cancellation, the run deadline and any other terminal transition of a
  buffered run or step MUST remove it from the buffer in that transaction (`*.dequeued`) without consuming
  a slot or a permit. A paused waiter MUST be skipped by grants until resumed. Pausing a run whose
  run-level grant is made changes nothing for that grant; but pausing a run MUST return the slot
  (`rate_limit.returned`) and the permit (`concurrency.released`) of each of its gated steps that is granted
  and not leased, and make them wait again, and resuming it MUST make them ask again (FR-022-051). The same
  return happens when a granted step leaves `READY` other than by its lease (a review marks it, it is
  cancelled, its run ends). Lowering a limit MUST NOT revoke a grant. A run that ends before it was ever
  granted finishes without its workflow failure handler (as D1197). Reruns, 017 ad hoc starts, child runs,
  and 018 schedule firings, batch reruns and Signal/Update-with-Start are ordinary admissions. Draft test
  trees, when US06 exists, use the draft-test pool and never share a window with ordinary runs.
- **FR-022-047 — Quotas, ceilings and fairness**: Buffered runs MUST count against `active_runs`,
  `tenant_active_runs` and `queued_runs` (FR-022-058). `limit`, `per`, the queue timeout, waiters per key and
  live keys per namespace MUST have the defaults and hard ceilings of the plan; the effective value is the
  minimum of declaration, namespace policy and deployment; zero refuses. The hard per-second ceiling is
  published only once the benchmark of SC-022-029 has met its floor; until then the deployment ceiling is
  the lower default of the plan. A grant pass MUST select keys, not waiters, share them between tenants
  through the scheduler's fair selection (`crates/scheduler/src/fairness.rs`), and bound the keys it visits
  and the grants it makes per key, so that a flood on one key or tenant cannot delay an eligible grant of
  another key or tenant by more than one further pass.
- **FR-022-048 — Visibility**: A run MUST show its rate-limit state (waiting or granted, since, queue
  deadline, group, key label). A single-run read (`GET .../runs/{runId}`) MUST also show its position in the
  key's queue and an estimated grant time that is absent when it cannot be computed and is never a promise;
  run listings MUST NOT compute them. A step waiting for a slot or a permit MUST be shown as such in the run's
  steps. A namespace view MUST list rate-limit keys with their grants in the current window, waiters, oldest
  wait and the instant the next slot frees. Key labels follow FR-022-057; raw key values MUST NOT appear in
  any response, log, span, metric or audit (FR-022-022).
- **FR-022-049 — Observability and alerts**: The capability MUST emit the plan's spans and fixed-label
  metrics (decisions by outcome, wait time, waiters, grant lag behind a free slot) and raise per-namespace
  alerts when grants lag behind free slots (`rate_limit_grant_lagging`) and when a key's buffer nears its
  ceiling (`rate_limit_queue_saturated`), each with a runbook section. Labels follow FR-022-033.
- **FR-022-050 — Capability, SDKs and Studio**: `rateLimits` MUST be a capability enabled separately from
  `concurrencyKeys` under the rules of FR-022-031, and both MUST be activated through the instance-capability
  registry (FR-022-053). The six SDKs (TypeScript, Go, Python, C#, Java, Rust; FR-022-035 predates the Rust
  SDK and reads "five") MUST author identical canonical IR for `concurrency` and `rateLimit` and pass the
  shared conformance scenarios `flow-concurrency` and `flow-rate-limit`. The Studio MUST author the same IR,
  show a run or step waiting for a rate slot or a permit with its position and estimate from the single-run
  read, and list the keys. The worker protocol MUST NOT change.

#### Gates on readiness, grants and keys (US02 and US07, added after the review of 2026-11-13)

- **FR-022-051 — The readiness gate is in the reducer**: For a node that declares `concurrency` or
  `rateLimit`, every event that today makes a step `READY` (`step.released`, `step.retry_due`,
  `step.retry_requested`) MUST make it `WAITING` with wait kind `RATE_LIMIT` or `CONCURRENCY` instead, and
  only the grant event that completes its last declared gate (`rate_limit.granted`,
  `concurrency.acquired`) MUST make it `READY`. This holds whatever produced the event: a worker completion,
  the retry pass, a timer, a signal, an approval, an update, a finished child, an operator retry, an
  admission or a resume. At the single point where a transition is persisted (`persist` in
  `crates/infra-postgres/src/execution.rs`), one adapter MUST collect every gate key that the reducer's
  events touch, lock them once in the order of FR-022-044, and journal for each gated step either its grant
  or `*.queued` in the same commit; a commit that leaves a gated step released with neither is an invariant
  breach (`verify_run`). The task-ready outbox hint MUST NOT be written for a step that ends the
  transaction `WAITING`.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-203, D022-204):* a gated step may also leave its readiness completed
  by a `step.cache_hit` (asked before its gates) or a `step.mocked` (a draft test's pin), without any grant; the
  invariant reads "its grants, a `*.queued`, a `step.cache_hit` or a `step.mocked`". The task-ready hint is written only
  for a step that is `READY` when its commit ends.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-232, D022-238):* `step.cache_missed` is one of the hint's arms; every
  path that ends a run or announces a task goes through `persist` (ADR-0146), checked by `scripts/check-architecture.py`.
- **FR-022-052 — One grant routine and one timeout routine**: Every waiter, run or step, whatever its
  gates, MUST be granted by one routine and timed out by one routine. The grant routine is driven by key: a
  scheduler pass selects keys that have waiters and may have room (FR-022-047), under the fence of the
  key's home shard, and for each waiter it tries takes, in one transaction: the waiter's run (`SKIP
  LOCKED`) → its `runs` row → its semaphore → its windows → usage rows. It MUST check every gate the waiter
  declares and grant all of them or none. **Reservation rule**: on each gate, a waiter at position `p`
  (the number of eligible waiters of that key ahead of it) is granted only when the key's counted holders or
  grants plus `p` are below its own limit; eligible means not paused, not past its deadline, of a live run,
  and admitted by its own limit, so an older waiter refused only by its other gate keeps its place and an
  older waiter refused by its own lower limit does not block (D1196). A waiter whose run is terminal is an
  invariant breach: it is alerted and skipped, and never counts as ahead. The timeout routine, at the
  waiter's deadline, MUST journal the `*.wait_timed_out` of the gate that refuses it then (`rate_limit`
  first when both do) and the `*.dequeued` of the other, in one transaction.
- **FR-022-053 — Fleet activation**: `concurrencyKeys` and `rateLimits` MUST be instance capabilities
  (`flow-control-concurrency-v1`, `flow-control-rate-v1`, required of `CONTROL_PLANE` and
  `EXECUTION_ENGINE`) activated through feature 018's instance-capability registry (ADR-0050 decision 9,
  ADR-0082). Until an operator activates one, publication and starts that use it MUST be refused
  (`VALIDATION_FAILED`, `FLOW_CONTROL_ACTIVATION_PENDING`); activation MUST be refused while a live instance
  of either kind lacks it; after it, an instance without it refuses to serve (`UNAVAILABLE`,
  `BINARY_CAPABILITY_MISSING`). The current version of the keyed-digest key MUST be a database row read in
  each deciding transaction, never a per-process setting.
- **FR-022-054 — A step's key fails closed**: When a step-level key fails to evaluate or its canonical value
  exceeds 256 bytes, the step MUST fail `FLOW_CONTROL_KEY_INVALID`, and when it would create a key beyond
  the namespace's live-keys ceiling it MUST fail `FLOW_CONTROL_LIMIT`, in both cases through its failure
  route, without spending an attempt and without ever being granted. It never runs ungated.
- **FR-022-055 — Child starts**: A child node (including a `child` node in a collection body) whose child
  run's start would be refused by a full per-key buffer, a full `queued_runs` or the live-keys ceiling MUST
  fail with `RATE_LIMIT_QUEUE_FULL`, `CONCURRENCY_QUEUE_FULL` or `FLOW_CONTROL_LIMIT`, a retryable step
  failure handled by the node's retry policy and failure route; no child run is created.
- **FR-022-056 — Key cardinality and retirement**: Publication MUST refuse a key that reads only the run's
  identity (run id, business id, idempotency key) (`FLOW_CONTROL_KEY_UNBOUNDED`). A window row MUST carry the
  longest period among its live grants and waiters, and the purge MUST keep a grant until that period plus
  one minute has passed since its count instant. Rotating the digest key MUST work per key: while a rotation
  is in progress, a deciding transaction computes both digests and uses the row that exists under the old
  version, else the new one; the old version is retired when no row uses it, and no key is ever split
  between two rows.
- **FR-022-057 — Classification of keys**: Publication MUST refuse a concurrency or rate-limit key that
  reads a location classified `secret` (FR-016-002 carriers; `FLOW_CONTROL_KEY_SECRET`) and warn for `pii`
  (`FLOW_CONTROL_KEY_PII`). A key label MUST be shown only to a caller who may disclose, at its level, every
  location the key reads; to others the label is `null`. A label stays an online oracle for those who may
  see it (G1185).
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decision D022-229; security Major 4):* the rule extends to `cache.key`: a key over a
  `secret` location is refused (`CACHE_KEY_SECRET`), over `pii` warned (`CACHE_KEY_PII`), and the label rule applies to
  every surface that shows a cache key, its counts or a hit's source run.
- **FR-022-058 — Namespace counters**: `queued_runs` and live keys per namespace MUST be usage rows of
  ADR-0050 slot 8b, sharded like `run_active_counts` (16 slots), changed in the transaction that changes
  what they count and maintained by `SECURITY DEFINER` triggers; the runtime role reads them only. A run
  waiting for both gates counts once in `queued_runs` and once in each key's waiters.
- **FR-022-059 — The one admission port is enforced**: The gate MUST run inside `PgRunStore::create`
  (`crates/infra-postgres/src/execution.rs`), the one function that inserts a new run; the only other
  inserts into `runs`, the continue-as-new successors of `crates/infra-postgres/src/chain_progress.rs`,
  MUST assert that their definition declares no run-level gate (refused at publication, FR-022-017,
  FR-022-039). `scripts/check-architecture.py` MUST fail when `INSERT INTO runs` appears elsewhere.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-211):* the gate runs in `Engine::start`, as D022-049 decided for the
  slice ("inside `PgRunStore::create`" is superseded); `run_start_errors` of `scripts/check-architecture.py` refuses a
  domain start call outside it, and `PgRunStore::create` still refuses a gated definition a process does not serve.
- **FR-022-060 — A grant does not re-authorize**: Admission authorizes a start; a grant from the buffer
  MUST NOT re-check the initiator's permissions (a buffered run is an admitted run, like one waiting on a
  timer), and cancelling it is the remedy for a revoked initiator. The run keeps its original initiator in
  its journal and audit.

#### Cross-cutting

- **FR-022-031 — Additive IR and capability gates**: New fields and kind MUST be additive in IR v1
  (ADR-0032). `stepCache`, `earlyReturn`, `failureHandler` and `concurrencyKeys` MUST be separately enabled;
  using a disabled one MUST fail before a version exists; disabling MUST stop new publications and starts
  while existing runs drain with compatible readers.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-206):* each of `stepCache`, `earlyReturn` and `failureHandler`, and the
  surfaces of standalone steps and draft test runs, is also a capability of 018's instance-capability registry
  (`step-cache-v1`, `early-return-v1`, `failure-handler-v1`, `step-executions-v1`, `draft-test-runs-v1`, required of
  `CONTROL_PLANE` and `EXECUTION_ENGINE`), activated as FR-022-053 says; until activation their use is refused
  `VALIDATION_FAILED`, `FLOW_CONTROL_ACTIVATION_PENDING`.
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-236, D022-239):* a binary lists a registry capability only once every
  behaviour slice of it has merged (and, for `step-cache-v1`, `early-return-v1` and `failure-handler-v1`, the routing of
  ADR-0146). Once a journal holds a new event an older binary cannot decode the run: rollback is switching a capability
  off, never a binary downgrade.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-256; supersedes "once every behaviour slice of it has merged"):* a
  binary serves a capability only when its manifest of artefacts (event types, `Evaluation` variants, migrations,
  routes, the acceptance test present and not ignored) is complete; rollback is `platform_requirement_remove`, then the
  switch, and a switch never stops a phase or an obligation in flight. Publication refuses `continueAsNew` with `cache`,
  `onFailure` or `earlyReturn` (`FLOW_CONTROL_COMBINATION`).
- **FR-022-032 — Authorization and tenant boundary**: Operations MUST follow the plan's matrix (new
  `cache:invalidate`, `failure-handler:manage`, `step:standalone`; test runs need `definition:write` and
  `run:start`). Tenant and namespace come from the authenticated context; foreign resources MUST be opaque
  `NOT_FOUND`; replays MUST reauthorize; authorization MUST fail closed.
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-220):* the catalog holds 53 permissions today
  (`crates/domain/src/identity.rs`); the three new ones make 56; machine roles hold none.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-244):* the built-in roles gain `RunRecovery`.
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
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decision D022-220):* "the five SDKs" reads "the six SDKs", as FR-022-050 says.
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
  *Amended 2026-11-24, delegate (OD-2026-11-24-4; decisions D022-205, D022-213, D022-217):* cache entries hold no inline copy: they
  reference the producer's payload row by foreign key, so 020's discovery finds them; their `source_run_id` (no foreign
  key, archival must not cascade) is registered with the hook "delete by `source_run_id`". 020 is not on the tree: a
  022-side port deletes a source run's entries and pending obligations and lists the runs that received a copy, and
  022 creates the registry file with its own entries; SC-022-016 stays pending until 020 calls the port (gap G022-021).
  *Amended 2026-11-25, delegate (OD-2026-11-24-4; decisions D022-223, D022-230):* payload foreign keys are `ON DELETE RESTRICT`;
  erasure locks a source run's store obligations before its entries; erasing bytes shared by several runs, and ordering
  a run's deletion against the dispatcher, are feature 020's obligations (gap G022-021).
- **FR-022-038 — Reset rules**: Every journal event type this feature adds MUST declare its reset rule in
  018's reset-rule registry (`COPY`, `REMATERIALIZE`, `EXCLUDE`, `REFUSE`), as listed in
  `contracts/journal-events.md`: permit events and `result.returned` are re-materialized by the reset
  transaction, cache hits re-create their provenance row, and every reset point inside a failure-handler
  phase (after `failure_handler.started`) or after a queue timeout or dequeue is refused; a reset run never
  inherits `handler_for`.
  *Amended 2026-11-25 after the re-reviews, delegate (OD-2026-11-24-4; decision D022-253):* `step.cache_missed` is `REMATERIALIZE` (the reset copies
  `steps.cache_lookup` from the source step); `failure_handler.draining` is `REFUSE`, as the phase's other events.

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
  FR-038-001, gains a line for a plain admission before any admission change, and is otherwise unchanged).
  No gate adds anything to any claim. What a declaring definition adds to its own admission and
  completions is recorded as a budget line in the change that adds it, and the budget suite runs on every
  change to the files that can move it (`scripts/select-test-targets.py`).
- **NFR-022-008 (time)**: Every instant a rate decision uses is `clock_timestamp()` read in a statement
  after the deciding transaction has locked the key's window, and that instant is the one recorded.
  Replicas whose own clocks disagree grant the same waiters; a database clock that moves backwards delays
  grants and never adds one. The instant precedes the commit by the transaction's duration (G1184).

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
  *Amended 2026-11-25 (decision D022-240):* "a full cache-entries ceiling" for "a full content quota"; and a store over an
  invalidation, a store against erasure and a dead entry each behave as FR-022-005 as amended says, in barrier tests.
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
  *Amended 2026-11-24 (decision D022-220):* in all six SDKs.
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
  *Amended 2026-11-24 (decision D022-220):* the six SDKs, for the seven scenarios of `contracts/conformance.md`
  (the slice added `flow-rate-limit`).
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
  *Amended 2026-11-24 (decision D022-217):* pending until feature 020 calls the 022-side port (gap G022-021); until
  then the evidence is the port's own test against the registry file and the next lookup's miss.

- **SC-022-017**: With `limit: 10, per: second` and 1,000 same-key starts, across an engine restart, no
  one-second interval of the journals holds more than 10 grants of the key, grants follow admission order,
  each run is granted once, and starts beyond the buffer ceiling are refused with their code. A second key
  whose own buffer is saturated at the same time, so that its grants come from the pass, is served at its
  own limit; and with more keys and tenants holding waiters than one pass may visit, every eligible key of
  another tenant is granted within two passes. (FR-022-040..042, FR-022-047, FR-022-052)
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
- **SC-022-024**: With 64 or more steps of key A of one activity waiting for a permit (and, separately,
  for a slot) and one ready step of key B of the same activity, the next poll leases B, and the claim's
  probe counters show no candidate of key A. (FR-022-019, FR-022-051; FR-038-002)
- **SC-022-025**: For every readiness source of FR-022-051 there is a test in which a gated step is granted
  or queued in the same commit, never `READY` without a grant; a seeded commit that breaks it fails
  `verify_run`; no task-ready hint is written for a step left waiting. (FR-022-051)
- **SC-022-026**: A head waiter refused only by its other gate is granted before any younger waiter would
  exhaust its key; a timeout of a dual-gated waiter journals one `wait_timed_out` and one `dequeued` with the
  precedence of FR-022-052; a waiter of a terminal run raises the alert and is skipped. (FR-022-044,
  FR-022-052)
- **SC-022-027**: With one live engine lacking the capability, activation is refused and a definition
  using the capability is refused `FLOW_CONTROL_ACTIVATION_PENDING`; after activation, a process started
  without it answers `503`/`BINARY_CAPABILITY_MISSING`. (FR-022-053)
- **SC-022-028**: A step key error or oversize fails the step without an attempt; a child node meeting a
  full buffer fails with its code and creates no run; identity-only, `secret` and `pii` keys are refused or
  warned; a caller without disclosure rights sees `keyLabel: null`; a digest key rotation with live rows
  never splits a key; a granted step paused, reviewed or cancelled before its lease returns its slot and
  permit. (FR-022-046, FR-022-054..057)
- **SC-022-029**: With three engines and `limit: 100, per: second` on one key with a saturated buffer,
  the grants sustain at least 90 a second for 60 seconds and never exceed 100 in any second; the report
  states environment and failures. (FR-022-047, FR-022-052; NFR-022-004)

## Assumptions

- Cache, concurrency and early return target the root graph first; bodies follow with per-instance
  evidence (G1160). Only inline outputs (≤ 256 KiB) are cached, mocked or pinned (G1161).
- A failure handler never changes the outcome (O1161) and does not run for `CANCELLED`.
- Draft test runs never run in `prod` namespaces (O1160); unmocked steps are real executions with real
  effects in the chosen non-production namespace.
- *Amended 2026-11-24:* O1160, O1161 and O1162 are answered as these assumptions say (decisions D022-200 to D022-202,
  delegate, OD-2026-11-24-4). The business-ID policies of 018 exist: a standalone step accepts them (D022-209).
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
recommendation; the owner accepted all six recommendations on 2026-11-13 (OD-2026-11-13-16).
*Amended 2026-11-24:* O1160, O1161 and O1162 are answered by delegation (OD-2026-11-24-4), each with its
recommended option (a) (decisions D022-200 to D022-202).

## Release 0.3.0 slice

Decided by the owner on 2026-11-13 (OD-2026-11-13-11): throughput control for flows with execution limits,
as "Concorrência + taxa".

| In the slice | Requirements | Success criteria |
|---|---|---|
| US02 — concurrency limits by key with a durable queue, as specified | FR-022-017..022 | SC-022-003, SC-022-004, SC-022-015 (creators that exist) |
| US07 — rate limits per window with buffering | FR-022-039..050 | SC-022-017..023 |
| Gates on readiness, grants and keys (both stories; review of 2026-11-13) | FR-022-051..060 | SC-022-024..029 |
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

*Amended 2026-11-24 (OD-2026-11-24-1):* the stories and requirements this section leaves out are now in release 0.3.0
too; see the next section.

## Release 0.3.0, the rest of 022

Decided by the owner on 2026-11-24 (OD-2026-11-24-1, "Inclua tudo na 0.3.0"), with the design questions delegated
(OD-2026-11-24-4). The approved requirements above stand, as amended on 2026-11-24.

| In release 0.3.0 now | Requirements | Success criteria | Design |
|---|---|---|---|
| US01 — the step result cache | FR-022-001..007 | SC-022-001, SC-022-002 | ADR-0135 (proposed) |
| US03 — failure handlers | FR-022-012..016 | SC-022-005, SC-022-006 | ADR-0136 (proposed) |
| US04 — early return | FR-022-008..011 | SC-022-007 | ADR-0136 (proposed) |
| US05 — standalone steps | FR-022-023..025 | SC-022-008 | ADR-0137 (proposed) |
| US06 — draft test runs | FR-022-026..030 | SC-022-009 | ADR-0137 (proposed) |
| Cross-cutting for these stories | FR-022-031..035, FR-022-037, FR-022-038 | SC-022-010..016 | decisions D022-206, D022-211..213, D022-217 |

What the rest of the feature changes in the as-built engine, in order of the commit:

1. **Admission** goes through the one port `Engine::start` for every new creator (standalone, draft test, handler
   dispatch, the derived handler child), in the slots of ADR-0050 decision 7 as amended (D022-211, D022-214).
2. **Evaluation rounds before `persist`**, in this order: the lookup round (`Evaluation::CacheLookup`, D022-203), then
   the gates round (`Evaluation::Gates`, ADR-0094), each asked once the planner has nothing else to emit; the early
   result (`Evaluation::EarlyResult`) and a handler child (`Evaluation::Child`) are asked where the planner reaches them.
   A hit or a mock completes a step before its gates are asked, so it takes no permit and no slot (D022-204).
3. **`persist`** writes the transition, the result, the handler phase's events and the namespace dispatch row (the
   policy row read without a lock, D022-208), and announces only a step that is `READY` when the commit ends.
4. **Every end of a run** goes through the reducer: the collection- and loop-finish paths are routed (D022-207).

Still pending on another feature: FR-022-036 (031, gap G022-022), SC-022-016 (020, gap G022-021), and the effect kinds
of `TEST_RUN_UNMOCKED_EFFECT` (021, 024, 025, 030; gap G022-023). The plan is in
[plan.md](plan.md#release-030-the-rest-of-022) and the slices in [tasks.md](tasks.md#release-030-the-rest-of-022-od-2026-11-24-1).
