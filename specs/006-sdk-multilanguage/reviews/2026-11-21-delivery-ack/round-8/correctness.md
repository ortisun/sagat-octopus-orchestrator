**Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 8**

## Verdict

Round 8 removes the engine breaker and the cut-off command. In their place it puts one server-side rule: a lease is flagged at most once per attempt (Decision 006-066). That rule is the right shape, and it closes the cycling of my round-7 Major 1 without the observation problems the breaker had.

Three things still break the guarantees as round 8 now words them:

- **The host's new evidence clause accepts an outage.** "One of its own hand-backs was answered after its give-up" is also true after an outage that ends while those hand-backs are being retried or queued. So the host falls back after an ordinary engine outage (Major 1).
- **The column the bound rests on has no stated retention rule, and its value is ambiguous.** The rule written for "the marks" allows clearing it with the lease, which is the pattern the floor build's statements already use on every write of a row that holds no lease. A builder who follows that pattern silently turns the bound off (Major 2).
- **"No failure of the acknowledgement's path can spend an attempt" is false.** A committed acknowledgement whose answer is lost, or a slow one, ends in a hand-back of an acknowledged lease, which is an ordinary failure that spends the attempt (Major 3).

**Counts:** 0 Blockers, 3 Majors, 5 Minors, 9 Nits.

Nothing was built or run. Claims about the server build come from reading `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases`. Claims about PostgreSQL and tonic behaviour are from their source as I know it, and are marked as such.

## Status of round-7 findings

### My report

| Round 7 | Status | Reason |
|---|---|---|
| Major 1: a failure of `AcknowledgeTasks` alone can spend attempts by cycling | **Resolved for cycling, provided the column is kept** | Decision 006-066 removes (a), (b) and (d), and the cap side of (c), without observing anything. It holds only if `provisional_withdrawn_attempt` survives until the next claim and carries the right value (Major 2). The host trigger of (c) is rewritten ("after its give-up", no time floor, the run defined), but the new clause accepts an outage (Major 1). A different route to a spent attempt remains (Major 3). |
| Minor 1: the gate opens before the beat decides it | **Resolved in text** | The beat and the decision under the returned epoch both have to succeed; the window is measured from the start of the beat; the gate is checked again at the hand-over. Residue, confirmed in code: the heartbeat reads the epoch without a lock (Minor 1 below). |
| Minor 2: the liveness rule refuses every worker call | **Resolved** | Narrowed to capability calls. |
| Minor 3: four definitions of "the server's reason" | **Resolved** | One envelope rule everywhere; the split happens after any failure that carries the envelope. Residue: a host-side deadline caused by the engine's own queue carries no envelope (Minor 3). |
| Minor 4 and Minor 5: the breaker | **Resolved by removal** | |
| Minor 6: timeouts set once against per statement | **Resolved** | Set once to 1 s, deadline checked before each statement, the call ends by 4 s. Residue: the heartbeat derivation still assumes a 3 s call (Minor 2). |
| Minor 7: §6.2 sentence order | **Resolved** | |
| Minor 8 and Minor 9: `end-lease-marks` and its table | **Resolved by removal** | |
| Nits 1, 4 and 8 | **Moot** | Breaker and `fsync` condition removed. |
| Nits 2, 3, 5, 6 and 9 | **Resolved** | |
| Nit 7: the host's count | **Half resolved** | A call that does not qualify "neither counts nor breaks the run". Still not stated: whether an `ACKNOWLEDGED` that a 2 s rule turns into `CANCELLED` resets the run (Nit 5). |

### Other round-7 reports, in my domain

| Finding | Status |
|---|---|
| Security Major 1: a flood holds the breaker closed | Resolved by removing the breaker and adding the bound, subject to Major 2. |
| Architecture Major 1: where the liveness rule lives, the precondition, its scope, its boundary | Resolved: request (7) is a hard precondition, narrowed to capability calls, measured from the start of the beat. Residue: Minor 1. |
| Design Major 1: the third trigger and the cap | Resolved by the bound. The rewritten trigger has a new hole (Major 1). |
| Design Major 2: the breaker | Resolved by removal. |
| Design Major 3: the liveness rule stopped heartbeats and reports | Resolved by the narrowing. |

## Blocker

None.

## Major

**Major 1. The host's evidence clause counts a call ended by an outage that heals while its hand-backs are still being sent. The host then falls back after an ordinary engine outage.**

Confidence: confirmed in the documents (the rule, the retry rule of §5 and the eight-at-once limit); how often it happens is plausible.

- **Where:**
  - `sdks/SPEC.md:383-386`
  - `stream-completion.md:964-972` of the diff (host item 5), which justifies the clause with "which tells a path that swallows this one call from an outage, in which the hand-backs fail too"
  - Decision 006-065 (`research.md:713`)
- **Scenario** (default 30 s time-to-live, one engine or every engine behind the balancer):
  1. Three flagged frames are read at t = 0. Every engine becomes unreachable at t = 1 s (a deploy, a node failure, a partition) until t = 27 s.
  2. Every acknowledgement attempt ends with a transport failure, which carries no envelope.
  3. Give-up comes at about 24 s. The hand-backs fail, then retry under §5: up to five times, 200 ms doubling, scaled by a random factor in [0.5, 1]. The last try lands at about 27 to 30 s.
  4. Under the cap of eight hand-backs on their way at once (`stream-completion.md:370`), the hand-backs of a 64-lease frame are queued and sent even later.
  5. Hand-backs are answered after t = 27 s: "one of its own hand-backs was answered after its give-up". All three calls count, and the host registers declining for 10 minutes.
  6. A second outage within the period doubles that, up to an hour. Every host with three calls in flight does the same, and `SagatWorkersDecliningAcknowledgement` fires fleet-wide.
  7. With a partition (packets dropped rather than refused), each hand-back try waits out its own deadline, so the retries span much longer and the window is wider.
- **Why it matters:** the decision says the host declines "only on evidence of its own path". Here the server's outage switches the protection off. This is round 6's security Major 1 in another form.
- **Why the tests miss it:** Task 006-058's case "a 30 s `UNAVAILABLE` outage … with three flagged frames in flight does not fall back" depends on timing.
  - If the harness's time-to-live puts the give-up after the outage ends, the calls get verdicts and the case passes for the wrong reason.
  - At 30 s it flakes on the jitter of the hand-back retries.
- **Smallest fix:** evidence must overlap the failure, not follow it.
  - After a hand-back of a given-up lease is answered `OK`, the host sends one more `AcknowledgeTasks` for that lease. A working path answers it `NOT_HELD`, because the token no longer matches.
  - The call counts only if that probe also ends with no answer, or with a status that has no envelope. A path that swallows this call swallows the probe too; a healed outage answers it. This costs one call per given-up call.
  - Alternatively, accept only a hand-back answered on its first try, sent at the give-up instant, and drop queued hand-backs from the evidence. This is weaker: an outage that ends in the last backoff before the give-up still counts.
  - Tests: an outage of every call that ends 3 s after the give-up, with three flagged frames, at 18 s and 30 s time-to-live: no fallback. The swallowed-call case still falls back.

**Major 2. The bound rests on `steps.provisional_withdrawn_attempt`, whose retention and value are not specified. The rule written for "the marks" permits the form that loses it.**

Confidence: plausible; the text permits it, and the floor build's own statement pattern produces it.

- **Where:**
  - `stream-completion.md:421-423` (the marks section header: "Columns of `steps`, all written for a lease's generation")
  - the marks table (`stream-completion.md:431`)
  - `stream-completion.md:474-478` (the allowed forms: "the engine's other statements keep them (`ELSE` the column) or clear them with the lease")
  - `stream-completion.md:563` and `:566` ("set to the attempt … the attempt set back by one")
  - Decision 006-042 (`research.md:454`), Decision 006-066 (`research.md:727`), Task 006-047
- **(a) Retention.**
  - The column is listed among the marks, and the marks are "written for a lease's generation" and cleared "with the lease". This column is bound to an attempt and must outlive the lease it records.
  - Every lease column in the floor build's root and scoped updates uses `CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL …` (`crates/infra-postgres/src/execution.rs:7394-7407`, `:7066-7079`). That nulls the column on every write of a row that holds no lease, not only at the lease's end.
  - A `READY` step is rewritten between the withdrawal and the next claim:
    - by a re-hint (`effects.rehint` touches `READY` steps, `execution.rs:5339-5349`, then `upsert_step` writes them, `:5353-5361`);
    - by a cancellation request, or any other run transaction that touches the step.
  - A builder who adds the column "with the lease", by analogy, nulls it at the first re-hint. The next lease of the attempt is then flagged again, and cycling to the cap of four returns. The static check permits that form ("clear them with the lease"), so it does not catch it.
  - A `NOT_SENT` withdrawal of the second, unflagged lease of the attempt is a statement that "clears the lease". Under the same rule it clears the column, and the third lease is flagged again.
- **(b) Value.**
  - The same cell says "`provisional_withdrawn_attempt` set to the attempt, … the attempt set back by one".
  - The domain sets the attempt back before the projection is written (`crates/domain/src/execution/state.rs:1474-1477`), and `upsert_step` binds the result as `$7` (`execution.rs:7307`, `:7380`).
  - Written from `$7`, the column holds N − 1. The next claim writes N, so the column never matches, the lease is never left unmarked, and there is no bound.
  - The right value is the stored `attempt` before the set-back, which is the attempt `step.lease_withdrawn` carries (`execution.rs:7675-7676`) and the one the next claim writes. In SQL that is `= attempt` on the right-hand side of the same `UPDATE`.
- **Why the tests miss it:**
  - Task 006-047's matrix sets the column directly.
  - Task 006-050's "withdrawn once and the next lease of its attempt unflagged" has no write between the withdrawal and the claim. It catches (b) but not (a).
- **Smallest fix:**
  - Take the column out of "the marks".
  - State that the two provisional withdrawals set it, to the stored attempt before the set-back, and that every other statement, the claim included, leaves it out of its `SET` list. Nothing clears it; a later attempt number supersedes it.
  - Add that form to the static check as the only allowed one.
  - Tests: withdraw, re-hint the `READY` step, request cancellation of a sibling (a run transaction that touches the step), then claim: unmarked. Also: a `NOT_SENT` withdrawal of the second lease, then a third claim: unmarked. Root and scoped.

**Major 3. A committed acknowledgement whose answer is lost, or a slow one, still spends the attempt. This contradicts the new guarantee "no failure of the acknowledgement's path can spend an attempt".**

Confidence: confirmed in the documents.

- **Where:**
  - the guarantee: decision record, Consequences (`ADR-0099…md:227-228`), decision 5 (`:92-100`), `sdks/SPEC.md:378-379`, `stream-completion.md:657` ("never an attempt"), failure-table row `:739`
  - the rules that break it: decision 7 and Task 006-049 (a repeated acknowledgement "answered … `CANCELLED` (`too_late` …) without [the margin]"); `sdks/SPEC.md:368-370` (both 2 s rules); `stream-completion.md:572` (the hand-back of an acknowledged lease is an ordinary retryable failure)
- **Scenario (i), a lost answer:**
  1. A lease with an 18 s earlier deadline is read at 5.9 s.
  2. Attempt 1 is sampled at 6.5 s with 11.5 s left, and commits.
  3. Its answer is lost, and the host's 6 s bound ends the attempt at about 11.9 s.
  4. The retry is sampled at about 12.2 s with 5.8 s left, under the 6 s margin. The server answers `CANCELLED` (`too_late`), because the lease is already acknowledged.
  5. The host hands it back. That is an ordinary failure, and the attempt is spent. With `maxAttempts = 1` the step fails for good on work that never started.
- **Scenario (ii), a slow acknowledgement:** an attempt of more than 4 s is answered `ACKNOWLEDGED` with 6.3 s left at the sample. 6.3 − 4.5 is under 2 s, so the host turns it into `CANCELLED`, hands it back, and the attempt is spent. If the heartbeat-first rule's heartbeat fails, the hand-back spends the attempt too.
- **An incoherence behind it:** an unmarked 1.8 lease's hand-back with no capability served is withdrawn under Architecture decision 0098. An acknowledged lease's identical hand-back spends the attempt. The server's evidence is the same in both cases.
- **How old it is:** the behaviour predates round 8, and in round 6 (Minor 1) I accepted the ordinary failure as better than a review. What is new is the guarantee that rules it out.
- **Why the tests miss it:** Task 006-050's bound case uses "every acknowledgement attempt failing at the server". No case has a committed acknowledgement with a lost answer, or a slow successful one.
- **Smallest fix:**
  - Treat a hand-back under the reserved code of an acknowledged lease that is still `LEASED`, with no capability served and a mark that counts, as the `NOT_STARTED` withdrawal. Under the cap, it sets `provisional_withdrawn_attempt`, so the bound still allows one per attempt.
  - Or keep "an acknowledged lease is an attempt", and narrow the three sentences to "no failure of the path cycles a lease to the cap". In that case, add the lost-answer and 2 s cases to Gap 006-047 and to "Bad".
  - Test: a committed acknowledgement whose answer is dropped, with the repeat under the margin and `maxAttempts = 1`.

## Minor

**Minor 1. The heartbeat reads the requirement epoch without a lock. A floor engine whose beat overlaps an activation's feature step keeps its gate open under the old epoch and serves capabilities unrecorded for up to one beat.**

Confidence: confirmed in code.

- **Code:**
  - `instance_heartbeat` reads `platform_requirement_epoch` with a plain `SELECT` (`20270113000000_instance_registry.up.sql:269`), while `instance_register` takes it `FOR SHARE` (`:197`).
  - The census is one statement (`:333-337`). The activation then holds the epoch through the feature's step, bounded by a 2 s lock timeout, a 3 s statement timeout and a 3 s step (`crates/infra-postgres/src/instances.rs:56-63`).
  - `decide()` does not read the requirements when the epoch is unchanged (`crates/application/src/instances.rs:678-680`).
- **Scenario:**
  1. Floor engine F has been silent for more than 30 s, and the census excludes it.
  2. Its heartbeat commits during the feature step and returns epoch E. F's gate was also at E, so "the gate's decision under the epoch it returned" succeeds with no read, and the gate opens.
  3. The activation commits E + 1. F serves `ResolveSecrets` unrecorded until its next beat (10 to 12 s).
- **Severity:** only a host that breaks the procedure can turn this into a hidden capability, hence Minor.
- **Fix:**
  - Request (7) should require the beat to read the epoch `FOR SHARE`, as registration does. It then waits for the activation (at most about 5 s, under the processes' 5 s statement timeout) and returns E + 1.
  - Test with a barrier: hold the activation between the census and the commit, beat a floor engine, commit, then call a capability: refused.

**Minor 2. The first-heartbeat rule lost its "at once" clause, and the derivation still assumes a 3 s call.**

Confidence: confirmed by the arithmetic.

- **Where:** `stream-completion.md:602-610`; Decision 006-046 (`research.md:500-506`); `sdks/SPEC.md:476`.
- **What changed:** round 7 sent the first beat "at once when [a third] is under 1.5 s". Round 8 replaced that with "before the handler starts when that time is under 2 s".
- **The gap:** with `T − p` (time left at the sample, less the time from the sample to the host's receipt) between 2 and 3 s, the beat leaves at a third and has two thirds of `T − p`, from 1.33 to 2 s, to commit. That is below the gate's heartbeat bound of 2 s at the 99th percentile. A beat that misses expires an acknowledged lease into an unknown outcome or a review.
- **The 3 s assumption:** the call now ends up to 4 s after arrival, so `p` reaches about 4 s. "2 s when `T` is 6 s and `p` is the call's 3 s bound" is no longer the worst case: (6 − 4) × 2 / 3 = 1.33.
- **Fix:** restore "at once when a third is under 1.5 s" (or raise the before-the-handler threshold to 3 s), and redo the derivation with `p` up to 4 s.

**Minor 3. A host-side deadline caused by the engine's own queue carries no envelope and counts.**

Confidence: plausible; from tonic's layering as I know it.

- **Code:** the gRPC server applies `concurrency_limit_per_connection(64)` and `max_concurrent_streams(128)` (`crates/api-grpc/src/server.rs:157-158`). In tonic the per-connection limit wraps the service stack outside the `.layer(admission)` (`:164`). So a call waits there before the "arrival" from which the server's 3 s deadline runs.
- **Scenario:** under a slow database, a host with many slots multiplexed on one connection has its acknowledgement attempt wait in that queue past the host's 6 s bound. The host sees `DEADLINE_EXCEEDED` with no envelope, while its heartbeats on the same connection are answered as the queue drains. Sustained for one give-up, the call counts, and three such calls decline the host.
- **Claim this contradicts:** Decision 006-065's "the platform puts its error envelope on every status it makes" does not cover outcomes the transport makes for the platform.
- **Fix:**
  - Stamp the arrival in the outermost layer of the gRPC server, or exempt `AcknowledgeTasks` from the per-connection limit, so the server always answers within its bound.
  - Test a saturated connection: no fallback.

**Minor 4. Step 5 of section 7 restarts PostgreSQL "before the engines start", but a virtual machine reverted with its memory has engines still running.**

Confidence: plausible.

- **Where:** `docs/runbooks/disaster-recovery.md:92`; Decision 006-062 (d); decision 12.
- **Why it matters:** section 4 stops the writers only for point-in-time recovery. After a revert, the engines reconnect within seconds, and the pass can withdraw a provisional lease whose acknowledgement the revert lost, while its handler runs. This is the reasoning the design used to reject an operator's fence for failover ("not in time").
- **Fix:** step 5 says to stop engines and control planes before reverting or rolling back storage under a running server (or at once if it was not planned), and Gap 006-048 names a revert done with engines running.

**Minor 5. Without the breaker, a persistent server-side failure of the acknowledgement has an unstated cost and no lever but the switch.**

Confidence: confirmed in the documents.

- **Where:** failure-table row `stream-completion.md:739`; considered option 8 ("nothing else is needed"); "Bad".
- **The cost:** for as long as the failure lasts, every flagged first lease of every attempt waits a full give-up. That is 24 s at the default time-to-live, and up to the 120 s clamp for long ones. The host never declines on it, because the answers carry the envelope.
- **The only lever:** `SAGAT_LEASE_WITHDRAWAL=off`, which also stops Architecture decision 0098's `NOT_SENT` withdrawals.
- **Fix:** state the cost in "Bad" and in the runbook symptom of `SagatAcknowledgementsFailing`, or add a switch that stops marking only (the alternative raised in round-7 Nit 6).

## Nit

1. `stream-completion.md:389` still says the activation entry refuses while `lease-withdrawal-v1` "is not recorded for engines". Decision 10, Decision 006-047 and Task 006-057 say "for both kinds".
2. `stream-completion.md:345-346` says an engine silent at the census "serves nothing until its beat". It is capability calls only, until its gate is decided again.
3. Decision 12 lists "a restore" among the losses that bring another server start, then excludes the restores that do not. Say "a restore that starts a server".
4. Considered option 7 rejects "a bound of its own on repeated withdrawals", while decision 5 now adopts one. Say why this one differs: it reads the row the claim writes, needs no count, and has no reset rule beyond the attempt.
5. The host's run:
   - concurrent calls have no stated order; give-up order was dropped;
   - it is not stated whether an `ACKNOWLEDGED` that a 2 s rule turns into `CANCELLED` resets the run;
   - the run has no age limit, so two counted calls can wait for days for a third.
6. The give-up counts from the expiry. With `timeoutMs` earlier than the expiry less 6 s, the pass withdraws first, so:
   - the host holds a slot for a withdrawn lease until its give-up;
   - its hand-back carries a withdrawn token and is not an `OK` answer, so a host with full slots never gathers evidence.

   Give up at the earlier deadline.
7. With the bound, each step-attempt makes at most one flagged call. The conformance cases "falls back at the third counted call" (Task 006-058) must run at least three flagged step-attempts within the window; name the workload.
8. "The port returns it as a witness type (`RequirementsHeld`) with a private constructor in the adapter": the decision function and its unit test live in the application crate, so the constructor cannot be private to the adapter. Say how it is restricted (for example a sealed constructor checked by the fitness function).
9. "The gate is checked again at the hand-over, after the capability record commits": a floor engine writes no capability record. Say "after the work that hands over, before the answer is sent". The parity test also mixes a claim-time condition (`provisional_withdrawn_attempt`) into the macro of "the mark counts"; keep them apart.

## The checks you asked for

- **The once-per-attempt bound:**
  - Every path that hands an attempt back on a provisional lease sets the column: the pass's `NOT_ACKNOWLEDGED`, and the hand-back's `NOT_STARTED`, including after expiry and after an unbound `NOT_HELD`. `NOT_SENT` deliberately does not.
  - Every other end of a provisional lease ends the attempt (the cap, a missing requirement, a report or heartbeat counted as a violation, `recovered`), so it does not need the column.
  - A lease can be flagged twice in one attempt only through `NOT_SENT` (intended), or if the column is cleared or written as N − 1 (Major 2).
  - The claim writes the stored attempt plus one, which is the withdrawn lease's attempt. The comparison survives withdrawals only if the withdrawal writes the stored attempt before the set-back.
  - Scoped rows are updated in place (`execution.rs:7055`), and the path lock orders the pass against the claim. A drained instance is never claimed again.
- **Can any failure still spend an attempt:** yes, a lost or slow acknowledgement (Major 3). Mixed causes at the cap (Architecture decision 0098's own) and a failover (`recovered`) remain, and are accepted.
- **The host's counting rule:** Major 1, Minor 3, Nits 5 to 7.
- **The liveness boundary:** measuring from the start of the beat is safe by 5 s. `last_seen` is at or after the start, the census excludes at `last_seen` plus 30 s, and the engine refuses at the start plus 25 s; 25 s is 30 s less the processes' 5 s statement timeout. The residue is the unlocked epoch read (Minor 1).
- **The restart step:**
  - Correct for the listed cases. Minor 4 covers the running engines.
  - The `fsync` reasoning is right: the write-ahead record reaches the kernel at commit, and only the loss of the operating system loses it, which restarts the postmaster.
  - Raising `synchronous_commit` to `local` covers a crash that the postmaster recovers in place.
  - As I know PostgreSQL, `PgStartTime` is set once in `PostmasterMain` and survives that crash recovery; Task 006-044 records it.
- **Contradictions:** the guarantee against Major 3; the marks rule against the new column (Major 2); considered option 7 against decision 5 (Nit 4); the two stale lines (Nits 1 and 2); the at-once rule (Minor 2).
- **Arithmetic:**
  - Eight columns on `steps`: correct.
  - 104 against 94 round trips: correct.
  - 25 s: correct.
  - 4 s: correct, plus up to 100 ms of rollback.
  - (30 − 6 − 6) / 6 = 3: correct.
  - (6 − 3) × 2 / 3 = 2: correct as arithmetic, but `p` is now up to about 4 s, which gives 1.33 s (Minor 2).

## Evidence to ask for once built

Each is a test to add, then its run:

- `cargo test -p sagat-tests --test integration_durable_execution`:
  - a provisional withdrawal, a re-hint and a run transaction touching the `READY` step, then a claim: unmarked; and the column equal to the attempt `step.lease_withdrawn` carries (Major 2);
  - a `NOT_SENT` withdrawal of the second lease, then a third claim: unmarked (Major 2);
  - a committed acknowledgement whose answer is dropped, repeated under the margin, with `maxAttempts = 1`: no attempt spent (Major 3);
  - an activation held between its census and its commit while a floor engine beats, then a capability call: refused (Minor 1).
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`:
  - an outage of every call ending 3 s after the give-up, with three flagged frames, at 18 s and 30 s time-to-live: no fallback (Major 1);
  - a saturated connection: no fallback (Minor 3);
  - at least three flagged step-attempts in each fallback case (Nit 7).
- The hosts' unit tests: `T − p` of 2.4 s, first beat sent at once (Minor 2).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-7/ (correctness.md, security.md, architecture.md, egm-design.md)
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round7-to-8.diff
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/state.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/server.rs
