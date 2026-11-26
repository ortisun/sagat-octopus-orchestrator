**Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 9**

## Verdict

Round 9 fixes the three round-8 Majors, mostly in the right direction:
- the probe ties the host's evidence to the time of the failure;
- the attempt's flag now has a retention rule and the right value;
- a lease its host did not start is withdrawn on its hand-back, whether or not it was acknowledged.

I checked the new hand-back rule against the code. A `Heartbeat` and a hand-back are ordered by the run lock. The first heartbeat makes the step `RUNNING` through `StepStarted`. The hand-back reads `Leased` under that same lock. So whether the step ran is decided on locked state. The `FORWARD`-only stop rule matches the claim query.

Two things are still wrong enough to steer the build badly:
- **Major 1:** the one allowed form of the attempt's flag cannot be written into the code as it stands. The statements that write a withdrawal are the same three that write the claim and every other transition. So "every other statement, the claim included, leaves it out of its `SET` list" is impossible. The condition has to be a bound flag, and the text never says so. This is the round-8 architecture Major 1, items 1, 4 and 5, still open.
- **Major 2:** the new early give-up turns a database failover of more than about 6 s into spent attempts or reviews for every flagged lease waiting for its acknowledgement. Round 8 ran those leases, and so does 1.7.

**Counts:** 0 Blockers, 2 Majors, 8 Minors, 9 Nits.

Nothing was built or run. Claims about the code come from reading `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`. Claims about tonic and PostgreSQL behaviour are from their source as I know it, and are marked as such.

## Status of round-8 findings

### My report

| Round 8 | Status | Reason |
|---|---|---|
| Major 1: the evidence clause accepted a healed outage | **Resolved** | The probe overlaps the failure: once a hand-back is answered `OK`, the host sends one more `AcknowledgeTasks`, and a working path answers it `NOT_HELD`. Residue: the probe needs an `OK` hand-back, which a hand-back that loses to the pass never gets (Minor 5). |
| Major 2: retention and value of `provisional_withdrawn_attempt` | **Partly resolved** | Taken out of the marks; never cleared; set to the stored attempt before the set-back; given its own allowed set. But that allowed set contradicts the statements it must live in (Major 1). |
| Major 3: a lost or slow acknowledgement spent the attempt | **Resolved, with residues** | A hand-back of a `LEASED` lease with no capability served is withdrawn, whether provisional or acknowledged. Residues: a hand-back that lands after the expiry loses to the pass and becomes a review (Minor 2); a heartbeat sent before the handler whose answer is lost (Minor 3). |
| Minor 1: unlocked epoch read in the heartbeat | **Resolved in text** | Request (7) reads the epoch `FOR SHARE`. Where that read goes against the registry's stated lock order is Nit 7. |
| Minor 2: the 3 s threshold | **Resolved** | (`T − a`) ≥ 3 leaves two thirds, 2 s, to commit; the arithmetic is right (below). Stale leftovers: Nit 1. |
| Minor 3: the queue in front of the server's deadline | **Partly resolved** | The fix asks for something tonic's builder cannot do as configured, and leaves out the wait for HTTP/2 stream credit (Minor 6). |
| Minor 4: engines running during a revert | **Resolved** | Runbook step 5, Decision 006-062 (d) and Gap 006-048. |
| Minor 5: unstated cost | **Resolved** | Stated in "Bad" and in the runbook symptom of `SagatAcknowledgementsFailing`. |
| Nit 1 (stale "for engines") | Resolved | |
| Nit 2 (silent engine serves "nothing") | Resolved | |
| Nit 3 ("a restore that starts a server") | Resolved | |
| Nit 4 (considered option 7 against decision 5) | Resolved | |
| Nit 5 (the host's run) | Half resolved | The order of calls and the reset by an `ACKNOWLEDGED` turned into `CANCELLED` are now stated. The run still has no age limit (Nit 8). |
| Nit 6 (give-up counted from the expiry) | Open | Nit 8. |
| Nit 7 (three flagged step-attempts per fallback case) | Resolved | In Task 006-058 and in the acceptance matrix. |
| Nit 8 (witness constructor) | Open | `stream-completion.md:416` is unchanged (Nit 8). |
| Nit 9 (wording at the hand-over; parity test) | Half resolved | The wording is fixed. The parity test still mixes in the claim-time condition (`stream-completion.md:514`; Nit 8). |

### Other round-8 reports, in my domain

| Finding | Status |
|---|---|
| Architecture Major 1 (the attempt's flag governed by the marks' rules) | Partly resolved; its items 1, 4 and 5 are Major 1 below. |
| Design Major 1 (throughput under a server-side failure) | Resolved by the early give-up, which introduces Major 2. |
| Design Major 2 (`CANCELLED` for compensations and reconciliations) | Resolved, and consistent with the claim query: `crates/infra-postgres/src/workers.rs:97-98` and `:114-115` claim a non-`FORWARD` step while a stop reason is set; `:131` and `:153` claim compensations regardless. |
| Design Minor 1 (the pass setting the flag) | Resolved: only the hand-back sets it, plus a withdrawal at the cap. |
| Design Minor 2 (the guarantee was false) | Resolved in substance. The new Consequences sentence still claims too much (Nit 4). |

## Blocker

None.

## Major

**Major 1. The one allowed form of `provisional_withdrawn_attempt` contradicts the statements it must be written in. The claim is one of those statements, and the condition can only be a bound flag, which the text does not name. A builder who obeys "the claim leaves it out" writes a separate statement, and the value becomes N − 1 again.**

Confidence: confirmed in code.

- **Where:**
  - `stream-completion.md:450-459`: "in each of the three statements that write a withdrawal … Every other statement, the claim included, leaves it out of its `SET` list".
  - `stream-completion.md:490-492`: the claim writes "in the statement that already writes the lease, in each of the three: `upsert_step`, `upsert_scoped_instance` and the path statement".
  - `stream-completion.md:504-506`: the static check, with the negative test "a claim that assigns it".
  - Decision 006-066 (`research.md:873-877`); Task 006-050.
- **The code:**
  - A root withdrawal runs through `self.engine.run(...)` (`crates/application/src/worker/withdrawal.rs:380-388`). A scoped one runs through `tx.persist_addressed` (`:461`).
  - Both end in the generic projection statements: `upsert_step` (`crates/infra-postgres/src/execution.rs:7378-7421`), `upsert_scoped_instance` (`:7055-7101`), and the path statement (`crates/infra-postgres/src/execution/path.rs:1237-1267`).
  - The same statements write the claim (`fresh_grant`, `execution.rs:7301-7302`), the heartbeat (`renewal`, `crates/application/src/worker.rs:1902-1910`), the completions and the settles.
  - So "the three statements that write a withdrawal" are the claim's statements. The flag's `CASE` sits in the claim's `SET` list whatever is done.
- **The condition is not on the row:**
  - From the old row and the new values, SQL can detect that this is a withdrawal: the old status is `LEASED`, the new one `READY`, `$7` = `attempt − 1`, and the generation is unchanged.
  - It cannot detect the cause. `NOT_STARTED` must set the flag; `NOT_ACKNOWLEDGED` and `NOT_SENT` below the cap must not.
  - It cannot detect "brings the count to four" either, because that count is the journal read `lease_withdrawals` (`execution.rs:7677-7695`).
  - So the condition must be a bound boolean computed by the withdrawal command. The marks' general rule forbids exactly that ("never on a bound flag alone"), and the new allowed set does not say it is permitted.
- **Failing scenario:**
  1. A builder reads "every other statement, the claim included, leaves it out". To keep the flag out of the claim's statement, they add a dedicated `UPDATE steps SET provisional_withdrawn_attempt = attempt …` after the projection write, in the same transaction.
  2. `upsert_step` has already written `attempt = $7`, which is N − 1 (`crates/domain/src/execution/state.rs:1517-1526` with `withdraw_lease`). The dedicated statement therefore stores N − 1.
  3. The next claim writes N, never equals the column, and marks every time. Withdrawal cycling to the cap of four returns.
  4. "PostgreSQL evaluates `SET` against the old row" (`stream-completion.md:455-456`) holds only inside the statement that sets the attempt back.
  5. Alternatively, a builder who puts the `CASE` in the shared statements is refused by the static check's own negative test, "a claim that assigns it".
- **Why the tests miss it:**
  - "The column holds the attempt `step.lease_withdrawn` carries" (the bound row of the acceptance matrix, Task 006-050) catches the N − 1 variant. Nothing catches the static check being written so that it accepts or forbids the wrong thing.
  - The static check's negative cases are specified in terms ("leave it out of the claim") that the correct implementation itself breaks.
- **Smallest fix:**
  - State the form as `provisional_withdrawn_attempt = CASE WHEN $n::boolean THEN attempt ELSE provisional_withdrawn_attempt END`, in each of the three generic projection statements only. `$n` is true only from the withdrawal command's decision: a `NOT_STARTED` withdrawal of a counting mark, or `before = 3`. Every other caller of those statements, the claim included, binds false.
  - Say the bound boolean is allowed for this column alone, because it is computed under the run lock, or the path lock, after the `FOR UPDATE` re-read, and a stale writer can only bind false, which is the safe direction.
  - Every statement other than those three (the acknowledgement, the capability records, `ensure_node_kind`, the operations queries) leaves the column out.
  - Rewrite the static check's negative cases as: a `CASE` outside the three statements; `THEN $m` (a bound attempt); a branch that yields `NULL`; a second statement that assigns it in the same transaction.

**Major 2. The early give-up turns a database failover of more than about 6 s into spent attempts or reviews for the flagged leases awaiting acknowledgement. Round 8 and 1.7 ran those leases.**

Confidence: confirmed in the documents.

- **Where:**
  - The early give-up: `sdks/SPEC.md:378-380`, `stream-completion.md:685-686`, Decision 006-065 (`research.md:857-859`).
  - The envelope covers "the store unavailable": `research.md:853`, `stream-completion.md:690-691`.
  - The hand-back of a mark that does not count is an ordinary failure: `stream-completion.md:471-473`, `:600`, the failover row `:784`.
  - The hand-back's retries: `FailStep` "at most 5 times" (`sdks/SPEC.md:281`), with the defaults of 200 ms doubling (`sdks/SPEC.md:31`).
  - The outage row, which this contradicts: `stream-completion.md:774`.
- **Scenario** (default 30 s time-to-live, a Patroni or managed failover of 15 s, a lease claimed and read just before it):
  1. At t = 0 the primary dies. Every `AcknowledgeTasks` attempt reaches a healthy engine and is answered `UNAVAILABLE` with the envelope (reason `store`, or `ACKNOWLEDGEMENT_DEADLINE` while pool acquisition waits).
  2. At about t = 6 s every attempt in the last receive bound has ended this way, so the host gives the lease up at once and hands it back.
  3. The hand-back fails with the store unavailable. Its five tries span roughly 1.5 to 3 s plus the calls, and the host abandons it at about t = 10 s.
  4. At t = 15 s the standby is promoted, and `pg_postmaster_start_time()` changes. The lease's mark no longer counts.
  5. At the expiry the pass treats it as "a mark that does not count": `expire_attempt`, counted `recovered`. That is an unknown outcome, and a review for an `Activity`, for work that never ran.
  6. If the failover takes 9 s instead, the hand-back lands after the promotion. It is then an ordinary failure, and the attempt is spent; with `maxAttempts = 1` the step fails for good.
  7. Round 8 had no early give-up. There the host kept repeating until about t = 24 s, its repeat after the promotion was answered `ACKNOWLEDGED` (`recovered`, 15 s left, at least the margin), and the task ran. At 1.7 the task ran at once.
  8. Every flagged lease that is unacknowledged when the outage starts is hit. At 1,000 claims a second and about 1 s from claim to acknowledgement, that is about 1,000 leases per failover.
- **Also:** while the server sheds `AcknowledgeTasks`, the same rule turns each shed call, which needs no run lock, into a hand-back, which takes the run lock and writes a journal event and an outbox row, plus a second claim. That is load amplification during overload. The bound once per attempt limits it to one round per step-attempt, but it is a positive feedback on exactly the shedding the class exists for.
- **Why the tests miss it:** the outage cases of Task 006-058 and of the Fallback row ("an outage of every call … that ends 3 s after the give-ups", "a 30 s `UNAVAILABLE` outage") check only that the host does not fall back. None changes the database incarnation, and none asserts that no review appears and no attempt is spent.
- **Smallest fix:**
  - Give up early only on evidence that the database answers. Either:
    - (a) the attempts' envelope reasons are `shed`, `statement_timeout`, `lock_timeout` or `conflict`, and never `store`; and `ACKNOWLEDGEMENT_DEADLINE` is split so that a deadline reached while acquiring a connection is excluded; or
    - (b) the host gives up early only while another call of its own that needs the database (a heartbeat, a report) was answered `OK` in the same receive bound.
  - Otherwise keep repeating until the timed give-up, as in round 8.
  - Correct the outage row at `stream-completion.md:774` for a failover.
  - Tests in `integration_durable_execution` and the conformance scenario: a 15 s failover with three flagged frames in flight, at 18 s and 30 s time-to-live. Expected: no review, no attempt spent, the leases run as `recovered`. Also: a shed of `AcknowledgeTasks` alone for 2 minutes, with the rate of withdrawal transactions reported.

## Minor

**Minor 1. Decision 7, Decision 006-051 and Decision 006-045 say a recovered lease's hand-back is withdrawn. Decision 9, the withdrawals table and the failover row say it is an ordinary failure. Followed, the first version withdraws a lease whose capability record a failover may have lost.**

Confidence: confirmed in the documents.

- **Where:**
  - ADR decision 7 (`ADR-0099…md:120-124`): "of an acknowledged lease or a lease whose mark does not count … the host hands the lease back, which is withdrawn (decision 5)".
  - Decision 006-051 (`research.md:685-688`): "its hand-back is withdrawn while the lease is `LEASED`".
  - Decision 5 and Decision 006-045 have no "whose mark counts" qualifier.
  - Against them: decision 9 (`:132-141`, "a lease whose reservation does not count is an ordinary retryable failure"), `stream-completion.md:471-473` and `:600`, and the failover row `:784`.
- **Scenario:**
  1. Asynchronous replication. A capability record commits on the old primary and is lost in the promotion.
  2. A repeated acknowledgement after the promotion answers `CANCELLED` (`too_late`, `recovered`), and the host hands back.
  3. Under decision 7 the server withdraws it: the step goes back to `READY` with the attempt reused, although a secret may have been served. That breaks "a capability served is never followed by a withdrawal".
- **Why it is not Major:** Task 006-050 pins "a lease claimed on another incarnation … handed back: an ordinary failure", so the build's own test settles the conflict.
- **Fix:** in decision 7, Decision 006-051 and Decision 006-045, write "withdrawn while its mark counts, otherwise an ordinary failure".

**Minor 2. An acknowledged lease handed back after its expiry races the pass, which ends it as an unknown outcome. The lost-answer path reaches the expiry by construction, so "an acknowledgement whose answer was lost costs no attempt" holds only when the hand-back wins.**

Confidence: confirmed in the documents.

- **Where:**
  - Decision 5 (`ADR-0099…md:98-100`).
  - The lost-answer row (`stream-completion.md:755`).
  - The give-up "never while an attempt is in flight (… at most 6 s)" (`sdks/SPEC.md:376-377`).
  - "Acknowledged, and expired or timed out → `expire_attempt`" (`stream-completion.md:599`).
- **Scenario:**
  1. 18 s threshold, lease read at 5.9 s, so 12.1 s left at receipt. Attempt 1 commits and its answer is lost on a path that keeps dropping answers.
  2. Attempt 2 starts at about 6.3 s. The give-up instant is 6.1 s after receipt (about 12 s), but attempt 2 is in flight and is waited for until about 12.3 s.
  3. The hand-back leaves at the expiry (18 s) or after it. If the pass runs first, the acknowledged `LEASED` lease gets `expire_attempt`: a review for an `Activity` that never ran.
  4. For a provisional lease this race was harmless, because both outcomes withdraw. For an acknowledged lease it is not.
- **Why the tests miss it:** the new Hand-back case ("a committed acknowledgement whose answer is dropped and whose repeat comes under the margin") has the repeat answered, so the hand-back is early.
- **Fix:**
  - Either bound the wait for an in-flight attempt so that the hand-back leaves at least one heartbeat bound (2 s) before the expiry, abandoning the attempt; or accept the race and say so.
  - In both cases, extend Gap 006-041 and the row at `stream-completion.md:755` to "or lands after the expiry and after the pass".
  - Test: the barrier "a hand-back 1 ms after the expiry with the pass held", repeated for an acknowledged lease with the pass winning: an unknown outcome, counted `acknowledged`.

**Minor 3. The heartbeat sent before the handler (the rule under 3 s) can commit `RUNNING` and lose its answer. The host then hands back a `RUNNING` lease, which spends the attempt for work that never ran.**

Confidence: confirmed in code and documents.

- **Code and text:**
  - The first heartbeat emits `StepStarted` (`crates/domain/src/execution/command.rs:508-512`, `state.rs:1528-1537`).
  - Host item 3: "hands the lease back if it fails" (`stream-completion.md:669-670`).
  - The table row "`RUNNING` (heartbeated) … handed back → an ordinary retryable failure, the attempt spent" (`:602`).
- **Scenario:** `ACKNOWLEDGED` arrives with 2.5 s left after the attempt. The heartbeat commits, its answer is lost, and the retries within 2.5 s fail. The host hands back, and with `maxAttempts = 1` the step fails for good.
- **Fix:** name it in Gap 006-041 and in "Bad". Optionally: on a failed heartbeat-before-handler with no answer at all, the host does not hand back and lets the lease lapse. That costs a review instead of an attempt, so stating the trade-off is the minimum.

**Minor 4. `withdrawn_lease_token_hash` is kept only for `NOT_ACKNOWLEDGED`. Now that acknowledged leases are withdrawn on their hand-back, a handler that runs after `ACKNOWLEDGED` on a lease its host also handed back leaves no counted violation.**

Confidence: confirmed in the documents.

- **Where:** the marks table (`stream-completion.md:441-444`); decision 12's list of what the server sees (`ADR-0099…md:160-174`); Gap 006-042.
- **Scenario:**
  1. An SDK defect in the give-up against a late `ACKNOWLEDGED` (Task 006-059 tests "never while an attempt is in flight", but not the interleaving) hands the lease back and also runs the handler.
  2. The hand-back withdraws the acknowledged `LEASED` lease (`NOT_STARTED`). The handler's later `Heartbeat` or `CompleteStep` is refused as stale.
  3. Because the hash is not kept for `NOT_STARTED`, the call is neither counted nor audited. The work runs again elsewhere with the same attempt number.
  4. Before round 9, this hand-back spent the attempt and nothing was hidden.
- **Fix:**
  - Keep `withdrawn_lease_token_hash` for every withdrawal of a counting mark, `NOT_STARTED` included, or at least of an acknowledged one, and count a later report, heartbeat or capability call with it under `SagatCallsAfterWithdrawal`.
  - Add to Task 006-058 a host case: a late `ACKNOWLEDGED` arriving after the give-up decision is never both run and handed back.

**Minor 5. The probe needs an `OK` hand-back. A hand-back that lands after the pass has withdrawn the lease is refused as stale, so no probe is sent and the call can never count.**

Confidence: confirmed in the documents.

- **Where:** `sdks/SPEC.md:387-388`; `stream-completion.md:696-698`.
- **Scenario:**
  1. At an 18 s time-to-live on a path that swallows only `AcknowledgeTasks`, the give-up waits for the in-flight attempt up to the expiry (as in Minor 2). The hand-back then races the pass.
  2. When the pass wins, the hand-back is refused as stale: the server answered with its envelope, but not `OK`. No probe is sent, and the call does not count.
  3. The fallback case "at an 18 s time-to-live (the fallback)" then depends on the pass's period, and flakes. The same happens when `timeoutMs` is earlier than the expiry less 6 s (round-8 Nit 6): the pass withdraws first, every time.
- **Fix:** send the probe after any answer to the hand-back that carries the server's envelope (`OK` or a stale refusal), since either shows the server reachable; or give up at the earlier deadline (Nit 8).

**Minor 6. "The server's deadline starts in the outermost layer of the gRPC server, before the per-connection concurrency limit" cannot be done with `Server::layer`. Calls waiting for HTTP/2 stream credit wait in the client, where no stamp can see them.**

Confidence: plausible; from tonic's source as I know it.

- **Code:** `crates/api-grpc/src/server.rs:157-164` configures `.concurrency_limit_per_connection(64)`, `.max_concurrent_streams(128)` and `.layer(admission)`.
- **How tonic layers them:** tonic wraps the service it builds from the user's layers in its own per-connection stack (recover error, concurrency limit, `grpc-timeout`). So any `.layer()` sits inside the limit.
- **Scenario:** a host multiplexes more than 128 calls on one connection under a slow database. Calls 65 to 128 wait inside tonic's limit, ahead of any user layer. Call 129 and later wait in the client for a stream. Either way the host sees `DEADLINE_EXCEEDED` with no envelope.
- **Fix:**
  - Task 006-049 replaces `concurrency_limit_per_connection` with a `ConcurrencyLimitLayer` added after the stamping layer (or exempts `AcknowledgeTasks` from it).
  - State that stream credit beyond `max_concurrent_streams` is the client's, which is why the SDK should not put the acknowledgement behind its own saturated connection.
  - Test with more than 128 calls in flight on one connection.

**Minor 7. With the flag set at the fourth withdrawal, a marked lease at the cap cannot be produced through this build, so the "at the cap" rows and their tests cannot be set up as written. Two rows of the acceptance matrix contradict each other.**

Confidence: confirmed in the documents.

- **Where:**
  - `stream-completion.md:588-589` (the fourth withdrawal sets the flag).
  - Table rows `:595` and `:598`.
  - Acceptance matrix rows `:832` ("at the cap, a marked lease past its expiry is ended by one pass") and `:835` ("after four withdrawals a marked lease is ended by the pass"), against `:833` ("four `NOT_SENT` withdrawals leave the fifth lease unflagged").
  - Task 006-050 ("after four withdrawals of the attempt … a marked lease past its expiry … `capped`"; "at the cap, the pass and the hand-back in both orders").
- **Why:** after four withdrawals by this build, the fifth claim is unmarked. A marked lease at the cap exists only when the earlier withdrawals were written by a binary that does not set the flag (the floor build, before the activation).
- **Fix:** state that the rows are reachable only for withdrawals written without the flag, and build those tests by writing the four journal events with the column left unwritten (through the floor build's statement, or by SQL), saying so in the task. Or drop the rows and the `capped` clause for marked leases.

**Minor 8. Two normative lines still describe the round-8 rule for acknowledged leases.**

Confidence: confirmed in the documents.

- `stream-completion.md:754`: "A repeated acknowledgement with less than the margin left → … for an acknowledged lease an ordinary failure". It is now withdrawn while `LEASED` and its mark counts.
- `stream-completion.md:727-728` (rollback, step 2): "Only provisional leases change meaning when the requirement goes; an acknowledged lease is an attempt either way". An acknowledged `LEASED` lease's hand-back is now a withdrawal before the removal and an ordinary failure after it. That is the safe direction, but the drain's justification is false as written.
- **Fix:** correct both lines. Optionally, let the drain also wait for acknowledged `LEASED` leases, or say that their hand-back after the removal spends the attempt.

## Nit

1. Leftovers of the 3 s derivation:
   - `stream-completion.md:654` still says "(the two together fit: (6 − 3) × 2 / 3 = 2)".
   - Decision 006-046 (`research.md`, the sentence "with a call of 3 s …") still presents that as the gate's check.
   - With `p` up to 4 s, it is the 3 s heartbeat-before-handler threshold that makes them fit.
2. The probe's answer:
   - It must not go through the `NOT_HELD` verdict handling (`dropped`, freeing a slot that was already freed). Say it is evidence only.
   - Say whether it is one attempt or a call with the retries of §5.
   - Its cost is about three statements, counted as `not_held{reason="no_match"}`, which a dashboard reads as a mismatched token. Give it a reason of its own.
3. The early give-up's window ("every attempt for it since one receive bound ago"): say whether an attempt still in flight blocks it, and whether an attempt counts by its start or by its end. With give-ups now per lease, "calls are ordered by their give-ups" should say which lease's give-up orders a call.
4. Consequences: "a lease its host did not start is never charged an attempt for it while it is `LEASED`" is false for a lease whose mark does not count, before the activation, and when the hand-back loses to the pass (Minors 1 and 2, Major 2). Qualify it.
5. `SagatAcknowledgementsFailing` filters `NOT_STARTED` withdrawals "of marked leases, other than those of a `FORWARD` lease with a stop (labelled `stop`)". `worker_leases_undelivered_total{cause, outcome}` has neither a mark label nor a stop label. Name the series the rule reads.
6. Stale or broken text:
   - `docs/adr/README.md`: the entry still says "an acknowledged one is an attempt", runs two sentences together ("written on a lease that has not run…"), and drops Architecture decision 0082 from "refines", while the decision record says it relies on that amendment.
   - Decision 006-061 (`research.md:806`) still says "an acknowledged lease is an attempt".
7. Request (7) adds an epoch read `FOR SHARE` to `instance_heartbeat`. That function updates its own `serving_instances` row first (`20270113000000_instance_registry.up.sql:260-269`), and the file's stated rule is "the requirement epoch row first … No function waits for a lock while it holds a later one" (`:11-13`). Say the `FOR SHARE` read moves to the top, as in registration (`:197`).
8. Still open from round 8:
   - The give-up counts from the expiry rather than the earlier deadline (Nit 6), which also starves the probe (Minor 5).
   - The run has no age limit (Nit 5).
   - The parity test still mixes in the claim-time condition (`stream-completion.md:514`; Nit 9).
   - The witness constructor is still "private in the adapter" (`stream-completion.md:416`; Nit 8; architecture round-8 Minor 2).
9. Decision 7 cancels a `FORWARD` step of a saga whose cancel policy does not cancel work in flight (`crates/domain/src/execution/sagas.rs:159-163`), where 1.7 would have run it. That is harmless, since nothing ran and there is nothing to compensate, but state it as intended.

## The checks you asked for

- **Withdrawing an acknowledged `LEASED` lease on its hand-back:**
  - *Against a heartbeat:* ordered by the run lock. The heartbeat opens with `Lock::Wait` (`crates/application/src/worker.rs:1887`) and emits `StepStarted`. The hand-back reads `StepStatus::Leased` from the state loaded under the same lock (`crates/application/src/worker/withdrawal.rs:222-226`). Either order gives one outcome: a withdrawal (the heartbeat then finds the token gone, `STALE_ATTEMPT`) or an ordinary failure.
  - *Against the acknowledgement:* the hand-back holds the run lock, then waits `FOR UPDATE`. The acknowledgement holds `FOR NO KEY UPDATE SKIP LOCKED` and never waits for a row, so there is no cycle. Acknowledgement first: the hand-back withdraws an acknowledged lease. Hand-back first: the acknowledgement answers `RETRY`, then `NOT_HELD`.
  - *Against the pass:* both are under the run lock. After the expiry, the pass wins for an acknowledged lease with an unknown outcome (Minor 2).
  - *`RUNNING` detection:* correct (above); the only residue is the lost heartbeat answer (Minor 3).
  - *The settle under a stop:* the withdrawal puts the step back to `READY`. The drain then cancels a `FORWARD` step, and keeps and leases again a compensation or reconciliation (`crates/domain/src/execution/plan.rs:351-362`, `:424-428`).
  - *The trust argument:* equal to Architecture decision 0098's for an unmarked 1.8 lease. What the change loses is the evidence path (Minor 4).
- **The probe:**
  - It costs one call of about three statements, with no lock and no audit record, per given-up call that otherwise qualifies.
  - It misleads when the hand-back loses to the pass (Minor 5), and when the swallowing is partial or depends on the size of a call. In that case the host never declines, and each flagged lease waits a full give-up; the bound keeps attempts safe and the `NOT_STARTED` clause of the alert fires.
- **The early give-up:** Major 2; Nit 3.
- **The `FORWARD`-only stop rule:**
  - Consistent with the claim query (`crates/infra-postgres/src/workers.rs:97-98`, `:114-115`, `:131`, `:153`).
  - The heartbeat's stop comes from `cancel_requested`, which the drain never sets on a compensation, and not on a reconciliation that is a recovery obligation (`crates/domain/src/execution/plan.rs:354-362`).
  - The settle: as above. The saga case is Nit 9.
- **The attempt's flag against the projection statements:** Major 1. The value (the old row's `attempt`, inside the same `UPDATE`) is right.
- **Contradictions after the base merge:**
  - The flag's allowed set against the shared statements (Major 1).
  - The recovered hand-back (Minor 1).
  - Row 754 and the rollback's step 2 (Minor 8).
  - The capped rows (Minor 7).
  - The README entry and Decision 006-061 (Nit 6).
- **Arithmetic:**
  - (`T − a`) / 3 ≥ 1 s and 2 (`T − a`) / 3 ≥ 2 s for `T − a` ≥ 3: correct. The real time left at receipt is at least `T − a`, because the time from the sample to the receipt is at most `a`.
  - (30 − 6 − 6) / 6 = 3: correct.
  - 25 s = 30 s − 5 s: correct.
  - 104 against 94: unchanged.
  - "About 6 s of delay" for the early give-up: correct to the give-up. The hand-back, the re-hint and the second claim come on top.
  - The stale "(6 − 3) × 2 / 3 = 2" is Nit 1.

## Evidence to ask for once built

Each is a test to add, then its run:

- `cargo test -p sagat-tests --test integration_durable_execution`:
  - the static check's negative cases as rewritten in Major 1, and the column equal to the old row's attempt in each of the three statements, root and scoped (Major 1);
  - a 15 s failover with flagged leases awaiting acknowledgement: no review, no attempt spent (Major 2);
  - a recovered lease answered `CANCELLED` and handed back: an ordinary failure (Minor 1);
  - an acknowledged lease's hand-back racing the pass after its expiry, both orders (Minor 2);
  - a call carrying the token of a `NOT_STARTED`-withdrawn acknowledged lease: counted (Minor 4);
  - the cap cases set up with the flag left unwritten (Minor 7).
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`:
  - a failover of 15 s and a shed of 2 minutes (Major 2);
  - the fallback at an 18 s time-to-live with the pass running every second (Minor 5);
  - more than 128 calls in flight on one connection (Minor 6);
  - a late `ACKNOWLEDGED` against the give-up (Minor 4).
- The hosts' unit tests: a heartbeat sent before the handler whose answer is lost (Minor 3).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-8/ (correctness.md, architecture.md, egm-design.md)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution/path.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/state.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/command.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/plan.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/sagas.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/saga.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round8-to-9.diff
