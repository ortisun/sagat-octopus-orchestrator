# Re-review (architecture): feature 022 rate gate, a67ba790 / a3a2d2d3

Reviewer: `architecture-reviewer` (read-only, diff and spec only), 2026-11-20, over commits a3a2d2d3 and a67ba790 of
`feature/022-throughput-control` (diff `d5e0c792..a67ba790`). Text as returned; resolutions in
`rate-gate-rereview-responses.md`.

I found one Blocker. Fixes the slice brought in itself also create a new Major, and the release gates are not yet recorded everywhere they need to be.

I could not run anything: this session had no shell. Every judgement below comes from reading the diff and the current tree, so the authors' green-suite claims are not re-verified. For the same reason I could not run `git branch -a` or `git worktree list` for the scope check.

**Verdict:** most of the earlier findings are closed: the three Blockers, both Majors about the count instant and the hint, the refusal kind, and the test door. The layering, the single decision core and the lock order hold. But the fix for security Major 1 opened a hole in the ceiling that T022-077 is ticked for (Blocker 1). It also leaves window rows with no bound except a purge that is known not to scale (Major 1). Four gates are left open on purpose: G022-016/T022-086 (reset), T022-079 (consistency check), T022-081 (barriers) and G022-018 (purge scaling). They are recorded only in part where they bind (Major 2).

What blocks a merge: Blocker 1 (fix it or untick T022-077) and Major 2 (doc lines and one test). Major 1 blocks the rate capability's activation and the release.

## Status of the earlier Blockers and Majors

| Finding | Status | Where |
|---|---|---|
| Architecture Blocker 1 / correctness Major 1 (checker tests garbled) | **Resolved** | `scripts/tests/test_check_architecture.py:809` now has one `EveryRunIsCreatedWithACheckedInput`; the stale test is gone; new class at `:966`. A `unittest.main()` at `:620`, older than this slice, remains (see Nit). |
| Architecture Blocker 2 (upgrade oracles) | **Resolved** | `release_upgrade.rs:205`, `release_upgrade_inflight.rs:181`, `release_upgrade_callback_cutover.rs:123` assert `20270206000100`, the newest migration in the tree. |
| Architecture Blocker 3 (hot-path budget) | **Resolved** | The reasons in `plan.md:595-596` and `work_budget.rs` match the code. `existing_keys` costs one statement whenever a round has a window (`readiness_gate.rs:556`). `live_keys` runs only for a new window. `window_due_after_grant` runs only `if waiting` (`:750`). The activation read is real. Gap: no line for a run-level rate start (Minor 6). |
| Architecture Major 1 / correctness Major 5 (reset of a rate-limited start) | **Not resolved** (kept, with a gate) | `readiness_gate.rs:1122` still answers 503 `BINARY_CAPABILITY_MISSING`, which is retryable. The gate is only partly recorded (Major 2). |
| Architecture Major 2 / security minor 4 (purge test door) | **Resolved by a check** | `purge_as_of_door_errors` in `check-architecture.py`, with two fixtures (`test_check_architecture.py:966`). There is no row in the principles table (Minor 5). |
| Architecture Major 3 (rate consistency across runs) | **Not resolved** | `verify_start_rate` in `execution.rs` covers only the run's own start. Step scope, `grants_total` and cross-run `key_consistency` remain T022-079, which runbook §6 does not list (Major 2). |
| Architecture Major 4 (fault suite and barriers) | **Partly** | The production warning is in `docs/runbooks/flow-control.md:184`. The two barrier tests asked for were not added. My reading of the lock order finds no deadlock (see "What holds"). |
| Architecture Major 5 / security minor 6 (purge) | **Partly** | Added: `SKIP LOCKED` (`maintenance.rs:1341`), 500 grants per key (`passes.rs:96`), a `rate_purge`/`done` counter. Still missing: shard-driven work, an order, an index, an `outcome="failed"`, and the oldest-purgeable-age gauge. The candidate query (`maintenance.rs:1312`) is still a full scan. See Major 1. |
| Architecture Major 6 / correctness minor 1 (refusal kind) | **Resolved** | `start_refused(…, key_of, …)` at `readiness_gate.rs:1550`; `start_may_wait` passes the refusing gate's kind; the unit test covers both dual-gated cases. |
| Correctness Major 2 (count instant) | **Resolved in code** | The decision instant is bound into the readiness count (`$8`) and into the queue's `admits` (`$12`), deadlines included. No test can see this race (see Fitness summary). |
| Correctness Major 3 (rest later than counts) | **Resolved in effect** | The rest uses the hint the key was selected by (`maintenance.rs:266,276`), which is early and never late. It does not track the pass's real start time (Minor 1). |
| Correctness Major 4 (grant through another key's pass) | **Resolved** | `window_due_after_grant` (`readiness_gate.rs:1436`) is called from `grant` and from `grant_from_queue` under a window lock already held; a test is named. |
| Security Major 1 (purge lag counted against another tenant's live keys) | **Resolved as stated** | `live_keys` (`readiness_gate.rs:619-627`) and the usage query count only live windows. The same change opens Blocker 1 and Major 1. |
| Security Major 2 (lease-pass cost) | **Partly** | `st.attempt >= u.attempt` (`maintenance.rs:311-333`) removes the journal probes for grants that were never leased. Every unleased grant of each shard is still read, probed in `steps` and sorted every 500 ms tick. The index is deferred, and G022-018 only talks about the purge. |

## Blockers

- **Blocker 1. An aged window is reused without the `rate_limit_live_keys` check.**
  - **Where:** the Windows branch of `existing_keys` (`readiness_gate.rs:581`) treats any window row as existing, even one whose grants have all aged out. `live_keys` (`:619-627`) no longer counts such a row. The start path uses the same pair (`decide_start_gates`, `:1097`).
  - **Effect:** a namespace at its ceiling can bring back every aged window the purge has not reached yet, without any check. So live keys exceed `rate_limit_live_keys` by the size of the purge backlog, not just G022-002's accepted race margin.
  - **Principle concerned:** T022-077 is ticked `[x]` claiming this ceiling (rule 11, and "ticked only with the evidence that closes it").
  - **Smallest fix:** put the same liveness predicate into the Windows existence statement (one statement, so the budget is unchanged). Add a test: a namespace at its ceiling with one aged window is refused `FLOW_CONTROL_LIMIT` when that window is reused.

## Major

- **Major 1. Window rows now have no bound except purge throughput, and that gap is not a release gate. This is new, introduced by the security Major 1 fix.**
  - **Why rows pile up:** a run-scope grant is never given back, and a window row goes only when the purge reaches it.
  - **Purge throughput:** at most `RATE_PURGE_KEYS × PURGE_ROUNDS` = 64 × 20 = 1,280 keys per housekeeping (`passes.rs:91-96`, every 300 s by default per `settings.rs:589-591`). That is about 4.3 keys/s. Replicas scan the same keys in the same order and skip each other's.
  - **Purge cost grows with the backlog:** each call full-scans `rate_limit_grants` (`maintenance.rs:1312`), so the drain slows as the backlog grows.
  - **Effect:** sustained new distinct keys above that rate grow rows without bound, in every namespace. G1199's stated purpose ("a caller can create key rows without bound in one namespace") is lost for windows. G022-018 (`research.md:1138`) says "Live-key counting no longer depends on it" — that is exactly why nothing bounds the rows now.
  - **Smallest fix:**
    - List G022-018 in runbook §6 and mark it "release gate" in research and tasks.
    - Add a coarse row cap on the new-window path only: refuse `FLOW_CONTROL_LIMIT` (`rate_limit_live_keys`) when the namespace's total window rows reach, say, twice the ceiling. That is one statement, and only for a new window.
    - Fold the purge ordering, shard partitioning and index into T022-062/089.

- **Major 2. The open gates are not recorded everywhere they bind.**
  - **Runbook §6** (`flow-control.md:184`) lists T022-080, 081, 086 and 104. It omits:
    - **T022-079:** ADR-0088's "Enforced by" and its Evidence section both require the journal window invariant before activation.
    - **G022-018:** see Major 1.
  - **Research:** G022-016's status is "Open; T022-086" (`research.md:1136`). Compare G022-015 at `:1135`, which says "release gate … the release cannot close with it open".
  - **tasks.md:** T022-079 (`:384`), T022-081 (`:392`) and T022-086 (`:399`) carry no gate note.
  - **No test of the interim reset refusal:** nothing pins `readiness_gate.rs:1122`. Dropping that condition would silently turn the refusal into `CONCURRENCY_PERMIT_UNAVAILABLE` (via `start_may_wait`) or a grant.
  - **Judgement on the interim behaviour:**
    - Reset (retryable 503; pending resets retried forever and holding `pending_resets`): acceptable for a draft only once the gate binds in all three places and a test pins it.
    - T022-079 and T022-081: acceptable as interim states. The lock order holds by reading, and `verify_start_rate` covers the start.
    - G022-018: not acceptable as recorded (Major 1).
  - **Fix:** the doc lines above, plus one integration test. The test resets a rate-limited run, expects `BINARY_CAPABILITY_MISSING` with nothing written, and checks that a pending reset stays pending and counted.

## Minor

1. **The rest can lag behind the pass (new).** It uses the key's selection hint as "the pass start" (`maintenance.rs:266,276`). After a scheduler lag that hint is far in the past, so the key is due again every tick and walks forward one grant age-out per tick. It is early, never late, but it means repeated wasted visits. Fix: pass the pass's own start instant (a database clock read before its first count), as correctness Major 3 proposed, or take `greatest($5, t0)`.
2. **A failed hint update is never retried (new).** If a window hint update fails after the lease update has committed (`maintenance.rs:367-389`, `?` on the first failure), the remaining windows keep a hint that is late by the lease-recording lag. Their grants are not selected again on the next tick. Record this in D022-065, or retry by lowering hints for windows that have waiters and recently recorded leases.
3. **Stale documents.**
   - The port's doc (`application/src/maintenance.rs:160-161`) still says lease recording touches "no window"; the adapter now writes window hints after its commit.
   - ADR-0088 decision 4 says lease recording is "the one transaction that holds no run lock"; the purge holds none either. Decision 6 says the pass deletes unleased grants; the reducer does that now.
   - "Enforced by" still says "None exists yet".
   - The earlier minor 8 (ADR revision note) is therefore still open.
4. **The new exemption in the gate-row rule has no fixtures.** `GATE_ROW_MAINTENANCE` (`check-architecture.py:457-460`) has no test. It matches by statement prefix anywhere in `maintenance.rs`, so a second `UPDATE rate_limit_grants g SET leased_at …` there would pass. Add an allowed/refused fixture pair.
5. **Missing principles rows.** `docs/engineering/architecture-principles.md` has no row for `purge_as_of_door_errors`, for the gate-row rule and its maintenance exemption, or for rate-table consistency (T022-079).
6. **No budget line for a run-level rate-limited start.** The newly admitted path (granted, queued, and new window with the live count) has no line in `plan.md:584-596` or `work_budget.rs` (NFR-022-007, T022-076).
7. **The live-window predicate cannot use the index.** `g.leased_at + make_interval(…) > clock_timestamp()` (`readiness_gate.rs:626`, and the usage query in `quotas.rs`) cannot use `rate_limit_grants_count_idx`, so it probes every grant of every window in the namespace. Write it as `g.leased_at > clock_timestamp() - make_interval(secs => w.max_period_ms / 1000.0)`, and reuse it in Blocker 1's fix.
8. **Telemetry gaps.** The purge has no `outcome="failed"` and no oldest-purgeable-age gauge. The catalog in `docs/runbooks/execution-engine.md:127` still lacks `rate_lease`, `rate_purge`, `gate_grant` and `gate_timeout`. Architecture minor 2 is only half fixed.
9. **Usage still counts every key version.** The usage query counts every key version while the gate counts only versions in force, so nit 7 is not fixed. The `RateLimitLiveKeys` doc (`domain/src/quota.rs:124`) and the OpenAPI text still say "keys that have a window row" instead of live windows.
10. **Weak spots in `verify_start_rate`.**
    - Its invariant-10 query scans the tenant's journal by jsonb fields with no namespace filter and no index.
    - Its "purged" branch accepts a missing row as soon as the window has passed. That is up to a minute (or a longer `max_period_ms`) earlier than FR-022-056 allows the purge to delete it.
11. **Still open from the earlier review:** the down migration's refusal is untested (minor 6); the per-second clamp to 20 is silent (minor 9); the unordered lease update between two replicas during a takeover (minor 1) was accepted.

## Nit

- `test_check_architecture.py:620`: the mid-file `unittest.main()` predates this slice. Classes after it are skipped when the file is run as a script, but not under `-m unittest discover`, which is how CI runs it.
- The budget suite was red between 295b158b and a67ba790. The per-commit gate did not run on the branch commits; the CI selector covers it (`select-test-targets.py:65`).

## What holds

- **Layering:**
  - The domain additions are pure (quota resources, ceilings, `DEPLOYMENT_PER_SECOND_LIMIT`).
  - The application only gains two port methods.
  - The scheduler reaches the store through that port.
  - No `serde_json` business representation, no empty mappers.
- **One decision core:** `gates_of`/`decide_filed` serve both levels and both gates, which closes G022-012.
- **Lock order:** the start takes semaphores, then windows (ADR-0050 slot 7, then 8b).
- **No new deadlock from the lease pass:**
  - The lease `UPDATE … SET leased_at` fires only the BEFORE UPDATE guard. The count and prune triggers fire only on INSERT and DELETE.
  - The foreign key to windows is not rechecked on an update of `leased_at`. So the statement locks grant rows and no window.
  - The hint updates run after that commit, one window per autocommit statement, holding nothing else.
  - The purge locks the window, then deletes only leased rows. The lease pass touches only unleased rows, and a return deletes only rows whose attempt the journal shows as unleased. These sets are disjoint, so there is no cycle with admission, grants, returns or the purge.
  - The only possible cycle is two lease statements running at once during a takeover (accepted minor 1).
- **Migrations:**
  - `NOT VALID`, then validated under SHARE UPDATE EXCLUSIVE.
  - The down migration refuses while a rate limit is retained.
  - No new definer functions; the existing triggers keep `search_path = pg_catalog, pg_temp`.
  - The restated resource lists are guarded across merge order with 041 by `tests/contract/flow-control-step-policies/quotas.rs:63`. That holds only if someone runs it on the merged tree, since a merge into develop runs no CI.
- **Compatibility:**
  - `flow-control-rate-v1` is listed only when built, switched on and keyed.
  - Older processes do not list it, so activation is refused while one is live.
  - The test door and its feature flag are gone.
  - The enums only grow.

## Fitness summary

| Rule | Check |
|---|---|
| Domain free of adapters | holds (`check-architecture.py`) |
| A capability is listed only when served | holds (control-plane, engine registry and admission tests) |
| Upgrade oracle at the latest migration | holds |
| Hot path | holds for the listed lines; no line for a run-level rate start (Minor 6) |
| Grant rows written only by the gate and the two maintenance statements | holds; the exemption has no fixture (Minor 4) |
| No caller-chosen clock shortens a grant | holds (`purge_as_of_door_errors`); no principles row |
| One instant per decision | no check. Smallest: a checker rule that refuses `clock_timestamp()` inside the rate and concurrency count statements of `readiness_gate.rs` and `gate_queue.rs` |
| `rate_limit_live_keys` bounds live windows | broken (Blocker 1); smallest check is the test named there |
| Window rows are bounded | no check (Major 1) |
| Start lock order; lease recording never holds a grant and a window together | no check (T022-081) |
| Rate rows, counters and journals agree across runs | start only (`verify_start_rate`); the rest is T022-079 |
| Interim reset refusal | no check (Major 2) |
| Down migration refuses with retained limits | no check |
| Restated quota lists name every resource | holds (`quotas.rs:63`) |
