# Security review, round 2: delivery acknowledgement (ADR-0099, worker protocol 1.8)

I compared the rewritten design with the code in `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack` and with the ADR-0098 build in `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases`. There is no implementation of ADR-0099 yet. "Verified" below means I checked the code at the cited lines. "Design reading" means the defect is in the text alone.

**Verdict:** no Blocker, 3 Major, 8 Minor, 6 Nit. The four Majors of round 1 that were about the call's mechanics are now fixed in the text: forged tokens no longer cause lock or audit amplification, scoped instances are designed, the mark is bound to the lease's generation, and the trust assumption is stated for the owner. The new defects are all at the edges of the new call:
- what the call lets a token holder renew;
- whose authority it checks, and in which namespace;
- the violation evidence, which fires on hand-backs that the protocol itself requires.

---

## Round 1 status (my report, and the findings of other reports that fall in my domain)

| Round-1 finding | Status | Reason |
|---|---|---|
| Security Major 1: an acknowledged lease receives secrets and can still be withdrawn as "nothing ran" | **Partly resolved** | Resolved for leases whose mark counts: D006-045 makes an acknowledged lease an attempt, and decision 9 refuses capabilities before the acknowledgement. Not resolved for unmarked leases, including leases of a 1.8 session after the bound of three or after `acknowledgement_kept` is cleared. Those still receive capabilities while `LEASED`, and the ADR-0098 build withdraws any `LEASED` step on `TASK_NOT_STARTED` (`sagat-wt-withdraw-leases/crates/application/src/worker.rs:2430-2436`, verified). The new call also makes that window as long as the attempt deadline allows (new Major 2). The round-1 fix, a first-capability-use mark, was not adopted. |
| Security Major 2: scoped instances | **Resolved** | D006-054 names the path primitives: the pre-read and the write use the full address, a variant of `live_path_fence`, the scoped withdrawal, and a scoped row in every task's evidence. `begin_path` and `begin_current_path` take the chain lock and then the run revision lock (`crates/infra-postgres/src/execution.rs:850-861`, `895-906`, verified), as the design assumes. |
| Security Major 3: the mark is not tied to its lease | **Resolved** | The mark counts only while `lease_offered_generation = generation`. Every lease raises the generation by one (`crates/domain/src/execution/state.rs:1497-1502`), and a step state starts at generation 0 only when its instance is first inserted (`state.rs:2738-2766`, `2900-2906`, `3359-3365`: each refuses an existing key). Verified. |
| Security Major 4: 64 run locks and full state loads per call before any token check; audits outside the bound | **Resolved** (one residue) | Unlocked pre-read on `steps_pkey` (`migrations/20261027000000_scoped_instance_identity.up.sql:30-31`, verified), a lock-only open, never two run locks at once, at most one audit per call, and a 64-run budget case. The residue: a caller who holds real tokens still gets up to 64 lock-and-write transactions per call for duplicate acknowledgements (new Major 2). The audit mechanism as written cannot be built (new Minor 1). |
| Security Major 5: the trust assumption | **Partly resolved** | Fixed: the assumption is in ADR decision 13 for the owner, the FR-037-006 clarification is conditional, G006-042 is widened, and the withdrawn token hash is kept. Still wrong: the violation evidence fires on hand-backs the protocol requires (new Major 1); it cannot be attributed to a session (new Minor 2); and it does not cover `ATTEMPT_TIMEOUT_BEFORE_START` (new Minor 8). |
| Security Minor 1: call-level permission check and audit | Partly resolved | `worker:execute` is checked first (step 1). The "bounded denial gate" path does not exist for this case (new Minor 1). |
| Security Minor 2: a separate fence, evaluated after the token matched | Resolved | D006-050 and T006-053. |
| Security Minor 3: where the requirement dependency is enforced | Partly resolved | The guard is now in the trigger, but the existing function is described wrongly and the guard does not serialise against a hand edit (new Minor 5). |
| Security Minor 4: no bound on withdrawals | Partly resolved | The bound of three exists. It is defeated by "cleared by any report" (new Minor 3). |
| Security Minor 5: rollback | Resolved | There is a floor and the `down` migration refuses while any mark counts. Residue in Nit 1. |
| Security Minor 6: `Debug` of the element that carries the token | Resolved | `TaskAcknowledgement` is in `skip_debug`, and T006-043 has a token canary on the request and on the element. |
| Security Nits 1 to 4 (expiry, attempt and generation, attempt of at least 1, `worker_id`, observability, per-element bound) | Resolved | Wire bullets and step 3. |
| Architecture Minor 5: forged tokens take 64 run locks | Resolved | Same reason as Major 4. |
| Correctness Minor 4: folding refusals into a verdict loses an audit | Partly resolved | Same reason as Minor 1. |
| Correctness Major 3: hand-back with declared secrets after a cancellation | Resolved in design | The `CANCELLED` path (D006-051) and the request to the ADR-0098 build (D006-059). Ordering defect in new Minor 7, wording in Nit 4. |
| Architecture Major 7(b): no fitness function for the capability refusal | Resolved, with a gap | The descriptor-driven test. Gap in new Minor 6. |

---

## Major

### Major 1. Hand-backs the protocol requires are recorded as protocol violations and as "a handler ran after withdrawal" (design reading, against the code paths)

**Where.**
- ADR-0099 decision 1 (line 43: "MUST NOT make any other call for it, before ... `ACKNOWLEDGED`") and decision 13 (line 115).
- D006-053 (`research.md:391`).
- `stream-completion.md`: host item 1 (line 438: the excess and the leases received while stopping are handed back and not acknowledged), item 4 (line 447: a lease given up "is handed back"), item 5 (a refused call hands the leases back), and the failure rows at lines 485, 499 and 500.
- `sdks/SPEC.md:507`: `stop()` "hands back every lease still waiting for its acknowledgement".

**What is wrong.**
- Every hand-back is `FailStep(TASK_NOT_STARTED)` on a lease that is still provisional. D006-053 and row 499 make any `FailStep` on a provisional lease a violation: it is counted and it clears `acknowledgement_kept`.
- Decision 1 forbids "any other call" before the verdict, so the normative rule contradicts the procedure it governs.
- A host gives a lease up at its monotonic estimate of the expiry, clamped up to 120 s. The server withdraws at the expiry plus one pass period. So the hand-back after a give-up often arrives after the `NOT_ACKNOWLEDGED` withdrawal, carrying the withdrawn token. Row 500 then counts it, audits it and alerts: "a handler ran on a lease the journal says never ran". `SagatReportsAfterWithdrawal` fires on any increase.

**Failing scenarios.**
1. A conforming host that stops during a rolling deploy, receives a frame larger than its credit, or falls back on `UNIMPLEMENTED` loses `acknowledgement_kept`. All its later leases are unmarked, so a frame lost on the way to it goes to review again (G006-001 reopens), and nothing says so.
2. A conforming host whose acknowledgement never reached the server, with a clock behind, gives up after the withdrawal. The server writes an audit record and pages the operator for a violation that did not happen.

The evidence the owner is asked to rely on, in place of verification, becomes noise, and a real violation is lost in it.

**Fix.**
- Amend decision 1: "... any other call for it except the hand-back of §6.2 (`FailStep` with `TASK_NOT_STARTED`)".
- Exclude `FailStep` with code `TASK_NOT_STARTED` from both evidence rules, the violation rule and the report-after-withdrawal rule. A hand-back says nothing ran, which agrees with the withdrawal.
- Add three tests: a hand-back on a provisional lease leaves `acknowledgement_kept` true; a hand-back carrying a withdrawn token is not counted as a report after withdrawal; and a heartbeat or report carrying a withdrawn token is counted.

### Major 2. `AcknowledgeTasks` is a renewal channel for any lease, and it never starts the step (verified for the code it would interact with)

**Where.**
- `stream-completion.md:360` (the conditional write "renews the lease as a heartbeat renews it", for verdicts `acknowledged`, `duplicate` and `renewed`).
- The row "A duplicate acknowledgement ... only the renewal is written".
- Every engine "serves `AcknowledgeTasks` whatever it knows". Nothing restricts the call to sessions accepted at 1.8, or to leases whose mark counts.

**What is wrong.**
- Today a renewal goes through `heartbeat`. Its first beat applies `StepStarted` and moves the step to `RUNNING` (`crates/domain/src/execution/command.rs:506-518`; path variant at `568-589`).
- The acknowledgement renews with no transition, so a step renewed only through it stays `LEASED` for its whole life, up to `attempt_deadline_at`, and without limit when the step has none, because `least()` ignores a null.
- The ADR-0098 build withdraws any `LEASED` step whose attempt and generation match on `TASK_NOT_STARTED`, with no attempt consumed and no review (`sagat-wt-withdraw-leases/crates/application/src/worker.rs:2430-2455`). D006-045 protects only leases whose mark counts.

**Failing scenarios.**
1. A worker holds an unmarked lease (any session below 1.8, or a 1.8 session after the bound of three or after `acknowledgement_kept` was cleared). It:
   - calls `AcknowledgeTasks` every few seconds instead of `Heartbeat`, and gets verdict `renewed`;
   - calls `ResolveSecrets`, which is served because the lease is not provisional and commits a `SecretResolved` audit (`crates/application/src/worker/secrets.rs:208-217`);
   - runs for minutes, then sends `TASK_NOT_STARTED`.

   The journal records "no handler ran", the attempt is given back, and the next host resolves the same secrets with the same attempt number. That is round-1 Major 1, which below 1.8 was limited to one time-to-live, now stretched to the attempt deadline. D006-045's sentence "Capabilities are served only after the acknowledgement, and from then on the lease is treated as started" is false for these leases.
2. A worker holding 64 leases on 64 runs sends duplicate acknowledgements at the per-subject budget: 20 calls a second times 64 runs is 1,280 run-lock transactions and 1,280 `steps` updates a second. Heartbeat allows 20. Each one delays the completions of the other workers on those runs.

**Fix.**
- Renew only on the provisional-to-acknowledged transition.
- Answer `duplicate` (the mark counts and `lease_acknowledged_at` is set, which is terminal for the lease) and `renewed` (no mark counts) from the pre-read, with the current expiry, and with no lock and no write.
- In the ADR-0098 build, either make the `NOT_STARTED` withdrawal require that no capability was served to the attempt (the round-1 fix: one statement in the audit transactions that already exist), or record the unmarked-lease residue as a named gap of ADR-0098 that this design says it widens unless the renewal is removed.
- Add a test: an unmarked lease acknowledged twice is not renewed and takes no run lock.

### Major 3. The acknowledgement decides the binding on data that is not the run's, and without the locks a revocation waits for (verified)

**Where.** `stream-completion.md:353` ("Read the caller's bound patterns and authorization once") and step 3 ("against the caller's bound patterns read once per call"); D006-043.

**What is wrong.**
- **The data is per namespace.** `bound_patterns` is scoped to the run's namespace and takes a transaction-scoped shared advisory lock, `binding:{tenant}:{namespace}:{subject}` (`crates/infra-postgres/src/execution.rs:4805-4825`, `crates/infra-postgres/src/bindings.rs:15-27`). A replacement takes that same key exclusively (`bindings.rs:88-92`). The code's own comment says the observed authority is kept "through its audit commit".
- **Membership is not re-read.** `ensure_bound` also calls `current_worker_authorized`, which re-reads the membership under the row lock a revocation waits for and refuses a database-runner role (`execution.rs:1017-1044`; `crates/application/src/worker.rs:2715-2740`). The design names neither this check nor the namespace.
- **"Once per call" cannot work across transactions.** A call spans up to 64 run transactions, which may be in different namespaces. A transaction-scoped lock cannot be held across them, and one set of patterns cannot be right for two namespaces. Steps carry `namespace_id` (`migrations/20261009000100_execution.up.sql:117`), so the pre-read can return it.

**Failing scenario.** Subject S is bound to `*` in namespace A. Its binding for `pay-*` in namespace B is removed after S claimed a B lease. `Heartbeat` on that lease now answers `NotBound`. S sends one `AcknowledgeTasks` call listing an A task first and the B task second. The patterns are read once, in A, so the B lease is acknowledged and renewed. Together with Major 2, S keeps the B step leased until its attempt deadline and blocks it from going to a bound worker. That defeats the documented purpose of evaluating the binding at use: "a binding that is removed stops a worker that already holds a lease from renewing it" (`worker.rs:2712-2714`).

**Fix.**
- Per run, inside that run's transaction and after the run lock, apply the same checks as `ensure_bound`, in the order `Heartbeat` takes them: the namespace secrets share lock, the membership lock, then the binding lock and the patterns of the run's namespace.
- Caching is allowed only inside one transaction.
- Recount the "about 6 more statements" of D006-057 and the `ACKNOWLEDGEMENT` budget line, because this adds three or four statements per run.
- Add two tests: a call mixing an A task and a B task after revocation in B answers `NOT_HELD` for B; and a membership revocation held open across the call is waited for.

---

## Minor

### Minor 1. "Through the bounded denial gate" cannot be built as written (verified)

**Where.** `stream-completion.md` steps 2 and 3; D006-043.

**What is wrong.**
- The gate writes a record only when the call's status is `PERMISSION_DENIED` (`crates/api-grpc/src/service.rs:367-385`), and only for an `AccessDenial`, which is `MissingPermission` or `Refused` (`crates/application/src/error.rs:305-311`).
- An acknowledgement answers OK with per-task verdicts, so a binding refusal folded into `NOT_HELD` never reaches it.
- `CrossTenantAccessDenied` is not an `AccessDenial` at all. Today it is written directly by `missing_task_error` through `RunStore::record_denial` (`worker.rs:2564-2586`).
- The gate's volume argument relies on the per-subject budget (`crates/api-http/src/denial.rs:18-20`). That still holds at one record per call, but only if the mechanism is specified.

**Fix.** Specify that:
- the application returns at most one `AccessDenial` (the binding refusal) in its result;
- the adapter spawns it through `spawn_audit_access_denied` whatever the call status;
- the cross-tenant record is written as `missing_task_error` writes it, at most once per call.

Test both counts with a call of 64 refusals.

### Minor 2. The violation flag cannot be attributed to a session (verified: no session reference on `steps`)

**Where.** D006-053; the `worker_sessions.acknowledgement_kept` row (`stream-completion.md`, the mark table).

**What is wrong.** The step row records no claiming session (`migrations/20261009000100_execution.up.sql:111-141`). Report requests carry `worker_id`, which is "validated and ignored", and sessions are unique per subject and worker name (`execution.up.sql:205`). The design does not say which session is flagged:
- If it is looked up by `worker_id`, any host of the same identity can clear a sibling session's flag.
- If it is looked up by subject, one violation clears all of them.
- If it is looked up before the token matches, a caller without the token can clear it.

Registering again under the same worker name replaces the row, which may reset the flag to `true`.

**Fix.**
- Store the claiming session id with the mark, in the claim statement.
- Clear that session's flag only after the token, binding, attempt and generation have matched.
- State whether registering again keeps the flag.

### Minor 3. The bound of three is reset by a hand-back, so a step can loop for ever (design reading)

**Where.** `stream-completion.md:332` ("cleared by an acknowledgement and by any report"); D006-052.

**What is wrong.** A worker that alternates "never acknowledge" (a `NOT_ACKNOWLEDGED` withdrawal, count 1) with "hand back with `TASK_NOT_STARTED`" (a `NOT_STARTED` withdrawal, and the hand-back is a report, so the count goes back to 0) keeps a step in withdraw-and-lease-again for ever. No attempt is consumed, no review opens, and the bound never trips. That is the loop round-1 Minor 4 asked to close.

**Fix.** Clear the counter only on an acknowledgement or on a report other than `TASK_NOT_STARTED`, and add that sequence as a test of T006-051.

### Minor 4. The report-after-withdrawal evidence writes one audit row per call, and catches only some uses of a withdrawn token (design reading)

**Where.** The failure row at `stream-completion.md:500`; D006-053.

**What is wrong.**
- **Flooding.** A host that never acknowledges collects withdrawn tokens. Every heartbeat it sends with one writes an audit row (and costs a full `begin`, as any stale heartbeat does today), up to 20 a second per subject.
- **Narrow coverage.** Only reports and heartbeats are checked. A capability call with a withdrawn token is stronger evidence and is not covered.
- **Lossy by design.** Only the last withdrawn hash is kept per step.

**Fix.**
- Record an identical event once per step and withdrawn hash, the way `lake_read_denial` does (`worker.rs:2157-2181`), and count every call.
- Include capability calls.
- State the last-hash-only limit in G006-042.

### Minor 5. The registry guard is described wrongly and does not serialise against a hand edit (verified)

**Where.** `stream-completion.md:311-316`; T006-046.

**What is wrong.**
- **Not as described.** `platform_requirements_guard` is neither `SECURITY DEFINER` nor pinned to a `search_path` (`migrations/20270113000000_instance_registry.up.sql:98-114`). Its "siblings" with those properties are the process and operator functions. Only the owner can write the table, so invoker rights are correct, but the new queries must be schema-qualified (as `platform_requirements_epoch_moved` is, line 139).
- **A race.** Both functions and the runbook take the epoch `FOR UPDATE` first (lines 323 and 364; `docs/runbooks/instance-registry.md:157-161`). A hand edit that sets the flag and deletes `lease-withdrawal-v1` before it updates the epoch reads, under READ COMMITTED, a `delivery-acknowledgement-v1` insert that has not committed yet. Both then commit and the dependency is broken.
- **Closed sets.** The capability is a closed set in three `CHECK` constraints (lines 42-44, 54-55 and 83-84). Forgetting them fails closed but is not planned.

**Fix.**
- Keep the guard as an invoker function with `public.`-qualified names.
- Have it take `SELECT ... FROM public.platform_requirement_epoch FOR UPDATE` before it reads; this is a no-op for the functions, which already hold the row.
- Extend the three `CHECK` constraints in T006-044.

### Minor 6. The descriptor-driven test must find tokens nested inside messages, and must fail rather than skip (verified for the proto)

**Where.** D006-050; T006-053.

**What is wrong.** `WriteBlob` is client-streaming, and its token is inside `WriteBlobHeader` within the request's oneof (`specs/001-platform-foundation/contracts/worker.proto:161-169`). A scan of top-level request fields misses it. A test that has no request builder for a newly found method might skip it.

**Fix.** Walk request types transitively through nested and oneof fields, and fail when a lease-token method outside the four exemptions has no case.

### Minor 7. The `CANCELLED` path runs after the write that acknowledges (design reading)

**Where.** `stream-completion.md:360-366`.

**What is wrong.** Step 3's write sets `lease_acknowledged_at` and then step 4 decides `CANCELLED`. But ADR-0098's withdrawals "also require the lease not to be acknowledged" (withdrawal section). Taken literally, the `NOT_STARTED` withdrawal refuses the lease step 3 just acknowledged. The host, told `CANCELLED`, drops it, and the lease lapses into a review. The binding check must also come before that withdrawal.

**Fix.** Exclude "provisional and `cancel_requested`" from the acknowledging write, and send that match to the `CANCELLED` path after the binding check. Test it root and scoped, with declared secrets.

### Minor 8. `ATTEMPT_TIMEOUT_BEFORE_START` also rests on the trust assumption, with a worse failure mode (design reading)

**Where.** ADR decision 7, decision 13 (line 115), G006-042, the FR-037-006 clarification, and `stream-completion.md:386`.

**What is wrong.** A provisional lease past its attempt deadline is recorded as a known retryable failure on the strength of the host's promise. A violating host that did run it has its work retried with the next attempt number, so with a different idempotency key, and the provider cannot deduplicate. Decision 13, G006-042 and the 037 clarification cover only the withdrawal.

**Fix.** Name this outcome in decision 13, in G006-042 and in the clarification of FR-037-006.

---

## Nit

1. **Rollback order** (`stream-completion.md:472`). Step 2 waits only for `worker_leases_provisional = 0`, so leases that are acknowledged and still live meet the older binary's `NOT_STARTED` rule, which would withdraw an acknowledged `LEASED` lease. Wait for the condition the `down` migration checks, "no mark counts", before rolling the binary back.
2. **The removal command after a crash.** If the command dies between its two transactions, running it again answers `NOT_RECORDED`. On that path it must still expire the 1.8 sessions and print what it expired. Record the count in `platform_requirement_history` or in the operator output that is kept.
3. **SDK logs.** Add a token canary on the SDK side for `AcknowledgeTasks` in T006-058 to T006-063 (Java `toString`, Python `repr`, Go `%v` of the request).
4. **The relaxation asked of ADR-0098** (D006-059 (1), `research.md:423`). It should say that only the `cancel_requested` and stop-reason conditions of `attempt_authority_fence` are relaxed. The token, binding, attempt, generation and expiry checks stay (`sagat-wt-withdraw-leases/.../worker.rs:2385-2386` already verifies the token and binding first).
5. **Timing class.** Run the cross-tenant existence statement for every unmatched run id of a call, whatever exists locally, and write the record off the answer path. Then a foreign run and a missing one differ by nothing a caller can measure. Today's `Heartbeat` already differs by one awaited insert; say so if you keep it.
6. **The pre-read is built from bound arrays** (`unnest`), never by interpolating the 64 addresses. Also state that `step_id`, when present, is checked as `verify` checks it.

---

## Classes I checked and found nothing new

- **The tenant boundary in SQL.** The pre-read, the chain lookup, the chain lock and the run lock are all filtered by the caller's tenant (`execution.rs:469-475`, `RUN_FOR_UPDATE`). A foreign run is never locked, and `NOT_HELD` is the same answer for foreign, missing and wrong-token tasks. `TASK_NOT_ACKNOWLEDGED` and `cancel_requested` reach only a token holder.
- **Forged tokens.** With the pre-read, a call costs one indexed statement and no lock. Admission class `Lease` (`crates/api-grpc/src/limits.rs:499-509`) is acceptable at that cost.
- **The migration and the runtime role.** `sagat_app` has table-level `SELECT, INSERT, UPDATE` on `steps` and `SELECT, INSERT, UPDATE, DELETE` on `worker_sessions` (`execution.up.sql:273`, `277`). No column-level grant or row-level security exists on either table, so the new columns need no grant. The operator functions are not executable by `sagat_app` (`instance_registry.up.sql:385-409`).
- **Debug and spans.** `TaskRef` redacts the token (`worker.rs:489-494`). The request and the element go into `skip_debug`. Metric labels are closed sets.
- **Injection and parsing.** No dynamic SQL is proposed. Input is bounded: 64 tasks, distinct ids, attempt of at least 1, path depth at most 8.
- **The exactness argument.** Both writers hold the run lock, and the acknowledgement refuses a provisional lease past its expiry.

Nothing needs to be run for this round. When the build exists, run `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_durable_execution` and the fault suite of T006-052.

Files cited:
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/secrets.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/scoped.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/error.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/command.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/state.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/bindings.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/build.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/denial.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs`
