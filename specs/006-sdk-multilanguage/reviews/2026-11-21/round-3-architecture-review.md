# Architecture review, round 3: lease withdrawal (pull request #260, worktree `sagat-wt-withdraw-leases` at `45a3f42b`) and ADR-0098

**Verdict.** I found no Blocker. The layering holds and the code fixes of round 2 are in place:
- the holder's minor is capped at the build's ceiling, with a test;
- the capability lists, ups and downs, are checked by a Rust test that runs on every pull request touching `infra-postgres`, `application` or anything they depend on;
- the six SDKs hand back only from protocol 1.8, each with a test of both sides;
- `each_kind_live`, the floor's isolation guard, the one-second bound, the partial index, the dead-token heartbeat and the tracker token are each backed by a test.

One Major is left, and it is a documentation record, not code. Round-2 Major 1 asked this branch either to do the other requests of D006-059 (the delivery-acknowledgement design) or to record why not. Neither was done. ADR-0098 and the research heading also cite a decision D006-089 that does not exist. By the terms of OD-2026-11-21-4 ("no Blocker and no Major"), ADR-0098 should stay Proposed until that record is added. It is one research entry and two ADR sentences. After that, nothing I read in the decision itself argues against acceptance. Twelve Minors follow.

I worked read-only with file tools and no shell. I ran no tests, no `git branch -a`, no `git worktree list` and no scripts. Every "verified" below means verified by reading. I read the sibling worktrees `sagat-wt-022` and `sagat-wt-delivery-ack` directly; a search over every sibling worktree timed out. Line numbers refer to the worktree root unless a path says otherwise.

## Round-2 architecture findings

| Round-2 finding | Status | Evidence (read) |
|---|---|---|
| Blocker 1: `apps/execution-engine/tests/registry.rs` red | **Closed** | `registry.rs:298` compares with the whitespace removed. `apps/execution-engine/src/lib.rs:799-800` matches. |
| Blocker 2: owner decisions open | **Closed** | G006-031 is closed by D006-080 (delegate). Every SDK now gates on minor 8, and a host below 1.8 behaves as on `develop`: `WorkerHost.cs` `HandBackMinor`, Go `task.go` `handBackMinor`, Java `HAND_BACK_MINOR`, Python `HAND_BACK_MINOR`, Rust `HAND_BACK_MINOR`, TypeScript `HAND_BACK_MINOR`. A test of both sides exists in all six. Acceptance is conditional (OD-2026-11-21-4). |
| Major 1: holder's minor not capped; the other D006-059 requests | **Cap closed; the rest open** | The cap is `.min(self.hand_back_ceiling)` at `crates/application/src/worker/withdrawal.rs:232-234,269-271`. The setter is gated behind `test-support` (`:138`). Test: `tests/contract/durable-execution/grpc/lease_withdrawal.rs:886`. The 1.8 reservation in `sdks/SPEC.md` section 9 is still missing (Minor 4). D006-059 requests (1), (2), (5), (6), (7) and (8) are neither done nor recorded (Major 1). |
| Major 2: capability-list check off the pull requests; downs; stamp order | **Lists and downs closed; stamp order open** | `crates/infra-postgres/tests/capability_lists.rs` has two tests, the second for downs, and `scripts/check-architecture.py:636-672` mirrors them. CI runs `cargo test -p <changed and dependents>` (`.github/workflows/ci.yml:110-121`, `scripts/select-test-targets.py:450`). ADR-0098 decision 8 is corrected. D006-033 still says "Down migrations are not checked yet" (`research.md:487`). Nothing enforces stamp order and nothing records it (Minor 1). |
| Major 3: generic command cannot express 022's rule | **Closed on this branch; adoption pending** | `each_kind_live` is at `apps/control-plane/src/capability_activation.rs:37,183-202,274,308`. The census returns `out_kinds_live` (`20270208000200_lease_withdrawal_floor.up.sql:967-978`). There is a unit test and an integration test. The under-lock judge is untested (Minor 2). `docs/runbooks/instance-registry.md:136-137` and ADR-0082 still say each feature has its own command (Minor 3). The 022 worktree still has `flow-control-activate` and `live_capable_step`, and nothing in it mentions `capability-activate`. |
| Major 4: no six-language conformance scenario | **Moved, with a reason** | D006-081 moves it to the build of protocol 1.8. I could not find the target task in the delivery-acknowledgement tasks (Minor 5). |
| Minor 1: rollback text wrong about the down | **Open** | ADR-0098 lines 95-96 and `20270208000000_lease_withdrawal.down.sql:1-3` (Minor 6). |
| Minor 2: ADR-0082 decision 7 contradicted | **Closed** | ADR-0098 "Refines" line. |
| Minor 3: documents disagree with code | **Partly closed** | The `disabled` row is fixed (runbook line 74). D006-004 still names `lease-withdrawal-activate` (`research.md:234`). |
| Minor 4: ADR incomplete | **Partly closed** | The ADR-0029 header is done. Considered options still give no alternatives for decisions 5, 7 and 8 (Minor 6). |
| Minor 5: no held-lock crossing test | **Closed** | `tests/integration/platform-foundation/migrations.rs:831` and the reverse order after it, with `pg_stat_activity` as the barrier. There is a timing caveat (Minor 9). |
| Minor 6: the cap trusts `withdrawn_before` | **Open, not recorded** | `crates/domain/src/execution/command.rs:1855` (Minor 11) |
| Minor 7: test seams in production ports | **Open, not recorded** | `crates/api-grpc/src/service.rs:115`, `crates/api-grpc/src/undelivered.rs:31-37`, `crates/application/src/worker.rs:495` (Minor 11) |
| Minor 8: `with_withdrawal_concurrency` order | **Open, not recorded** | `crates/api-grpc/src/service.rs:104-106`. D006-088 raises the stakes: a guard now holds a token of whichever tracker its sink captured (Minor 11). |
| Minor 9: Python `__main__` guard | **Closed** | It is at the end of `scripts/tests/test_check_architecture.py`. |
| Minor 10: alert; Studio label | **Label closed; alert open** | `frontend/apps/studio/src/model/events.ts`, with its test. No match for `worker_leases_undelivered` under `infra/` (Minor 11). |
| Notes | **Mixed** | Closed: the runbook says "every engine". Partly closed: the removal function carries `lock_timeout = 1s`, but the manual SQL of `instance-registry.md` section 5 still holds `platform_requirement_lock()` for a human's time. Neither fixed nor recorded: fairness across tenants (D006-038) and the hot requirement row. |

## New findings

### Major 1. The D006-059 requests are not recorded, and ADR-0098 cites a decision that does not exist
- **Where:**
  - `specs/006-sdk-multilanguage/research.md:542` (the heading "D006-080 to D006-089"), but the list ends at D006-088 (`:617`).
  - ADR-0098 line 100 (Evidence: "D006-080 to D006-089").
  - ADR-0098 lines 44 and 59 cite "ADR-0099 decision 9". ADR-0099 is not on this branch.
- **What the sibling says.** `sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md:802-832` (D006-059, under ADR-0099, which the owner accepted in OD-2026-11-21-5 on that branch) lists what it asks of this build:
  - (3), the ceiling, is present.
  - (2) `NOT_ACKNOWLEDGED` in `WithdrawalCause` and (7) the registry-silent refusal are "hard preconditions" for this build to be the rollback floor.
  - (5), (6) and (8) are also missing.
  - (4) is a "Refined by ADR-0099" pointer.
  - Those still to do "go into a follow-up of pull request #260". Whether #260 is released before that follow-up is "a release choice".
  - Request (7) is to be recorded in ADR-0098 as an amendment of ADR-0082 decision 5.
- **On this branch:** none of this appears. A search for `D006-059` or "follow-up" in the feature's documents finds nothing.
- **Principle:** AGENTS rule 2 (update dependents when another decision changes yours), and the round-2 request "either do them, or record that they are declined and why".
- **Consequence:**
  - Once accepted, ADR-0098 is the reference for the worker protocol's guarantees, yet it is silent about obligations that another accepted ADR puts on it.
  - The release owner reading `develop` cannot see that shipping #260 without the follow-up is a choice with a cost: that build is not a valid rollback floor for 1.8.
  - If #260 merges before #264, `develop` cites an ADR-0099 that is not there.
- **Smallest fix:**
  - Add D006-089, "What ADR-0099 asks of this build". Give each of requests (1) to (9) its status: done, which is (3) (`hand_back_ceiling`); moved to the follow-up of #260, with D006-059's own reason; or at acceptance, which is (4). State the release consequence in one sentence.
  - Add one sentence to ADR-0098 (Consequences, or the "Refines" line): "Refined by ADR-0099 when it merges; the requests it makes of this build and their status are D006-089."

### Minor 1. The merge rule for feature 022 needs the word "restamp", and stamp order has no fitness check
- **Where:**
  - D006-033 (`research.md:481-487`) still ends with "Down migrations are not checked yet", which contradicts D006-083.
  - The test message (`crates/infra-postgres/tests/capability_lists.rs:123`) and `scripts/check-architecture.py:670` both say "takes the union … in its own, newest migration".
- **Why it matters.** I worked through the version order by reading. 022 has `20270130000200_flow_control_capabilities` and its validate migration `20270130000700_flow_control_validate`.
  - If 022 merges second, keeps those stamps and adds a new union migration after `20270208000200`, then `each_down_migration_restores_the_capability_list_before_its_up` fails on this branch's `20270208000000_lease_withdrawal.down.sql`. That down restores the list without 022's values, but the list before it, by version, now has them.
  - The error points at this feature's down. The natural quick fix, editing it, turns every check green.
  - It also leaves 022's two migrations older than migrations already applied. sqlx applies an unapplied older version without complaint. On any database already at `20270208000200`, 022's VALIDATE then fails on registry rows that list `lease-withdrawal-v1`.
  - So the literal reading "a new migration after the newest" is unsafe. The safe rule is: restamp 022's capability migration and its validate after `20270208000200`, with the union in the up and this branch's list in the down.
- **Smallest fix:**
  - Say "restamp, never add" in D006-033 and in both messages, and delete the stale sentence.
  - Add a stamp-order check. The smallest is a refusal in `sagat_infra_postgres::migrate` of any pending migration whose version is below the highest one applied, with a unit test over two fixture migrations. A CI step comparing the versions a pull request adds with the base branch's highest would also do.

### Minor 2. The under-lock judge of `each_kind_live` has no test
- **Where:** `apps/control-plane/src/capability_activation.rs:303-309`. No entry sets `each_kind_live`, so a mutation that deletes `|| check_kinds(entry, &census.kinds_live).is_err()` keeps every test green.
- **Why it matters.** This is the code 022 will rely on (D006-082: "under the epoch lock the count is made again and judged the same way").
- **Smallest fix:** extract the closure into a named function (for example `refused_under_lock(entry, census, expect, first)`) and unit-test it with an entry that sets the field.

### Minor 3. The shared registry documents do not follow the generic command or the new bound
- **Where:**
  - `docs/runbooks/instance-registry.md:136-137` still says "Each feature … has its own command".
  - ADR-0082 has no line pointing to ADR-0098 decision 4.
  - `ALTER FUNCTION platform_requirement_remove(text, text) SET lock_timeout = '1s'` (`20270208000200_lease_withdrawal_floor.up.sql:964`) bounds the removal of every requirement, `schedules-v1` included. Only `docs/runbooks/lease-withdrawal.md` says so.
- **Smallest fix:**
  - Update section 5 of the instance-registry runbook: one command for every requirement, the one-second bound on removal, and a `SET LOCAL lock_timeout` in its manual SQL.
  - Add one amendment line to ADR-0082.
  - Write the checklist of what 022 must adopt in D006-082: entries with kinds ALL, `empty_fleet: false` and `each_kind_live: true`; remove `flow-control-activate` and `live_capable_step`; registry tokens on the command line; `--expect-live`; the restamp of Minor 1; its tokens in `Capability::ALL` and the parity test.

### Minor 4. The register of protocol minors does not reserve 1.8, and the owner decision it cites is not on this branch
- **Where:**
  - `sdks/SPEC.md:353` cites "OD-2026-11-21-6", which is not in this branch's `docs/owner-decisions.md`. It is only in `sagat-wt-delivery-ack/docs/owner-decisions.md:15`.
  - The register (`sdks/SPEC.md:718-730`) still gives "next" to drafts 020, 021, 025 and 026.
  - Six SDKs and the server (`HAND_BACK_PROTOCOL_MINOR = 8`) hard-code 8.
- **Why it matters.** The register is the guard against two features taking the same minor (OD-2026-11-15-8). If #260 merges first, `develop` contradicts itself.
- **Smallest fix:** add the register row ("1.8: reserved for delivery acknowledgement, OD-2026-11-21-6; offered after its throughput gate") and the owner-decision row. Alternatively, merge #264 first.

### Minor 5. The task that D006-081 moves the conformance scenario into was not found (suspected)
- **Where:** D006-081 (`research.md:564-572`) says the scenario "is recorded as a task of that build, next to … Tasks 006-058 to 006-064".
- **What I found:** in `sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md:414-501` I found no task that names a six-language scenario of a stopping host handing back an unstarted lease. That checkout may lag its remote.
- **Smallest fix:** add the sentence to Task 006-058 on that branch, or open a gap here that stays open until the task exists (AGENTS rule 8).

### Minor 6. ADR-0098 is not yet complete for acceptance in four places
- **Rollback (lines 93-96):** "the requirement removed first, then the down migration" is still wrong. The down refuses while the history names the capability (`20270208000000_lease_withdrawal.down.sql:11-14`), and the history exists after any activation. The header comment of that down (`:1-3`) repeats the error. The runbook (lines 110-114) is right.
- **Considered options:** there are no alternatives for decisions 5, 7 and 8. Decision 5 changed materially this round. Point to the "Weighed" lists of D006-080, D006-031 and D006-032.
- **"Decided by" (line 4):** it names only D006-030 to D006-039, not D006-080 to D006-088.
- **"Enforced by" (line 6):**
  - It omits `capability_lists.rs`, the isolation, crossing and bound tests in `migrations.rs`, `a_heartbeat_with_a_handed_back_token_is_stale`, the ceiling test, the per-SDK tests of both sides, `the_drain_gets_what_is_left_of_the_grace_less_one_second`, and the census test.
  - It still cites `release_upgrade_inflight.rs` as proving "no withdrawal before the activation". Correctness round-2 Minor 5 showed that assertion cannot fail. D006-004 (`research.md:234-240`) has the same problem and still names `lease-withdrawal-activate`.
- **Smallest fix:** correct the four places and add "superseded by D006-034" to D006-004.

### Minor 7. The feature's contract document still states the old host rule
- **Where:**
  - `specs/006-sdk-multilanguage/stream-completion.md:132` ("hand the excess leases back") and `:242-246` (rule 5, "A host hands back what it drops") have no "from 1.8". Under D006-010 this file is the contract.
  - `:220` lists the decisions only up to D006-039.
  - `docs/runbooks/lease-withdrawal.md:7` lists them up to D006-087, but D006-088 exists.
- **Smallest fix:** add "from protocol 1.8 (D006-080); below it the lease lapses" in both places, and extend the decision ranges.

### Minor 8. Operability of the index build and the release notes
- **Step order:** the index pre-build is step 4 of "Before the activation" (`docs/runbooks/lease-withdrawal.md:35-44`). It has to happen before the migration, which comes before step 1's upgrade. An operator following the order builds it too late.
- **Release notes:** `docs/releases/unreleased.md` has no entry for this feature. Missing are the new event, the SHARE lock on `run_events` during the build, the capability, the command and the SDK behaviour. The batch index of feature 018 is listed there with its recipe (line 105).
- **A comment that disagrees with the code:** the up migration's comment (`20270208000200_lease_withdrawal_floor.up.sql:4-5`) says the down migration's look at the journal is an index probe. But the down of `000200` drops the index before `20270208000000.down.sql:19` scans `run_events`.
- **Smallest fix:** move the step to the top under "Before upgrading", add the release-notes entry, and correct the comment.

### Minor 9. Three tests can fail for the wrong reason
- **The crossing test races the new bound.** `migrations.rs:831` (`a_removal_that_waits_for_a_withdrawal_sees_its_event_and_is_refused`) now runs against the one-second `lock_timeout`. The withdrawal must commit within one second of a 50 ms poll seeing the wait, or the removal fails with "lock timeout" instead of the guard's message. That is a wall-clock dependency (AGENTS rule 18).
  - Fix: on that test's own database, run `ALTER FUNCTION platform_requirement_remove(text, text) SET lock_timeout = '30s'` first. The bound is proved by its own test at `:947`.
- **The index test proves the index is valid, not that it is used.** `migrations.rs:947` asserts `indisvalid` only.
  - Fix: run `EXPLAIN` on the guard's `EXISTS` with `enable_seqscan = off` and assert `run_events_lease_withdrawn_idx` in the plan.
- **The ceiling test depends on today's constant.** `lease_withdrawal.rs:886` relies on `PROTOCOL_MINOR = 6`. It turns red when the 1.8 build raises the constant.
  - Fix: set `hand_back_ceiling: HAND_BACK_PROTOCOL_MINOR - 1` explicitly.

### Minor 10. The Rust SDK changes a production constant under `cfg(test)`
- **Where:** `sdks/rust/src/worker.rs:119-124`: `NEWEST_MINOR` is 6 in a build and 8 in the unit tests.
- **Why it matters.** The unit tests never exercise the shipped acceptance check, and no test proves that a newer accepted minor is refused.
- **Smallest fix:** make the ceiling a field that a test-only constructor sets, as `hand_back_ceiling` does on the server, and add one test that accepted 1.7 is refused under the shipped ceiling.

### Minor 11. Four round-2 Minors are neither fixed nor recorded
- **Which:**
  - The cap trusts its caller (`command.rs:1855`).
  - Test seams sit in production ports: `service.rs:115`, `undelivered.rs:31-37`, `worker.rs:495`.
  - `with_withdrawal_concurrency` depends on call order (`service.rs:104-106`).
  - There is no alert on `outcome="capped"` or `outcome="error"`.
  - Two round-2 notes (fairness across tenants, the hot requirement row) are in the same state.
- **Smallest fix:** record each as a gap with its reason in `research.md`, or fix it.

### Minor 12. The fitness index has no row for the new rule
- **Where:** `docs/engineering/architecture-principles.md:44` says "When you add a rule worth keeping, add its row". There is no row for "capability lists in SQL equal `Capability::ALL`, and each down restores the list before its up".
- **Smallest fix:** add the row, pointing to `crates/infra-postgres/tests/capability_lists.rs` and `scripts/check-architecture.py`.

## Layering and compatibility (verified by reading)

- **Domain:** untouched this round.
- **Application:** the additions use only `std`: the `hand_back_ceiling` field, `effects.reported = None` in both `NOT_STARTED` branches (`worker.rs:2483`, `scoped_reports.rs:382`), and the setter behind `test-support`. `scripts/check-architecture.py` already refuses `test-support` outside dev-dependencies.
- **Adapters and apps:** `api-grpc` holds a `TaskTrackerToken`. `infra-postgres` maps census tokens to `InstanceKind` and refuses a token it does not know. The policy (`check_kinds`) lives in `apps/control-plane`. There is no `serde_json` in the domain and no empty mapper. A small existing smell grows by one field: `lacking()` builds a fake `CapabilityCensus` only to format a message (`capability_activation.rs:328-337`).
- **Migration `20270208000200`:**
  - The census gains one output column, and readers select by name, so older binaries of this feature keep working.
  - The function keeps `SECURITY DEFINER`, its pinned `search_path` and its owner-only grant.
  - `run_events` is not partitioned, so `CREATE INDEX CONCURRENTLY` works.
  - The down restores the previous definitions and holds no data.
- **Upgrade oracles:** the three released-v0.2 upgrade tests assert `20270208000200` (`release_upgrade.rs:205`, `release_upgrade_callback_cutover.rs:123`, `release_upgrade_inflight.rs:181`). No other file asserts `20270208000100`.

## Fitness functions for the rules this round relies on

| Rule | Check | Fails when broken? |
|---|---|---|
| Hand-back judged at the build's ceiling | `lease_withdrawal.rs:886` | Yes, until 1.8 changes the constant (Minor 9) |
| SDKs hand back only from 1.8 | one test of both sides in each of the six SDKs | Yes, per SDK. Six-language conformance moved (Minor 5). |
| Capability lists of the ups | `capability_lists_follow_the_code`; `check-architecture.py` at a release | Yes, on every pull request touching `infra-postgres`, `application` or their dependencies |
| Downs restore the list before them | `each_down_migration_restores_the_capability_list_before_its_up`; the Python mirror with two negative fixtures | Yes |
| Migration stamp order | none | No (Minor 1) |
| `each_kind_live` | `an_entry_that_requires_each_kind_live_refuses_a_kind_with_none`; `the_census_names_the_kinds_that_have_a_live_process` | Partly: the judge under the lock is untested (Minor 2) |
| Removal only under READ COMMITTED; bounded; index probe | `migrations.rs:831`, the reverse order, the isolation test, `:947` | Yes, with a timing caveat; index use not proved (Minor 9) |
| A handed-back token is dead for heartbeats | `lease_withdrawal.rs:397` | Yes |
| Drain waits for a unary claim | `a_graceful_shutdown_waits_for_a_unary_claim_in_progress_and_its_withdrawal`; `drain_bound` unit test | Yes (the commit says it failed on the old code; I did not see the run) |

## What I suspect but did not verify

- **Delivery-acknowledgement checkout:** that the checkout in `sagat-wt-delivery-ack` matches its remote branch (Minor 5).
- **Hand-back minor timing in three SDKs:** Java, Python and TypeScript read the host's current accepted minor when they hand back, not the minor of the session that received the lease, as `sdks/SPEC.md` section 6.2 says to. Their receive loop and their re-registration appear to run on one thread, so the two should be the same. I did not trace that end to end.
- **A public option that can claim 1.8:** `WithProtocolVersion` in Go is public. A user who sets "1.8" on an SDK of this build would claim a minor whose acknowledgements that SDK does not send. That belongs to ADR-0099 rather than this pull request.

**Counts: Blockers 0, Majors 1, Minors 12.**

Files: `docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md`, `specs/006-sdk-multilanguage/research.md`, `specs/006-sdk-multilanguage/stream-completion.md`, `sdks/SPEC.md`, `docs/runbooks/lease-withdrawal.md`, `docs/runbooks/instance-registry.md`, `crates/infra-postgres/tests/capability_lists.rs`, `scripts/check-architecture.py`, `crates/infra-postgres/migrations/20270208000200_lease_withdrawal_floor.up.sql`, `crates/infra-postgres/migrations/20270208000000_lease_withdrawal.down.sql`, `apps/control-plane/src/capability_activation.rs`, `tests/integration/platform-foundation/migrations.rs`, `tests/contract/durable-execution/grpc/lease_withdrawal.rs`, `sdks/rust/src/worker.rs`, `../sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`, `../sagat-wt-delivery-ack/docs/owner-decisions.md`, `../sagat-wt-022/apps/control-plane/src/flow_control.rs`.
