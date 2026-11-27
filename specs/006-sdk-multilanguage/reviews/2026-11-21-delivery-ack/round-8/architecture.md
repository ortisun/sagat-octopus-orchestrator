# Architecture review, round 8: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**No Blockers. One Major.** It blocks Task 006-044 and Task 006-050, the tasks that create and write the new column. It does not block merging this design once the text is corrected. Counts: 0 Blockers, 1 Major, 5 Minors, 10 Nits.

- **Round 7 is answered.**
  - My Major 1 is resolved.
  - Minors 2, 3, 5, 6, 7 and 8 are resolved. Minors 5 to 8 are resolved by removing what they were about.
  - Minor 1 is resolved by the server's error envelope, which exists in code (`crates/api-grpc/src/status.rs:9,16-45`; admission and the request timeout go through `to_status`, `crates/api-grpc/src/limits.rs:673-691`).
  - Minor 4 is resolved only in intent. The witness type, as written, cannot be built without breaking the layers (Minor 2 below). The defect came from my own round-7 wording.
- **The breaker was rightly withdrawn.** A bound on the row needs no observation, has no state per process, has no effect across tenants and needs no thresholds. Decision 006-066 is the better design.
- **The new column is the Major.** `provisional_withdrawn_attempt` is now what keeps "no failure of the acknowledgement's path spends an attempt" true. But it sits in the marks table, and three things there contradict what the column needs:
  - the table says every mark is "written for a lease's generation";
  - the allowed forms let other statements "clear them with the lease";
  - the pass "clears the lease and its marks".

  The static check would accept the form that breaks the bound. Nothing says the claim keeps the column, or which attempt number it stores given that the withdrawal sets the step's attempt back by one.
- **Everything else holds together end to end.** I checked the decision record, the contract, `sdks/SPEC.md`, the register and the tasks. The liveness amendment is in the right place, and precondition (a) now carries it.

**Limits.** I had no shell, so I could not run `git branch -a` or `git worktree list`. A glob over `sagat-wt-*` found Architecture decision 0100 only in this worktree. The floor-build branch has no `AcknowledgeTasks`, `lease_claim_incarnation` or `provisional_withdrawn_attempt` under `crates/` or `apps/`. I found no duplicate work.

---

## Status of the round-7 findings

### My Major

| Round 7 | Status | Reason |
|---|---|---|
| Major 1: the liveness rule's record, precondition, scope and boundary | **Resolved** | <ul><li>**Record.** It is now request (7) of Decision 006-059, an amendment of Architecture decision 0082 decision 5 carried by the build of Architecture decision 0098, with an entry in Architecture decision 0082's Amendments section at acceptance. That follows the precedent of Architecture decision 0083. The decision record's Refines line no longer claims Architecture decision 0082 directly; the README index still does (Nit 1).</li><li>**Precondition.** Precondition (a) lists (1) and (3) to (8), and (7) is named a hard precondition (`tasks.md:139-145`).</li><li>**Scope.** It is narrowed to capability calls. Heartbeats, reports, hand-backs and claims keep flowing, with the reasons given (`stream-completion.md:410-417`).</li><li>**Boundary.** It is measured from the start of the last successful beat with a margin. "A beat succeeds" now means the beat plus the gate's decision under the epoch it returned. The gate is checked again at the hand-over.</li><li>**Test.** The boundary test with the beat's answer held is in Task 006-046 and in the released-v0.2 row of the acceptance matrix.</li><li>**Runbook and cost.** Section 3 of the instance-registry runbook is named, and the availability cost is in the Consequences.</li><li>**Residues** (Minor 5, Nits 6 and 7): the status of the refusal is not named; the hand-over point "after the capability record commits" does not exist on the floor build; the 25 s is tied to a bound that can be configured.</li></ul> |

### My Minors and Nits

- **Minor 1 (the server's reasons): resolved.** One rule: the envelope `x-sagat-error`. It is written the same way in `sdks/SPEC.md` §5 and §6.2, host item 5, Decision 006-065, and the test of Task 006-049. A gateway's or a mesh's status carries no envelope.
- **Minor 2 (silent wrong order): resolved.**
  - The skip reason `requirement_missing` comes right after `not_activated`.
  - `SagatDeliveryAcknowledgementInactive` watches it.
  - There is a row in the failure-modes table.
- **Minor 3 (process kinds): resolved.**
  - `lease-withdrawal-v1` is read as recorded only for both kinds, and `delivery-acknowledgement-v1` for engines.
  - Request (8) asks the same of `lease_withdrawal_required()`.
  - Task 006-044 tests each recording: both kinds, engines only, none.
- **Minor 4 (structural check of the read): partly resolved.** The rule is now a type the decision requires, and it is in the fitness table of Task 006-054. As written it cannot be built (Minor 2).
- **Minors 5 and 6 (`end-lease-marks`, `platform_operator_actions`): resolved by removal.**
  - A restart of PostgreSQL after a restore that does not start the server does what the command did, with no table, function or census. The drill checks it.
  - Architecturally this is better: no deployment-level operator table without an owner decision, and no `down` ambiguity.
- **Minors 7 and 8 (the breaker's home, and the cost gate with the breaker open): resolved by removal.** Nothing in Decision 006-066's bound needs a home per process.
- **Nits:**
  - Nit 1 is resolved (a stub and an index line), with a vocabulary residue (Nit 2).
  - Nits 2, 3, 5, 6 and 7 are resolved.
  - Nit 8 is moot.
  - **Nit 4 is not resolved.** No check fails when "(pending Task 006-054" stays after that task is ticked. The runbook test went with the command (Nit 9).

### Other reviewers' round-7 findings in my domain

| Finding | Status |
|---|---|
| Security Major 1 (a flood holds the breaker closed; leases cycle to the cap) | **Resolved** by Decision 006-066, except the cap case (Minor 3) and the clearing hazard (Major 1) |
| Security Minors 1, 3; design Major 2, Minor 4; correctness Minors 4, 5, 8, 9 | **Moot** (breaker, command and table removed) |
| Security Minor 2, correctness Minor 3, design Minor 1 (the reasons) | **Resolved** (the envelope) |
| Security Minor 4, correctness Minors 1 and 2, design Major 3 (liveness) | **Resolved** (see my Major above) |
| Security Minor 5 (process kinds) | **Resolved** |
| Correctness Major 1, design Major 1 (cycling to the cap) | **Resolved in design** by Decision 006-066; the column's rules must be fixed (Major 1) and the cap case closed (Minor 3) |
| Correctness Minor 6 (timeouts set once) | **Resolved**: set once to 1 s, a call ends at most 1 s past its deadline (4 s), and the test is "within 4.5 s" |
| Design Minors 5, 6 (`fsync`, one writable server) | **Resolved**: the `fsync` condition is dropped with its reasoning, and one writable server is stated in decision 12, the contract and the runbook |
| Security Nit 1 (audited once per step and generation) | **Resolved** (`stream-completion.md`, failure-modes row) |

---

## Major

### Major 1. `provisional_withdrawn_attempt` is governed by the marks' rules, which let the lease-clearing statements erase it and leave its value ambiguous; the static check would accept the form that breaks the bound

**Where:**
- `stream-completion.md:421-432`: the table header "all written for a lease's generation", and the new row at 431.
- `stream-completion.md:474-481`: the allowed forms, "the engine's other statements keep them (`ELSE` the column) or clear them with the lease"; "the withdrawal sets `withdrawn_lease_token_hash` and `provisional_withdrawn_attempt` from the row it holds".
- `stream-completion.md:482-483`: "a mark left by a binary that does not know the columns never counts, because the next lease has the next generation".
- `stream-completion.md:563`: the pass "the lease and its marks cleared, `withdrawn_lease_token_hash` kept, `provisional_withdrawn_attempt` set".
- Decision 006-042 (`research.md:456`), which lists it among the marks.
- Decision 006-066 (`research.md:727`).
- Architecture decision 0099 decision 5 (lines 92-104).
- Task 006-044 (`tasks.md:172`), Task 006-047 (`tasks.md:199`), Task 006-050 (`tasks.md:240`).

**What is wrong.**

1. **The withdrawal is not an update of its own.** On the floor build the withdrawal is an event applied to the run state:
   - `withdraw_lease` sets the step `READY` and the attempt to `attempt - 1` (`crates/domain/src/execution/state.rs:1474-1479`).
   - The row is then written by the projection statements, the same three the claim uses: `upsert_step`, `upsert_scoped_instance` and the path statement (`crates/infra-postgres/src/execution.rs:7378-7421`).
   - Those statements bind `attempt = $7` from the projection, which after the set-back holds N − 1.
   - They clear lease data with `CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL …`, as `lease_expires_at`, `attempt_deadline_at` and `leased_by` show.
2. **Erasure (the bound breaks).** The column sits among the marks, and the marks may be "cleared with the lease". An implementer who follows the pattern clears it on any later write of the `READY` row. Examples:
   - a re-hint (`hint_count`, `last_hinted_at`);
   - a `NOT_SENT` withdrawal, or an unmarked `NOT_STARTED` withdrawal, of the second lease, which Decision 006-066 says does not set the column;
   - any settle that rewrites the row.

   The next claim then flags again, and leases cycle to the cap of four. At the cap the pass ends the attempt with `expire_attempt` (an unknown outcome and a review), or a hand-back spends the attempt. That is the round-7 correctness Major 1 and security Major 1, reopened. The static check accepts this, because "clear them with the lease" is an allowed form.
3. **The wrong attempt number (the bound breaks silently).** "Sets it to the attempt" does not say which attempt:
   - the withdrawn lease's attempt N, which is the old row's `attempt`, or the event's `attempt`;
   - or the step's attempt after the set-back, N − 1, which is `$7`.

   Only N works: the claim compares the column with the attempt it claims, N again. Binding `$7` is the natural form in these statements, and it breaks the bound with every test of the marks still green except Task 006-050's.
4. **"From the row it holds" contradicts the rule above it.** "No statement assigns a column of the marks from a value it read earlier." A value read under `FOR UPDATE` and then bound is exactly the read-back form the static check must refuse. The form that is allowed has to be spelled out as SQL. The condition that it is a provisional withdrawal must come from the decision (a bound cause), since a `NOT_SENT` withdrawal of a provisional lease must not set it. So it is necessarily a bound-flag form, which the check today forbids for the claim. The check needs its own allowed set for this column.
5. **The claim's own form.** "The claim's [assignment], in SQL conditioned on a new lease" covers every mark. Nothing says the claim keeps this column. A claim that resets the marks of a new lease would erase it at the very moment it must be read.
6. **The tests miss it.** Task 006-050's evidence ("a provisional lease withdrawn once and the next lease of its attempt unflagged") passes if no write of the `READY` row happens between the withdrawal and the next claim. That is the usual case in a test, but not in production, where re-hints happen.

**Why it matters.** Decision 006-066 replaced the breaker. It is now the only thing that makes "no failure of the acknowledgement's path can spend an attempt" true (Consequences, line 227). A rule that, followed literally, removes it is a broken precondition of the build.

**Smallest correction:**
- **Take the column out of the marks table.** Give it a row of its own as attempt-scoped data, not lease data: not bound to a generation, not cleared with the lease, never cleared at all. A later attempt number never equals it, and the `down` drops it.
- **State its one writing form, in each of the three statements:**

  ```sql
  provisional_withdrawn_attempt = CASE WHEN <bound withdrawal cause NOT_ACKNOWLEDGED, or NOT_STARTED of a lease whose provisional mark counts>
                                            AND status = 'LEASED' AND generation = <fence>
                                       THEN attempt              -- the old row's value: the withdrawn lease's attempt
                                       ELSE provisional_withdrawn_attempt END
  ```

  Say that PostgreSQL evaluates `SET` against the old row, so `attempt` here is N even though `attempt = $7` writes N − 1 in the same statement. Say also that every other statement, the claim included, keeps the column.
- **Static check.** A per-column allowed set in `scripts/check-architecture.py`, with negative tests for:
  - a clear-with-lease form on this column;
  - `= $n` (a bound attempt);
  - a claim that assigns it.
- **Task 006-050.** Between the withdrawal and the next claim, add a write of the `READY` row (a re-hint). Then add a `NOT_SENT` withdrawal of the second lease and a third claim. All of them must be unflagged, root and scoped.
- **Task 006-047 matrix.** Add the column at N, N − 1 and null against a claim of N.
- **Say the direction is safe.** A path that resets a step's attempt on the same row can only make the column match a later attempt, which means fewer flags.

**Blocking:** before Task 006-044 (the schema and its static check) and Task 006-050. The text fix is small and should land before this design merges.

---

## Minors

### Minor 1. At the cap, a flagged lease buys no protection and adds a way to spend the attempt; the Consequences overclaim

**Where:**
- `stream-completion.md:737`: "the claim keeps marking" at the cap.
- Decision 006-066: "A `NOT_SENT` withdrawal does not set it".
- Architecture decision 0099 decision 5 (lines 97-99) and Consequences (line 227): "no failure of the acknowledgement's path can spend an attempt".

**What is wrong.**
- After four withdrawals of an attempt, none of them provisional (four `NOT_SENT`, or unmarked `NOT_STARTED`), the fifth lease is flagged, because the column does not hold the attempt.
- That lease can never be withdrawn, since the withdrawal command refuses a fifth. So its flag protects nothing.
- If its acknowledgement then fails on the server's side, the host hands it back and the hand-back is an ordinary failure, or the pass ends it with `expire_attempt`. The attempt is spent by a failure of the acknowledgement path.
- Unflagged, the same lease would simply have run. The case is rare, but the record states the opposite as a guarantee.

**Fix.**
- The withdrawal command already knows `withdrawn_before` (`crates/domain/src/execution/command.rs:1855-1864`). Let the withdrawal that reaches the cap, whatever its cause, also set the column, with the same form as Major 1.
- The column then means "do not flag this attempt again". That costs no statement and makes the Consequences true.
- Add one case to Task 006-050: four `NOT_SENT` withdrawals, then the fifth claim is unflagged.

### Minor 2. The witness `RequirementsHeld` "with a private constructor in the adapter" cannot be taken by the application's decision function without the application depending on the adapter

**Where:**
- `stream-completion.md:402-404`;
- Decision 006-047;
- Task 006-050 (`tasks.md:236-240`);
- the decision record's "Enforced by";
- the fitness table of `docs/engineering/architecture-principles.md` (Task 006-054).

**What is wrong.**
- The decision function is in `crates/application`: it extends `LeaseDeadlines`, and the withdrawal command is in `crates/application/src/worker/withdrawal.rs`.
- A type whose constructor is private to `crates/infra-postgres` must be defined there, and the application cannot name it. Layers point inwards (architecture principles line 15, enforced by `check-architecture.py`).
- Defined in the application instead, its constructor must be public for the adapter to call it, and then the type enforces nothing.
- So an implementer will either break the layers or build a type that checks nothing. My round-7 wording produced this.

**Fix.** Make the witness an associated type of the store port.
- For example, `trait WithdrawalTx { type Held: RequirementsRead; async fn read_requirements(&mut self) -> Result<Self::Held, StoreError>; … }`, with the accessors on a trait defined in the application.
- The decision function and the command are generic over the transaction and take `&T::Held`. A path that holds a `T` can obtain a `T::Held` only from `read_requirements`, because the adapter's concrete type has a private constructor.
- Tests use a test adapter's own type.
- Add the row to the fitness table naming this shape. If it proves awkward with the pure function's parity test, fall back to a static check that the witness's constructor is called in one file of `crates/infra-postgres`.

### Minor 3. Every hand-back of a provisional lease uses the attempt's one flag, including a host's graceful stop, a declining host and a `CANCELLED` verdict; the cost is not stated

**Where:**
- Decision 006-066 ("whatever fails on the acknowledgement's path …");
- `stream-completion.md:673-674` (a stopping host "hands back the rest");
- `sdks/SPEC.md` §6.2 (a host that declines hands back what waits);
- Gap 006-047;
- Consequences "Bad".

**What is wrong.**
- The server sees one code, `TASK_NOT_STARTED`, so it cannot tell a failure of the path from a planned hand-back.
- A rolling restart of a worker fleet hands back every provisional lease in flight. Each of those attempts then has its next delivery unprotected, at the moment a further stream break is most likely.
- That is a defensible trade-off: the alternative is a reason on the hand-back, an additive field. But the record presents the bound as costing "at most one give-up", which describes only failures.

**Fix.**
- Name the planned hand-backs (stop, decline, `CANCELLED`, `too_late`) in Decision 006-066 and Gap 006-047, and add one sentence to "Bad".
- Report the share of `withdrawn_before` skips (see Nit 3 for the name) in Task 006-056 and in the worker-rollout symptoms of `docs/runbooks/sdks.md`.
- Optionally add, as a considered option, a hand-back reason that tells the server not to set the column.
- This is an owner item.

### Minor 4. The refusal under request (7) names no status or reason, and its hand-over point does not exist on the floor build

**Where:**
- `stream-completion.md:410-417`;
- Decision 006-059 (7);
- Task 006-046;
- Architecture decision 0082 decision 3, whose closed gate answers `UNAVAILABLE`.

**What is wrong.**
- "Serves no capability call" does not say what the caller receives. The SDKs retry a capability call by the general rule of §5, at most four attempts. So whether a 25 s registry silence spends attempts depends on that answer. A typed error is required (`AGENTS.md` rule 10).
- "Checked again at the hand-over, after the capability record commits" describes this build. The floor build, where the rule matters, writes no capability record.

**Fix.**
- `UNAVAILABLE` with `x-sagat-error` set to a stable value, either the closed gate's `BINARY_CAPABILITY_MISSING` or a new `REGISTRY_SILENT`, carrying `retry-after-ms`.
- Count it with a closed reason.
- Phrase the hand-over as "before the first byte of the answer is sent (after the capability record commits, in this build)".
- Assert the status in Task 006-046's test.

### Minor 5. The failure counter's closed list misses outcomes the call can end with

**Where:** `stream-completion.md:758-759`, `worker_task_acknowledgement_failures_total{reason}` (`shed`, `deadline`, `lock_timeout`, `statement_timeout`, `store`), against item 7 of the server's acknowledgement (`stream-completion.md:883`): a deadlock, a serialization failure, a lock or statement timeout, the call's deadline, or an unavailable store.

**What is wrong.** A deadlock and a serialization failure have no label. An implementer will either drop them or invent one, which breaks the closed vocabulary (architecture principles line 26).

**Fix.**
- Add `conflict`, for a deadlock or a serialization failure.
- Let the telemetry test of Task 006-054 assert that every `UNAVAILABLE` and `RESOURCE_EXHAUSTED` the handler returns is counted under one of the listed reasons.

---

## Nits

1. **The README index line for Architecture decision 0099** (`docs/adr/README.md:51`) still ends "refines … Architecture decision 0082 …". The record's Refines line no longer says that. The line also omits the bound of once per attempt. Align both.
2. **"Withdrawn" is not a status of the template.** `docs/adr/TEMPLATE.md:3` lists Proposed, Accepted and Superseded. The checker accepts any Status line (`scripts/check-agent-config.py:215`). Either add "Withdrawn (date)" to the template and to rule 13 of the README, or write `Superseded by Architecture decision 0099 (folded in as considered option 10)`, which uses the existing vocabulary. The round-6 reviews link the stub's old file name; they are historical, so leave them.
3. **The skip reason `withdrawn_before` collides with the domain's `withdrawn_before: u32`.** In the domain, that parameter is the attempt's journal count of withdrawals, the input of the cap (`crates/domain/src/execution/command.rs:1855`). An implementer may wire the skip reason to the cap count. Rename it, for example `flagged_before`.
4. **Decision 12 of the record** (lines 165-167) lists the restores that do not start the server without "a storage snapshot rolled back under a running server". The contract (`stream-completion.md:460`), Gap 006-048 and the runbook (section 7, step 5) list it.
5. **The header** says the design follows Architecture decision 0098 "as amended on its branch (decisions 5, 7 and 8 …)". Add the liveness amendment, request (7), which is now a hard precondition.
6. **The 25 s margin** is "the liveness window less the beat call's bound". That bound comes from the processes' statement timeout, 5 s by default and configurable (`crates/infra-postgres/src/instances.rs:44-45`). State a fixed 5 s margin, or derive it and refuse to start when it is not positive.
7. **Section 3 of `docs/runbooks/instance-registry.md`** is assigned twice: request (7) asks the floor build to update it, and Task 006-046 updates it again. Give it one owner, the floor build, and let Task 006-046 check it.
8. **The parity test** lists "the attempt withdrawn before or not" (`stream-completion.md:487`). But the macro decides whether a mark counts, and the claim's marking condition has no twin as a pure function. Either give it one, or leave it to Task 006-047's matrix and drop it from the parity list.
9. **Round-7 Nit 4 is still open.** Nothing fails if "(pending Task 006-054" stays in `docs/runbooks/disaster-recovery.md:92` after the task is ticked. The smallest check is a rule in `scripts/check-design-docs.py`: a "(pending Task NNN-xxx" marker whose task is ticked fails.
10. **"Enforced by"** could name the static check's per-column allowed set (Major 1) and the cap case (Minor 1) once they exist.

---

## The checks you asked for

- **The new column against every writer:**
  - claim: must keep it, unstated;
  - acknowledgement: does not touch it;
  - capability paths: do not touch it;
  - `NOT_ACKNOWLEDGED` withdrawal: sets it, form unstated;
  - `NOT_STARTED` of a provisional lease: sets it, form unstated;
  - `NOT_SENT` and unmarked `NOT_STARTED`: must keep it, and the "clear with the lease" rule says otherwise;
  - completion, failure and `expire_attempt`: harmless either way, because the attempt changes;
  - binaries of the floor build: keep it by not knowing it;
  - `down`: drops it.

  The static check as described has one set of forms for all marks and accepts erasure (Major 1).
- **The liveness amendment.**
  - Placement: correct. It is in the record of the build that carries it first, and it amends Architecture decision 0082 decision 5 through its Amendments section.
  - Precondition chain: (7) is hard in (a). Task 006-046 keeps it and tests the boundary. Architecture decision 0099 decision 10 cites it rather than claiming it.
  - Residues: Minor 4, Nits 5 to 7.
- **The stub of Architecture decision 0100.**
  - It satisfies rule 1 of the README: the number is retired and listed.
  - It satisfies the checker: the heading, a Status line, a unique number.
  - Only the status word is new (Nit 2).
- **End to end.** The decision record, the contract, `sdks/SPEC.md` §5 and §6.2, Decision 006-040 to Decision 006-066, Gap 006-040 to Gap 006-048 and Task 006-043 to Task 006-067 agree on all of the following:
  - the envelope;
  - the two 2 s rules (with `attempt_time_left_ms`, an additive optional field);
  - the counted-call trigger with no time floor;
  - the requirements for both kinds;
  - the restart after a restore;
  - the removal of the breaker, the command and `fsync`, with no stale text left (a search for `breaker`, `acknowledgement_failing`, `no_durability`, `end-lease-marks`, `platform_operator_actions`, `1.5 s`, `50 ms` and `unreasoned` finds only historical lines in the register and the record).

  The exceptions are Major 1, Minor 1, Nits 1 and 4.
- **Compatibility.**
  - The contract only grows (field 4, optional).
  - The migration is additive, and its `down` refuses on a counting provisional mark, the history, or a live process that lists the capability.
  - The rollback is now in one restart with the switch set back. It still needs the restart with the switch off first, so two restarts in all.
  - Older readers are unaffected.
- **Identifier forms.** New prose uses the full forms ("Gap 2000 of the registry", "Decision 006-066", "Owner decision 2026-11-20-6"). Short forms remain only where a tool reads them. I found no refused abbreviation in the added lines.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| A lease is flagged at most once per attempt | Task 006-047's matrix; Task 006-050's test at 18 s and 30 s | A per-column allowed set in the static check, with negative tests for clear-with-lease, a bound attempt and a claim assignment; a re-hint and a `NOT_SENT` withdrawal of the second lease in Task 006-050 (Major 1) |
| No flag at the cap | none | Set the column on the cap-th withdrawal; one case in Task 006-050 (Minor 1) |
| No withdrawal without the requirements read | The witness type (Task 006-050) | Made buildable as an associated type of the port (Minor 2) |
| The server's own trouble never declines a host | The envelope test over the handler and admission (Task 006-049); the conformance cases with and without the envelope (Task 006-058) | none |
| A floor engine silent at the census serves no capability unrecorded | Task 006-046, the boundary test, the released-v0.2 case; the floor build's own test under request (7) | A typed status asserted (Minor 4) |
| The wrong order is visible | `requirement_missing` and the `promtool` case (Task 006-054) | none |
| Process kinds of each requirement | The function's result for each recording (Task 006-044); the matrix (Task 006-047) | none |
| A mark counts only on its server | The parity test; the matrix; the drill with a snapshot-style restart and the runbook's restart | none |
| The acknowledgement and the capability record survive a crash | The `synchronous_commit = off` tests (Task 006-049, Task 006-052) | none |
| A closed vocabulary of failures | The telemetry test (Task 006-054) | Add `conflict` and assert full coverage (Minor 5) |
| A pending runbook step is closed when its task is | none | A rule in `check-design-docs.py` (Nit 9) |

## Items for the owner

1. **Accept Architecture decision 0098 first, with requests (1) to (8) of Decision 006-059 in its code.** Request (7) amends Architecture decision 0082 decision 5 for capability calls, on every deployment of the floor build: a registry connection that fails while the request pool works stops capability calls on that engine after about 25 s. The entry goes into Architecture decision 0082's Amendments section.
2. **Accept Architecture decision 0099 with both assumptions of decision 12.** What remains under Gap 006-048:
   - a restore that does not start the server, done without the runbook's restart of PostgreSQL;
   - more than one writable server.
3. **The bound of once per attempt (Decision 006-066).**
   - The second delivery of an attempt whose first provisional lease was withdrawn is not protected.
   - Every hand-back uses that one flag, a graceful worker stop included (Minor 3).
   - In exchange, no failure of the path spends an attempt, and nothing is shared across tenants (Minor 1 is needed for this to be exact).
4. **A planned restart of PostgreSQL ends the protection of the leases in flight.**
5. **Owner question 006-002:** the reservation of minor 1.8.
6. **The delegate's thresholds:**
   - the 15% cost bound and "fewer than 0.1% answered other than `OK`";
   - the 18 s threshold;
   - the trigger of three counted calls, with no time floor;
   - the decline period: 10 minutes, doubling to an hour;
   - the alert thresholds: 5% and 1% over 10 and 15 minutes.
7. **Retiring the number 0100**, and the status word for it (Nit 2).
8. **The constraints that stay:**
   - before the activation, and after a removal, a 1.8 hand-back spends the attempt;
   - there is no emergency stop for the provisional leases in flight;
   - the rollback needs two restarts of every engine.
9. **Release coupling for 0.3.0:**
   - Task 041-3, Task 041-7, and Task 041-15 to Task 041-20;
   - both builds of Architecture decision 0098;
   - the gate of Task 006-056.

## What blocks what

- **Major 1:** the text before this design merges; the code before Task 006-044 and Task 006-050.
- **Minor 1 and Minor 2:** before Task 006-050.
- **Minor 3:** the text before merge; the reporting in Task 006-056.
- **Minor 4:** a request to the floor build before it merges; Task 006-046 asserts the status.
- **Minor 5:** before Task 006-054.
- **Nits 1, 2, 4 and 5:** before this design merges. The others during the build.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (header lines 9-14; decision 2 at 57-73; decision 5 at 92-104; decision 10 at 138-151; decision 12 at 155-168; Consequences at 222-245)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md (lines 9-13, 51, 53)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/TEMPLATE.md (line 3)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 397-417, 421-493, 563-566, 673-674, 737-739, 756-789)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-042 at 456, Decision 006-059 at 640, Decision 006-066 at 727, Gap 006-047 at 756)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (precondition (a) at 139-145; Task 006-044 at 170-180; Task 006-046; Task 006-047 at 196-210; Task 006-050 at 236-255; Task 006-054 at 286-296)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§5 line 280; §6.2 lines 363-390)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (line 82; section 7, step 5 at line 92)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/engineering/architecture-principles.md (fitness table, lines 13-41)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/state.rs (lines 1474-1479, 1517-1527)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/command.rs (lines 1849-1941)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs (lines 7222-7421, the projection's `upsert_step`)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/instances.rs (lines 44-63)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/status.rs (lines 9-45)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/limits.rs (lines 673-691)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md (decisions 3 and 5; Amendments at 49-52)
