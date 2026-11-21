# Architecture review, round 2, 2026-11-21

The `architecture-reviewer` agent (read only), given the diff of the fixes of the first round (`0f482673..557aff1b`, the six
SDKs' code left out) and the documents of the feature. Its final message, verbatim (only its closing list of files is
shortened to paths relative to the worktree):

---

# Architecture re-review: `bugfix/withdraw-undelivered-leases` at `557aff1b` (feature 006, ADR-0098)

I had read-only file tools and no shell. I ran no tests, no `git branch -a` and no `check-architecture.py`. I found the sibling worktrees by reading `.git/worktrees/*/HEAD`. Line numbers refer to `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` unless a path says otherwise.

**Verdict.** The layering is sound and most first-round findings are closed in code. Three things block the merge:
- **NB1:** a test on this tree is red.
- **NB2:** ADR-0098 and gap G006-031 still need the owner.
- **NM1:** the server reads the host's protocol minor from the session row without capping it at the build's own minor, which the sibling design forbids.

---

## Status of the first round's Blockers and Majors

| Round 1 | Status | Evidence / what is left |
|---|---|---|
| **B1** a handler can raise `TASK_NOT_STARTED` and loop forever | **Closed for this build** | Three layers now stop it: (1) a hand-back counts only from minor ≥ 8, and this build accepts at most 6 (`worker.rs:75`); (2) at most 4 withdrawals per attempt, enforced in the domain (`command.rs`, `WithdrawalsSpent`); (3) the SDKs rewrite the code when a handler raises it. Tests: `a_hand_back_from_a_session_older_than_1_8_is_an_ordinary_failure_with_its_flag`, `one_attempt_is_handed_back_at_most_four_times`, `one_attempt_is_withdrawn_at_most_four_times`. **Left over:** the minor is read without a cap (NM1). |
| **B2** removing the requirement lets builds that cannot replay serve again | **Closed** | A trigger on `platform_requirements` refuses the removal, each withdrawal reads the requirement under a `FOR SHARE` lock, and `SAGAT_LEASE_WITHDRAWAL=off` stops new writes. Tests: `the_requirement_cannot_be_removed_once_a_journal_holds_a_withdrawal`, `a_requirement_the_registry_does_not_record_or_the_switch_off_withdraws_nothing`, and the four down guards in `migrations.rs`. **Left over:** no test holds a lock while a withdrawal and a removal cross (m5); the rollback text is still wrong (m1). |
| **B3** two branches rewrite the same three CHECK lists | **Partly closed** | The check exists and fails when the newest definition differs from the code. But it does not run on the pull request it guards, and it ignores down migrations and stamp order (NM2). |
| **M1** ADR-0098 overturns ADR-0029 decision 5 | **Closed in text; acceptance still needed** | The supersession is stated with its argument. ADR-0029 gets a sentence in its body; its header still says nothing (m4). ADR-0098 is still Proposed (NB2). |
| **M2** a graceful shutdown does not wait for claims in progress | **Closed** | The producer and the unary claim now run on the tracker (`service.rs`, `Withdrawals::spawn`), and `drain_bound` uses what is left of the grace. Test: `a_graceful_shutdown_waits_for_a_claim_in_progress_and_its_withdrawal`. `drain_bound` has no unit test. |
| **M3** nothing limits concurrent withdrawals | **Closed** | A semaphore sized to a quarter of the pool. The unit test checks the division only, not that a sixth withdrawal waits. See the fairness note. |
| **M4** evidence the documents claim is missing | **Closed** | The 0.2 upgrade assertions, the down-guard test, `verify_run` after every withdrawal, and a scoped withdrawal for each cause on PostgreSQL (`bounded_worker.rs`). D006-004 is corrected. |
| **M5** two divergent activation commands | **Partly closed** | `capability-activate` exists, but it cannot express feature 022's rule, and 022 has not adopted it (NM3). |
| **M6** host rule in SPEC with no SDK and no conformance | **Half closed** | The SDK implementations are now on this branch (for example `sdks/go/worker/task.go:117-119`). There is still no six-language conformance scenario (NM4). |

---

## Blockers

### NB1. `apps/execution-engine/tests/registry.rs:298` now fails
- **The cause:** the test checks that the source text contains `WorkerGrpc::new(state, workers).with_serving_gate(`. The fix split that call across lines at `apps/execution-engine/src/lib.rs:799-803`. A search of `apps/execution-engine/src` finds zero occurrences of the string.
- **What breaks:** this is the ADR-0082 fitness check that the worker API obeys the gate. It runs on this pull request because the package changed.
- **Smallest fix:** compare with whitespace removed, e.g. `source.split_whitespace().collect::<String>().contains("WorkerGrpc::new(state,workers).with_serving_gate(")`. Alternatively, build `worker_api` in two statements so the original call stays on one line.

### NB2. Two owner decisions are still open, and this branch makes one of them live
- **ADR-0098 acceptance.** The ADR (line 4) says acceptance is the owner's, because decision 1 overturns part of accepted ADR-0029 decision 5. `docs/owner-decisions.md` has no entry dated 2026-11-21.
- **G006-031 (`research.md:530-536`).** This branch now carries the six SDKs' hand-back.
  - The SDKs hand back on every minor (`sdks/go/worker/task.go:117-119`: `take` calls `handBack` whenever `why` is set, with no check of the minor).
  - The server honours a hand-back only from 1.8, so today every hand-back is an ordinary failure.
  - Consequence: on merge, a stopping host's unstarted lease of a one-attempt `Activity` becomes `step.failed` (its `onError` route or a failed run). On `develop` the same lease lapses into a review.
  - That changes a guarantee of the worker protocol. G006-031 itself says the choice is the coordinator's, then the owner's.
- **Smallest fix that needs no owner decision:** the SDKs hand back only when the accepted minor is ≥ 8 (option 1 of G006-031). Below 1.8, leases lapse exactly as on `develop`. Otherwise, record the owner's answer before merging.

---

## Major

### NM1. The holder's minor comes from the session row, without a cap, which the sibling design forbids
- **Where:**
  - `crates/application/src/worker/withdrawal.rs:23` (`HAND_BACK_PROTOCOL_MINOR = 8`) and `:181`.
  - `:220` and `:253` (`root_hand_back` and `path_hand_back`).
  - `crates/infra-postgres/src/execution.rs` (`lease_holder_minor`: `worker_sessions.accepted_protocol_minor` read as stored).
- **What the delivery-acknowledgement branch says** (`/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack`):
  - ADR-0099 decision 9: the reservation is read "never from the session row, which registering again can change". Also: "a build of ADR-0098 never honours a hand-back from a 1.8 session: it clamps the holder's minor to its own ceiling".
  - D006-059(3) lists this as an amendment this branch makes "before it merges".
- **Consequence:** during the rolling upgrade to the 1.8 build, an engine of this build serves a `FailStep` for a session that a 1.8 engine registered (stored minor 8). It honours the hand-back without 1.8's rules, so an acknowledged lease, or one a capability was served to, is withdrawn. Once 0.3.0 ships, deployed engines cannot be fixed after the fact.
- **The register:** `sdks/SPEC.md:713-722` (§9) does not reserve 1.8. "next" goes to drafts 020, 021, 025 and 026 in the order their contracts merge. The constant claims a minor the register has not given out.
- **Smallest fix:**
  - Decide on `minor.min(PROTOCOL_MINOR)` in both `*_hand_back` functions.
  - Make `as_protocol_1_8` in the tests raise both the stored minor and the ceiling, as D006-059(3) says, and add a test that a stored 8 under ceiling 6 is an ordinary failure.
  - Reserve 1.8 in §9, or say so in G006-030.
- **The other D006-059 asks:**
  - (2) `NOT_ACKNOWLEDGED` added to `WithdrawalCause` and its codec, as the rollback floor.
  - (5) the derivation of the cap of four says whether the 1.8 causes count.
  - (6) the guard's error text says "of unmarked leases" before the migration is applied anywhere. The text is at `20270208000000_lease_withdrawal.up.sql`, in the `RAISE` of the replay-floor guard.
  - (7) an engine whose registry heartbeat has been failing longer than the liveness window serves no worker call.

  Either do them, or record in this branch's `research.md` that they are declined and why. Today the two branches silently disagree.

### NM2. The capability-list check does not run on the pull request it is meant to protect, and covers only half of B3
- **When CI runs it:** the `architecture` job (`.github/workflows/ci.yml:331-347`) runs `check-architecture.py` only when `full == 'true'`, which means a release. On a pull request the job runs only `unittest discover`, and only when a file under `scripts/` changed (`scripts/select-test-targets.py:416-417`).
- **The gap:** 022's rebased pull request changes migrations and `instances.rs`, not `scripts/`, so nothing checks it. A merge into `develop` runs no CI at all.
- **Statements that are false:** ADR-0098 line 53 ("checks on every pull request") and D006-033 ("the order of the merges cannot drop a capability").
- **What is not checked:**
  - Down migrations. The test even blesses a down that restores an arbitrary older list (`test_check_architecture.py`, `test_the_newest_migration_listing_every_capability_passes`). If 022 lands first, this branch's `.down.sql:30-42` would erase 022's two values.
  - Stamp order. 022's `20270130000200` is older than `20270208000000`. If 022 lands second, a database already at `20270208` applies it out of order. Its validate migration then fails, because every process of this build lists `lease-withdrawal-v1`. `.claude/skills/migration/SKILL.md:7` forbids such a stamp, but nothing enforces it.
- **Smallest fix:** move the rule into a Rust `#[test]` in `crates/infra-postgres`, which already depends on `sagat_application`. It would:
  - read `migrations/*.sql`;
  - compare the newest up of each of the three checks with `Capability::ALL.iter().map(as_str)` directly (no regex over Rust);
  - assert that each down restores exactly the list of the previous up.

  That test runs on every pull request that touches `infra-postgres` or `application`. Then correct line 53 and D006-033, and tell the 022 session to restamp after `20270208`.

### NM3. The generic `capability-activate` cannot express 022's rule, and 022 has not agreed to it
- **What 022 requires:** at least one live, capable process of **each** kind (`/Users/marcelo/Documents/Projects/sagat-wt-022/apps/control-plane/src/flow_control.rs:54-56`, `live_capable_step`; `20270130000200_flow_control_capabilities.up.sql:24-33`).
- **What the generic command offers:** `Activatable` (`capability_activation.rs:29-34`) has only `empty_fleet`, which looks at the total across kinds, and the step is always `census_step` (`:288`).
- **Consequence:** an entry `{kinds: ALL, empty_fleet: false}` activates with two control planes live and no engine. 022 refuses exactly that.
- **Governance:** D006-034 decides 022's command line ("Feature 022 … drops `flow-control-activate`") from 006's research. Nothing in the 022 worktree mentions `capability-activate` or D006-034 (AGENTS rule 2). `instance-registry.md:136-137` still says "Each feature … has its own command".
- **Smallest fix:**
  - Add `each_kind_live: bool` to `Activatable`, backed by a census that returns counts per kind, or let an entry name its own step.
  - Record the adoption in 022's research, plus one line amending ADR-0082 (or the runbook's §5).
- This does not block this branch; it blocks 022's adoption.

### NM4. The hand-back has no six-language conformance scenario (AGENTS rule 8)
- No file under `tests/contract/sdk-conformance` mentions the hand-back or `TASK_NOT_STARTED`. The behaviour is covered only by each SDK's own unit tests (`handback_test.go`, `test_hand_back.py`, `WorkerHandBackTest.java`, …).
- **Smallest fix:** one scenario, `SAGAT_SDK_SCENARIO=hand-back`. In it, a host that stops with an unstarted lease sends `FailStep TASK_NOT_STARTED`, retryable, and a handler that raises the code is reported as `WORKER_ERROR`. The expected journal is the same in all six languages.

---

## Minor

1. **The rollback text is wrong about the down migration.**
   - ADR-0098:89-90 says the way back is "the requirement removed first, then the down migration".
   - But the down migration also refuses while the history names the capability (`.down.sql:11-14`), and the history exists after any activation. So after an activation, only a binary rollback is possible.
   - The down's header comment (`.down.sql:1-3`) still says "remove the requirement first".
   - Say "binary only; the schema needs the history exported and removed (`instance-registry.md` §5)".
2. **ADR-0082 decision 7 is contradicted without a record.** That decision says a capability is "never listed for what the binary could do and the process will not". An engine with `SAGAT_LEASE_WITHDRAWAL=off` still lists `lease-withdrawal-v1` (`apps/execution-engine/src/capabilities.rs:29-30`). The reason is sound (the capability means "can replay"), but ADR-0098's "Refines" line should say it refines decision 7 for a replay capability.
3. **Documents disagree with the code.**
   - D006-004 (`research.md:234`) still names `lease-withdrawal-activate` and "with no step of its own", with no "superseded by D006-034" note.
   - The `disabled` row (`lease-withdrawal.md:61`) says "the lease lapses". For a hand-back, `hand_back` returns `Err(Disabled)`, which is an ordinary failure, not a lapse.
   - The heading of the new research section omits G006-031.
4. **ADR-0098 is incomplete for its new decisions.** Its Considered options have no alternatives for decisions 5, 7 and 8; point to the "Weighed" lists of D006-030 to D006-032. Its Consequences and Evidence omit G006-031 and the fact that the activation is forward-only. ADR-0029's header has no "superseded in part by ADR-0098" line.
5. **The crossing in D006-032 has no held-lock test.** The claim is that a withdrawal and a removal cannot cross. I traced the PostgreSQL semantics and they are sound, but no test fails if `lease_withdrawal_required()` loses its `FOR SHARE`. Add a fault test that:
   - holds a withdrawal transaction after the read;
   - runs `platform_requirement_remove` concurrently and watches it wait in `pg_stat_activity`;
   - releases the withdrawal and asserts the removal is refused (rule 18).
6. **The cap trusts the number its caller passes in.** `withdraw_lease` and `withdraw_*_instance` take `withdrawn_before`. A future caller that passes `0` bypasses the cap, and no test fails. Fix with a newtype that only `lease_withdrawals` can construct, or a source-text test over the call sites. The round-1 request to add `Withdraw` to the property model (`crates/domain/tests/execution_properties.rs`) is still not done.
7. **Test seams sit in production ports.** `ClaimWatch::committed` (`crates/application/src/worker.rs:495`) is a test barrier inside an application port. `with_delivery_probe` (`service.rs:115`), `DeliveryProbe` and `Abandonment` are still public (round-1 Minor 2). Gate them behind `test-support`.
8. **`with_withdrawal_concurrency` depends on call order.** It replaces the whole `Withdrawals`, tracker included (`service.rs:106`, "call it before `withdrawals` is read"). Anything that captured the old one would sit on a tracker nobody drains. Pass the concurrency to the constructor, or resize only the semaphore.
9. **The new Python test class is defined after the `if __name__ == "__main__"` guard** (`scripts/tests/test_check_architecture.py:726-729`). Running the file directly skips it; CI's `discover` still runs it. Move the guard to the end.
10. **Operability.**
    - There is no alert on `outcome="capped"` or `outcome="error"` in `infra/observability`, although the runbook says a growing `capped` count means a loop.
    - Studio still has no label for `step.lease_withdrawn` and still merges attempts by number (round-1 Minor 6; AGENTS rule 10).

---

## Notes

- **The layering is right.**
  - The domain stays pure: the cap constant and `WithdrawalsSpent` live there, and there is no serde or I/O.
  - `ClaimWatch` and `hand_back` in the application use only `std`. `hand_back` is a pure function tested at its edges.
  - The counter now has one writer and a test over its labels, which closes round-1 Minor 1.
  - The three new methods on `RunTx` and `PathExecutionTx` are justified: the requirement must be read under the lock, in the same transaction.
  - `lease_holder_minor` does put a read of worker sessions into the execution port. ADR-0099 moves the reservation onto the lease (`steps.lease_hand_back_generation`), which would remove that cross-module read too.
- **The requirement read is hard-coded.** `lease_withdrawal_required()` names `lease-withdrawal-v1`. ADR-0099 needs a second requirement read in every withdrawal (`delivery-acknowledgement-v1`), so a `platform_requirement_held(text)` would serve both.
- **A new lock dependency.** Runtime transactions now hold `FOR SHARE` on a requirement row that the operator's removal (holding the epoch `FOR UPDATE`) waits for. The command's 2 s lock bound covers this. The manual SQL in `instance-registry.md` §5 sets no `lock_timeout`, so add one there.
- **Fairness across tenants.** The withdrawal semaphore is first-in first-out for all tenants. One tenant's dropped streams can delay another tenant's withdrawals past the lease's time-to-live, and those leases then lapse into review. Say so in D006-038, or give each tenant a share.
- **A mixed setting gives mixed behaviour.** With `SAGAT_LEASE_WITHDRAWAL` different across engines, the same hand-back is a withdrawal or a spent attempt depending on which replica serves the `FailStep`. The runbook should say "every engine".
- **Branches:** `sagat-wt-delivery-ack` and `sagat-wt-022` complement this branch and do not duplicate it. This branch now also contains the SDK hand-back work; I inferred that from the contents of `sdks/` and could not confirm it with git.

---

## Fitness functions

| Rule relied on | Check today | Fails when broken? | Smallest addition |
|---|---|---|---|
| The worker API obeys the gate | `apps/execution-engine/tests/registry.rs:298` | **Red now** (NB1) | Compare with whitespace removed |
| A hand-back counts only from 1.8 | `hand_back` unit test, contract test | Yes, but does not cover a stored 8 under ceiling 6 | Cap at the build's minor and test it (NM1) |
| Cap of four per attempt | domain, contract and unit tests | Yes, unless a caller passes 0 | Newtype or source-text check (m6) |
| Requirement as replay floor | contract test of the refused removal, migration test | Yes | Held-lock crossing test (m5) |
| Capability lists in SQL = `Capability::ALL` | `check-architecture.py` | **Only at a release or when `scripts/` changes** (NM2) | Rust test in `infra-postgres`, downs included |
| Shutdown waits for claims | contract test with `drain` | Yes | Unit test of `drain_bound` |
| Withdrawal concurrency bound | division unit test | Partly | Test that a sixth withdrawal waits |
| Six SDKs, one semantics for the hand-back | none | No | Conformance scenario (NM4) |
| Closed metric labels | label test | Yes | none |

---

## What the owner must decide
1. Accept ADR-0098, since it overturns part of accepted ADR-0029 decision 5.
2. G006-031: for one-attempt steps, should a hand-back below 1.8 spend the attempt, or should the SDKs keep letting those leases lapse into review? If the answer is to keep `develop`'s behaviour, the SDKs hand back only from 1.8.
3. Through the coordinator, whether D006-059's asks bind this branch. NM1's cap on the minor is a one-line fix and I would make it either way.

Files referenced: `apps/execution-engine/tests/registry.rs`, `apps/execution-engine/src/lib.rs`,
`apps/execution-engine/src/capabilities.rs`, `apps/control-plane/src/capability_activation.rs`,
`crates/application/src/worker/withdrawal.rs`, `crates/application/src/worker.rs`, `crates/infra-postgres/src/execution.rs`,
`crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql` and `.down.sql`, `crates/api-grpc/src/service.rs`,
`scripts/check-architecture.py`, `scripts/tests/test_check_architecture.py`, `scripts/select-test-targets.py`,
`.github/workflows/ci.yml`, `docs/adr/ADR-0098-*.md`, `docs/adr/ADR-0082-*.md`, `docs/runbooks/lease-withdrawal.md`,
`docs/runbooks/instance-registry.md`, `specs/006-sdk-multilanguage/research.md`, `sdks/SPEC.md`, `sdks/go/worker/task.go`;
and in sibling worktrees `sagat-wt-022/apps/control-plane/src/flow_control.rs`,
`sagat-wt-022/crates/infra-postgres/migrations/20270130000200_flow_control_capabilities.up.sql`,
`sagat-wt-delivery-ack/docs/adr/ADR-0099-*.md` and `sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md` (D006-059).
