No Blockers in round 12: 1 Major, 5 Minors and 7 Nits.

# Architecture review, round 12: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:**
- **Major 1 (Task 006-044 proceeds but needs request (9)).** The text must be fixed before this design merges, and before Task 006-044 starts.

**Round 11:**
- **Major 1 (the floor depends on a release rule): resolved.** The floor is now the registry's.
  - The follow-up lists `lease-withdrawal-v2` on both kinds, and a replay-floor guard keeps it from being removed.
  - Every withdrawal of a marked lease reads `lease-withdrawal-v2`. A release of pull request #260 without the follow-up is refused registration once `lease-withdrawal-v2` is recorded. Running processes that lack it close their gate at their next beat (`docs/runbooks/instance-registry.md` section 3).
  - The precondition no longer rests on what gets released.
  - What is left is smaller: the guard is one-way (Minor 2), one refusal lives only in the command (Minor 1), and the test has no binary to run against yet (Minor 5).
- **Major 2 (precondition (a)): partly resolved.**
  - The list of waiting tasks is right for 006-045 to 006-047, 006-050, 006-051 and 006-053 onward.
  - But Task 006-044 is listed as proceeding. It creates the function that reads `lease-withdrawal-v2` and rewrites the same closed capability checks that request (9) rewrites (Major 1).

**Limits:**
- I had no shell, so I could not run `git branch -a` or `git worktree list`.
- I searched every `crates/` tree under `/Users/marcelo/Documents/Projects` for `lease-withdrawal-v2`, `instance_heartbeat_reporting_change` and `REGISTRY_SILENT`. Nothing matched. So the follow-up has not started anywhere, and nothing is duplicated.

---

## Status of the round-11 findings

| Round 11 | Status | Reason |
|---|---|---|
| **Major 1** (the floor rests on a release rule) | **Resolved** | <ul><li>Request (9) is in the record's decision 10, its Rollback and Compatibility, Decision 006-055, Decision 006-059, the contract, precondition (a), Task 006-046, Task 006-057 and Task 006-066.</li><li>"Released before it" no longer matters.</li><li>Residue: Minors 1, 2 and 5.</li></ul> |
| **Major 2** (tasks proceed that need the follow-up) | **Partly resolved** | <ul><li>The waiting list is correct, except Task 006-044 (Major 1).</li></ul> |
| Minor 1 (port key) | Resolved | <ul><li>`mark_withdrawn_attempt(&instance, generation)` is on both ports.</li><li>An unconsumed mark fails the commit, and a test proves it.</li><li>The collection-item sibling test runs through `upsert_scoped_instance` and `upsert_path_instance`.</li></ul> |
| Minor 2 ("the withdrawal command") | Resolved | The term is left only where the cap is applied (record line 222, `stream-completion.md:645`, Decision 006-052). |
| Minor 3 (series named for the follow-up) | Resolved | The series is named in request (7), the contract, Task 006-046's evidence and Task 006-054's runbook symptoms. |
| Minor 4 (per-connection layer) | Resolved, with residue | <ul><li>Decision 3 has the sentence, Task 006-049 has the release-0.2 start test, and Task 006-057 turns the reserve on.</li><li>"From the activation" in Consequences is wrong (Nit 2).</li></ul> |
| Minor 5 (stream start unobservable) | Partly resolved | <ul><li>The SDK's own permit can be built in every SDK.</li><li>"A channel of its own" does not mean "a connection of its own" in two SDKs, or behind a pooling proxy (Minor 3).</li></ul> |
| Nits 1 to 9 | All resolved | <ul><li>"Once built" is in the Refines line.</li><li>The failure row for `REGISTRY_SILENT` exists.</li><li>`mark` is listed once, and the marks counter is incremented after the commit.</li><li>`upsert_path_instance` is named.</li><li>The Task 006-058 sentence has moved.</li><li>The "Bad" text is merged.</li><li>README line 13 is in Task 006-054.</li><li>The beat is a function with its own name.</li><li>The failover outcome is in Gap 006-047 and in "Bad".</li></ul> |

**Other reviewers' round-11 findings in my domain:**
- **Correctness Major 2, security Major 1 and design Major 1** (the floor): resolved by request (9), with the residue of Minors 1, 2 and 5.
- **Correctness Minor 1** (the tasks that proceed): partly resolved (Major 1).
- **Correctness Minor 3** (the port key): resolved.
- **Correctness Minor 4** (no lease carries a timeout): resolved by `attempt_deadline_epoch_ms = 24`. The register notes in the other drafts were not updated (Nit 3).
- **Correctness Minor 5 and security Minor 1** (the heartbeat after a `NOT_ACKNOWLEDGED` withdrawal): the vocabulary is resolved. The server has nothing stored to choose the kind from (Minor 4).
- **Correctness Minor 7 and design Minor 3** (whether the transport started a stream): partly resolved (Minor 3).
- **Security Minor 2** (a held epoch hides a committed change): resolved. The beat reads the committed epoch with a plain read and reports separately whether an owner transaction holds the row.
- **Design Minor 4** (the reserve has no switch): resolved, apart from Nit 2.
- **Correctness Major 1 and design Minors 1 and 2** (the early give-up): outside my domain. The redesign adds no layer or contract problem (Nit 7).

---

## Major

### Major 1. Task 006-044 is listed as proceeding, but it reads `lease-withdrawal-v2` and rewrites the closed capability checks that request (9) also rewrites

**Where:**
- Precondition (a), `tasks.md:142-146`: "Task 006-043, Task 006-044, … proceed".
- Task 006-044, `tasks.md:175-187`:
  - "`delivery_acknowledgement_required()` … returning `lease-withdrawal-v2` as recorded only for both kinds";
  - "the three closed capability checks extended `NOT VALID` and then validated";
  - its evidence, "the function's result for each recording (both kinds, engines only, none)".
- Decision 006-059 (9): `lease-withdrawal-v2` "joins the closed lists of known capabilities".
- The base pattern, `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql:12-29`. Each check (`serving_instances_capabilities_known`, `platform_requirements_capability_known`, `platform_requirement_history_capability_known`) is dropped and added again under the same name, with the full list of values.

**What is wrong:**
- Until the follow-up's migration exists, `platform_requirements_capability_known` refuses the value `lease-withdrawal-v2`. So Task 006-044's evidence cannot be produced. The server owner must then do one of three things:
  1. Stop.
  2. Add `lease-withdrawal-v2` to the three checks and to `Capability::ALL`. That duplicates request (9), against rule 2, the way feature 016 was duplicated.
  3. Write the migration with this design's own value only. Whichever of the two migrations is applied second then replaces the named check with a list that lacks the other's value. The validating migration fails, or a process of the follow-up build that lists `lease-withdrawal-v2` fails the check when it registers.
- The precondition's own rule says that tasks which "read a requirement's kinds" wait. `delivery_acknowledgement_required()` is exactly that read.
- A smaller point: Task 006-052, which proceeds, edits the same capability handlers where request (7) puts its second gate check. That is a merge conflict, not a duplication.

**Why it matters:**
- Rule 2 (implementing another feature's work without an approved dependency).
- The migration discipline: these checks are rewritten as full lists, so two branches that each rewrite one of them must be ordered.
- A precondition that sends the build the wrong way is the same defect as round-11 Major 2.

**Smallest correction:**
- Move Task 006-044 to the waiting list, or split it:
  - the columns and `worker_sessions.declines_acknowledgement` proceed;
  - `delivery_acknowledgement_required()`, the three checks and `Capability::ALL` wait for the follow-up's merge.
- Say that this design's migration version sorts after the follow-up's, and that its lists include `lease-withdrawal-v2`.
- Optionally, say that Task 006-052 rebases onto the follow-up's capability paths before it merges.

**Blocking:** the text before this design merges; before Task 006-044 starts.

---

## Minors

### Minor 1. The refusal "while any registered instance, live or not, lacks `lease-withdrawal-v2`" is now a guarantee, but it lives only in the command, and the records still call that refusal a convenience

**Where:**
- The record's decision 10 (lines 156-160).
- Considered option 10 (lines 238-240): "the command's refusal is a convenience".
- Decision 006-045 (`research.md:234-236`: "…lacks it, which is not").
- The contract (`stream-completion.md:405-412`).
- Architecture decision 0100's Decision and Consequences: "Nothing is left for it to enforce"; "The registry is unchanged … apart from its capability".

**What is wrong:**
- The refusal is a field `requires` of the Rust `Activatable` entry. An owner who calls `platform_requirement_activate('delivery-acknowledgement-v1', …)` directly, as the hand SQL of `docs/runbooks/instance-registry.md` section 5 allows for removals, skips it.
- Then an engine of a #260 release can still hold an open gate:
  - it is lapsed but still registered, and
  - it lacks request (7), and
  - it comes back and serves capability calls before its beat closes its gate.
- That reopens round-11 Major 1, point 3. A secret served with no `lease_capability_generation` recorded, followed by a hand-back the server withdraws, is the violation decision 8 forbids.
- Two residues are not recorded either, even with the command:
  - The base keeps serving on a lost registry connection (runbook section 3), and deletes its row after a day. A #260 engine silent for more than a day therefore passes the census.
  - A process that started under `REGISTRY_FULL` serves with no row at all.

**Why it matters:**
- Rule 11: a guarantee is stated without its condition.
- Two records (option 10 and Architecture decision 0100) now contradict the design.

**Smallest correction:**
- Either enforce the refusal in the database or forbid it by hand:
  - **In the database:** a `BEFORE INSERT` check on `platform_requirements` for `delivery-acknowledgement-v1` that raises while a row of `serving_instances` lacks `lease-withdrawal-v2`. That is one function, in this design's migration.
  - **Or in the runbook:** say in section 5 that `delivery-acknowledgement-v1` is never activated by hand, and test that the history's actor is the command.
- Rewrite option 10 so that the first refusal is a convenience and the second is not.
- Update Architecture decision 0100's Consequences sentence, or point it to option 10.
- Add both residues (a row deleted after a day while its process serves, and a process with no row) to Gap 006-047 or to "Bad".

**Blocking:** before Task 006-057.

### Minor 2. The guard on removing `lease-withdrawal-v2` is wider than its purpose, and the record does not name the one-way door it opens

**Where:**
- Decision 006-059 (9): "the replay-floor guard refuses its removal while a journal holds `step.lease_withdrawn`, as it refuses that of `lease-withdrawal-v1`".
- The record's Rollback (lines 286-291).
- Considered options: none covers `lease-withdrawal-v2`.
- The Refines line.

**What is wrong:**
- The base guard tests `EXISTS (SELECT 1 FROM run_events WHERE event_type = 'step.lease_withdrawn')` (`20270208000000_lease_withdrawal.up.sql:55-56`). That includes every `NOT_SENT` withdrawal of Architecture decision 0098.
- On any deployment where 0098 has already withdrawn a lease, activating `lease-withdrawal-v2` is therefore permanent at once. This is true before any 1.8 cause exists, and even if delivery acknowledgement is never activated.
- From then on, rolling back below the follow-up build is impossible.
- The only cause that a #260 release cannot replay is `NOT_ACKNOWLEDGED`. That cause exists only after `delivery-acknowledgement-v1` has been recorded at least once.
- The record has no considered option for this new capability. The alternatives were:
  - the release rule of round 11;
  - a guard keyed to `platform_requirement_history` naming `delivery-acknowledgement-v1`, which is cheap, exact and never deleted.
- The Refines line does not say that this design relies on Architecture decision 0098 decision 8's floor moving to `lease-withdrawal-v2`.

**Why it matters:**
- It is a hard-to-reverse decision, so it needs a record with its alternatives and consequences.
- The rollback scope changes for operators who never use 1.8.

**Smallest correction:**
- Narrow the guard: "refuses removing `lease-withdrawal-v2` while `platform_requirement_history` names `delivery-acknowledgement-v1`".
- Or keep the guard as it is, and state the one-way door in "Bad" and in the runbook step that activates `lease-withdrawal-v2`.
- Either way, add `lease-withdrawal-v2` as a considered option, with the release rule as the rejected alternative.
- Add "(9): the floor's own capability, Architecture decision 0098 decision 8" to the Refines line.

**Blocking:** before the follow-up builds request (9). The choice is the owner's.

### Minor 3. "A channel of its own" does not give the acknowledgement a connection of its own in two SDKs, or through a pooling proxy

**Where:**
- `sdks/SPEC.md` §6.2 (lines 362-365).
- `stream-completion.md:569-571` and `:728`.
- The record's decision 3 (lines 86-89).
- Task 006-059.
- The Fallback row: "the acknowledgements going on their own channel".

**What is wrong:**
- grpc-js and grpcio (C-core) share subchannels from a global pool across channels to the same target with equal arguments. A second channel then rides the same HTTP/2 connection, so the acknowledgement waits again for that connection's stream credit. That is the condition round-11 Minor 5 removed from the counting rule.
- The other four SDKs separate naturally, unless they share a handler or clone a channel: a separate `ClientConn` in Go, a separate `ManagedChannel` in Java, a separate handler in .NET, and a separate `Endpoint::connect` in tonic.
- An HTTP/2 sidecar or proxy also pools upstream connections, whatever the host does. In that case only the server's reserve of 4 holds, and the design does not say so.

**Why it matters:**
- Rule 8: one semantics in six SDKs.
- The case "a connection with more than 128 calls does not fall back" can fail in two languages for a reason the text does not name.

**Smallest correction:**
- Define the rule as "a transport connection of its own".
- Name the means per SDK in §6.2. For grpc-js and grpcio that is `grpc.use_local_subchannel_pool = 1`, or a distinguishing channel argument.
- Add a conformance assertion that the server sees the acknowledgement on a different connection from the stream (the peer address held in `CappedConnectInfo`).
- Say in decision 3 that behind a pooling proxy the reserve is what holds. That is also why the reserve stays, even though hosts have their own channel.

**Blocking:** before Task 006-058 and Task 006-059.

### Minor 4. The split heartbeat kinds have nothing stored to decide them from

**Where:**
- `stream-completion.md:730-734` and `:857`.
- Decision 006-053 (`research.md:721-727`).
- The marks table (`stream-completion.md:477`): `withdrawn_lease_token_hash` is "read only on the stale path".
- Task 006-050's evidence (`tasks.md:302`).

**What is wrong:**
- On the stale path the server matches only `withdrawn_lease_token_hash`. No column stores the cause of that withdrawal.
- `provisional_withdrawn_attempt` cannot stand in for it. It is also set by the withdrawal that reaches the cap, whatever its cause. So a `NOT_ACKNOWLEDGED` withdrawal at the cap would be counted as `heartbeat_after_hand_back`, which is not alerted. That silences the earliest sign the split was added for.
- So the builder must invent one of two things:
  - a ninth column, which changes "Eight columns on `steps`" in Consequences and the static check's set of allowed forms; or
  - a journal read on the stale path, which no cost line counts.

**Why it matters:**
- Telemetry built from stored values only (the identical-record rule).
- An unstated change to the data model.

**Smallest correction:**
- Store the fact in the same assignment that writes the hash. For example, `withdrawn_on_hand_back boolean`, written only by the withdrawal and listed in the static check's allowed forms.
- Or name the read: the step's latest `step.lease_withdrawn`, by its index, on the stale path only.
- Update the column count.

**Blocking:** before Task 006-050 and Task 006-053.

### Minor 5. The check that a #260 build without its follow-up is refused has no binary in the released-upgrade harness

**Where:**
- Task 006-066 (`tasks.md:458-460`).
- The acceptance matrix's "Released v0.2" row.
- The record's "Enforced by".
- `scripts/test-release-upgrade-profile.py:19`, `:170-171`. It pins one release (`RELEASE`, `v0.2.0`) and builds two phases.

**What is wrong:**
- No pinned build of #260 exists for the case to start. It would be:
  - a third phase pinned to #260's merge commit, if #260 is never released alone; or
  - a released tag, if it is.
- Without that, the case may use a v0.2 binary, which lists neither `lease-withdrawal-v1` nor `lease-withdrawal-v2`. It would then be refused for the wrong reason, and the test passes without proving request (9).

**Smallest correction:**
- Name the binary: a third pinned phase with its own source-manifest check, or the release tag if one exists.
- Assert that the binary lists `lease-withdrawal-v1`.
- Assert that the refusal is `CAPABILITY_MISSING` with `out_missing = {lease-withdrawal-v2}`.
- Assert that the same binary, registered before the recording, closes its gate at its next beat.

**Blocking:** before Task 006-066.

---

## Nits

1. **The matrix's "Gate" row is stale.** `stream-completion.md:939` still reads "`lease-withdrawal-v1` for both kinds, recorded by hand for engines only among the cases". It should read `lease-withdrawal-v2`. As written, the case passes because `lease-withdrawal-v2` is missing, not because of the kinds.
2. **"From the activation" is wrong.** Consequences (record line 260) says "Every other class of call has 60 places of a connection's 64 from the activation". The reserve is keyed to the compiled minor, so it applies when the build of Task 006-057 is deployed, before any activation.
3. **The register notes are out of date.** The notes added to drafts 019, 020, 021, 023, 024, 025, 026, 031, 032 and 035 say "`TaskLease` field 23 … taken". Field 24 is taken too. The descriptor baseline would catch a clash, but the notes are the register.
4. **Indentation.** In the `TaskLease` snippet, field 23 is indented four spaces instead of two (`stream-completion.md:289`).
5. **`sdks/SPEC.md` §6.5 (lines 413-416).** At 1.8 the hand-back is "honoured only while the operator has activated `delivery-acknowledgement-v1`". It also needs `lease-withdrawal-v2`. This is optional for a text aimed at hosts.
6. **"Unless the operator removes it"** (record line 261) names no procedure. Section 5 of the instance-registry runbook has no step for deleting a silent row. Add one to the runbook update of Task 006-046 or Task 006-057.
7. **The repeat "at once" on evidence.**
   - After a promotion, every host sends every waiting lease's next attempt at the same moment. A jitter of up to 200 ms costs nothing and spreads the surge.
   - Say whether frames queued behind the permit of 4 may be merged into one call of at most 64 leases. Without merging, a host that receives frames faster than its 4 permits drain gives leases up having sent no attempt at all.

---

## The checks you asked for

- **Layers:**
  - Nothing new puts I/O, a framework or `serde_json` into the domain.
  - The port is now keyed by `ExecutionInstanceId` on the application's run and path ports, and the domain's `withdraw_lease` takes plain values. Both are correct.
  - The new columns and the function are adapter-side.
- **Decisions:**
  - Request (9) is a hard-to-reverse amendment of Architecture decision 0098 decision 8. It has no considered option, and no consequence recorded for its one-way guard (Minor 2).
  - Option 10 and Architecture decision 0100 contradict the new load-bearing refusal (Minor 1).
- **Compatibility:**
  - Field 24 is added on flagged leases only, and those go to sessions at 1.8 or later. That is additive.
  - The register notes are stale (Nit 3).
  - The beat keeps its two-argument function for older binaries.
  - The migration ordering with the follow-up is unresolved (Major 1).
- **Operability:**
  - The new series (`instance_heartbeats_change_in_progress_total{kind}`) and kinds (`heartbeat_after_hand_back`) come from closed vocabularies with bounded labels.
  - One kind has nothing stored to decide it from (Minor 4).
  - The cost of the reserve is stated, but with the wrong start (Nit 2).
- **Scope:**
  - Task 006-044 would duplicate request (9) (Major 1).
  - No other branch or worktree carries the follow-up.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| No admitted binary lacks `lease-withdrawal-v2` once a `NOT_ACKNOWLEDGED` withdrawal can exist | `instance_register`'s requirement check; the beat's gate closing; the replay-floor guard extended to `lease-withdrawal-v2` (Task 006-046) | Minor 5: a pinned #260 binary in `test-release-upgrade-profile.py`, refused with `out_missing = {lease-withdrawal-v2}` |
| No registered binary without request (7) when `delivery-acknowledgement-v1` is recorded | The command's `requires` entry (Task 006-057) | Minor 1: a database check on insert, or the runbook's prohibition plus a test of the history's actor |
| The closed lists hold every capability of both branches | `crates/infra-postgres/tests/capability_lists.rs` | Major 1: Task 006-044 waits, and its migration sorts after the follow-up's |
| The acknowledgement has a connection of its own | Fallback case with more than 128 calls | Minor 3: assert a separate connection at the server, per SDK |
| The kind of a heartbeat after a withdrawal follows its cause | Task 006-050's two heartbeat tests | Minor 4: name the stored value, and add it to the static check |
| An unconsumed mark fails the commit | Task 006-050's refused-commit test | none |
| The older configuration keeps 64 places per connection | Task 006-049's release-0.2 start test | none |

## Items for the owner

1. **The guard on removing `lease-withdrawal-v2` (Minor 2).**
   - As written, activating `lease-withdrawal-v2` makes the follow-up build the permanent floor on any deployment that has a withdrawal of Architecture decision 0098.
   - The alternative is to narrow the guard to the history of `delivery-acknowledgement-v1`.
2. **Activating `delivery-acknowledgement-v1` by hand (Minor 1).** Forbid it in the runbook, or enforce the refusal in the database.
3. **The follow-up of pull request #260 now carries request (9) as well,** and Task 006-044 waits for it. Who builds requests (1), (2) and (5) to (9), and when, is still open.
4. **Releasing #260 alone is no longer a guarantee issue,** but it has a cost: activating `delivery-acknowledgement-v1` is refused until the rows of that release age out, which can take a day, unless the operator deletes them.
5. **Still open:**
   - accepting Architecture decision 0099 with both assumptions of decision 12;
   - Owner question 006-002;
   - the delegate's thresholds;
   - the constraints that stay (no emergency stop, two restarts for a rollback).

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (142-146, 175-187, 302, 318-327, 458-460)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (289, 405-412, 477, 569-571, 730-734, 939)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (234-236, 721-727, Decision 006-059 (9))
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (Refines line, 86-89, 156-160, 238-240, 260-261, 286-291)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-withdrawn-a-fleet-requirement-may-require-another.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§6.2 lines 362-365, §6.5 lines 413-416)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql (12-29, 52-65)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (180-245, 250-277)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md (sections 2, 3 and 5)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/scripts/test-release-upgrade-profile.py (19, 170-171)
