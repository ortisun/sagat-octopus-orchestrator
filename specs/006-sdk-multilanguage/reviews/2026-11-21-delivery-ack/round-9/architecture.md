I found no Blockers in this round: there are 2 Majors, 6 Minors and 7 Nits.

# Architecture review, round 9: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:**
- **Major 1 (the base lacks the requests made of it):** the text must be fixed before this design merges. The requests must be in the floor build before it merges into `develop`. Task 006-043 is already gated on this by precondition (a).
- **Major 2 (the revision of Decision 006-045 is only half applied):** the text must be fixed before this design merges, and before Task 006-050 starts.

**Round 8 is mostly answered:**
- My round-8 Major is **partly resolved**. The two hazards are gone from the contract and from Decision 006-066: the column could be erased, and it could hold the wrong attempt. What is left is in Minors 1 and 2.
- Minors 1, 3, 4 and 5 are resolved. Minor 2 (the witness type) is not resolved; it is Minor 3 below, with the shape I accept.
- My round-8 Nits 1, 2, 3, 5 and 7 are resolved, and Nit 9 is planned in Task 006-054. Nits 4, 6, 8 and 10 are partly resolved or not resolved; Nits 1 and 3 below cover two of them.

**The revision of Decision 006-045 is sound:**
- Withdrawing an acknowledged lease that is still `LEASED` with no capability served agrees with decision 5 of Architecture decision 0098. That decision asks only that the step be "still `LEASED`".
- It gives an acknowledged lease the same rule as an unmarked lease of 1.8.
- It stays within constitution principle III, under the trust assumption that decision 12 now states.

**Limits.** I had no shell, so I could not run `git branch -a` or `git worktree list`. I read the worktree's git metadata instead:
- this branch is `docs/delivery-acknowledgement`;
- its base is `origin/bugfix/withdraw-undelivered-leases`, merged three times;
- that branch's own worktree, `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases`, is on the same branch.
- A search of every `sagat-wt-*` worktree found Architecture decision 0100 only here, and no acknowledgement module anywhere.

I found no duplicated work.

---

## Status of the round-8 findings

| Round 8 | Status | Reason |
|---|---|---|
| **Major 1** (the attempt flag governed by the marks' rules) | **Partly resolved** | <ul><li>**Done in the contract:** the column has its own paragraph as attempt data, it is never cleared, its one form takes the stored attempt, and PostgreSQL's old-row evaluation is stated (`stream-completion.md:450-458`).</li><li>**Done in Decision 006-066:** the same rules are written there.</li><li>**Done in the static check:** it has its own allowed set and three negative tests (`stream-completion.md:504-506`).</li><li>**Done in Task 006-050:** it adds the re-hint, the `NOT_SENT` case and the cap case.</li><li>**Left (Minor 1):** the form lives in the very statement the claim executes, so "the claim leaves it out of its `SET` list" cannot be built as written. The old-row guard was dropped.</li><li>**Left (Minor 2):** Decision 006-042 still lists the column among the marks. Task 006-047's matrix and negative tests were not updated.</li></ul> |
| Minor 1 (the cap) | Resolved | The withdrawal that reaches the cap sets the column; there is a case in Task 006-050 and a row in the failure-modes table. The record's "Enforced by" still states the old claim (Nit 1). |
| Minor 2 (the witness type's layering) | **Not resolved**, by choice | `stream-completion.md:416` still says "a private constructor in the adapter". See Minor 3 for the shape I accept. |
| Minor 3 (planned hand-backs) | Resolved | They are named in Decision 006-066, Gap 006-047 and "Bad". The reason field is left to the owner. |
| Minor 4 (the refusal's status) | Resolved in Decision 006-059 (7), decision 10 and Task 006-046 | The contract paragraph is not updated (Nit 3). |
| Minor 5 (`conflict`) | Resolved | It is in `stream-completion.md:793-794`, with the coverage assertion. |
| Nits | 1, 2, 3, 5 and 7 resolved; 9 planned; 4 not resolved; 6, 8 and 10 partly resolved | <ul><li>Nit 1 is moot: the base replaced the prose index with a table.</li><li>Nit 9 is planned in Task 006-054.</li><li>Nit 4 is not resolved: decision 12 still leaves out the snapshot rolled back under a running server.</li><li>Nit 6: Minor 5 is about its new form.</li><li>Nit 8: the parity list still has the attempt-flag item.</li><li>Nit 10: see Nit 1.</li></ul> |

**Other reviewers' round-8 findings in my domain:**
- Security Nit 6 (request (8) in its migration) and security Nit 4 (`fsync` as an assumption) are resolved in the text. The build of request (8) is part of Major 1.
- Correctness Minor 3 and security Minor 3 (the deadline in the outermost gRPC layer) are resolved; the evidence is in Task 006-049.
- Design Major 2 (`CANCELLED` only for `FORWARD` tasks) is resolved across the record, the contract, the register and the tasks.

---

## Majors

### Major 1. The merged base lacks the hard preconditions this design says it carries

**Where:**
- the record's header line 4: "as amended on its branch … and the amendment of Architecture decision 0082 decision 5 … which is accepted and merged first";
- the Refines line 5: "it relies on that build's amendment of Architecture decision 0082 decision 5";
- `stream-completion.md:397-398`: "the floor build's control planes already decode the cause";
- `stream-completion.md:424-431`;
- Decision 006-059;
- precondition (a) (`tasks.md:139-145`);
- Task 006-045: "as the server build added it";
- Task 006-046: "as the floor build has it".

**What is wrong.** The base merged into this branch is the build of Architecture decision 0098 with its review fixes. Checked against that tree:

| Request | In the merged base? | Evidence |
|---|---|---|
| (2) the cause `NOT_ACKNOWLEDGED`, a hard precondition | **No** | `WithdrawalCause` has only `NotSent` and `NotStarted` (`crates/domain/src/execution/event.rs:438-458`). |
| (7) the liveness amendment, a hard precondition | **No** | There is no `REGISTRY_SILENT` and no refusal of capability calls in `crates/` or `apps/`. `crates/application/src/instances.rs:31-37` still implements decision 5 of Architecture decision 0082 unchanged. That record's Amendments section (lines 49-52) has no entry. |
| (8) `lease_withdrawal_required()` checks both kinds | **No** | The function returns `FOUND` for the row, whatever the kinds (`20270208000000_lease_withdrawal.up.sql:71-77`). |
| (6) the error text says "of unmarked leases" | **No** | Line 57 of the same migration. |
| (1) for a step with declared secrets | **Not found** | `checked_secret_report` is passed the cancellation allowance only for `CANCELLED` (`crates/application/src/worker.rs:2451-2459`), so a hand-back with a stop asked is refused as stale by `report_authority_fence`. |
| (5) the cap's derivation counts the causes of 1.8 | **No** | Decision 7 of the merged record does not say. |
| (3) the holder's minor clamped to the build's ceiling | **Yes** | `hand_back_ceiling` (`crates/application/src/worker/withdrawal.rs:229-237`). |

The sibling worktree of that branch has none of the missing requests either.

**Why it matters:**
- Precondition (a) stops Task 006-043, so the code cannot go wrong silently. But the record and the contract state as fact what the tree contradicts.
- Nothing tracks who adds (2) and (7) to pull request #260 before it merges.
- If #260 merges without them, (2) and (7) must move into this design. Without them this build has no rollback floor and no liveness argument.

**Smallest correction:**
- In Decision 006-059 and precondition (a), record each request's status against the merged head: present (3); missing (1) for the secret fence, (2), (5), (6), (7) and (8). Name pull request #260 as the place they must land.
- Change the header and the Refines line to "relies on, once built". At `stream-completion.md:397`, say "will decode".
- Add a fallback for the case where #260 merges first:
  - (2) moves into Task 006-045, and (7) into Task 006-046.
  - (6) and (8) become `CREATE OR REPLACE FUNCTION` in Task 006-044's migration. A later migration can replace a function, so the words "before that migration is applied anywhere" make them sound harder than they are.
- This is an owner item (item 1).

**Blocking:** the text before this design merges; the requests before #260 merges into `develop`.

### Major 2. The revision of Decision 006-045 is not carried through, and normative text still contradicts it

**Where:**
- `tasks.md:254-255`: "with the expiry condition lifted for a provisional lease; an acknowledged lease, … handed back: an ordinary failure". This contradicts lines 245-251 of the same task.
- `tasks.md:268-269`, Task 006-050's evidence: "the hand-back after `CANCELLED` (`stop_asked`, and `too_late` 1 ms past the expiry) of an acknowledged lease with declared secrets, an ordinary failure". The new rule withdraws it.
- `stream-completion.md:754`, failure table: "for an acknowledged lease an ordinary failure".
- `specs/037-compensation-safety/spec.md:153-155`, the first clarification of Requirement 037-006: "not acknowledged and with no capability served … decision 9 of Architecture decision 0099".
- The same file, line 162: "handed back after the verdict `CANCELLED`, before it was acknowledged". Line 166 then says the opposite.
- `research.md:805-806`, Decision 006-061: "and an acknowledged lease is an attempt".

**Why it matters:**
- Tests come first (rule 18). The evidence of Task 006-050 asserts both outcomes for the same case.
- Requirement 037-006 is a product requirement of another feature. An implementer checking compliance against it would refuse to withdraw an acknowledged lease.

**Smallest correction:** make each of these places say what decision 5 now says:
- a lease that is `LEASED`, with no capability served and a mark that counts, is withdrawn on its hand-back, acknowledged or not;
- a `RUNNING` lease, or one to which a capability was served, is an ordinary failure.

In Requirement 037-006, change the first clarification's condition to "not `RUNNING`, with no capability served". Then add one line to Task 006-067: "the Decision 006-045 wording checked in 037".

**Blocking:** before Task 006-050; the text before merge.

---

## Minors

### Minor 1. The attempt flag's form is placed in the statement the claim runs, and it lost its old-row guard

**Where:**
- `stream-completion.md:450-458` and `504-506`;
- Decision 006-066 (`research.md`, "It is written in one form, in the three statements that write a withdrawal … every other statement, the claim included, leaves it out of its `SET` list").

**What is wrong:**
1. **The claim and the withdrawal share the statement.** On the merged code the root withdrawal and the claim write the row through the same `UPDATE` of `upsert_step` (`crates/infra-postgres/src/execution.rs:7378-7421`), called once per touched node per append (`:5353-5362`).
   - "The claim leaves it out of its `SET` list" therefore cannot hold.
   - The negative test "a claim that assigns it" has no statement to apply to.
   - What actually protects the column on a claim is the `ELSE` branch with a false bound condition.
2. **The condition is a bound flag with no row fence.** Round 8 proposed `AND status = 'LEASED' AND generation = <fence>` against the old row; it was dropped.
   - `withdraw_root` runs `engine.run` once per lease of a run in one transaction (`crates/application/src/worker/withdrawal.rs:363-404`). A later append can touch a row already withdrawn earlier in the same transaction.
   - If the condition is derived from the state rather than from that append's own effects, `attempt` on the right is then N − 1 and the bound breaks.
3. **The cap is decided twice.** "A withdrawal that brings the attempt's count to four" re-derives the cap outside the withdrawal command, the one place considered option 7 keeps it.
4. **One case is ambiguous.** "A lease whose mark counts" does not say whether an unflagged 1.8 lease's hand-back (only `lease_hand_back_generation`) sets the column.

**Fix:**
- Say the form is in each of the three statements for every caller. A caller other than a withdrawal binds the condition false. Every statement other than these three must not name the column. The static check tests exactly that, and the "claim that assigns it" negative test becomes "a form whose condition refers to a new lease".
- Make the form fence itself:

  ```sql
  provisional_withdrawn_attempt =
      CASE WHEN $flag AND status = 'LEASED' AND generation = $fence
           THEN attempt
           ELSE provisional_withdrawn_attempt END
  ```

  Add a test with two leases of one run withdrawn in one transaction, where the second's settle touches the first's row.
- Have the domain's `withdraw_lease` return whether this withdrawal is the last allowed one, and bind that.
- Say "flagged at its claim" (`lease_offered_generation` set).

**Blocking:** before Task 006-044 (the static check) and Task 006-050.

### Minor 2. Residues of round-8 Major 1 in the register and the tasks

- **Decision 006-042** (`research.md:576-584`) still lists `provisional_withdrawn_attempt` among "the marks … bound to the lease's generation", which "count while the step holds a lease". That contradicts Decision 006-066. Remove it from that list.
- **Task 006-047's matrix** (`tasks.md:205`) still reads "the attempt withdrawn before or not". Round 8 asked for the column at N, N − 1 and null against a claim of N.
- **Task 006-047's static check** (`tasks.md:208`) lists only the bound-flag and read-back negative tests. The attempt flag's three negative tests are in the contract and in Task 006-054's fitness row, but in no task's evidence. Add them to Task 006-047, which builds the check.

**Blocking:** before Task 006-044 and Task 006-047.

### Minor 3. The witness type still cannot be built as written. The shape I accept

**Where:** `stream-completion.md:416-418`; `tasks.md:243`.

**What is wrong.** The text still says "a private constructor in the adapter", and the application's decision function takes the type.
- On the merged base the port method is `lease_withdrawal_required(&mut self) -> Result<bool, StoreError>` on the application's transaction traits (`crates/application/src/execution.rs:904`, `:1589`).
- An application function cannot name a type defined in `crates/infra-postgres` (layers point inwards).

**The shape I accept:**
- An associated type on the port. For example, on the run and path transaction traits:
  - `type Requirements: RequirementsHeld;`
  - `async fn delivery_acknowledgement_required(&mut self) -> Result<Self::Requirements, StoreError>;`
- `RequirementsHeld` is a trait of `crates/application` with read-only accessors.
- The PostgreSQL adapter's concrete type has a private constructor.
- The decision function and the withdrawal command are generic and take `&Tx::Requirements`.
- The pure function's unit and parity tests implement the trait with a test type.

**What this proves, and what it does not:**
- It proves that a withdrawal path holding `Tx` cannot decide without calling the read.
- It does not prove that the witness is from the same transaction. That remains the job of the barrier tests where a removal is held between the read and the commit.

Write this shape into the contract now rather than leaving it to Task 006-050. If it proves awkward, the fallback is a static check that the constructor is called only in one file of `crates/infra-postgres`.

**Blocking:** before Task 006-050.

### Minor 4. The cap clause is folded into an alert that does not exist, and two alert clauses have no series

**Where:**
- `stream-completion.md:808-813`;
- Task 006-054: "six alerts (the capped clause folded into Architecture decision 0098's alert …)".

**What is wrong:**
- Neither the merged base nor `infra/observability` defines `SagatLeaseAttemptCapped`. The name appears only in this design and its reviews. As written, the cap of a marked lease is watched by no alert.
- `SagatAcknowledgementsFailing` excludes "those of a `FORWARD` lease with a stop (labelled `stop`)", and counts `NOT_STARTED` withdrawals "of marked leases". `worker_leases_undelivered_total{cause, outcome}` has neither a mark label nor a `stop` label, and no other listed metric does. The closed vocabulary is incomplete.

**Fix:**
- Own `SagatLeaseAttemptCapped` here, or add it as request (9) to the floor build.
- Name the series. For example, add a bounded label `mark` (`marked`, `unmarked`) and `stop` (`true`, `false`) to the counter in this build. That is an additive change: the floor build's sums by `cause` and `outcome` are unchanged.
- Put both in Task 006-054's telemetry test.

**Blocking:** before Task 006-054.

### Minor 5. The fixed 5 s margin makes valid configurations refuse to start

**Where:** Decision 006-059 (7): "an engine whose configured bound exceeds it refuses to start".

**What is wrong:**
- The processes' statement timeout is configurable (`crates/infra-postgres/src/instances.rs:44-45`). An engine of 0.2 configured above 5 s would refuse to start after the upgrade. That is a break of an older configuration.
- The registry has a connection of its own (decision 8 of Architecture decision 0082), so the beat's bound need not follow the request pool's.

**Fix:** set the registry connection's own statement timeout to at most 5 s, so the margin holds by construction, and refuse nothing. State it in request (7) and in section 3 of the instance-registry runbook. This is an owner-visible operations change of the floor build (item 4).

**Blocking:** before the floor build merges.

### Minor 6. The new rule has no race test against the pass

**Where:**
- Decision 006-045 revised;
- the withdrawal table row "`LEASED`, provisional or acknowledged … whether or not a deadline passed";
- Task 006-051 (`tasks.md:273-281`).

**What is wrong:**
- An acknowledged lease past its expiry is withdrawn on its hand-back but ended by the pass with `expire_attempt`, which means a review. The first holder of the row decides between no review and a review.
- Task 006-051 has "a hand-back 1 ms after the expiry with the pass held … and the same after `CANCELLED`", which does not say whether the lease is acknowledged. Task 006-050 has both orders only at the cap.
- Rule 9 asks for the race between a timeout and a signal to be tested.

**Fix:** add to Task 006-051: an acknowledged `LEASED` lease 1 ms past its expiry, with the hand-back's withdrawal and the pass in both orders, root and scoped, asserting the journal (`step.lease_withdrawn` or `expire_attempt`) and exactly one outcome.

**Blocking:** before Task 006-051.

---

## Nits

1. **"Enforced by" and the Consequences overclaim** (record lines 6 and 231-235):
   - "Enforced by" still says "no attempt spent by any failure of the acknowledgement path". It should say "no withdrawal cycling", and name the attempt flag's own allowed set.
   - "A lease its host did not start is never charged an attempt … while it is `LEASED`" is false in three cases: at the cap, before the activation, and for a mark that does not count. Add "within the cap, while the requirement is recorded".
2. **The index table:**
   - The row of 0100 gives the status as "(the number is retired)", while the table promises "the status written at its top (shortened to its first clause)", which is "Superseded by Architecture decision 0099 (2026-11-21)".
   - The heading still begins "Withdrawn:".
   - Nothing checks that the table lists every file and agrees with its Status. The smallest check is in `scripts/check-agent-config.py`: one row per `ADR-NNNN-*.md`, and the row's status starts with the file's first clause.
3. **The floor engine's refusal in the contract:**
   - `stream-completion.md:424-431` and Task 006-046 (`tasks.md:190`) still say "at the hand-over, after the capability record commits". The floor build writes no capability record.
   - Neither names `REGISTRY_SILENT`. Use the wording of Decision 006-059 (7), and name the counter that "counted" means.
4. **The probe is counted with forged tokens.** The probe's `NOT_HELD` is counted as `no_match`, together with forged and old tokens, so probe traffic looks like probing by an attacker. Count it apart: a server reason such as `withdrawn` when the step row is `READY` at the request's generation, or an SDK hook.
5. **The attempt flag is not in the journal.** `provisional_withdrawn_attempt` is execution input that the journal does not record. Say that losing it (a projection rebuilt from the journal) only means more flags, bounded by the cap, which sets it again.
6. **A mixed-fleet case.** A floor engine's withdrawal at the cap, inside the learning window of Gap 006-044, does not set the column. Name it in Gap 006-044 or Gap 006-047.
7. **Decision 12 is still incomplete.** Round-8 Nit 4 is not resolved: decision 12 of the record (lines 155-168) still leaves "a storage snapshot rolled back under a running server" out of its list of restores. The contract, Gap 006-048 and the runbook list it.

---

## The checks you asked for

- **The merge with the moved base:**
  - Architecture decision 0098 as merged names 1.8 and Architecture decision 0099 in its decisions 5 and 8. That agrees with this design's refinement: the reservation is read from the lease, and the ceiling is clamped.
  - Its decision 5's condition "the step is still `LEASED`" agrees with the revised Decision 006-045.
  - Its build lacks requests (1) for the secret fence, (2), (5), (6), (7) and (8) (Major 1), and has no capped alert (Minor 4).
  - The index rows for 0099 and 0100 match the headings. The 0100 row's status wording drifts (Nit 2).
- **The witness type:** accepted shape in Minor 3.
- **Decision 006-045 revised, against decision 5 of Architecture decision 0098 and the constitution:**
  - It is consistent with both. Principle I holds because the first commit wins under the row lock. Principle III holds under the trust assumption of decision 12, which now covers "hand back only what it did not start".
  - It is incomplete in the documents (Major 2) and has no race test against the pass (Minor 6).
  - It widens the trust assumption, so it is an owner item.
- **Coherence:** the record, `sdks/SPEC.md` §5, §6.2, §6.3 and §9, and "Leases that never reach a host" rule 5 agree. The exceptions are Major 2, Minor 2 and Nit 3.
- **Identifier forms:**
  - New prose uses the full forms.
  - Short forms remain in the register's entry lines and in text inherited from the base (`D006-030` in `sdks/SPEC.md` §6.2, `OD-2026-11-20-6` in the merged record and in Requirement 037-006's first clarification). These are tool-read or the base's own text.
  - I found no refused abbreviation in the added lines.
- **Compatibility:**
  - The contract only grows.
  - The migration is additive, with a guarded `down`.
  - The column needs no `down` step of its own.
  - The one regression is Minor 5's refusal to start.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| An attempt is flagged at most once after a hand-back, and never at the cap | The attempt flag's allowed set in the static check; Task 006-050's cases | The old-row fence and a test with two withdrawals in one transaction (Minor 1); negative tests in Task 006-047 (Minor 2) |
| No withdrawal without the requirements read | The witness type | The associated-type shape (Minor 3) |
| A lease that has not run is withdrawn on its hand-back, acknowledged or not | Task 006-050's evidence | The evidence made consistent (Major 2); the race against the pass (Minor 6) |
| `NOT_ACKNOWLEDGED` replays on the rollback floor | Precondition (a) | Status recorded; a fallback task (Major 1) |
| A floor engine silent at the census serves no capability call | Task 006-046; the floor build's own test | Not built yet (Major 1); the registry's own timeout (Minor 5) |
| The cap of a marked lease is alerted | none | Own the alert, or request (9) (Minor 4) |
| Closed vocabulary of the alert clauses | Telemetry test (Task 006-054) | The `mark` and `stop` labels defined (Minor 4) |
| The index agrees with each record | none | A row-per-file and status check (Nit 2) |
| A pending runbook marker is closed with its task | Planned in Task 006-054 | none |

## Items for the owner

1. **Pull request #260 must carry requests (1) for the secret fence, (2), (5), (6), (7) and (8) of Decision 006-059 before it merges into `develop`.** Otherwise this design takes them over. Request (7) amends decision 5 of Architecture decision 0082 for every deployment of the floor build.
2. **The revised Decision 006-045.** An acknowledged lease that its host hands back while it is `LEASED`, with no capability served, is withdrawn rather than charged an attempt. It rests on the same unverifiable promise of the host (decision 12). It also changes how Requirement 037-006 reads.
3. **Accept Architecture decision 0099 with both assumptions of decision 12.** The second now names `fsync` on.
4. **Liveness operations.** Should the engine refuse to start when its configured statement timeout is over 5 s, or should the registry connection carry its own 5 s bound (Minor 5)?
5. **The bound after a hand-back.** Planned hand-backs use the attempt's one flag. A reason field on the hand-back is the considered alternative (Gap 006-047).
6. **Still open from round 8:**
   - Owner question 006-002 (the minor 1.8 reservation);
   - the delegate's thresholds;
   - retiring 0100 under the status "Superseded";
   - the constraints that stay (no emergency stop, two restarts for a rollback);
   - the release coupling for 0.3.0.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (lines 4-6, decision 5, Consequences)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (decisions 5, 7, 8)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md (decisions 5 and 8; Amendments)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md (lines 51-53, 137-139)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (397-431, 450-458, 504-506, 754, 808-813)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-042 at 576, Decision 006-059 at 769, Decision 006-061 at 803, Decision 006-066)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (139-145, 190, 205-208, 245-269, 273-281, 299)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md (151-168)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs (438-458)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql (57, 71-77)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (5353-5362, 7378-7421)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs (229-237, 363-404)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (2451-2459, 2909-2934)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/instances.rs (31-37)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/instances.rs (44-45)
