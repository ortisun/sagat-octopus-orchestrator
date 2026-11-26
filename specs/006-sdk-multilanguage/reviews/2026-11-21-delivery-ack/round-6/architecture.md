# Architecture review, round 6: delivery acknowledgement (Architecture decision 0099, Architecture decision 0100, worker protocol 1.8)

## Verdict

**No Blockers. One Major should be settled before the owner accepts the second assumption of decision 12.** Counts: 0 Blockers, 1 Major, 9 Minors, 12 Nits.

- **Round 5.** My Major 1 is resolved. Seven of my nine Minors are resolved. Minor 5 (the audit of `end-lease-marks`) and Minor 6 (the restores) are partly resolved. Nits 1 to 5, 7 and 8 are resolved. Nit 3 is resolved as I wrote it, but my advice was wrong (Minor 2 below). Nit 6 is resolved for the column type, but the function count is still short (Nit 10).
- **The split.** Architecture decision 0100 follows the template and cleanly takes over the relation between requirements. Three things around it need work:
  - its decision 2 names a removal command that does not exist (Minor 3);
  - its claim "whatever the caller" rests on my round-5 Nit 3, which was wrong (Minor 2);
  - several texts still describe the round-5 design (Minor 7).
- **What is new is mostly at the edges:**
  - `end-lease-marks` has no table for its audit record and no check that the engines are stopped (Minor 1);
  - the rollback waits for leases it does not need to wait for (Minor 4);
  - the capability record is the one record whose durability the design does not secure (Minor 5);
  - the mixed-fleet test names a process kind that cannot serve the call (Minor 8).
- **The Major is an improvement, not a defect.** One more stamp, the server's start time, would end mechanically the marks of almost every restore that keeps the timeline. The owner would then accept a much smaller Gap 006-048, and the operator's step would cover one rare case instead of the common ones.

**Limits of this review.**
- I had no shell, so I could not run `git branch -a` or `git worktree list`.
- I could not confirm that number 0100 is free on every other branch. A glob over the sibling directories timed out, and the main checkout shows no `docs/adr/ADR-009*` or `ADR-010*` at all.
- Under `crates/` and `apps/`, the server branch has no `AcknowledgeTasks`, `acknowledge_before_start`, `lease_claim_timeline`, `end_lease_marks` or `DependencyNotRecorded`. I found no duplicate of this work.

---

## Status of the round-5 findings

### My Major

| Round 5 | Status | Reason |
|---|---|---|
| Major 1: a committed acknowledgement lost with the timeline unchanged (`synchronous_commit = off`, `fsync = off`) | **Resolved** | <ul><li>The settings statement raises `synchronous_commit` to `local` and never lowers a stronger value (decision 3; "Durable commit"; Decision 006-043 and Decision 006-062 (a)).</li><li>The probe refuses `fsync = off` with its own reason, `no_durability`.</li><li>Both are in decision 12, with tests in Task 006-049 (`ALTER ROLE sagat_app SET synchronous_commit = off`, read from an adapter hook) and Task 006-047 (`fsync` off), and in "Enforced by".</li><li>Two loose ends: the runbook does not name either setting (Nit 6), and one sentence still says the capability record is covered when it is not (Minor 5).</li></ul> |

### My Minors and Nits

- **Minor 1: resolved.** `Capability::requires()` exists from Task 006-046, so the check passes from its first pull request. One ordering ambiguity of the same kind remains, for `Capability::ALL` (Nit 4).
- **Minor 2: resolved.** The meaning is now stated per kind in decision 10, Decision 006-047 and "Negotiation and activation". The mixed-fleet test contradicts it (Minor 8).
- **Minor 3: resolved.** Decision 006-059 (4) and the "Refines" line keep Architecture decision 0098's decisions 5 and 8 about their own build, with pointers only. One stale sentence remains (Minor 7).
- **Minor 4: resolved.** The probe runs on the request pool, outside the heartbeat's call, with a unit test (Task 006-047).
- **Minor 5: partly resolved.** The command exists and is audited, but the design names no table for the record. The platform's audit table is tenant-scoped, and the operation is not (Minor 1).
- **Minor 6: partly resolved.**
  - Done: the restores are listed, the step runs "after every restore of any kind", and the proven topology is named.
  - Still open: an automatic failover that keeps the timeline, where no operator step can be in time. The drill paragraph sends the operator to a step they cannot run (Minor 9). The Major closes this.
- **Minor 7: resolved.** `SagatDeliveryAcknowledgementInactive` exists, with its `promtool` case in Task 006-054.
- **Minor 8: resolved.** There are notes in drafts 032 and 035, and the "next" row and the context list both drafts.
- **Minor 9: resolved** by Architecture decision 0100 (with findings below).
- **Nits 1, 2, 4, 5, 7 and 8: resolved.**
- **Nit 3: resolved as written, but my advice was wrong** (Minor 2).
- **Nit 6:** the column is now `bigint`, but the Consequences still count too few functions (Nit 10).

### Other reviewers' round-5 findings in my area

| Finding | Status |
|---|---|
| Security Major 1 (a floor process serves a capability it never records, and this build then withdraws the hand-back) | **Resolved** by Decision 006-064: the hand-back and every marked withdrawal read `delivery-acknowledgement-v1` under its share lock, and the rollback switches marking off, drains and then removes. Two caveats: the test copies the reviewer's wrong process kind (Minor 8), and the argument inherits a residual from Architecture decision 0082 (Nit 8). |
| Design Major 1 (common restores keep the timeline) | **Resolved as asked** (an unconditional step, the list, the drill paragraph). The Major below removes most of the list mechanically. |
| Correctness Major 1 (the pass writes nothing for an expired lease) | **Resolved.** Every outcome other than a withdrawal ends the attempt with `expire_attempt` in the same transaction. A test shows the lease is not picked again (Task 006-050; evidence row "The pass ends what it does not withdraw"). |
| Correctness Major 2 (a failure of this call alone never stops) | **Resolved** by the third trigger ("other calls answered, whatever status"), with a two-minute test. One operability residue remains (Nit 9). |

---

## Major

### Major 1. Stamp the server's start time with the timeline: it ends mechanically the marks of almost every restore and switch that keeps the timeline, and leaves the operator's step for one rare case

- **Where:**
  - Architecture decision 0099, decisions 2 and 12;
  - Decision 006-062 (c) and (d), and Gap 006-048 (`research.md:703`);
  - `stream-completion.md`, "The timeline", "After any restore" and the failure rows "Any restore …" and "A switch to another cluster …";
  - `docs/runbooks/disaster-recovery.md`, section 6 ("To drill your own deployment") and section 7, step 5.
- **What is wrong.** Today the exactness depends on an operator after these events:
  - a restored snapshot;
  - a base backup started without `recovery.signal`;
  - a logical dump restored into another cluster;
  - a managed service's restore;
  - a logical-replication cut-over.

  The operator must run `end-lease-marks` before the engines start. If they skip it, round-4 design Major 1 returns: a false "nothing ran" while the handler runs. Two of those cases are the platform's own suggestions (section 3: "your platform's snapshot tool"). The drill paragraph also asks the operator to run step 5 when "your own … failover tools" keep the timeline. After an automatic failover the engines are already running, so that cannot be done (Minor 9). The owner is being asked to accept all of this as Gap 006-048.
- **Why it matters.** `pg_postmaster_start_time()` is executable by every role on every PostgreSQL I know of. It changes in each of those cases, because each of them starts a new server process:
  - the snapshot or base backup started as a server;
  - the cluster a dump is restored into;
  - the cut-over target;
  - the standby that is promoted;
  - the server a point-in-time recovery starts.

  Compared by equality, it never involves a clock comparison. That one value covers the cases the timeline misses, and every case the timeline covers.

  What it does not cover:
  - a dump restored into the same server, never restarted, that it was taken from;
  - possibly a backend crash that the postmaster recovers from without restarting. Under `synchronous_commit = off` that crash loses commits, but the durable commit of the resolved Major 1 already covers it.

  The cost is that a planned restart of PostgreSQL gives the marks in flight at that moment today's meaning. That is the safe direction, and at most one lease time-to-live of protection is lost.
- **Smallest correction.**
  - Stamp the start time with the timeline at the claim, in one new column, or in the same column as a pair. A mark counts only when both equal the current values.
  - Evaluate it once per statement as a scalar subquery, as the timeline is; it adds no statement.
  - Add it to the parity test and to the matrix of Task 006-047.
  - Add a drill case: a snapshot-style restart (stop the server, copy the data directory, start the copy) finds a provisional mark not counted.
  - Shrink Gap 006-048 to "a dump restored into the same running server" and keep `end-lease-marks` as the step for it. Make the step unconditional anyway, as a second safeguard.
  - Correct the drill paragraph (Minor 9).
- **Blocking:** yes for acceptance. It changes what the owner accepts in decision 12. It is not blocking for the rest of the design.

---

## Minors

### Minor 1. `end-lease-marks` has no table for its audit record, no check that the engines are stopped, and an update with no stated bound

- **Where:** `stream-completion.md:437-442`; Task 006-044; Task 006-054; Decision 006-062 (d).
- **What is wrong:**
  - **No table for the audit record.** "Writes one audit record with the operator, the database role and the number of marks it ended" names no table. `audit_records` is tenant-scoped; the registry's migration says so and is why the registry has its own history table (`20270113000000_instance_registry.up.sql:70-71`). `platform_requirement_history` cannot take it either: `action` is checked against `ACTIVATED`/`REMOVED`, `capability`, `kinds` and `epoch` are `NOT NULL`, and there is no count column. So the migration of Task 006-044 needs a schema decision that the design has not made.
  - **No check that the engines are stopped.** "Before the engines start" is prose only. If engines ran first, the pass may already have withdrawn a lease whose acknowledgement was lost, and the command can neither prevent nor detect that.
  - **No bound on the update.** "Sets `lease_claim_timeline` to null where it is not" does not limit itself to leases that can count. If the claim timeline is left on finished rows ("keep them … or clear them with the lease" leaves this open), the update is a scan of `steps` that a bounded transaction may never finish, during a restore.
- **Fix:**
  - Name the record's home:
    - either a deployment-level append-only table of its own, with `platform_requirement_history`'s guards;
    - or that table extended with an action and a count column, in the same migration.
  - Have the command refuse while any execution engine is live (seen within 30 s), and accept `--expect-live 0` as the operator's statement that none should be live. This reuses the census pattern of `capability_activation.rs`.
  - Restrict the update to `status IN ('LEASED','RUNNING') AND lease_claim_timeline IS NOT NULL`, through the index the leased-steps scan already uses. Report that count.
  - Give the command `--dry-run`, as its siblings have.
  - Add tests: refused with a live engine, the audit row written, a second run a no-op (already in Task 006-054).

### Minor 2. The dependency trigger is safe only inside the activation functions, and the owner can set the flag those functions set; my round-5 Nit 3 was wrong

- **Where:**
  - Architecture decision 0100, decision 3 ("runs only inside an activation function that already holds the epoch row, and reads without locking") and Consequences ("no order of activations or removals, by the command or by hand, can record a requirement without what it requires");
  - Decision 006-047; `stream-completion.md:388-391`.
- **What is wrong.** The guard of Architecture decision 0082 passes whenever `sagat.registry_change = 'activation'`. The registry's migration says: "The flag is a seat belt against a hand edit, not a secret: the owner can set it" (`20270113000000_instance_registry.up.sql:92-95`). A hand edit that sets the flag can race an activation function:
  1. It inserts `delivery-acknowledgement-v1`. Its dependency trigger sees `lease-withdrawal-v1`, reading without a lock.
  2. Concurrently, `platform_requirement_remove('lease-withdrawal-v1')` holds the epoch row. Its trigger does not see the uncommitted insert, and it commits.
  3. The hand edit then moves the epoch, which satisfies the deferred check, and commits.

  The end state has the requirement without its dependency. I advised "it need not lock anything" in round 5; that advice assumed only the functions can pass the guard.
- **Fix.** Start the trigger's function with `PERFORM 1 FROM public.platform_requirement_epoch FOR UPDATE`.
  - Inside the activation functions the transaction already holds that lock, so the statement never waits. That keeps Architecture decision 0082 decision 2, which concerns a feature's step taking a lock others may hold.
  - For a hand edit, it puts the check behind the epoch lock.
  - Alternatively, make the dependency check a deferred constraint trigger, which runs at commit after the epoch row is locked.
  - Add the race, with the flag set by hand, to Task 006-046.

### Minor 3. Architecture decision 0100 decision 2 says "the command" refuses a removal, but no command removes a requirement

- **Where:** Architecture decision 0100, decision 2 (`RequiredBy`); Task 006-046; `stream-completion.md:387-388`.
- **What is wrong.**
  - `capability-activate` only activates (server branch `apps/control-plane/src/capability_activation.rs`, `cli.rs:149-204`).
  - A removal is the owner's hand SQL, `SELECT * FROM platform_requirement_remove(...)`, in `docs/runbooks/instance-registry.md` section 5, in `lease-withdrawal.md:93`, and in this design's Rollback step 2.
  - `RequiredBy` would only be reachable through `PgInstanceStore::remove` (`crates/infra-postgres/src/instances.rs:232`), which no operator path calls. So the typed refusal on removal is a variant with no surface. What the operator actually meets is the trigger's error text.
- **Fix.** Pick one:
  - **Restate decision 2 for activation only.** Removal is refused by the trigger, with a distinct text that names the requirement that needs it. `RequiredBy` is how the store maps that text for its callers. Show the text in `instance-registry.md` section 5 and in the rollback.
  - **Add an audited `capability-remove` command.** This is larger, and it is a scope question for the owner, since feature 022 would use it too.

### Minor 4. The rollback drains acknowledged leases that need no drain, and that drain has no bound

- **Where:**
  - `stream-completion.md`, "Rollback" step 2 ("Wait until no mark counts (`worker_leases_marked` is 0) … the longest earlier deadline in flight") and step 4 (the `down` "refuses while a mark counts");
  - Decision 006-055; Architecture decision 0099, Consequences ("Rollback").
- **What is wrong.**
  - **Acknowledged leases keep counting.** A mark counts while the step is `LEASED` or `RUNNING`, acknowledged or not (decision 2). The expiry of an acknowledged lease moves with every heartbeat, and the attempt deadline exists only "when set". So an acknowledged activity without `timeoutMs` keeps `worker_leases_marked` above zero for as long as it runs: hours, for a batch job. The removal of the requirement and the `down` wait for it.
  - **Only provisional leases need the drain.** After the removal, an acknowledged lease is an attempt either way: its hand-back is an ordinary failure, and its expiry is `expire_attempt`. An unmarked 1.8 lease's hand-back is an ordinary failure too (Decision 006-064). Only provisional leases change meaning when the requirement goes, and nothing renews them, so they drain within one lease time-to-live plus the pass.
- **Fix.**
  - Step 2 waits for `worker_leases_provisional` to reach 0.
  - Before the binary is rolled back, the `down` refuses only while a mark counts provisionally. Dropping an acknowledged mark loses nothing, because the floor build treats the lease as an attempt.
  - Add a long-running acknowledged lease to the rollback case of Task 006-066.

### Minor 5. The capability record of an unmarked 1.8 lease is not durable, and the hand-back of that lease relies on it

- **Where:**
  - `stream-completion.md:431-434` ("a lost claim, withdrawal or capability record is safe or covered by it");
  - Decision 006-064; decision 8 ("in a transaction that commits before anything is handed over").
- **What is wrong.**
  - **The ordinary case is covered.** A capability is refused to a provisional lease. So for a marked lease the capability record always follows a durable acknowledgement, and the acknowledgement already makes the hand-back an ordinary failure.
  - **The unmarked case is not.** A 1.8 lease that is not marked but carries `lease_hand_back_generation` is still withdrawn on hand-back while the requirement is recorded. It is unmarked when the session declines, the claim is `short_deadline`, or the engine's probe reports `no_durability`. Its only protection against a served capability is `lease_capability_generation`, written under the database's `synchronous_commit`.
  - **The failure path.** With `synchronous_commit = off`, a crash of the primary after the serve loses the record, and the timeline stays the same. The host's later hand-back is then withdrawn as "nothing ran" after credentials were handed out. That is the very case security Major 1 was about.
  - **What the Major covers.** The Major's start-time stamp covers this whenever the crash restarts the server. It does not cover a backend crash recovered in place.
- **Fix.** Pick one:
  - **Raise `synchronous_commit` in the capability record's transaction too.** It writes once per lease generation, so this costs one settings statement on the first capability call.
  - **Correct the sentence** and add the case to Gap 006-048, under the host's promise.

  I prefer the first, for one statement.

### Minor 6. At the deadline the acknowledgement discards its connection, which adds connection churn exactly when the database is slow

- **Where:** `stream-completion.md:476-482` ("the transaction is abandoned (the connection is discarded, so it rolls back)"); Decision 006-043; Task 006-049.
- **What is wrong.** The deadline fires when the database is slow. Then every acknowledgement in flight past 3 s throws away a pooled connection, and the pool opens new ones: a backend process fork, authentication, TLS. That adds load to an overloaded primary and pressure on `max_connections`, which is the opposite of graceful degradation.
- **Fix.**
  - The settings statement already bounds each statement to "the smaller of 1 s and the time left", so the deadline normally falls between statements. There, send `ROLLBACK` and return the connection.
  - Discard only when a rollback does not return within a short bound.
  - Count discards in a bounded counter, and assert in Task 006-049 that a pool held full for 5 s discards no connection.

### Minor 7. Several texts still describe the round-5 design and contradict the security fix or Architecture decision 0100

- **Where, and what to change:**
  1. `stream-completion.md:356-359`: "the hand-back and the pass's withdrawal of a lease whose mark counts need only `lease-withdrawal-v1`". This contradicts lines 342-344 and 396-400 and Decision 006-064. Write "need both requirements, and not the switch".
  2. `stream-completion.md:749` (the "Gate" evidence row): "the trigger's pair equals the entry's `requires`". Task 006-057 likewise has "its activation entry, with `empty_fleet: false` and `requires`". Architecture decision 0100 decision 1 says the entries "read it and do not repeat it". Write "equals `Capability::requires()`" and remove `requires` from the entry.
  3. Decision 006-047, last sentence: "This refines Architecture decision 0082". The README entry of Architecture decision 0099 likewise has "refines … Architecture decision 0082". Its header no longer says so; Architecture decision 0100 does. Move both to Architecture decision 0100.
  4. Task list precondition (c) asks the owner to accept Architecture decision 0099 only. Task 006-044 builds Architecture decision 0100's trigger, which refines an accepted record, so add "and Architecture decision 0100".
- **Why it matters.** Implementers read the contract and the tasks, not the decision record. Item 1 is the rule the security fix depends on.

### Minor 8. The mixed-fleet test has a floor control plane serve `ResolveSecrets`, which no control plane serves

- **Where:** Task 006-066 ("with `ResolveSecrets` served by a floor control plane"); `stream-completion.md:755` (the "Released v0.2" row).
- **What is wrong.**
  - The worker gRPC service is built only by the engine (server branch `apps/execution-engine/src/lib.rs:799`), and the design itself says a control plane "serves no worker call" (decision 10).
  - The scenario comes from the security reviewer's round-5 text, which assumed control planes serve the worker API.
  - As written, the test cannot be built. It also blurs why the requirement matters, which is the engines.
- **Fix.** Write "served by an engine of the floor build". Keep the before-activation and after-removal cases.

### Minor 9. The runbook text exists before its command and function, is conditional where it was meant to be unconditional, and asks for a step a failover cannot take

- **Where:** `docs/runbooks/disaster-recovery.md:82-84` and `:94-98`.
- **What is wrong:**
  - **Nothing checks the command exists.** Step 5 and the drill paragraph name `control-plane end-lease-marks` and `sagat_database_timeline()`, which exist on no branch. The text is marked "not before the release that builds it". But no check ties a command named in a runbook to `cli.rs`, and no task ties the marker to its removal. If the gate fails (Decision 006-057), the text stays for a command that never ships.
  - **The step is conditional.** "Only where the requirement … is recorded" asks the operator to read the registry first, while the design calls the step unconditional. The command can decide for itself, and be a no-op where nothing counts.
  - **The drill paragraph asks the impossible for a failover.** It says "where it does not change (…), your procedure must run step 5" for "your own restore and failover tools". After an automatic failover the engines are already up, and the pass runs within seconds. This is why considered option 9 was rejected.
- **Fix:**
  - Make step 5 unconditional and the command a no-op when nothing counts.
  - Replace the release marker with "(pending Task 006-054)", and make Task 006-054's evidence include removing it.
  - Add a unit test in the control plane: every `control-plane <word>` in `docs/runbooks/*.md` is a command `cli::parse` accepts. That is the smallest fitness function for a documented command.
  - For a failover tool that keeps the timeline, say it is unsupported until shown otherwise (Gap 006-048). With the Major, it is covered.

---

## Nits

1. **A name collision in one module.** `Capability::requires()` means "depends on". The same module already has `Requirements::requires(capability, kinds)`, which means "the platform requires this capability of these kinds" (server branch `crates/application/src/instances.rs:383`). Name the new one `Capability::depends_on()`.
2. **Architecture decision 0100, "Decided by".** The template asks for reviewers and a date; give them, as Architecture decision 0099 does. Also say what happens to it if Architecture decision 0099 is not accepted: it has no other user.
3. **Architecture decision 0100, two rules with no check:**
   - Decision 5, "the guard of Architecture decision 0082 is never rewritten for it": add to `scripts/check-architecture.py` that no migration after `20270113000000` redefines `platform_requirements_guard()`.
   - Decision 3's firing order: state the test, which is that a hand insert without the flag gets the guard's text, not the dependency's.
4. **Task 006-044 extends the three SQL capability lists, which the check of Decision 006-033 holds equal to `Capability::ALL`.** Task 006-046 is where "the capability [is] known to the registry". Say that the variant enters `Capability::ALL` in Task 006-044, or that the two tasks are one pull request. Otherwise that existing check fails between them.
5. **`statement_timeout` also cuts a commit that waits for a synchronous standby.** PostgreSQL warns that the transaction "has already committed locally", so such a commit is not "outside the bound" (`stream-completion.md:479-481`). Align that sentence with draft 032's note, which says correctly that the timeline covers it.
6. **Settings checks:**
   - `fsync`: verify in Task 006-044 that the runtime role can read it on the pinned image, and treat an unreadable value as `no_durability`.
   - Name `fsync` and `synchronous_commit` in the runbook symptom for `SagatDeliveryAcknowledgementInactive` (`docs/runbooks/sdks.md`).
   - Turning `fsync` off at run time leaves the marks already written exposed until they drain; one sentence in Gap 006-048.
7. **Why control planes are required of `delivery-acknowledgement-v1`.** Decision 006-064's argument needs only the engines. A floor control plane already decodes and shows the cause (precondition (a)). Requiring control planes adds a step to every rollback. Give one sentence of reason, or require it of engines only.
8. **Decision 006-064's "proves that every live process is of this build" is only as strong as Architecture decision 0082 decision 5.** A floor engine whose registry connection fails while its request pool works keeps serving, invisibly. Name that residual.
9. **The first fallback is not spread out.** A shed of the `Acknowledge` class alone while heartbeats pass is now evidence for fallback, which is a reasonable degradation. But every host then closes its receive and registers again at about the same moment. Add a random delay before the declining registration, as the return already has.
10. **Wording and counts:**
    - The Consequences of Architecture decision 0099 count "two functions" plus the trigger. The migration adds three functions (the timeline, `delivery_acknowledgement_required()` and `end_lease_marks()`) plus the trigger and its function.
    - The runbook's new lines are wrapped in a file of unwrapped lines, and they split a code span (`SELECT\nsagat_database_timeline()`).
    - `cli.rs`'s role sentence ("the owner role only for migrate, provision-tenant and schedule-trigger-handover") should gain the new command.
11. **The inactive alert needs no registry series.** The skip reasons are counted in order, with `not_activated` first, so `no_timeline` and `no_durability` are only counted while the requirement is known to be recorded. Say so, so the `promtool` case does not look for a registry metric.
12. **`LANGUAGE sql` cannot catch an error.** "Null instead of an error" therefore needs `has_function_privilege` and `pg_is_in_recovery()` guards in a `CASE`. Say how, or use `plpgsql` with an exception block.

---

## Architecture decision 0100 against the template, Architecture decision 0082 and the generic command

- **The template.** Every section is present. The decisions are stated as MUST rules, and there are four considered options, each with why it lost. The Consequences cover good and bad, compatibility and rollback; the operational cost is small and implied. The "Decided by" line lacks the reviewers and a date (Nit 2).
- **Architecture decision 0082.**
  - Decision 1 (only the owner records a requirement) is kept: the trigger only refuses.
  - Decision 2 ("it reads, it does not lock") holds literally inside the functions. The Minor 2 fix is a lock the transaction already holds, so it does not break that rule.
  - Decision 7 is untouched.
  - Two things are inaccurate: the claim "whatever the caller" (Minor 2), and the context's "recording goes only through its activation functions", which is a seat belt, not a rule.
- **The generic command** (server branch `capability_activation.rs`). The command reads `Activatable` entries. Making `requires` a relation on `Capability` rather than a field of the entry is right, and the stale texts should follow it (Minor 7). The removal side has no command (Minor 3).

## Whether Architecture decision 0099 and Architecture decision 0100 split cleanly

- **Mostly clean.** Architecture decision 0099 now points to Architecture decision 0100 for the relation, and its "Enforced by" no longer carries the trigger tests. Architecture decision 0100 is about the registry only and names its first pair.
- **What is left:**
  - Decision 006-047 and the README still attribute the refinement of Architecture decision 0082 to Architecture decision 0099 (Minor 7).
  - The tasks' precondition (c) omits Architecture decision 0100 (Minor 7).
- **The timeline correctly stayed** in Architecture decision 0099.

## `end-lease-marks`

- **It is not part of the generic activation command family, and should not be.** It records no requirement and changes lease data.
- **Architecture decision 0098 decision 4 ("only the one-way handover keeps its own command") is about activations,** so there is no conflict. One sentence in Architecture decision 0099 decision 12 would stop a reader from asking.
- **It should follow the operator-only pattern of Architecture decision 0082 and of `schedule-trigger-handover`:**
  - the owner role, with the function revoked from `PUBLIC` and granted to no role (as designed);
  - an operator name checked with `unprintable` and `Subject::parse`;
  - one bounded transaction;
  - a dry run;
  - an idempotent second run;
  - an append-only, deployment-level record, which is still missing (Minor 1);
  - a census refusal while engines are live, also missing (Minor 1).

## The runbook step written before the build

The step was written on design Major 1's advice, and having the procedure in this design is right. What it lacks is a marker tied to a task, a check that the command exists, and wording that does not condition the step (Minor 9).

## Build plan order with Architecture decision 0100

- Task 006-044 (the trigger), then Task 006-046 (`Capability::requires()`, the typed refusals, the equality check), then Task 006-057 (the entry). That order works.
- Two adjustments:
  - say when the variant enters `Capability::ALL` (Nit 4);
  - add the owner's acceptance of Architecture decision 0100 to precondition (c) (Minor 7).
- The Major adds one column or stamp to Task 006-044 and a case to the drill of Task 006-054.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| A committed acknowledgement survives a crash | The `synchronous_commit = off` test with an adapter hook (Task 006-049); the `fsync` probe test (Task 006-047) | Verify that `fsync` is readable (Nit 6) |
| A mark across a restart or a restore does not count | The timeline parity test and matrix; the drill reads the timeline | The start-time stamp in the same tests, and a snapshot-style restart in the drill (Major 1) |
| The capability record is complete when a hand-back is honoured | The requirement read under its lock; the mixed-fleet case | Durable commit of the record, or a stated gap (Minor 5); the correct process kind in the test (Minor 8) |
| The pass ends what it does not withdraw | "Not picked again" with the cap and with a removal (Task 006-050) | none |
| The dependency between requirements | Trigger tests in both orders; the equality check (Task 006-046) | The race with the flag set by hand (Minor 2); a check that the guard of Architecture decision 0082 is never redefined (Nit 3) |
| `end-lease-marks` runs before the engines and is audited | The command's test (Task 006-054) | Refusal while engines are live; a named audit table (Minor 1) |
| A documented operator command exists | none | A unit test over `docs/runbooks/*.md` against `cli::parse` (Minor 9) |
| The rollback ends | `worker_leases_marked` at 0 | Drain on `worker_leases_provisional`; a long acknowledged lease in Task 006-066 (Minor 4) |
| The deadline degrades gracefully | "A pool held full for 5 s answered `UNAVAILABLE` within 3.5 s" | No connection discarded in that test (Minor 6) |
| The probe never closes the gate | The unit test (Task 006-047) | none |
| The protection is not silently absent | `SagatDeliveryAcknowledgementInactive` with its `promtool` case | none |

## Identifiers (constitution 4.7.0)

- The added lines use full forms in prose ("Architecture decision 0100", "Owner decision 2026-11-20-6", "Requirement 032-009", "Decision 006-064").
- The old forms appear only where a tool reads them: the headings, the file names, the link targets, the register entry ids and the task lines.
- I found no abbreviation from the refused list among the added lines.

## Items for the owner

1. **Accept Architecture decision 0098 first**, as amended on its branch, with requests (1) to (6) of Decision 006-059 in its code before it merges. Request (2), the cause, and request (3), the clamp, are not there yet.
2. **Accept Architecture decision 0099 with both assumptions of decision 12.** The second one should be accepted after the Major is applied: the residual then shrinks to a dump restored into the same running server, and a backend crash that is recovered in place, which the durable commit covers.
3. **Accept Architecture decision 0100.** It refines the accepted Architecture decision 0082.
4. **Answer Owner question 006-002**, the reservation of 1.8.
5. **Before the activation, and after a removal, a 1.8 hand-back spends the attempt,** as below 1.8. This is the cost of the security fix.
6. **There is no emergency stop for the marks in flight.** Removing the requirement makes them lapse as today. The rollback needs a restart of every engine with `SAGAT_LEASE_WITHDRAWAL=off`.
7. **Supported topologies for 0.3.0:** managed services that hide the write-ahead-log functions (`no_timeline`, alerted), and whatever the Major leaves.
8. **Whether removing a requirement gets an audited command,** for this feature and feature 022 (Minor 3).
9. **The delegate's thresholds:** the 15% bound and the new criterion "fewer than 0.1% of calls end at the deadline". The gate's result is to be reported.
10. **Release coupling for 0.3.0:**
    - Task 041-3, Task 041-7, and Task 041-15 to Task 041-20;
    - both builds of Architecture decision 0098;
    - the gate.

## What blocks what

- **Major 1:** before the owner accepts decision 12.
- **Minors 1 and 2:** before Task 006-044, because they settle schema.
- **Minor 3:** before Task 006-046.
- **Minor 5:** before Task 006-052.
- **Minor 6:** before Task 006-049.
- **Minors 4 and 8:** before Task 006-066.
- **Minors 7 and 9:** before this design merges, because the documents contradict each other.
- **The Nits:** during the build.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (decisions 2, 10, 12; Consequences)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-a-fleet-requirement-may-require-another-and-the-registry-refuses-either-order.md (decisions 2, 3, 5; Consequences)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/TEMPLATE.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 342-400, 418-442, 476-482, 646-660, 749, 755)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-047 at line 501, Decision 006-062 at 650, Decision 006-064 at 675, Gap 006-048 at 703)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (preconditions at lines 138-149; Task 006-044, Task 006-046, Task 006-054, Task 006-057, Task 006-066)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (lines 82-84, 94-98)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (decision 4)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (lines 70-71, 92-114, 147-152)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql (lines 52-77)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/capability_activation.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/cli.rs (lines 149-204, 243-248)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/instances.rs (lines 108-131, 383, 815-826)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/instances.rs (line 232)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/execution-engine/src/lib.rs (line 799)
