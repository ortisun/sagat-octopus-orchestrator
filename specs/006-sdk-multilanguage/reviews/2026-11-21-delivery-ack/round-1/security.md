# Security review: delivery acknowledgement (ADR-0099, worker protocol 1.8), round 1

I reviewed the design documents in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack` against the code they name. There is no implementation to review yet. A claim marked "verified" was checked in the code at the cited lines. Anything else is labelled as a design-level concern.

The central claim holds for a host that keeps the protocol: the acknowledgement and the withdrawal both take the run lock and both check the lease, so only one of them can commit for a given lease. The problems are around that claim:
- an acknowledged lease still receives capabilities and can still be withdrawn;
- scoped (collection) instances are not designed at all;
- the provisional mark is not tied to the lease it describes;
- each call costs the server far more than the design says, before any token is checked;
- the dependence on the host keeping the protocol is stated more strongly than the server can know.

No Blocker. Five Major, six Minor, four Nit.

---

## Major

### Major 1. An acknowledged lease receives secrets and blobs and can still be withdrawn as "nothing ran" (verified)

**Where.** 
- ADR-0099 decision 7 and its "Consequences" ("a provisional lease never receives a secret, a database credential, a blob or a log stream").
- `research.md` D006-020 ("A credential or a blob is not handed to a lease the server may withdraw and give to another host").
- The `stream-completion.md` failure table, row "The acknowledgement's answer is lost after its commit".
- ADR-0098 decision 5, which withdraws on `TASK_NOT_STARTED` for any step still `LEASED`.

**What is wrong.** The acknowledgement writes no `step.started`, so the step stays `LEASED` until the first heartbeat. Short tasks never send one: the ADR's context says a task shorter than its heartbeat interval sends no heartbeat at all, and the design calls these "most tasks". Every capability is served once the lease is acknowledged. ADR-0098 decision 5 then lets the host hand back any `LEASED` step with `FailStep TASK_NOT_STARTED`, which withdraws it with no attempt consumed and no review.

**Failing scenario.** A 1.8 host acknowledges, calls `ResolveSecrets` (which commits a `SecretResolved` audit), runs the effect, then sends `TASK_NOT_STARTED`. This can happen because the host is stopping, buggy or hostile, or because someone holds a stolen token and a bearer token of the same tenant. The server journals "no handler ran" while it holds a committed record that the attempt received its credentials. The step returns to `READY` and is leased to another host with the same attempt number and idempotency key, and that host resolves the same secrets again.

**Why it matters.** Decision 7's stated purpose is false as written. Principle III is bent: an outcome is recorded as known while the server holds evidence that the attempt started.

**Smallest fix.**
- The capability path records first use on the step row. `ResolveSecrets` (`crates/application/src/worker/secrets.rs:208-217`) and `ReadBlob` already commit an audit in a transaction, so this adds one statement and no new transaction.
- The `NOT_STARTED` withdrawal requires that mark to be empty; otherwise it is an ordinary retryable failure that consumes the attempt.
- Add a test: acknowledgement, then `ResolveSecrets`, then `TASK_NOT_STARTED` must not be withdrawn.
- If you decide not to fix it, state the window as a gap and remove the sentence from decision 7.

### Major 2. Scoped (collection) instances have no design, and their capability checks use different queries (verified)

**Where.** 
- `stream-completion.md`, "The server's acknowledgement", step 1 (`begin(tenant, run, Lock::Wait)`), and the sections "The provisional mark" and "The withdrawal".
- ADR-0099 decisions 2, 3 and 7.
- Tasks T006-047, T006-049, T006-050 and T006-053.

**What is wrong.** Every scoped step is handled on separate code paths:
- **Heartbeat:** `crates/application/src/worker.rs:1828-1830` sends a non-root instance to `heartbeat_path`, which uses `begin_current_path` (`crates/application/src/worker/scoped.rs:93-151`), not `begin`.
- **Capabilities:** scoped capabilities go through `live_path_fence` and then `PgPathExecutionTx::lease_unexpired` (`scoped.rs:62-89`, `crates/infra-postgres/src/execution/path.rs:242-256`). This applies to secrets (`crates/application/src/worker/secret_resolution.rs:80-88`) and to blobs (`worker.rs:2067-2070`).
- **Lease writes:** scoped leases are written by a separate statement (`execution/path.rs:1227`).

The design names only the root primitives.

**Failing scenario.** Suppose the scoped claim marks the lease provisional but the refusal is added only to the root query (`crates/infra-postgres/src/execution.rs:4846`). Then a provisional collection item receives secrets and blobs before it is acknowledged, which is exactly what decision 7 forbids. If instead scoped claims are not marked, a frame lost on the way to a collection item still goes to review at 1.8, and the design presents gap G006-001 as closed.

**Smallest fix.** For each of the claim, the acknowledgement, the capability refusal, the lost-lease withdrawal and the reset, name both the root and the path primitive. Add one test per path. Alternatively, explicitly exclude scoped leases as a named gap, with a test that a scoped lease of a 1.8 session is never marked.

### Major 3. The provisional mark is not tied to the lease it describes, so a missed clear silently changes a lease's meaning (verified for the number of writers)

**Where.** 
- ADR-0099 decision 2 ("Every write that clears or replaces a lease clears both columns").
- `stream-completion.md`, "The provisional mark".
- D006-013.
- T006-047, which tests only the claim and a reset.

**What is wrong.** The rule is prose with nothing enforcing it. `steps` lease columns are written by at least these:
- the generic root step update (`execution.rs:7045` and `7370`);
- the path update (`execution/path.rs:1227`);
- the claim;
- the reaper;
- reset;
- ADR-0098's withdrawal.

**Failing scenarios.**
- **A stale offer mark.** One writer issues a new lease, for a 1.7 session or during the window of gap G006-008, and leaves the previous lease's `lease_offered_at` in place. The new lease then looks provisional and unacknowledged. Its 1.7 host is refused secrets, which is at least loud. Worse, a short task with no capability call whose completion is lost is withdrawn at expiry as `NOT_ACKNOWLEDGED`. It is then run again with the same attempt number and no review, although a 1.7 host ran it. That is a silent breach of principle III.
- **A stale acknowledgement mark.** A leftover `lease_acknowledged_at` makes a new 1.8 lease look acknowledged. It receives capabilities before its host has acknowledged it, and a lost frame lapses into review.

**Smallest fix.** Tie each mark to its token: store `lease_offered_token_hash` and `lease_acknowledged_token_hash` (or one hash plus the times). A lease is provisional only when the offered hash equals `lease_token_hash`, and acknowledged only when the acknowledged hash equals it. A writer that forgets the columns can then never make another lease provisional or acknowledged. Add a CHECK that both marks are null when the status is not `LEASED` or `RUNNING`. To list every writer, run `rg -n "lease_token_hash|lease_expires_at" crates/infra-postgres/src`.

### Major 4. Each call does up to 64 run-lock transactions, each loading the whole run, before any token is checked; plus up to 64 unbounded audit writes, inside the reserved capacity (verified)

**Where.** 
- ADR-0099 decision 3 ("it loads no run state") and option 1's cost.
- D006-014.
- `stream-completion.md`, "Wire" (1 to 64 tasks, admission class `Lease`) and "The server's acknowledgement", steps 1 to 4.
- T006-049 and T006-054.

**What is wrong.**
- **`begin` loads the whole run.** It locks the run row `FOR UPDATE` and the chain row `FOR UPDATE`, then loads every step row, the definition, the sagas, collections, loops and the chain (`execution.rs:282-460`, `462-511`). Heartbeat does this before the token check (`worker.rs:1831-1832`, then `verify` at `2842`). The claim "loads no run state" is false for anything built on `begin`.
- **Audit writes bypass the bound.** `missing_task_error` runs `run_exists_outside` and writes `record_denial` directly, once per run (`worker.rs:2564-2586`). This skips the bounded denial gate. That gate's volume argument assumes one record per request (`crates/api-http/src/denial.rs:14-20`).
- **The rate budget counts requests, not tasks.** The per-subject budget is 20 requests per second with a burst of 40 (`crates/api-http/src/limits.rs:70-75`).
- **These calls use the reserve.** The `Lease` class is admitted up to full capacity; only discovery is shed (`crates/api-grpc/src/limits.rs:499-509`, `548-563`).

**Failing scenario.** A worker with `worker:execute` and no lease at all sends `AcknowledgeTasks` with 64 entries and random tokens.
- **Run ids of its own tenant, in any namespace:** each entry costs a `FOR UPDATE` wait and a full state load. At the budget that is about 1,280 run locks per second, stalling those runs' transitions.
- **Run ids of other tenants:** each entry costs a cross-tenant lookup and an audit insert, about 1,280 audit rows per second from one subject.
- **Effect on other tenants:** these slow calls sit in the `Lease` reserve, the quarter of capacity kept for every tenant's heartbeats and completions. Once discovery fills its three quarters, other tenants' heartbeats are shed, their leases lapse, and they get reviews.
- **Timing:** the one-insert timing difference between a foreign and a missing run already exists in Heartbeat. A 64-entry call makes it far easier to measure.

**Smallest fix.**
- (a) Before taking any lock, run one indexed statement: `SELECT run_id, node, instance_path_nodes, instance_path_indices FROM steps WHERE tenant_id = $1 AND lease_token_hash = ANY($2) AND status IN ('LEASED','RUNNING')`. The token is the secret, so this answers nothing to a caller who lacks it. Open run transactions only for runs that matched.
- (b) Do one cross-tenant check for the remaining run ids and write at most one audit record per call, through the bounded gate.
- (c) Either specify a lock-only open (the run and chain locks plus the call's own step rows and the cached definition), or withdraw the "loads no run state" claim and redo the cost argument.
- (d) State that a call never holds two run locks at once.
- (e) Add a 64-run case to the `ACKNOWLEDGEMENT` budget line.

### Major 5. The guarantee depends on the host keeping the protocol; the 037 text says it as fact, and the server throws away the evidence it does get

**Where.** 
- ADR-0099 decision 4 and "Consequences".
- D006-015 and G006-006.
- The FR-037-006 clarification in `specs/037-compensation-safety/spec.md` ("is not an attempt either: a host of 1.8 starts nothing before the acknowledgement").
- Decision 7 ("served as today ... the safe direction").

**What is wrong.**
1. **The clarification is unconditional.** The protocol minor is a string the client declares itself (`protocol_version`, `specs/001-platform-foundation/contracts/worker.proto:76`), so any client can claim 1.8.
2. **Gap G006-006 is too narrow.** It covers only a client that "dies before any call". In fact, any client that runs a handler without acknowledging, and does not heartbeat or report within the lease's time-to-live, is re-run with no review. That includes a slow handler or a lost report. Today that case goes to review.
3. **Two kinds of proof are discarded.** A `Heartbeat`, `CompleteStep` or `FailStep` on a provisional, unacknowledged lease proves the host broke rule 1. A late report, refused as stale, whose token matches a lease withdrawn as `NOT_ACKNOWLEDGED` proves a handler ran on a lease the journal says never ran. The design records neither.
4. **This reinterprets constitution principle III.** Principle III says unknown external outcomes default to consultation or review. The delegate may propose; acceptance must be the owner's as a stated trust assumption, not a clarification.

**Smallest fix.**
- Make the 037 clarification conditional: "for a host that keeps the protocol; the server cannot verify it".
- Widen G006-006 to the slow-handler and lost-report cases.
- On any report or heartbeat that reaches a provisional, unacknowledged lease, record the session as non-conforming: stop marking its leases provisional, and count it in a bounded metric such as `worker_protocol_violations_total{method}`.
- Keep the token hash of a lease withdrawn as `NOT_ACKNOWLEDGED` on the row, as `reported_lease_token_hash` does. A late report that matches it should open a review, or at least increment an alerted counter.
- Put the trust assumption in the ADR's text for the owner to accept.

---

## Minor

### Minor 1. A revoked binding or membership becomes a silent `NOT_HELD`, and the call-level permission check is not stated (verified)

**Where.** `stream-completion.md`, "The server's acknowledgement", step 2; the failure row "a binding revoked after the claim"; D006-014.

**What is wrong.** Today Heartbeat answers a revoked binding with `NotBound` (403, `crates/api-http/src/error.rs:737`) and a lost membership with `Forbidden` (`worker.rs:2735`). Both are audited under gap G9 (`denial.rs:1-3`). Folded into a per-task verdict, they leave no audit record. The design also never says the call checks `Permission::WorkerExecute` once, before opening any run, as Heartbeat does (`worker.rs:1827`). Without that, a caller with only a viewer token reaches the run transactions.

**Smallest fix.**
- Check the permission at call level: `PERMISSION_DENIED`, audited, before the first run is opened.
- For per-task binding refusals, write one bounded audit record per call.
- Add both cases to T006-049's tests.

### Minor 2. "The query that already checks the lease" is shared with reports, and the refusal's position decides whether it leaks (verified)

**Where.** ADR-0099 decision 7; D006-020; T006-053.

**What is wrong.** `lease_unexpired_instance` (`execution.rs:4846-4867`) is called from:
- `attempt_authority_fence` (`worker.rs:2813-2840`), which serves both capabilities and `CompleteStep`/`FailStep` (including `secrets.rs:278` and `287`);
- `check_blob_access` (`worker.rs:2075`);
- `crates/application/src/worker/output.rs:273`;
- `crates/application/src/worker/scoped_output_refusal.rs:342`.

**Consequences.**
- Putting the provisional condition in that query would refuse reports on a provisional lease. That includes the `TASK_NOT_STARTED` hand-back of the excess leases that the host never acknowledges (`stream-completion.md`, host item 1), and contradicts decision 7.
- If the condition is evaluated before the token match and the binding check, `TASK_NOT_ACKNOWLEDGED` tells a caller without the token that the step has a provisional lease.

**Smallest fix.**
- Use a separate variant, called only from the capability paths (root and path).
- Return `TASK_NOT_ACKNOWLEDGED` only after the token hash has matched and the binding check has passed.
- Tests: a report on a provisional lease is served; a wrong token on a provisional lease gets the ordinary stale answer.

### Minor 3. Where the requirement dependency is enforced is unspecified, and removal is done by hand SQL (verified)

**Where.** ADR-0099 decision 9; `stream-completion.md`, "Negotiation and activation"; D006-017; T006-046.

**What is wrong.**
- **Removal checks are left to the caller.** `platform_requirement_remove` is generic and leaves dependency checks to "the caller" (`crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:353-378`). The runbook prescribes hand SQL when a feature has no removal command (`docs/runbooks/instance-registry.md:152-162`). The design names no removal command for either requirement.
- **Removal fails open.** If the operator removes `lease-withdrawal-v1` that way while `delivery-acknowledgement-v1` is recorded, engines keep writing `step.lease_withdrawn`. A rollback below ADR-0098 then cannot replay those runs.
- **The activation check can race.** The activation's check for `lease-withdrawal-v1` must be read after `platform_requirement_lock()` in the same transaction, or a concurrent removal slips past it.

**Smallest fix.**
- Enforce the dependency in the database: a guard in the existing `ENABLE ALWAYS` trigger function (`SECURITY DEFINER`, `search_path` pinned like its siblings) that refuses deleting the first requirement while the second exists, and inserting the second without the first.
- Name `control-plane delivery-acknowledgement-remove`.
- Test both orders with the race held open.

### Minor 4. Withdrawals for `NOT_ACKNOWLEDGED` have no bound, and the attempt timeout is switched off for them

**Where.** ADR-0099 decision 6; D006-019; D006-006 of ADR-0098 ("needs the server's own proof ... or the host's own statement").

**What is wrong.** A `NOT_ACKNOWLEDGED` withdrawal needs neither proof nor statement; silence is enough. D006-006's premise does not carry over. Hosts that can loop:
- an SDK whose acknowledgement answer is always unusable;
- a proxy that answers `UNIMPLEMENTED` for the new method, so the host hands every lease back;
- a hostile worker.

Each of them polls, gets no verdict, is withdrawn and polls again, forever. No attempt is consumed, `timeoutMs` never fires (the timeout check skips provisional leases), and no review opens.

**Smallest fix.**
- Keep a per-step count of consecutive `NOT_ACKNOWLEDGED` withdrawals, cleared by an acknowledgement. Above N, the next lease is not provisional (today's meaning applies) or the step fails with a distinct code.
- A host that gets `UNIMPLEMENTED` for `AcknowledgeTasks` registers again, so it negotiates down.

### Minor 5. The rollback text overstates what can be rolled back

**Where.** ADR-0099, "Rollback"; T006-044.

**What is wrong.**
- "Then roll the binary back" gives no floor. Once any `NOT_ACKNOWLEDGED` is journaled, a build older than this decision cannot replay those runs.
- A session accepted at 1.8 that meets an older build gets `UNIMPLEMENTED`. Host item 4 then hands every lease back, so the session runs nothing until it registers again, and nothing tells it to.
- The ADR says the down migration refuses "while a provisional mark is present"; T006-044 says "while any row has a mark".

**Smallest fix.** State the floor. Specify the host's reaction to `UNIMPLEMENTED` (register again). Align the down migration on "any mark".

### Minor 6. The message element that carries the token keeps the generated `Debug` (verified)

**Where.** `stream-completion.md`, "Wire" ("Its request and response join `skip_debug`"); T006-043.

**What is wrong.** `crates/api-grpc/build.rs:17-25` lists the types it skips. Skipping only `AcknowledgeTasksRequest` leaves `TaskAcknowledgement`, which carries `lease_token`, with a derived `Debug`. Any `?task` in a per-task log line would print the token. Existing request types such as `HeartbeatRequest` also derive `Debug`; that predates this design.

**Smallest fix.** Skip `TaskAcknowledgement` too, give it a redacting `Debug`, and run the token canary test on the element as well as the request.

---

## Nit

### Nit 1. The acknowledgement does not check the expiry or the generation and attempt

`verify` uses `lease_matches_instance`, which has no expiry predicate (`execution.rs:4999-5019`). Between expiry and the next lost-lease pass, a lease can be acknowledged and renewed, so the real deadline is the expiry plus one pass period. Because the acknowledgement does not load the state, the generation and attempt are not checked against the row either. In addition, `Attempt::parse_heartbeat` accepts attempt 0 (`crates/api-grpc/src/request.rs:53-70`).

Fix: say so, or add `lease_expires_at > clock_timestamp()` and the row's `attempt` and `generation` to the acknowledgement's statement, and require attempt 1 or more at 1.8.

### Nit 2. `AcknowledgeTasksRequest.worker_id` has no stated purpose

Say it is validated and ignored, as Heartbeat does (`crates/api-grpc/src/service.rs:891-893`), and never used as authority.

### Nit 3. Observability details to state

Add span links only for tasks answered `ACKNOWLEDGED`. State that the two gauges carry no labels, and that the server's `NOT_HELD` log line prints identifiers only.

### Nit 4. State the per-element size bound

The 64-task limit is checked after tonic has decoded the whole message, which is capped at `MAX_MESSAGE_BYTES` (`crates/api-grpc/src/server.rs:14`). State the per-element bound (path depth is already 8) so that 64 times the largest element stays small.

---

## Classes I checked and found nothing

- **The exactness argument.** Both writers take `begin` with `Lock::Wait` (`execution.rs:1563-1577`) and check the lease. The argument holds for a host that keeps the protocol.
- **The tenant boundary in SQL.** The run lock and the chain lock are both filtered by tenant (`execution.rs:469-475` and `RUN_FOR_UPDATE`), so a foreign run is never locked. The `NOT_HELD` body is the same for a missing run and a foreign one. The timing difference is the one in Major 4.
- **Migration privileges.** Grants on `steps` are table-level (no column grants found in the migrations), so the new nullable columns need no new grant. No new `SECURITY DEFINER` function is proposed, apart from Minor 3's fix. The acknowledgement's update fires the existing triggers on `steps`; count them in the budget line.
- **Metric labels.** All are bounded sets with no tenant, namespace or identifier.
- **Injection and parsing.** No dynamic SQL and no new free-form input.
