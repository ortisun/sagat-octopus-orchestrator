**Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 10**

## Verdict

Round 10 resolves round-9 Major 1. The attempt flag now has one coherent form, and that form fits the three projection statements as they are in the code. Round-9 Major 2 is partly resolved. The new evidence rule for the early give-up accepts an `OK` answer that came before the database failed. So the failover it was meant to wait out still turns the leases in flight at the failure into reviews or spent attempts. The failover test as written cannot catch this.

**Counts:** 0 Blockers, 1 Major, 6 Minors, 12 Nits.

Nothing was built or run. Claims about the code come from reading `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`.

## Status of the round-9 findings

### Majors from my report

**Major 1 (the attempt flag's form against the shared statements): resolved.**
- The form `CASE WHEN $flag AND status = 'LEASED' AND generation = $fence THEN attempt ELSE provisional_withdrawn_attempt END` is placed in all three projection statements, for every caller. `$flag` is bound true only by the withdrawal command.
- It fits the code:
  - `upsert_step` (`crates/infra-postgres/src/execution.rs:7378-7421`), `upsert_scoped_instance` (`:7055-7101`) and the path statement (`crates/infra-postgres/src/execution/path.rs:1237-1278`) are plain `UPDATE`s. So unqualified `status`, `generation` and `attempt` on the right of `SET` read the old row, as the text says.
  - Each statement runs once per touched row (`execution.rs:5353`, `:6189`).
  - Each withdrawal is its own `engine.run` or `persist_addressed` inside the run's transaction (`crates/application/src/worker/withdrawal.rs:380-388`, `:451-461`). So a second write of a row in the same transaction does see the first write, and the old-row fence does what it claims.
- `withdraw_lease` keeps the generation and sets the attempt back by one (`crates/domain/src/execution/state.rs:1474-1479`). So the fence matches the withdrawn row, and `attempt` on the right is N.
- One residue, about which rows get `$flag` (Minor 2).

**Major 2 (a failover turned into spent attempts or reviews by the early give-up): partly resolved.**
- Requiring another call answered `OK` stops the bulk of the failure: leases whose acknowledgement starts after the database failed.
- It does not stop leases whose window opened before the failure, because an `OK` from before the failure counts (Major 1 below).
- At an 18 s time-to-live, the new acceptance row asks for an outcome that cannot happen (Minor 1).

### Minors and Nits from my report

| Round 9 | Status | Reason |
|---|---|---|
| Minor 1: a recovered lease's hand-back | Resolved | Decision 7, Decision 006-045 and Decision 006-051 now say "while its mark counts, otherwise an ordinary failure". The Hand-back row of the matrix and Task 006-050 test it. |
| Minor 2: the hand-back against the pass past the expiry | Resolved | The race is accepted and stated (decision 5, Gap 006-041, the lost-answer row), and tested in both orders (Task 006-051, Exactness row). |
| Minor 3: a heartbeat before the handler whose answer is lost | Resolved | A new table row, Gap 006-041 and "Bad". A side effect of the round-10 change is Minor 6. |
| Minor 4: the hash not kept for a `NOT_STARTED` withdrawal | Resolved in substance | The marks table, decision 12, Decision 006-053 and Task 006-050 are updated. Two pieces are stale or missing (Nits 2 and 11). |
| Minor 5: the probe needed an `OK` hand-back | Resolved | Any answer with the envelope now triggers the probe. |
| Minor 6: tonic's per-connection limit | Resolved | The platform's own layer replaces it, and client-side stream credit is stated. How to key the layer is Nit 8. |
| Minor 7: the cap rows cannot be set up | Partly resolved | The failure table now says a marked lease at the cap exists only when earlier withdrawals were written without the flag. The acceptance matrix rows (`stream-completion.md:867`, `:870`) and Task 006-050 (`tasks.md:271-273`) still describe "after four withdrawals a marked lease…" with no setup (Minor 5). |
| Minor 8: two stale lines | Resolved | |
| Nit 1: the 3 s derivation | Partly resolved | `stream-completion.md` is fixed. `research.md:637-638` still says "with a call of 3 s … (6 − 3) × 2 / 3 = 2 s" (Nit 1). |
| Nit 2: the probe's answer | Resolved | One attempt, evidence only, reason `withdrawn`. |
| Nit 3: the window of the early give-up | Mostly resolved | Still open: which lease's give-up orders a call (Nit 4). |
| Nit 4: Consequences | Mostly resolved | One qualifier is still missing (Nit 12). |
| Nit 5: the alert's series | Resolved | The `mark` and `stop` labels are added. |
| Nit 6: README and Decision 006-061 | Resolved | |
| Nit 7: the order of the epoch read | Resolved | The epoch is read first, `FOR SHARE SKIP LOCKED`. |
| Nit 8: items left open from round 8 | Partly resolved | The run age and the witness are resolved. The give-up from the earlier deadline is stated in `stream-completion.md` but not in `sdks/SPEC.md` (Minor 3). The parity test still mixes in the claim-time condition (Nit 5). |
| Nit 9: the saga cancel | Resolved | |

### Other round-9 reports, in my domain

| Finding | Status |
|---|---|
| Architecture Minor 1 (old-row guard on the flag) | Resolved by the fence. |
| Architecture Minor 4 (alert that does not exist) | Resolved: `SagatLeaseAttemptCapped` is owned here. I confirmed the base defines no such alert. |
| Architecture Minor 6 (race against the pass) | Resolved. |
| Security Major 1 (`FOR SHARE` blocks beats) | Resolved, and consistent with my round-8 Minor 1. A beat that returns no epoch never opens a closed gate, and a floor engine live at the census still blocks the activation. |
| Design Minor 3 (a path that cuts calls by duration) | Resolved. The probe timing in its new test is Nit 10. |
| Design Minor 4 (the give-up window) | Resolved. |
| Design Minor 5 (sheds amplified into hand-backs) | Partly resolved. Sheds are waited out, but an `ACKNOWLEDGEMENT_DEADLINE` reached while waiting for a pooled connection on a saturated engine is not a shed, so it still gives up early. It is now measured by Task 006-056, which is acceptable. |
| Architecture Major 1 and design Major 1 (the preconditions missing from the merged base) | Stated correctly against the code: `WithdrawalCause` has only `NotSent` and `NotStarted` (`crates/domain/src/execution/event.rs:438-443`). The fallback branch introduces Minor 4. |

## Blocker

None.

## Major

**Major 1. The early give-up's evidence ("another call … answered `OK` in that time") accepts an `OK` from before the database failed. A failover therefore still gives up early the flagged leases whose acknowledgement was in flight when it started, and turns them into reviews or spent attempts. The failover test cannot catch it.**

Confidence: confirmed in the documents.

**Where:**
- `sdks/SPEC.md:379-382` ("answered `OK` in that time … A failover or a busy engine therefore waits for the ordinary give-up");
- `stream-completion.md:707` (item 4);
- the failover row, `stream-completion.md:788` ("Every call that reaches the database fails, the heartbeats too, so no host gives up early");
- Decision 006-065 (`research.md:870-873`);
- the Fallback row of the matrix (`stream-completion.md:878`);
- Task 006-058 and Task 006-059.

**Scenario:** a 30 s time-to-live, a busy host with running tasks heartbeating every few milliseconds, a 15 s failover starting at F.
1. Lease L is read and its first `AcknowledgeTasks` attempt is sent at F − 50 ms. A heartbeat of another task is answered `OK` at F − 20 ms, inside L's window.
2. At F the primary dies. L's attempts end `UNAVAILABLE` with the envelope (reason `store`, or `ACKNOWLEDGEMENT_DEADLINE`), which are answers other than a shed.
3. At F + 5.95 s, L has waited 6 s since its first attempt. Every attempt that ended was not a shed, and another call was answered `OK` in that time. So the host gives L up early and hands it back.
4. The hand-back fails while the database is down. Its five tries take about 3 s, and it is abandoned.
5. At F + 15 s the standby is promoted, and the server's start time changes.
6. At the expiry the pass meets a mark that does not count, so `expire_attempt` runs, counted `recovered`. For an `Activity` that never ran, the result is a review.
7. If the hand-back's last try lands after the promotion instead, it is an ordinary failure: the attempt is spent, and with `maxAttempts = 1` the step fails for good.
8. Without the early give-up, the host repeats until F + 24 s. Its repeat after the promotion is answered `ACKNOWLEDGED` (`recovered`, 15 s left), and the task runs. That is what the failover row promises.

**Who is hit:** every flagged lease that had an acknowledgement in flight or being retried at F, on a host that had any call answered in the few milliseconds before. At 1,000 claims a second and about 100 ms from claim to acknowledgement, that is tens of leases per failover.

**Why the tests miss it:** the new failover case ("a 15 s database failover with three flagged frames in flight … no early give-up, no review") has no other call answered inside the window. Three unacknowledged frames send no heartbeats, so the case passes whatever the rule says.

**Smallest fix:**
- Count only an `OK` to a call sent after one of this lease's failed attempts ended. For example: "another of its calls, sent after an attempt for this lease ended, was answered `OK`". A failover then fails every call that counts until the promotion.
- Write the same in Decision 006-065 and item 4.
- Add a running task to the failover case of Task 006-058 and of the matrix, heartbeating with an answer 20 ms before the failure. Expected: no early give-up, no review, no attempt spent.

## Minor

**Minor 1. At an 18 s time-to-live, the new failover acceptance case asks for an outcome that the design's own rules make impossible.**

Confidence: confirmed in the documents.

**Where:** the Fallback row (`stream-completion.md:878`): "a 15 s database failover with three flagged frames in flight, at an 18 s and a 30 s time-to-live: no early give-up, no fallback, no review, no attempt spent, the leases run as `recovered`". Also Task 006-058.

**Why:**
1. A lease claimed at c with an 18 s deadline has its ordinary give-up at about c + 12 s (time left at receipt minus 6 s).
2. A failover that starts while the frames are in flight (F ≥ c) promotes at F + 15 ≥ c + 15. That is after the give-up, and with 3 s left, which is under the margin of one receive bound.
3. So the host hands back while the database is down. The lease either lapses into `recovered`, which is a review, or lands after the promotion as an ordinary failure, which spends the attempt.
4. Gap 006-047 already accepts this ("nor a mark across another server's takeover"). The 18 s case can only be made to pass by changing the rules or the harness.

**Fix:** keep "no review, no attempt spent" for 30 s. At 18 s, either expect the outcome of Gap 006-047 (counted `recovered`, or an ordinary failure) or use a failover shorter than about 6 s. The failover row at `:788` should also say that a lease whose give-up passes before the promotion follows Gap 006-047.

**Minor 2. `$flag` is described as bound "by the withdrawal command", not per row. The fence (`status = 'LEASED' AND generation = $fence`) does not tell two steps of the same run apart, so a per-command binding flags any `LEASED` sibling the command writes.**

Confidence: plausible.

**Code:**
- One `Effects` is passed to every touched row of a command (`execution.rs:5353-5354`, `:6189-6190`).
- The lease grant and renewal are already filtered per instance (`execution.rs:7289-7300`, `:6952-6963`).
- Generations are per step, so two steps both `LEASED` at generation 1 are ordinary.

**Scenario:**
1. A `NOT_STARTED` withdrawal whose settle also writes the row of a `LEASED` sibling at the same generation, for example a stop's planning that touches it.
2. With a per-command `$flag`, the sibling's column is set to its current attempt N.
3. The sibling's next delivery at N after a `NOT_ACKNOWLEDGED` withdrawal is then unmarked and loses its protection.
4. This is the safe direction (fewer flags), but it is a wrong value, and the matrix test "the column holds the attempt `step.lease_withdrawn` carries" checks only the withdrawn row.

**Fix:** say that `$flag` is true only on the withdrawn instance's row (carried in `Effects` with its instance and filtered like `lease`), and that `$fence` is the withdrawn lease's generation. Add a test case: a withdrawal whose command touches a `LEASED` sibling leaves the sibling's column unchanged.

**Minor 3. `stream-completion.md` now counts the give-up from the earlier of the expiry and the attempt deadline. The normative host text, `sdks/SPEC.md` §6.2, still counts from the expiry.**

Confidence: confirmed in the documents.

**Where:** `stream-completion.md:706` against `sdks/SPEC.md:376-377` ("its time left at receipt (its expiry minus the host's clock at receipt) minus 6 s"). `stream-completion.md` names `sdks/SPEC.md` as the normative host text.

**Scenario:**
1. A lease with `timeoutMs` = 20 s and a 60 s time-to-live. An SDK that follows `sdks/SPEC.md` gives up at about 54 s.
2. The pass withdraws the provisional lease at 20 s, so the host's hand-back at 54 s is stale.
3. That is what round-9 Nit 6 and Minor 5 were about. The probe still fires (stale is an envelope answer), but every such lease waits about 34 s longer than it needs to, and the host holds its slot.

**Fix:** write the earlier deadline into `sdks/SPEC.md` §6.2 (`lease_time_left_ms` and `attempt_time_left_ms`), and into the give-up unit test of Task 006-059.

**Minor 4. The fallback of Decision 006-059 records `delivery-acknowledgement-v1` for both kinds. Task 006-057 still says "required of engines only" and asserts that the control plane does not list the capability.**

Confidence: confirmed in the documents.

**Where:** ADR decision 10 (`ADR-0099…md:146-148`, "if that request cannot be built into the floor, it is recorded for both kinds"), Decision 006-059 (`research.md`, the last sentences), and `tasks.md` (a) against Task 006-057 (`tasks.md:341-344`).

**Scenario:**
1. The follow-up of pull request #260 does not land, so the fallback applies.
2. The activation entry built by Task 006-057 records the capability for engines only. The control planes of this build do not list it, and the registry test asserts that they do not.
3. Either the requirement is recorded for engines only, and the stated exclusion of control planes that cannot replay the cause does not happen; or it is recorded for both kinds, and the activation's census refuses this build's own control planes.

**Fix:** make Task 006-057 conditional on the fallback. In the fallback branch, the control planes of this build list the capability, the entry records it for both kinds, and the registry tests assert both.

**Minor 5. The cap rows of the acceptance matrix and of Task 006-050 still cannot be set up through this build (round-9 Minor 7, carried over).**

Confidence: confirmed in the documents.

**Where:**
- `stream-completion.md:867`: "at the cap, a marked lease past its expiry is ended by one pass".
- `stream-completion.md:870`: "after four withdrawals a marked lease is ended by the pass".
- `tasks.md:271-273`: "after four withdrawals of the attempt … a marked lease"; "at the cap, the pass and the hand-back in both orders".

**Why:** after four withdrawals by this build, the column equals the attempt and the fifth claim is unmarked. Only the failure-table row says how such a lease exists.

**Fix:** in both places, write "four withdrawals written without the flag (through the floor build's statement, or by SQL leaving the column unwritten)".

**Minor 6. Now that the hash is kept for a `NOT_STARTED` withdrawal, a host that keeps the protocol can raise the violation alert itself.**

Confidence: plausible.

**Where:** `stream-completion.md:691` and `sdks/SPEC.md:369-371` (the heartbeat before the handler, "hands the lease back if that heartbeat fails"), with the hash and decision 12's list.

**Scenario:**
1. The heartbeat sent before the handler has its last attempt time out at the host.
2. The server, behind a proxy that does not pass on the cancel, is still waiting for the run lock.
3. The host's hand-back takes the lock first and withdraws (`NOT_STARTED`, hash kept).
4. The heartbeat then runs with the withdrawn token. It is counted and audited as a call after a withdrawal, and `SagatCallsAfterWithdrawal` fires, although nothing ran.

**Fix:** pick one.
- The host does not hand back while a heartbeat attempt for that lease may still be in flight at the server: it waits out the server's bound, as the give-up does for an acknowledgement in flight.
- Or name the false positive in Gap 006-042 and in the alert's runbook symptom.

## Nit

1. `research.md:637-638` (Decision 006-046) still has "with a call of 3 s … (6 − 3) × 2 / 3 = 2 s". It should say that the 3 s heartbeat-before-handler threshold is what makes the two fit.
2. `stream-completion.md:802`, the row "A report, heartbeat or capability call carrying the token of a lease withdrawn as `NOT_ACKNOWLEDGED`", should add "or on its hand-back", as decision 12 and the marks table now do.
3. The ADR's "Enforced by" (`ADR-0099…md:6`) reads "to be built with the decision, the attempt flag's allowed set in the static check among it, (`specs/…`)". The sentence is broken.
4. "Calls are ordered by their give-ups" (`sdks/SPEC.md:392`, item 5, Decision 006-065) still does not say which lease's give-up orders a call now that give-ups are per lease. Suggested: the last of its leases.
5. The parity test (`stream-completion.md:526-527`) still puts "the attempt withdrawn before or not" among the combinations of "the mark is current". That is a claim-time condition and not part of the macro (round-8 Nit 9, round-9 Nit 8).
6. "Losing it … only means more flags, which the cap bounds and then sets it again" (`stream-completion.md:458`). If the column is lost after the fourth withdrawal, no further withdrawal can set it, and the next marked lease ends `capped`. The outcome is the same as an unmarked lapse, but "sets it again" is false in that case.
7. The static check:
   - "a second statement that assigns it in the same transaction" cannot be checked by text; "any other statement" is what can be checked.
   - The check should accept the old-row columns either unqualified or qualified with the alias, because the scoped and path statements use `s.` (`execution.rs:7056`, `path.rs:1238`).
   - It should allow the three `INSERT`s to leave the column out.
8. The platform's per-connection layer: a layer added with `Server::layer` is built once and cloned per connection. A tower `ConcurrencyLimit` added there would share one semaphore across all connections, and a map keyed by peer address grows without an eviction point. Name the mechanism: a connection wrapper whose `Connected::ConnectInfo` carries the connection's own semaphore, dropped with the connection.
9. "answered with the server's envelope (`OK`, or a stale refusal)" (`sdks/SPEC.md:390`): an `OK` answer carries no error envelope. Write "answered `OK`, or refused stale with the envelope".
10. The proxy case ("cuts `AcknowledgeTasks` at 1 s while the server takes 2 s … no fallback") passes only if the probe is answered in under 1 s. Say that the injected 2 s applies to the acknowledgement of a held lease and not to the probe. Otherwise the third counted call falls back, which is arguably the right outcome for such a path.
11. The host case of round-9 Minor 4 (a late `ACKNOWLEDGED` arriving after the give-up is decided is never both run and handed back) is still not in Task 006-058 or Task 006-059.
12. Consequences:
    - "within the cap and while the requirement is recorded, a lease whose mark counts and that its host hands back while it is `LEASED` … is not charged an attempt" (`ADR-0099…md:241`) needs "before the pass ends it" (Gap 006-041).
    - "Bad" says twice that the liveness rule stops capability calls (`:250-251` and `:258`).

## Checks made against the code

- **The flag's form inside the three statements:** a plain `UPDATE` per row reads the old row. Each withdrawal is persisted per command inside the run's transaction. The attempt is set back by one in the same statement while the generation is kept. So the right-hand `attempt` is N, and a second write of a withdrawn row in the same transaction sees `READY` and keeps the value.
- **The cap as reported by the domain:** `withdraw_lease` and `withdraw_scoped` refuse at `before >= 4` (`crates/domain/src/execution/command.rs:1859`, `:1922`). "Reports as the last the cap allows" means `before == 3`, which the domain has to add. Task 006-050 implies this, but no task line says it.
- **The scoped flag:** scoped instances are rows of `steps` (`execution.rs:7003-7011`, `path.rs:1186-1194`), so one column serves root and scoped steps.
- **The floor's missing pieces:** the cause is absent (`event.rs:438-443`), and the clamp is present (`withdrawal.rs:232-234`, `:269-271`), as Decision 006-059 now says.
- **The retry on `REGISTRY_SILENT`:** it is safe to repeat. Step logs are stored at most once per attempt, generation and sequence (`specs/041-step-logs/contracts.md:23`). The other capability calls change nothing before the refusal, except the capability record, which leans the safe way (the hand-back becomes an ordinary failure).

## Evidence to ask for once built

- `cargo test -p sagat-tests --test integration_durable_execution`:
  - a withdrawal whose command touches a `LEASED` sibling at the same generation, with the sibling's flag unchanged (Minor 2);
  - the cap cases set up with the flag left unwritten (Minor 5).
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`:
  - a 15 s failover with a running task that heartbeats 20 ms before the failure and three flagged frames in flight, at 30 s: no early give-up, no review, no attempt spent (Major 1);
  - the 18 s variant expecting the outcome of Gap 006-047 (Minor 1);
  - a lease with `timeoutMs` = 20 s and a 60 s time-to-live, given up at about 14 s (Minor 3).
- The hosts' unit tests: no hand-back while a heartbeat before the handler may still be in flight at the server, or the false positive documented (Minor 6).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-9/ (correctness.md, egm-design.md; the headings of architecture.md and security.md)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/041-step-logs/contracts.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution/path.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/command.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/state.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round9-to-10.diff
