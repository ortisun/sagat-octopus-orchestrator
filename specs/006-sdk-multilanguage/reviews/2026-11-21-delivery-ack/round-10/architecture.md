I found no Blockers in round 10: there are 2 Majors, 6 Minors and 8 Nits.

# Architecture review, round 10: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**What blocks:**
- **Major 1 (the fallback's rollback floor is not enforced):** must be fixed in the text before this design merges, and must be decided before pull request #260 merges into `develop`.
- **Major 2 (Task 006-046 contradicts the revised request (7)):** must be fixed before this design merges and before Task 006-046 starts.

**Round 9:**
- My round-9 Major 1 is **partly resolved**. Its residue is Minor 1, and the fallback it asked for brought in Major 1.
- My round-9 Major 2 is **resolved**.
- Minors 2, 3, 4 and 6 are resolved. Minor 1 is resolved for the old-row fence, but the fence opens a new gap (Minor 2). Minor 5 is resolved in the contract but not in Task 006-046 (Major 2).
- Of the round-9 Nits, 1, 2, 4, 5, 6 and 7 are resolved and Nit 3 is partly resolved.

**Limits:**
- I had no shell, so I could not run `git branch -a` or `git worktree list`.
- I searched every `sagat-wt-*` worktree: Architecture decision 0100 is only in this one, and none has `REGISTRY_SILENT` or `NotAcknowledged` in its crates. So I found no duplicated work, and also no sign that the follow-up of pull request #260 has started.

---

## Status of the round-9 findings

| Round 9 | Status | Reason |
|---|---|---|
| **Major 1** (the base lacks the requests) | **Partly resolved** | <ul><li>**Done:** Decision 006-059 and precondition (a) record each request's status: only (3) is present. They name the follow-up of #260 and give a fallback. The record's header and decision 10 say "once they are built" and "will decode". The register's (6) and (8) now name `CREATE OR REPLACE FUNCTION` with `SECURITY DEFINER` and the pinned `search_path`.</li><li>**Left (Minor 1):** three normative passages still state the old fact. The deadline is "before release" in one sentence and "before #260 merges into `develop`" in the next. The tasks that the fallback moves work into do not say so.</li><li>**New (Major 1):** under the fallback, the registry no longer enforces the floor.</li></ul> |
| **Major 2** (the revision of Decision 006-045 only half applied) | **Resolved** | <ul><li>Task 006-050 (lines 250-260 and the evidence) now says what decision 5 says.</li><li>So do the failure table (`stream-completion.md:785-787`), Decision 006-061, both clarifications of Requirement 037-006 (`specs/037-compensation-safety/spec.md:153-169`) and Decision 006-051.</li><li>Task 006-067 checks the 037 wording.</li><li>A search for "not acknowledged", "before it was acknowledged" and "an acknowledged lease is an attempt" finds no rule left that contradicts it.</li></ul> |
| Minor 1 (the attempt flag's form) | Resolved, with a new gap | <ul><li>The fenced form is written in the record, the contract and Decision 006-066.</li><li>`withdraw_lease` reports the last withdrawal the cap allows, and there is a test with two withdrawals in one transaction.</li><li>The fence compares the generation without the step, which matters because a generation is a per-step counter (Minor 2).</li><li>The "flagged at its claim" wording was not taken (Nit 4).</li></ul> |
| Minor 2 (residues in the register and the tasks) | Resolved | <ul><li>Decision 006-042 no longer lists the flag among the marks.</li><li>Task 006-047's matrix has N, N − 1 and null, and the negative tests are in its evidence.</li><li>One of those negative tests cannot be built (Minor 2).</li></ul> |
| Minor 3 (the witness type) | Resolved | The associated-type shape is in `stream-completion.md:416-420`. The word "withdrawal command" now puts it in the domain, which breaks the layers (Minor 3). |
| Minor 4 (the cap alert and the labels) | Resolved | <ul><li>`SagatLeaseAttemptCapped` is owned here.</li><li>`mark` and `stop` are defined, and Task 006-054 counts seven alerts.</li><li>The denominator "marked claims" has no series (Minor 5).</li></ul> |
| Minor 5 (the 5 s margin) | Resolved in the contract and the register | The registry connection has its own statement timeout of at most 5 s. Task 006-046 still says "less the beat call's bound" (Major 2). |
| Minor 6 (the race against the pass) | Resolved | It is in Task 006-051, the acceptance matrix and decision 5. |
| Nits | 1, 2, 4, 5, 6 and 7 resolved; 3 partly resolved | <ul><li>Nit 1's text is fixed, but the new "Enforced by" sentence is garbled (Nit 1 below).</li><li>Nit 3 is fixed in the contract but not in Task 006-046, and the counter is still not named (Minor 4).</li></ul> |

**Other reviewers' round-9 findings in my domain:**
- Security Major 1 (the beat behind an owner transaction) is resolved by `FOR SHARE SKIP LOCKED`. But "undecided" is not defined, and the fleet-wide effect is not recorded (Minor 4). It is also not carried into Task 006-046 (Major 2).
- Security Minor 3 (the per-connection limit) is resolved by replacing tonic's limit. The configuration's compatibility is not stated (Minor 6).
- Security Minor 4 and design Major 1 are folded into Major 1 here.
- Security Minor 1 is resolved: the hash is kept for `NOT_STARTED`. Two tables were left behind (Nit 3).
- Security Minor 2 is resolved in decisions 7 and 9 and in Decision 006-045. Decision 2 was left behind (Nit 2).
- Design Minor 1 (the exactness wording) is resolved in decision 4 and the contract.

---

## Majors

### Major 1. Under the fallback the rollback floor is described, not enforced

**Where:**
- Decision 006-059, last sentences: "this design's dormant build then becomes the rollback floor … and `delivery-acknowledgement-v1` is recorded for both kinds".
- Precondition (a) (`tasks.md:142-147`).
- The record's Rollback paragraph: "The floor is enforced, not described, once it is built … if the cause cannot go into that build, this design's own dormant build becomes the floor".
- Decision 10.
- Contract "Rollback" step 3 (`stream-completion.md:765-769`).

**What is wrong.** The registry enforces the floor through one rule: `lease-withdrawal-v1` cannot be removed while a journal holds `step.lease_withdrawn` (decision 8 of Architecture decision 0098, lines 50 and 88). That rule keeps out a binary that does not list `lease-withdrawal-v1`. It does not keep out a binary that lists it but cannot decode the cause `NOT_ACKNOWLEDGED`. Under the fallback, the build of Architecture decision 0098 is exactly such a binary. Its sequence is:
1. The requirement is recorded for both kinds, so while it is recorded the registry refuses processes of the 0098 build. So far this holds.
2. Rollback step 2 removes `delivery-acknowledgement-v1`. It has to: the dormant build does not list it ("known to the registry but listed by no engine yet", Task 006-046), so it could not be admitted otherwise.
3. Nothing guards that removal. A trigger of its own was rejected in Decision 006-047, and the replay-floor guard looks only at the event type.
4. After the removal, the registry admits an engine or control plane of the 0098 build. That could be an older autoscaler template, or an operator who rolls back one step too far. It lists `lease-withdrawal-v1`, then fails to replay every run whose journal holds a `NOT_ACKNOWLEDGED` withdrawal.

"The registry never admits a binary below that build once any withdrawal exists" is true only when the 0098 build decodes the cause. The fallback is exactly the case where it does not.

**Why it matters:**
- Rule 11: no fake implementation of an unproven guarantee.
- Constitution principle I: the journal must replay on every admitted binary.
- Choosing the floor is hard to reverse. Decision 006-055 was written to close Blocker 1 of round 3, and the fallback reopens it without a record.

**Smallest correction.** Choose one of the following and write it into the record (Rollback and decision 10) and into Decision 006-055:
- **(a) Drop the fallback.** Make request (2) a release blocker of the 0.3.0 release that carries the 0098 build: no build of #260 is released without the cause. This is the smallest change and adds no mechanism. It is an owner item.
- **(b) Give the fallback its own enforced floor.** The dormant build lists a capability of its own, for example `withdrawal-cause-not-acknowledged-v1`, for both kinds.
  - It is recorded before Task 006-057.
  - The activation entry of `delivery-acknowledgement-v1` refuses while it is not recorded, and `delivery_acknowledgement_required()` returns it.
  - The replay-floor guard is replaced in the migration of Task 006-044 (`CREATE OR REPLACE`, keeping `SECURITY DEFINER` and the pinned path), so this capability cannot be removed while a journal holds a `NOT_ACKNOWLEDGED` withdrawal.
  - The rollback removes only `delivery-acknowledgement-v1`.
  - This revives considered option 10, the subject of the retired Architecture decision 0100, so the record must say why it is now accepted for this case.

Either way, add the case to the `provisional-lease-cutover` rollback evidence (Task 006-066): after the removal, a binary of the 0098 build is refused.

**Blocking:** the text before this design merges; the choice before #260 merges into `develop`.

### Major 2. Task 006-046 asserts the beat behaviour that the revised request (7) removed

**Where:** `tasks.md:189-199` (Task 006-046).

**What is wrong.** The task was not updated with request (7):
- **Its evidence contradicts the new beat:** "a beat racing an activation held between its census and its commit waits for the epoch and refuses after it". But the beat now reads the epoch `FOR SHARE SKIP LOCKED` and never waits for it. When the row is held, it commits `last_seen` and returns no epoch (Decision 006-059 (7); `stream-completion.md:431-434`; decision 10).
- **Its window is the old one:** "the liveness window less the beat call's bound". The contract now says 25 s, held by the registry connection's own statement timeout of at most 5 s. The task has neither that timeout nor a test for it.
- **Its re-check is the old one:** "checked again at the hand-over". The contract says "before the first byte of the answer is sent" (round-9 Nit 3).
- **It does not reflect the fallback:** under the fallback, request (7) is built here. The task still reads "as the floor build has it".

**Why it matters.** Tests come first (rule 18), and this build inherits the rule for its own engines. A test written from this evidence would assert that the beat waits, which is the hazard security Major 1 of round 9 removed. Either the test fails against a correct build, or the build is made to wait.

**Smallest correction.** Rewrite the evidence of Task 006-046 as:
- an owner transaction holding `platform_requirement_lock()` for 60 s: no gate closes, heartbeats and reports flow, capability calls are refused with `REGISTRY_SILENT` and served again after the commit;
- an engine that was silent past 25 s, whose first beat after it reconnects returns no epoch, keeps refusing until a beat with an epoch and its gate decision succeed;
- the registry connection's statement timeout read back as at most 5 s, with no configuration refused;
- the re-check before the first byte.

Add "built here under the fallback of Decision 006-059" to the task.

**Blocking:** before Task 006-046; the text before merge.

---

## Minors

### Minor 1. Residue of round-9 Major 1: three passages still state the old fact, and the deadline has two triggers

**Where:**
- `stream-completion.md:397-398`: "the floor build's control planes already decode the cause (precondition (a) of the task phase)". Round 9 asked for "will decode", and only decision 10 of the record was changed.
- `stream-completion.md:766-768`, Rollback step 3: "that build decodes the cause `NOT_ACKNOWLEDGED` (precondition (a) …), and the registry enforces it".
- `research.md:734-735`, Decision 006-055: "must decode `NOT_ACKNOWLEDGED` before it merges".
- The record's Refines line still reads "it relies on that build's amendment of Architecture decision 0082 decision 5", without "once built".
- **The deadline has two triggers:**
  - The record's header, Decision 006-059 and precondition (a) say the requests go into the follow-up "before any build of it is released".
  - The same two paragraphs then apply the fallback "if the follow-up cannot land before #260 merges into `develop`".
  - Between those two moments, either rule can be read as the one in force.
- Tasks 006-044 and 006-045 do not mention the fallback either. Task 006-045 still says "as the server build added it", and Task 006-044 does not list (6) and (8).

**Fix:**
- Use "will decode, once request (2) is built" in the three passages and in the Refines line.
- Pick one trigger. I propose "the first release that contains the 0098 build", which agrees with Major 1 (a).
- Add one conditional sentence each to Tasks 006-044 and 006-045.

**Blocking:** the text before merge.

### Minor 2. The attempt flag's fence names a generation without a step, and the channel from the decision to the statement is unspecified

**Where:**
- `stream-completion.md:510-521`, Decision 006-066 and the record's decision 2: "`CASE WHEN $flag AND status = 'LEASED' AND generation = $fence …`, for every caller".
- The negative tests of Task 006-047 (`tasks.md:211-212`).

**What is wrong:**
1. **A sibling step can be flagged.** A generation is a counter per step, so two `LEASED` siblings of one run can hold the same generation number. The projection statement runs once for each node an append touches (`crates/infra-postgres/src/execution.rs:7378-7421`, `WHERE … node = $3`). If `$flag` and `$fence` are carried per append, or per transaction, a sibling that the same append touches passes the fence. Its next lease of that attempt is then left unprotected. That is the safe direction for attempts, but it silently loses protection. The new two-withdrawal test covers a row already withdrawn, not a sibling still `LEASED`.
2. **The channel is unspecified.** The flag is neither in the domain state nor in the journal. The text never says how the value travels from `withdraw_lease`'s result into the call for that one node. That channel is the layer boundary of rule 5, so it should be named.
3. **One negative test cannot be built:** "a second statement that assigns it in the same transaction". A check of SQL text in `scripts/check-architecture.py` cannot see transactions.

**Fix:**
- Say that the application binds `$flag` true only in the projection call of the withdrawn node, through a port method of the transaction keyed by node and generation. For example: `mark_withdrawn_attempt(&node, generation)`, which the adapter consumes in that node's call. Every other call binds false.
- Add a test: two `LEASED` siblings with equal generations, one withdrawn, the other touched by the same append; the other's flag is unchanged.
- Replace the unbuildable negative test with "a second assignment of the column in one statement", or with "an assignment outside the three named functions".

**Blocking:** before Task 006-047 and Task 006-050.

### Minor 3. "The withdrawal command" now names the domain, so the witness shape crosses the layers inwards

**Where:**
- `stream-completion.md:418-420`: "the decision function and the withdrawal command are generic and take `&Tx::Requirements`".
- Decision 006-052 and `stream-completion.md:604-606`: the cap is applied by "the withdrawal command … from the journal count".
- Decision 006-066: "`$flag` is bound true only by the withdrawal command".

**What is wrong.** In the merged code, the command that applies the cap from the journal count is the domain's `execution::withdraw_lease(def, state, &node, fence, cause, before, now)`, called from `crates/application/src/worker/withdrawal.rs:385`. The pure decision function is in the application, beside `LeaseDeadlines` (`crates/application/src/execution.rs:1015`). So the text reads as follows:
- The domain's command takes an associated type of an application trait. That breaks the layers: domain ← application.
- The domain "binds" a SQL parameter.

My own round-9 wording carried the same ambiguity.

**Fix:** say that the application's withdrawal services (`withdraw_root`, `withdraw_path` and the hand-back's path) and the decision function are generic over `Tx` and take `&Tx::Requirements`. The domain's `withdraw_lease` receives plain domain values and returns whether this is the last withdrawal the cap allows. The application turns that into the per-node binding of Minor 2.

**Blocking:** before Task 006-050.

### Minor 4. A beat that returns no epoch stops capability calls fleet-wide, and nothing records, names or alerts it

**Where:**
- `stream-completion.md:431-437`: "which the gate reads as 'an activation in progress', capability calls undecided".
- Decision 006-059 (7).
- The failure table.
- The record's "Bad".
- Observability.

**What is wrong:**
- "Undecided" is not defined. Security's fix defined it as "refused with `REGISTRY_SILENT`". Read that way, an owner transaction held open on the epoch refuses every capability call of every engine for as long as it is open, for example the hand SQL of `docs/runbooks/instance-registry.md` section 5 left idle. The fleet's gates do not close, but `ResolveSecrets`, `ResolveDatabase`, blob transfers and `AppendStepLogs` stop everywhere.
- That is a new fleet-wide degradation:
  - It has no row in the failure table.
  - It has no line in "Bad"; that section names only the silent registry connection, and names it twice.
  - "Counted" names no series (round-9 Nit 3).
  - No alert fires.
- Workers below 1.8 do not repeat these calls (`sdks/SPEC.md` §5, "otherwise as before"), so their handlers fail.

**Fix:**
- Define "undecided" as "refused with `REGISTRY_SILENT`", or as "served under the last decided gate while the engine has not been silent". Pick the one the correctness reviewer accepts.
- Add a failure-table row and a line in "Bad".
- Name the counter, for example `worker_capability_calls_refused_total{method, reason="REGISTRY_SILENT"}`, which extends an existing series with one closed value.
- Add an alert clause, for example any such refusal for 2 minutes, and a runbook line: "never leave the owner transaction open".

**Blocking:** before Task 006-046 and Task 006-054.

### Minor 5. Two alerts divide by "marked claims", a series that does not exist, and `mark` carries two vocabularies

**Where:** `stream-completion.md:841-848`; Decision 006-056 ("ratios over marked claims").

**What is wrong:**
- `SagatLeasesNotAcknowledged` and the second clause of `SagatAcknowledgementsFailing` divide by "marked claims".
- The only marked-lease series listed is the gauge `worker_leases_marked`, which cannot be the denominator of a rate.
- `worker_lease_marks_skipped_total` counts the claims that were not marked.
- Nothing in the base or the design counts the claims that were marked.
- The label `mark` now has two value sets: `marked`/`unmarked` on `worker_leases_undelivered_total`, and `acknowledged`/`recovered`/`unmarked` on `worker_leases_expired_total`. Which value a mark that does not count takes on the first is not said.

**Fix:**
- Add a counter `worker_lease_marks_total`, incremented by the claim when it marks, at no statement cost, and use it in both alerts.
- Say that a mark that does not count is `unmarked` on `worker_leases_undelivered_total`, or rename that label `marked` (`true`, `false`).
- Add both to Task 006-054's telemetry test.

**Blocking:** before Task 006-054.

### Minor 6. Replacing tonic's per-connection limit changes the admission of every worker call, without its parameters or its compatibility stated

**Where:**
- `stream-completion.md:544-548`: "tonic's `concurrency_limit_per_connection` is replaced by a concurrency layer of the platform … with a reserve for the class `Acknowledge`".
- Task 006-049 (`tasks.md:221-222`, `238-239`).

**What is wrong:**
- Today `concurrency_per_connection` is a configuration key with a default of 64 (`crates/api-grpc/src/server.rs:46`, `84`, `157`). The design does not say whether it keeps its name, default and meaning, so an older configuration might break.
- The reserve's size is not given, so "answered within the bound" has no stated parameter.
- The change is not in the record's decisions or its "Bad", although it applies to every call on the worker service, transfers included.

**Fix:** add one sentence to decision 3 of the record and to the contract:
- the key and its default are unchanged;
- the reserve is N calls of the class `Acknowledge`, taken from within the limit, with N stated;
- calls over the limit wait as they do today.

Add a test that a configuration of release 0.2 starts with the same limit.

**Blocking:** before Task 006-049.

---

## Nits

1. **"Enforced by" is garbled:** "to be built with the decision, the attempt flag's allowed set in the static check among it, (`specs/…`)". Move "the attempt flag's allowed set" into the list beside the other static check.
2. **Decision 2 of the record** still says a mark that does not count is "for every withdrawal an unmarked lease". Decisions 7 and 9, Decision 006-045 and `stream-completion.md:618` add "except that its hand-back is an ordinary failure". Add the same exception to decision 2.
3. **Two tables were left behind by security Minor 1:**
   - The withdrawal table row for `NOT_STARTED` (`stream-completion.md:615`) does not say that `withdrawn_lease_token_hash` is kept.
   - The failure-table row (`stream-completion.md:802`) still reads "a lease withdrawn as `NOT_ACKNOWLEDGED`". Decision 12 and Decision 006-053 now add "or on its hand-back".
4. **The flag for an unflagged 1.8 lease's hand-back** (`stream-completion.md:621`) is still not stated (round-9 Minor 1, point 4). Every reading of it is harmless (it only means fewer flags), but say one, and put it in the pure function's unit test.
5. **The repeat on `REGISTRY_SILENT`** is tested only in each SDK's unit tests (Tasks 006-059 to 006-064). Rule 8 asks for shared conformance. Say why it is not there, as Decision 006-058 does for `RETRY`: the harness cannot silence a registry.
6. **Task 006-050** (`tasks.md:252`) says "which every other statement leaves alone". The contract now says the form is in all three statements for every caller. Use the contract's words.
7. **The probe after a recovered lease's hand-back** finds no withdrawn hash, because that hand-back is an ordinary failure. It is therefore counted `no_match` beside forged tokens. That is rare and harmless; name it in the reason's definition.
8. **`SagatLeaseAttemptCapped` alerts on the floor build's causes.** A fleet that runs only the 0098 build therefore has no cap alert. Say so in the alert's runbook symptom, or offer the rule to the floor build's follow-up.

---

## The checks you asked for

- **Layers:**
  - The witness shape keeps the adapter's constructor private and the trait in `crates/application`. That is correct, except for the word "withdrawal command" (Minor 3).
  - The flag's channel needs naming at the port (Minor 2).
  - Nothing new puts I/O, a framework or `serde_json` into the domain.
  - The new label, reason and alert are closed and bounded, except the missing denominator (Minor 5).
- **Decisions:**
  - The revised Decision 006-045 and decision 4's new split ("exactly one" for the pass, the host's word for a hand-back) agree with decision 5 of Architecture decision 0098 and with constitution principle III, under decision 12.
  - The fallback's floor is a new hard-to-reverse decision with no enforcement (Major 1).
  - The beat with `SKIP LOCKED` amends the registry's beat function. That is part of the amendment of Architecture decision 0082 decision 5, which is recorded at acceptance; that is enough.
- **Compatibility:**
  - The contract only grows.
  - The attempt flag's form adds two bound parameters to the three projection statements and no statement.
  - The registry timeout removes round 9's refusal to start.
  - Left open: the meaning of the per-connection configuration key (Minor 6), and capability calls of workers below 1.8 refused under `REGISTRY_SILENT` (Minor 4, owner item 1).
- **Index:** the 0100 heading and row now match its Status, and the index check is planned in Task 006-054.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| The journal replays on every admitted binary, under the fallback too | none under the fallback | Major 1: (a) release blocker, or (b) a capability of its own behind the replay-floor guard, with a cut-over refusal case |
| A beat never waits on the epoch | Task 006-046 (asserts the opposite today) | Major 2: the 60 s owner-transaction test |
| The attempt flag is set only on the withdrawn node | Static check and two-withdrawal test | Minor 2: the equal-generation sibling test; a buildable negative test |
| No withdrawal is decided without the requirements read | Witness type | Minor 3: generic in the application, not the domain |
| A capability call refused fleet-wide is visible | none | Minor 4: named counter and alert |
| Alert ratios have defined series | Telemetry test (Task 006-054) | Minor 5: `worker_lease_marks_total` |
| An older configuration starts with the same connection limit | none | Minor 6: a start test with a release-0.2 configuration |
| The index agrees with each record | Planned in Task 006-054 | none |

## Items for the owner

1. **The floor of the rollback.** Choose one:
   - no build of #260 is released without request (2), so the fallback is dropped (my recommendation, Major 1 (a));
   - the fallback with a capability of its own (Major 1 (b)), which revives considered option 10.
   
   Also choose the one moment that triggers it (Minor 1).
2. **The follow-up of pull request #260:** who builds requests (1) for the secret fence, (2), (5), (6), (7) and (8), and when. No branch has started it.
3. **Request (7)'s new behaviour, which applies to every deployment of the floor build:**
   - An open owner transaction on the epoch refuses capability calls fleet-wide.
   - Workers below 1.8 do not repeat them.
4. **The revised Decision 006-045 and its left alternative:** a first heartbeat before every flagged handler, at one transaction per short task.
5. **Accept Architecture decision 0099** with both assumptions of decision 12. The first now covers the window from `ACKNOWLEDGED` to the first heartbeat.
6. **Still open:**
   - Owner question 006-002 (the minor 1.8 reservation);
   - the delegate's thresholds;
   - the bound after a planned hand-back (Gap 006-047);
   - the constraints that stay (no emergency stop, two restarts for a rollback);
   - the release coupling for 0.3.0.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md (header, Refines, Enforced by, decisions 2, 4, 5, 10, 12, Bad, Rollback)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (decision 8, lines 50 and 88)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (397-398, 416-437, 510-521, 544-548, 604-621, 765-769, 802, 828-848)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-052, Decision 006-055 at 727-740, Decision 006-056 at 741, Decision 006-059, Decision 006-066)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (139-147, 186-199, 207-212, 221-239, 245-260)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md (151-169)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs (345-409)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/execution.rs (1015)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (7378-7421)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/server.rs (46, 84, 155-165)
