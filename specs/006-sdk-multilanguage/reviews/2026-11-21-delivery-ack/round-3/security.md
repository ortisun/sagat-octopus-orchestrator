# Security review, round 3: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Verdict: no Blocker, 2 Major, 6 Minor, 5 Nit.**

All three round-2 Majors in my report are resolved or have moved:
- The renewal channel is gone. I checked every place the documents describe the call.
- The binding is now checked per namespace, under the same locks Heartbeat takes.
- Hand-backs are exempt from the evidence rules.

Both new Majors come from the new decision 15: reserving `TASK_NOT_STARTED` for the host at 1.8.
- The design does not say how the server knows a session's minor when a `FailStep` arrives. The only data that exists can be changed by registering again.
- The design now owns the hand-back's semantics at 1.8, but it still exempts from the evidence the one lie the server can detect.

"Verified" means I read the cited code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`. "Design reading" means the defect is in the text alone.

---

## Status of round 2

### My report

| Round-2 finding | Status | Reason |
|---|---|---|
| Major 1: required hand-backs counted as violations and as calls after a withdrawal | **Resolved** | Decision 1 exempts the hand-back. Decision 12 and Decision 006-053 exempt hand-backs and count them apart. Task 006-053 tests a give-up hand-back before and after the withdrawal. One residue of the same kind is new Minor 4: an acknowledgement sent after a withdrawal. |
| Major 2: `AcknowledgeTasks` as a renewal channel; capabilities then a hand-back on unmarked leases | **Partly resolved** | The renewal is gone, verified in all four documents: decision 3, Decision 006-040, `sdks/SPEC.md` §6.3 ("the acknowledgement renews nothing"), and the "Nothing else" paragraph of `stream-completion.md`. Duplicates take no row lock: an `UPDATE` locks only the rows that meet its `WHERE` on its snapshot, and `lease_acknowledged_at IS NULL` excludes them. The capability-then-hand-back residue is left to the Architecture decision 0098 build, "or names that residue as its own gap" (Decision 006-059 (4)). Decision 15 now makes it belong to 1.8 alone. See new Major 2. |
| Major 3: binding decided on the wrong namespace's data and without the revocation's locks | **Resolved** | Step 3: per namespace, binding lock and patterns, then the secrets lock and the membership lock. This is the order of `ensure_bound` (`crates/application/src/worker.rs:2761-2815`, `crates/infra-postgres/src/execution.rs:1017-1044`, `4805-4825`). Task 006-049 has the two-namespace case and the revocation held open. Residues: new Minors 1 and 2. |
| Minor 1: the denial gate cannot be built as written | Resolved | At most one `AccessDenial`, audited through `spawn_audit_access_denied` whatever the status (`crates/api-http/src/denial.rs:114-132`). At most one cross-tenant record, written as `missing_task_error` writes it. |
| Minor 2: the violation flag cannot be tied to a session | Resolved (dropped) | The same problem of attributing a lease to a session returns in new Major 1. |
| Minor 3: a hand-back resets the bound of three | Resolved | Only an acknowledgement clears the counter (Decision 006-052). |
| Minor 4: calls after a withdrawal flood the audit and cover too little | Resolved | Audited once per step and withdrawn hash. Capability calls are included. G006-042 states that only the last hash is kept. |
| Minor 5: the registry guard | Resolved in the design | Invoker function, schema-qualified names, epoch row taken first, the three closed checks extended `NOT VALID`. New Minor 5 covers what the guard still does not check. |
| Minor 6: descriptor walk | Resolved | Nested, `oneof` and repeated fields; the test fails instead of skipping. |
| Minor 7: `CANCELLED` decided after the write | Resolved | The write requires `cancel_requested = false`; `CANCELLED` writes nothing. |
| Minor 8: `ATTEMPT_TIMEOUT_BEFORE_START` | Resolved (superseded) | A provisional lease is now withdrawn at its earlier deadline, and the 037 clarification covers that. |
| Nits 1 to 4 and 6 | Resolved | Rollback waits until no mark counts. No session expiry on removal. SDK token canaries are in Tasks 006-059 to 006-064. The relaxation asked of Architecture decision 0098 is narrowed. Addresses are bound arrays. |
| Nit 5: timing class | Partly resolved | The difference is now stated, not removed. New Nit 1. |

### Findings of the other round-2 reports in my domain

| Finding | Status |
|---|---|
| Design Blocker 1 and architecture Major 2 (the protocol punishes required hand-backs) | Resolved, as my Major 1 above. |
| Design Major 5 (admission class `Lease` starves heartbeats) | Resolved by the class `Acknowledge`. The wording is imprecise: Nit 5. |
| Design Major 2 (deadline measured at the start of the transaction) | Resolved: `clock_timestamp()` is evaluated under the row lock. |
| Architecture Major 4 (marks outlive `lease-withdrawal-v1`) | Partly resolved: the command checks marks, but the check races and hand SQL skips it (Minor 5). |
| Correctness Minor 4 (folding refusals loses an audit) | Resolved, as my Minor 1 above. |

---

## Major

### Major 1. Nothing says how the server knows "a session accepted at 1.8" for a `FailStep`, and the only data that exists can be changed by registering again (verified)

**Where.** Architecture decision 0099 decision 15; Decision 006-061; the table in `stream-completion.md`, "The hand-back code at 1.8"; `sdks/SPEC.md` §6.2.

**What the code has.**
- A `FailStep` names no session. `worker_id` is "validated and ignored" (wire bullets), and the token is the only authority.
- The claim stores the session id in `steps.leased_by` (`crates/infra-postgres/src/execution.rs:7062-7063`). The minor is stored on the session (`worker_sessions.accepted_protocol_minor`).
- Registering again under the same tenant, subject and worker name keeps the session id and overwrites `accepted_protocol_minor` and even `namespace_id` (`crates/infra-postgres/src/workers.rs:328-344`, `ON CONFLICT (tenant_id, subject, worker_name) DO UPDATE`). §6.1 says registering again replaces the session.
- The merged base does not implement the reservation. The root hand-back (`crates/application/src/worker.rs:2430-2436`) and the scoped one (`crates/application/src/worker/scoped_reports.rs:376-388`) withdraw from any session, with no cap. So prerequisite (a) of the task phase is not met in this worktree, and the design gives that build nothing to key the check on.

**Failing scenarios.** Assume the minor is read through `leased_by` when the report arrives, which is the only option the schema allows.
1. **The reservation is bypassed.** A rolling upgrade, or two processes configured with one worker name. A 1.4 host (an older SDK passes a valid handler code through unchanged) claims a lease, and its handler performs its effect and raises `TASK_NOT_STARTED`. Meanwhile a 1.8 process registers under the same name, which sets the minor to 8. The server reads 8 and withdraws the lease: the journal says "nothing ran", the attempt number and idempotency key are reused, and there is no review. This is the case the reservation exists to close.
2. **Genuine hand-backs consume attempts.** The 1.8 host's own fallback ("hand back what waits ... registers again offering 1.7") sets the minor to 7. Any hand-back still being retried (§5 retries it up to 5 times) becomes an ordinary retryable failure for a lease that never ran.
3. **A deleted session.** A binding revocation deletes the session row (`crates/infra-postgres/src/bindings.rs:254-261`). The rule then falls back on whatever the build chooses for a missing row, and nothing says to fail closed.

The per-attempt cap has the same gap: no column is named for "withdrawals on `TASK_NOT_STARTED` for this attempt number". Without one, the loop bound in Decision 006-061 is a promise only.

**Fix.**
- Make the decision a property of the lease, written by the claim like the mark. In each of the three lease statements, set a new `steps.lease_hand_back_generation bigint` to the lease's generation when the claiming session's accepted minor is at least 8. It counts only while it equals `generation`.
- The `FailStep` decides from that column, under the run lock. It never reads the session row at report time, and never uses `worker_id`.
- Name where the per-attempt cap is counted (a column beside this one, or the journal), and pass both to the pull request #260 build as request (5) of Decision 006-059.
- Tests:
  - a 1.4 claim, then registering again at 1.8 under the same name, then `TASK_NOT_STARTED`: an ordinary failure;
  - a 1.8 claim, then registering again at 1.7, then a hand-back: withdrawn;
  - a session deleted by a revocation between claim and hand-back: decided by the column;
  - each of these root and scoped.

### Major 2. At 1.8 the server sees two lies and records neither: a hand-back after a capability was served, and a capability call on a provisional lease (design reading, against verified code paths)

**Where.**
- Decision 12 ("What the server does see, it records").
- Decision 006-053 (hand-backs exempt).
- Decision 006-059 (4) (residue left to the Architecture decision 0098 build "or names that residue as its own gap").
- Decision 006-045 ("an unmarked lease keeps the rules of Architecture decision 0098").
- The failure row "A capability call on a provisional lease ... counted".

**What is wrong.**
1. **Unmarked leases at 1.8.** Until `delivery-acknowledgement-v1` is activated, every lease of a 1.8 session is unmarked. Afterwards, so is every lease with a short deadline or past the bound of three.
   - Such a lease is dispatched at once, and `ResolveSecrets`, `ResolveDatabase`, `ReadBlob` and `WriteBlob` serve it. The secrets path commits a `SecretResolved` audit (`crates/application/src/worker/secrets.rs:208-217`).
   - With no heartbeat the step stays `LEASED`, so a hand-back within the lease's time-to-live is withdrawn as `NOT_STARTED`: attempt reused, no review.
   - Under decision 15 only a 1.8 session can do this. The residue is therefore this design's, not Architecture decision 0098's, and "this design no longer widens" it is no longer true.
2. **Provisional leases.** A capability call on a provisional lease is by definition a violation of decision 1: a conforming host never meets `TASK_NOT_ACKNOWLEDGED`. It is only counted, and not audited. A host that tries `ResolveSecrets` before acknowledging, is refused, and then hands back leaves no record tying the two together.

**Failing scenario.** A hand-written or buggy 1.8 client, requirement not yet activated (the state in which release 0.3.0 ships, Decision 006-057):
1. It claims a lease and calls `ResolveSecrets`, which is served.
2. It calls the provider with the credential and never heartbeats.
3. Twenty seconds later it sends `FailStep(TASK_NOT_STARTED)`.

The journal records `step.lease_withdrawn` (`NOT_STARTED`). The next host resolves the same secrets under the same attempt number, and no counter, audit or alert links the two. The server had every fact it needed.

**Fix.**
- In the transaction every served capability call already commits (it holds the run lock or the path lock), set a new `steps.lease_capability_generation = generation`. That is one `UPDATE` beside an audit insert that already happens.
- A hand-back withdraws only while that column differs from `generation`. Otherwise it is an ordinary retryable failure, counted as `worker_protocol_violations_total{method="FailStep"}` and audited once per step and generation.
- Add "a capability call refused `TASK_NOT_ACKNOWLEDGED`" to the violations of decision 12, audited once per step and generation.
- This also answers Decision 006-059 (4) inside this design, so it no longer depends on the other build.
- Tests: secrets served to an unmarked 1.8 lease, then a hand-back, gives an ordinary failure plus one audit; a refused capability call plus a hand-back gives one violation audit and the withdrawal is kept; both root and scoped.

---

## Minor

### Minor 1. A call that spans namespaces takes the membership row lock before the next namespace's secrets lock, and costs up to about 130 statements rather than "about 6" (verified)

- **Lock order.** The documented rule is "this lock, then the rows of the authority" (`crates/infra-postgres/src/quotas.rs:303-306`; `execution.rs:1023-1026`). Running `current_worker_authorized` once per namespace yields secrets A, then membership, then secrets B, so the membership row is held before a later secrets lock.
  - I found no deadlock today. Every runtime taker of the membership row uses `FOR SHARE` (`migrations/20261126000000_authority_read_locks.up.sql:4-10`). The owner's membership change is a bare `UPDATE`. `replace_binding` takes one namespace key and the profile key (`bindings.rs:88-100`).
  - The first writer that follows the documented rule with a stronger lock would deadlock with this call. The design also gives no order between namespaces.
- **Cost.** An identity may hold 32 sessions (`MAX_SESSIONS_PER_IDENTITY`, `crates/application/src/worker.rs:199`), and each session is in one namespace. A call can therefore touch 32 namespaces at about 4 statements each.
- **Fix.** The simplest fix: a frame or poll answer comes from one session and so from one namespace. If the pre-read matches rows in more than one namespace, answer `INVALID_ARGUMENT`. That is no oracle, because the caller holds the tokens. Otherwise, take all binding keys in one sorted statement, read the patterns in one statement, take all secrets keys in one sorted statement, and take the membership lock once, last. Add a budget line for the multi-namespace call.

### Minor 2. Binding coverage is per row, not per namespace, and the pre-read has no node kind (verified)

- **Per row.** `ensure_activity_bound` checks the patterns against each step's activity, and only for `Activity` nodes. `ensure_bound` has a managed-database branch that skips the membership check (`worker.rs:2761-2815`). The text says "A refused binding answers `NOT_HELD` for that namespace's tasks", which suggests one decision per namespace.
- **Node kind.** `steps.activity` exists (`migrations/20261009000100_execution.up.sql:134`), but "node kind", which the pre-read is said to read, is not a column of `steps`.
- **Fix.** Decide coverage per row with `first_uncovered` on `steps.activity`. Answer `NOT_HELD` for managed and non-activity rows, which are never marked anyway. Add a parity test against `ensure_bound` over patterns, activity, node kind and role.

### Minor 3. Step 4 holds row locks while it waits, so "no other run's lock is involved" is false; and where `lock_timeout` is set is unclear (design reading)

- **Rows already locked.** While the statement waits up to R / 2 = 3 s for a busy run's row, it keeps the locks on the rows it has already locked. The writers of those rows take their run lock first: the lost-lease pass (`FOR UPDATE` re-read under the run lock), a cancellation, and a scoped settle. They then hold their run lock while they wait on this call, which stalls those runs' completions. On timeout the host repeats the call and the stall comes back at every backoff.
- **Advisory waits.** As written, `SET LOCAL lock_timeout` belongs to step 4. The advisory waits of step 3 behind a revocation are then bounded only by the call timeout (30 s by default). Meanwhile this call holds namespace A's shared locks, which delays A's secret creation (`lock_secrets` is exclusive).
- **Fix.**
  - Set `lock_timeout` and `statement_timeout` at the start of the transaction.
  - In step 4, lock with `SELECT ... ORDER BY <primary key> FOR UPDATE SKIP LOCKED` and update the rows returned. Commit what was written. If any row was skipped, end the call with a retryable status: on the repeat, the written rows answer `ACKNOWLEDGED` as duplicates.
  - Correct the failure row.

### Minor 4. An acknowledgement that carries a withdrawn token is "a call other than a hand-back", so conforming hosts produce false "a handler may have run" audits (design reading)

- **Where.** Decision 12 and Decision 006-053 say "any call other than a hand-back". The observability kinds list `report`, `capability` and `hand_back`, so they disagree with each other.
- **Why it happens.** A host repeats `AcknowledgeTasks` until it gives up. The give-up point is "time left at receipt minus R", clamped to 120 s. With a host clock behind the server by more than one period of the lost-lease pass, the repeats outlast the withdrawal (expiry plus the pass period). A conforming host then sends acknowledgements with the withdrawn token. Nothing runs, because the answer is `NOT_HELD`.
- **Fix.** Exempt `AcknowledgeTasks` from the rule for calls after a withdrawal. Count it as kind `acknowledgement`, with no alert. Add a test with a host clock 60 s behind.

### Minor 5. The registry dependency checks "recorded", not "recorded for every kind", and the mark check of the `lease-withdrawal-v1` removal is a check-then-act (verified)

- **Kinds.** `withdraws_leases()` requires `lease-withdrawal-v1` for all kinds (`crates/application/src/worker/withdrawal.rs:121-125`; `crates/application/src/instances.rs:383-396`). A requirement recorded by hand for only `EXECUTION_ENGINE` passes the planned trigger. Leases are then marked and never withdrawn, so they lapse into review. The marking condition names only the delivery requirement.
- **Race.** The removal command counts marks under the epoch lock, but claims never read the epoch. An engine that has not yet seen the delivery-acknowledgement removal (Gap 006-044) can mark a lease right after the count. The hand SQL of `docs/runbooks/instance-registry.md` §5 skips the count altogether.
- **Fix.**
  - The trigger requires `ARRAY['CONTROL_PLANE','EXECUTION_ENGINE'] <@ kinds` of `lease-withdrawal-v1`.
  - An engine marks only while both requirements hold for all kinds.
  - The removal refuses until the last `REMOVED` row of `delivery-acknowledgement-v1` in `platform_requirement_history` is older than two registry refresh periods, and only then counts marks.
  - The runbook names the hazard of doing this by hand.

### Minor 6. Decision 15's interaction with the bound of three is stated, but its storage and order are not, and Decision 006-052 is silent on attempts (design reading)

- **Ordering.** Every withdrawal of a marked lease increments both the run of withdrawals and the per-attempt cap. The design does not say whether the cap is checked before the mark's provisional test, or which one wins when both apply.
- **Attempts.** The run of three is cleared "only by an acknowledgement and by a reset". After three withdrawals the step is never marked again, so no acknowledgement can clear it. Its protection is lost for every later attempt, and the design does not say so.
- **Fix.** Write out the order, decision by decision, in the pure function of Decision 006-044. State or decide whether consuming an attempt clears the run. Add the sequence "three silences, then unmarked hand-backs up to the cap, then ordinary failure" to Task 006-050.

---

## Nit

1. **Timing class.** One call probes 64 run ids for existence in another tenant, and the awaited insert of a record marks a hit. That is 64 times the rate of `Heartbeat`. Write the cross-tenant record off the answer path (spawned, as the denial gate is), so a foreign run and a missing one differ by nothing a caller can measure.
2. **`CANCELLED` is narrower than `attempt_authority_fence`.** The fence also refuses a run with a stop reason. The acknowledgement reads only the step's `cancel_requested` and never reads the run. Either read `runs.stop_reason` in the pre-read, or state the difference.
3. **The "keep" rule is stated for `lease_acknowledged_at` only.** State it for all five columns. The counter and the withdrawn hash are written only relative to their current value, under the row lock. No statement built from restored state lists them (compare `execution.rs:7039-7072`).
4. **Debug.** `HeartbeatRequest`, `CompleteStepRequest`, `FailStepRequest`, `ResolveSecretsRequest` and `WriteBlobHeader` carry `lease_token` and still have the generated `Debug` (`crates/api-grpc/build.rs:17-25`). This predates the design, but the descriptor walker of Task 006-052 can close it cheaply: fail when any message with a `lease_token` field is not in `skip_debug`.
5. **Admission wording.** The class `Lease` also covers `ResolveSecrets` and `ResolveDatabase`, and `Transfer` shares its limit (`crates/api-grpc/src/limits.rs:500-552`). So the half of the reserve is not "for `Heartbeat`, `CompleteStep` and `FailStep`" alone. At `max` of 3 or less, half of the integer reserve is 0. Write the limit as a formula, test `max` from 1 to 8, and classify `AppendStepLogs` (1.7) explicitly. Today it falls to `Discovery`.

---

## Classes checked with no new finding

- **The tenant boundary in SQL.** The pre-read is by primary key within the caller's tenant (`steps_pkey`). No foreign row is locked. `NOT_HELD` is the same answer for foreign, missing, forged and stale tokens. `CANCELLED` and `TASK_NOT_ACKNOWLEDGED` reach only a token holder, and only after the binding check.
- **Lock order with the existing writers.**
  - A binding replacement takes the binding key exclusively, then the profile key, then sessions `FOR UPDATE`, then the step update (`bindings.rs:85-261`). The acknowledgement takes step rows only after all of its advisory locks, so one of the two waits for the other and no cycle forms.
  - Withdrawals and heartbeats take the run lock and then the row. The acknowledgement takes no run lock.
  - New shared advisory requests queue behind a waiting exclusive one. That is PostgreSQL's lock manager, not something in this code, so a stream of acknowledgements does not starve a revocation.
  - The membership `FOR SHARE` is the same that `Heartbeat` takes, once per call.
- **Triggers.** The step update fires `steps_quota_delta` and `steps_quota_flush`. Neither takes a tenant lock for these columns, because a delta is queued only when `child_input` or `correlation_key` changes (`migrations/20261024000000_durable_quotas.up.sql:206-213`, `242-246`).
- **Stolen or forged tokens.**
  - A forged token costs one indexed read and no lock.
  - A stolen token can acknowledge a lease, turning a lost delivery into a review, or hand it back. `Heartbeat` and `FailStep` already allow as much.
  - With the renewal gone, the 64 rows of a call are each written at most once per lease.
- **Audit volume.** At most one binding denial and one cross-tenant record per call, under the per-subject budget and the 32-write gate. Violations are once per step and generation (a violating heartbeat leaves the provisional state at once). Calls after a withdrawal are once per step and hash.
- **Runtime role and migration.** `GRANT SELECT, INSERT, UPDATE ON runs, steps TO sagat_app` (`execution.up.sql:273`). No column grant or row-level security has been added on `steps` since. The membership lock goes through a `SECURITY DEFINER` function with a pinned search path. The guard stays an invoker function, and the operator functions stay non-executable by `sagat_app` (`instance_registry.up.sql:385-409`).
- **Injection and parsing.** Bound arrays; 1 to 64 tasks; distinct ids; `step_id` checked as `verify` checks it; no dynamic SQL.
- **Rollback.** Wait until no mark counts. The floor decodes `NOT_ACKNOWLEDGED`. The `down` migration refuses while the history names the capability, consistent with `20270208000000_lease_withdrawal.down.sql`.

Nothing can be run yet. Once the build exists, run these:
- `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_durable_execution`
- the fault suite of Task 006-051
- the tests named in Majors 1 and 2

Files:
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/scoped_reports.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/bindings.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/quotas.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.down.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261126000000_authority_read_locks.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/build.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/denial.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md`
