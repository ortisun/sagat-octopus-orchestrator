No Blockers in round 11: 2 Majors, 5 Minors and 9 Nits.

# Architecture review, round 11: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:**
- **Major 1 (the rollback floor depends on a release rule the precondition no longer states):**
  - The text must be fixed before this design merges.
  - The release rule itself is the owner's, and must be settled before pull request #260 is first released.
- **Major 2 (precondition (a) lets tasks proceed that need the follow-up):** the text must be fixed before this design merges, and before Task 006-050 starts.

**Round 10:**
- My round-10 Major 1 is **partly resolved**. Dropping the fallback removes the dormant-build floor and its removal that nothing guarded. But the guarantee "the registry never admits a binary below that build" now holds only if no build of #260 is released without request (2). Only one sentence of Decision 006-059 still says that (Major 1).
- My round-10 Major 2 is **resolved**.
- Round-10 Minors:
  - Minor 1 is resolved except the Refines line (Nit 1); its trigger point is folded into Major 1.
  - Minor 2 is resolved in form, but the port key it proposed is wrong for scoped instances (Minor 1). That wording was mine.
  - Minor 3 is partly resolved (Minor 2).
  - Minor 4 is resolved except the failure-table row and one sentence (Nit 2), plus a new gap (Minor 3).
  - Minor 5 is resolved except a duplicate listing (Nit 3).
  - Minor 6 is resolved in the contract only (Minor 4).

**Limits:**
- I had no shell, so I could not run `git branch -a` or `git worktree list`.
- I searched every `sagat-wt-*` worktree. `WithdrawalCause` has only `NotSent` and `NotStarted` in both worktrees that carry it (`crates/domain/src/execution/event.rs:438-443`). So the follow-up of #260 has not started anywhere, and nothing is duplicated.

---

## Status of the round-10 findings

| Round 10 | Status | Reason |
|---|---|---|
| **Major 1** (the fallback's floor not enforced) | **Partly resolved** | <ul><li>**Done:** the fallback is removed from the record, the contract, Decision 006-059 and precondition (a).</li><li>**Left (Major 1):** precondition (a), the header, Decision 006-055 and the Rollback only require that a released build with (2) and (7) exists before Task 006-057. That does not exclude an earlier release of #260 without them. Only `research.md:786` still says "before any build of it is released". So two conditions are in force and they say different things.</li></ul> |
| **Major 2** (Task 006-046 asserted the old beat) | **Resolved** | <ul><li>`tasks.md:190-204` has: the 60 s owner-transaction test, the engine silent for 31 s that keeps refusing, the registry connection's timeout read back, the overload's `prosecdef` and `proconfig`, a missing epoch row raising, and the second check before the first byte or the write.</li><li>The fallback clause is moot now that the fallback is gone.</li></ul> |
| Minor 1 (the old fact in three passages; two triggers) | Resolved, with residue | <ul><li>`stream-completion.md:397-400`, Rollback step 3 (`:794-796`), Decision 006-055 (`research.md:737-741`) and decision 10 now say "will decode".</li><li>The Refines line is unchanged (Nit 1).</li><li>The trigger point is folded into Major 1.</li></ul> |
| Minor 2 (flag channel, sibling, buildable negative test) | Resolved in form | <ul><li>The port method, the sibling test and the buildable negative tests are present.</li><li>The key "node" does not identify a scoped row (Minor 1).</li><li>The third function is not named (Nit 4).</li></ul> |
| Minor 3 ("withdrawal command" names the domain) | Partly resolved | <ul><li>The clarifying sentence is at `stream-completion.md:421-423`.</li><li>The term is still used for the binder of `$flag` and the taker of the witness in four places (Minor 2).</li></ul> |
| Minor 4 (fleet-wide refusal unrecorded) | Resolved, with residue | <ul><li>"Undecided" is defined (`stream-completion.md:441-445`).</li><li>The counter, the alert `SagatCapabilitiesRefusedRegistrySilent`, the "Bad" line and the runbook line are present.</li><li>There is still no failure-table row, and Decision 006-059 keeps "undecided" (Nit 2).</li><li>The series is emitted by the follow-up, which is not told its name (Minor 3).</li></ul> |
| Minor 5 (denominator; two `mark` sets) | Resolved, with residue | <ul><li>`worker_lease_marks_total` exists and there is one `mark` set.</li><li>`worker_leases_expired_total` is listed twice with different sets (Nit 3).</li></ul> |
| Minor 6 (replacing tonic's limit) | Partly resolved | <ul><li>The contract (`stream-completion.md:564-568`) keeps the key, the default of 64 and the meaning. The semaphore is held in `CappedConnectInfo`, and the reserve is 4 from Task 006-057.</li><li>The record, Task 006-049 and Task 006-057 do not carry it (Minor 4).</li></ul> |
| Nits 1 to 8 | 1, 2, 3, 4, 6 and 7 resolved; 5 resolved but garbled; 8 not addressed | <ul><li>Nit 5's sentence splits a list (Nit 5 below).</li><li>Nit 8 is folded into Minor 3.</li></ul> |

**Other reviewers' round-10 findings in my domain:**
- **Security Major 1 and design Major 1** (the fallback): the fallback is gone. The overload fixes part 5 (the beat that older binaries decode). The re-admission of a #260 release without (2) remains in one case: if such a release ever exists. That is Major 1.
- **Security Minor 2** (the overload's constraints): resolved in the contract and in Task 006-046.
- **Security Minor 1 and design Minor 5** (a beat with no epoch): resolved.
- **Security Minor 3** (a heartbeat after a hand-back): resolved by the kind `heartbeat`, audited and not alerted.
- **Design Minor 6** (the layer is unscoped): partly resolved, as my Minor 6. Its replacement rule, "an attempt never started on a stream", is new and has no mechanism (Minor 5).
- **Design Nit 2** (the 0100 status and the index check): resolved. The README still names the other script (Nit 7).

---

## Majors

### Major 1. The floor is enforced only if no build of #260 is ever released without request (2), and the precondition no longer says so

**Where:**
- The record's header (line 9), Rollback (lines 271-277), decision 10 (lines 146-148) and Compatibility (line 268: "the build of Architecture decision 0098, which the recorded requirement excludes").
- Decision 006-055 (`research.md:737-741`): "so no binary below it is admitted once any withdrawal exists".
- Decision 006-059: the first sentence (`research.md:786`, "before any build of it is released") against the new end (`:818-822`, "a released build of the follow-up with (2) and (7) is a hard precondition of Task 006-057").
- Precondition (a) (`tasks.md:142-147`).
- Contract Rollback step 3 (`stream-completion.md:794-800`).

**What is wrong:**
- **The registry has one floor rule:** `lease-withdrawal-v1` cannot be removed while a journal holds `step.lease_withdrawn` (Architecture decision 0098 decision 8). It keeps out only binaries that do not list that capability.
- **A release of #260 without the cause passes that rule.** The security reviewer checked the code in round 10. That build lists `lease-withdrawal-v1` (`crates/application/src/instances.rs:121`, `:140`), and its codec returns `None` for any other cause (`crates/infra-postgres/src/event_codec.rs:495`).
- **The new precondition does not exclude such a release.** It requires that a good release exists before Task 006-057, not that no bad release exists. If 0.3.0 ships #260 and the follow-up lands in a later patch, these hold:
  1. A control plane of 0.3.0 is admitted at any time, because `delivery-acknowledgement-v1` is required of engines only. It then fails to replay every run that holds a `NOT_ACKNOWLEDGED` withdrawal. So Compatibility's "which the recorded requirement excludes" is false for control planes.
  2. After the rollback removes `delivery-acknowledgement-v1`, engines of 0.3.0 are admitted too. That is round-10 Major 1 again, for that release instead of the dormant build.
  3. An engine of 0.3.0, which lacks request (7), that is silent at the activation's census keeps serving capability calls to provisional leases. Decision 10 relies on (7) to prevent exactly that.
- **The Rollback's "so no mark … exists before the floor does" is true, but it is not the guarantee that is needed.** What matters is that no binary below the floor is admitted after a mark exists.

**Why it matters:**
- Constitution principle I: the journal must replay on every admitted binary.
- Rule 11: the record states a guarantee ("enforced, not described") that holds only under a condition it does not state.
- The condition is a release decision, which only the owner can take.

**Smallest correction:**
- Write one rule in precondition (a), the header, Decision 006-055, decision 10 and the Rollback. Suggested text: "No build of pull request #260 is released without requests (2) and (7); this is a release blocker of the first release that carries it (owner item). If one has been released, Task 006-057 does not start." Then make `research.md:786` agree with it.
- Add the check that fails when the rule is broken. In the `released-upgrade` job (Task 006-066, `scripts/test-release-upgrade-profile.py`), for every published release whose engine lists `lease-withdrawal-v1`:
  - a control plane and an engine of that release either replay a journal fixture that holds `NOT_ACKNOWLEDGED`, or are refused registration;
  - Task 006-057's evidence cites that run.

**Blocking:** the text before this design merges; the owner's release rule before #260 is first released.

### Major 2. "The other dormant tasks proceed without it" contradicts the tasks that write the cause or use the secret fence

**Where:**
- Precondition (a) (`tasks.md:143-146`): "Task 006-045 and Task 006-046 wait for that follow-up to merge … the other dormant tasks proceed without it".
- Against it:
  - Task 006-050: the pass withdraws with `NOT_ACKNOWLEDGED` (`tasks.md:250-251`), and a hand-back is withdrawn through the report fence of a step with declared secrets (`:262-263`, `:290`).
  - Task 006-051: the pass's withdrawal races.
  - Task 006-053: calls after a `NOT_ACKNOWLEDGED` withdrawal.

**What is wrong:**
- The base's `WithdrawalCause` has only `NotSent` and `NotStarted` (`crates/domain/src/execution/event.rs:438-443`). Its codec rejects any other cause.
- The secret fence of request (1) is not in the base either (`checked_secret_report`, Decision 006-059 (1)).
- Task 006-045 now only *confirms* the cause "as the follow-up of pull request #260 added it".
- So the server owner who reaches Task 006-050 must either stop, or add the variant, the codec, the events contract, the Studio's list and the fence. Adding them duplicates the follow-up's requests (1) and (2), against rule 2, which is how feature 016 was duplicated. It also races the follow-up on the same files.

**Why it matters:**
- This is a precondition that sends the build the wrong way.
- The duplicated cause would also blur which build is the floor, which Major 1 depends on.

**Smallest correction:**
- In precondition (a), list the tasks that wait for the follow-up's merge: Task 006-045, 006-046, 006-050, 006-051 and 006-053, and the cases of 006-054 and 006-058 that need a `NOT_ACKNOWLEDGED` event.
- Name the tasks that proceed: 006-043, 006-044, 006-047, 006-048, 006-049 and 006-052.

**Blocking:** the text before merge; before Task 006-050.

---

## Minors

### Minor 1. The port key "node and generation" does not identify a scoped row, so a sibling item of the same collection can take the flag

**Where:**
- `stream-completion.md:476-480` (`mark_withdrawn_attempt(&node, generation)`).
- Decision 006-066 (`research.md:908-909`).
- Task 006-050 (`tasks.md:257-258`).
- The "Bound" row of the acceptance matrix.

**What is wrong:**
- The scoped projection statements are keyed by `ExecutionInstanceId`, which is the node plus its path indices.
- `upsert_scoped_instance` already filters every effect by `&grant.instance == instance` (`crates/infra-postgres/src/execution.rs:6952-6963`). The path statement is `upsert_path_instance` (`execution/path.rs:1093`).
- Items of one collection share a node and are often at the same generation. A mark keyed by node would therefore pass the old-row fence (`LEASED`, equal generation) on a sibling item that the same append touches. Depending on which call consumes it, either:
  - the sibling's next lease of that attempt is left unflagged; or
  - the withdrawn item's flag is lost, so it is flagged again, up to the cap.
- That direction is safe for attempts, but the protection is silently lost.
- The new sibling test only covers root siblings, which are different nodes.

**Fix:**
- Key the method by the instance: `mark_withdrawn_attempt(&ExecutionInstanceId, generation)`, using the root instance for a root step. Put it on both the run-transaction port and the path-transaction port.
- Extend the sibling test to two items of one collection, same node and equal generations, through both `upsert_scoped_instance` and `upsert_path_instance`.
- Optionally, make the adapter's commit fail with an internal error when a mark was never consumed. That catches a lost flag.

**Blocking:** before Task 006-050.

### Minor 2. "The withdrawal command" still binds `$flag` and takes the witness

**Where:**
- The record's decision 2 (line 66): "whose condition only the withdrawal command binds true".
- Decision 006-066 (`research.md:906`).
- Task 006-050 (`tasks.md:253`: "which the decision function and the withdrawal command take"; `:268`: "`$flag` bound by the withdrawal command alone").
- `stream-completion.md:420`, before the clarifying sentence.

**What is wrong:**
- In the merged code, "the withdrawal command" is the domain's `withdraw_lease`.
- The new sentence says that the application's withdrawal services take `&Tx::Requirements` and bind the flag. The task text the server owner builds from still says the command does.
- So the round-10 layer inversion (domain ← application) survives in the tasks.

**Fix:**
- Use "the application's withdrawal service" in these four places.
- Keep "the withdrawal command" only where the cap is applied (Decision 006-052), or define the term once.

**Blocking:** before Task 006-050.

### Minor 3. The `REGISTRY_SILENT` series and alert depend on the follow-up's code, which is not told the series' name

**Where:**
- Decision 006-059 (7) (`research.md:807-808`: "counted").
- `stream-completion.md:445-446`, `:869`, `:889`.
- Task 006-046's evidence.

**What is wrong:**
- `worker_capability_calls_refused_total` does not exist in the base (no match in `crates/`).
- The refusal is built by the follow-up, before this design's Task 006-054. If the follow-up names its counter differently, `SagatCapabilitiesRefusedRegistrySilent` watches nothing. Its `promtool` cases would still pass, because they run on synthetic series.
- As with round-10 Nit 8 for `SagatLeaseAttemptCapped`, a fleet that runs only the floor build has the refusal but no alert. Neither runbook symptom says so.

**Fix:**
- Name the series and label in request (7).
- Add "counted in `worker_capability_calls_refused_total{method, reason="REGISTRY_SILENT"}`" to Task 006-046's evidence.
- Say in both alerts' runbook symptoms that they come with this build, or offer the rules to the follow-up.

**Blocking:** before Task 006-046 and Task 006-054.

### Minor 4. The replaced per-connection layer is still only in the contract

**Where:**
- The record's decision 3: no sentence (a search for "per-connection" and "tonic" in the record finds none).
- Task 006-049 (`tasks.md:243-244`).
- Task 006-057.

**What is wrong:**
- The contract (`stream-completion.md:564-568`) now settles the key, the default, the semaphore's lifetime and the reserve of 4 "only from Task 006-057". But:
  - no test shows that a configuration of release 0.2 starts with the same limit;
  - Task 006-057 does not list turning the reserve on;
  - the record does not mention the change, although it touches every call of the worker service.

**Fix:**
- Add one sentence to decision 3.
- Add a start test with a release-0.2 configuration to Task 006-049.
- Add "the reserve of 4 for the class `Acknowledge` enabled" to Task 006-057 and its evidence.

**Blocking:** before Task 006-049 and Task 006-057.

### Minor 5. "An attempt that the host's transport never started on a stream does not count" is a normative host rule with no mechanism in six SDKs

**Where:**
- `sdks/SPEC.md` §6.2 (line 393).
- `stream-completion.md:568`, `:749`.
- `research.md:895`.

**What is wrong:**
- Whether a call was given an HTTP/2 stream is a transport detail. grpc-java and grpc-go expose it through their tracer or stats hooks. The design names no mechanism for grpcio, tonic, grpc-js or Grpc.Net, and I know of no public hook in grpcio.
- This round removed the earlier sentence, "an SDK does not put the acknowledgement behind a saturated connection of its own". That was the mechanism every SDK could build.
- Rule 8 asks for one semantics with common conformance. An SDK that cannot observe the condition will count these attempts, and falls back on a connection with more than 128 calls. The fallback matrix says that must not happen.

**Fix:** define the condition at the SDK's own layer, or name the hook per SDK in §6.2. For the first, a suggested wording: "an attempt still waiting in the host's own admission when it ended", where admission is either:
- a per-connection limit the SDK holds below the server's `max_concurrent_streams`; or
- a channel of its own for `AcknowledgeTasks`.

The existing case "a connection with more than 128 calls does not fall back" then tests that one mechanism in all six SDKs.

**Blocking:** before Task 006-058 and Task 006-059.

---

## Nits

1. **The Refines line** (record line 11) still reads "it relies on that build's amendment of Architecture decision 0082 decision 5" without "once built".
2. **"Undecided" left over:**
   - Decision 006-059 still reads "which the gate reads as capability calls undecided" (`research.md:805-806`), before the new definition at `:815-817`. Replace it with the definition.
   - Add the failure-table row that round-10 Minor 4 asked for: "a registry connection failing, or an owner transaction held open across an engine's lapse: capability calls refused `REGISTRY_SILENT`; below 1.8 the handler fails".
3. **`worker_leases_expired_total{mark}` is listed twice** (`stream-completion.md:858` and `:860`).
   - The second listing omits `provisional`. That value can occur: a provisional lease at the cap, or with a requirement missing, ends with `expire_attempt`. Delete the second listing.
   - Also say that `worker_lease_marks_total` is incremented after the claim commits.
4. **"The three named functions"** of the static check: name the third, `upsert_path_instance` (`crates/infra-postgres/src/execution/path.rs:1093`), wherever the text says "the path statement".
5. **The inserted sentence in Task 006-058** (`tasks.md:385-386`), "the repeat on `REGISTRY_SILENT` is an SDK unit test …", splits the list. "Two minutes of answers with the envelope" now reads as part of it. Move the sentence to the end of the list.
6. **The record's "Bad" names the registry-connection failure twice** (lines 250-251 and 258-260). Merge the two.
7. **`docs/adr/README.md:13`** says `scripts/check-agent-config.py` checks the records. Task 006-054 now puts the index check in `scripts/check-design-docs.py`, so update that line in the same task.
8. **The overload `instance_heartbeat(uuid, bytea, boolean)`** does not say what its boolean means. Name the parameter.
9. **A new spent-attempt outcome is not recorded.** The failure table (`stream-completion.md:836`) and the Fallback row now expect it: a lease whose give-up passes during a failover (an 18 s time-to-live, or a long failover) is handed back as an ordinary failure. Gap 006-047 speaks only of a lapse into a review. Add a clause to the gap and to "Bad".

---

## The checks you asked for

- **Layers:**
  - Nothing new puts I/O, a framework or `serde_json` into the domain.
  - The domain's `withdraw_lease` returning plain values is correct.
  - The term "withdrawal command" still crosses the layers in the tasks (Minor 2).
  - The port key must be the instance (Minor 1).
- **Decisions:**
  - Removing the fallback needs no new record, because it removes a mechanism.
  - The release rule that the floor now rests on is a hard-to-reverse decision. It has no record, and it is the owner's (Major 1).
  - Adding "Withdrawn" to the template conflicts with no governance text. `scripts/check-agent-config.py` checks only that a Status line exists.
- **Compatibility:**
  - The overload keeps the two-argument beat for older binaries.
  - The connection key and its default are unchanged, but untested (Minor 4).
  - Control planes of a #260 release without the cause are not excluded (Major 1).
- **Operability:**
  - Eight alerts are defined, from closed vocabularies with bounded labels.
  - One series depends on another build naming it (Minor 3).
- **Scope:**
  - The follow-up's requests (1) and (2) are needed by Task 006-050, which the precondition lets proceed (Major 2).
  - No branch duplicates the work.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| No admitted binary lacks the cause once a withdrawal exists | Replay-floor guard (lease-withdrawal-v1 only) | Major 1: a `released-upgrade` case per published release that lists `lease-withdrawal-v1` (replay a `NOT_ACKNOWLEDGED` fixture, or be refused), cited by Task 006-057 |
| The cause and the fence exist before a task writes them | none | Major 2: the list of tasks that wait for the follow-up's merge, in precondition (a) |
| The attempt flag lands only on the withdrawn row | Static check, two-withdrawal test, root sibling test | Minor 1: the same-node collection sibling test through both scoped statements; optionally a commit that refuses an unconsumed mark |
| A beat never waits on the epoch | Task 006-046's 60 s owner-transaction test | none |
| A fleet-wide `REGISTRY_SILENT` refusal is visible | Alert and `promtool` cases | Minor 3: the series named in request (7) and confirmed in Task 006-046 |
| An older configuration keeps the same connection limit | none | Minor 4: a start test with a release-0.2 configuration |
| A host does not count attempts its transport never sent | Fallback conformance case (128 calls) | Minor 5: a mechanism every SDK can build |
| The index agrees with each record | Planned in `check-design-docs.py` (Task 006-054) | Nit 7: README line 13 |

## Items for the owner

1. **The release rule** (Major 1): no build of pull request #260 is released without requests (2) and (7). This is a release blocker of the first release that carries it, which is part of the release coupling for 0.3.0.
2. **The follow-up of pull request #260:** who builds requests (1), (2), (5), (6), (7) and (8), and when. No branch has started it, and Tasks 006-045, 006-046, 006-050, 006-051 and 006-053 wait for it (Major 2).
3. **Request (7)'s behaviour on every deployment of the floor build:** hosts below 1.8 fail their handler when a capability call is refused with `REGISTRY_SILENT`. This is now recorded in "Bad".
4. **A failover longer than a lease's give-up spends the attempt** at an 18 s time-to-live (Nit 9, Gap 006-047).
5. **Accept Architecture decision 0099** with both assumptions of decision 12.
6. **Still open:**
   - Owner question 006-002 (the reservation of minor 1.8);
   - the delegate's thresholds;
   - the constraints that stay (no emergency stop, two restarts for a rollback);
   - the alternative left by the revised Decision 006-045.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (lines 9, 11, 66, 146-148, 250-260, 268, 271-277)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/README.md (line 13)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (646, 737-741, 786, 805-822, 895, 906-909, 945-949)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (397-400, 418-423, 433-453, 470-485, 564-568, 749, 794-800, 836, 856-890)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (142-147, 186-204, 250-268, 290, 319, 350-351, 384-386)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§6.2, line 393)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs (438-443)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (6890-6967)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution/path.rs (1093)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs (326-404)
