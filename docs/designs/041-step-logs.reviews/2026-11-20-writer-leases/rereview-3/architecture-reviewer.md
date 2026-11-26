# Architecture re-read of decision D041-121 (design 041, writer leases, second re-read)

Reviewer: `architecture-reviewer` subagent, read-only, on the snapshot of branch head `21e4bd74`. Saved verbatim.

All paths are relative to the snapshot root `$SNAP` = `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/rr2/snapshot`, and every line number refers to that snapshot. I had no shell, so I could not run `git branch -a` or `git worktree list`, and I did not check scope against other branches again.

**Verdict.** No Blocker and no Major. All eight earlier Majors (A to H) are closed. C, G and H each leave a Minor behind (N1 to N4 and N6). The change brings in no new Blocker or Major. Eleven new Minors follow (N1 to N11). None blocks approval. N6 should be fixed before the fourth run's first round, because a missed setup defect costs a two-hour CI run.

## Status of Majors A to H

| Major | Status | Evidence |
|---|---|---|
| **A.** ADR-0097 decisions 4 and 8 contradicted 3b | **Closed** | Decision 4 now reads "The ensure … waits under the same budget, under no run lock" (ADR-0097:155-157). Decision 8 (:192-204) matches data-model §7 (:422-430): it has `kind="ended"`, `step_log_quota_ensures_total` with `error`, `step_log_quota_fallbacks_total`, `step_log_quota_ensure_affinity_misses_total`, `caller="marker"`, and the alert at 0.1% of `step_log_carries_total` over ten minutes. The Consequences (:288-293) say "each of its lock acquisitions waits at most the same budget". Only two Nits remain. |
| **B.** "At most once" was false | **Closed**, by option (b) | "At most twice, once under no run lock and once under it" now appears in ADR-0097:130-132 and :303-304, ADR-0095:103-105, design :913-915 and :1229-1235, and D041-121 (2) (research.md:132). D041-120 carries the "corrected by D041-121" note. Option (a), passing `busy` to the carry, is put to the owner under O041-041 (research.md:215). That is right, because it accepts more loss. The timing case is in T041-36: after an ensure answered `busy`, `pg_stat_activity` shows the fallback's single wait. One residual: the ADR wording leaves out the content-class wait at commit (N5). |
| **C.** No port named for the ensure | **Closed** | `CarryRoom` (`Room`, `Made`, `Limit`, `Held`, `Busy`, `Contended`, `Invalid`, `Error`) is returned by the store's method that begins a carrying report. The application maps `Limit` and `Held` to `CarryPlan::MarkOnly` with `tenant_quota`, and the adapter holds no rule (design :1222-1228, ADR-0097:126-128). ADR-0095 decision 3 (:71-77) and FR-041-008 (design :199-204) name the method. The T041-36 build item is rewritten (design :2706-2722), with a source-text check that nothing else calls `sagat_quota_step_log_ensure`. Residuals: where the planned charge comes from (N2), and the "Enforced by" lists (N7). |
| **D.** No `TenantQuotaBusy` in the domain | **Closed** | The variant is at `crates/domain/src/step_logs.rs:111-151`, with `marker()` returning `None` and `ALL` driving `parse`. The fitness test `tests/contract/platform-foundation/step_log_ceiling.rs:11-36` was seen failing before the variant (validation-report.md). T041-4 lists it, says it lies outside `plan_batch`'s comparison, and has T041-5 extend the test to the CHECK. Residual: CI does not run the test when only the domain changes (N10). |
| **E.** O041-041 would rest on the wrong regime | **Closed** | O041-041 (research.md:215) names the regime the fourth run covers and declares the 0.1% reopening threshold. O041-042 (:216) adds G041-027. The design repeats this at :2366-2369. Residual: the threshold's wording, and its echo in option 1c (N9). |
| **F.** Reading rules contradicted D041-115 | **Closed** | ADR-0095 and O041-038 are now asked only after a pass (design :2370-2372, O041-042). Every gate at each size says whether T041-5 waits (:2345-2360). The old (2b) share rule is replaced. Its 37 / 90 = 41% argument checks out: 28 refills plus 9 mark-only answers, against 90 carries above the p99 of 9,000. Residual: case (i) is circular (N8). |
| **G.** The setup could not run | **Closed** | The ended-lease fold now runs on `held.first_mut()` (`step_log_spike.rs:1805-1811`). `busy` and `contended` are retried at most 10 times (`:1814-1827`), as declared at design :2316-2318. The share index runs across replicas (`prewarm_from`, `replica_index × API_POOL`, `API_POOL` = 20 at main.rs:73). The test `the_setup_lease_fills_a_full_pool_and_folds_ended_leases` (`step_log_leases_tests.rs:1832`) failed on `9361a112` with the predicted message and passes now. Residual: no workflow runs that test (N6). |
| **H.** No production signal for the affinity regime | **Mostly closed** | Both counters exist: design :1242-1248, data-model :424-427, ADR-0097 decision 8. The Risks table has a new pooler row (design :1868). T041-25 adds the G041-027 runbook entry and the `scaling.md` change (:3048-3051). The ADR says 3b "has no effect" behind a transaction pooler (:137), and option 1c is restated (:227-236). Residual: the fallback counter's source is undefined and over-counts, and the affinity counter undercounts (N1, N3). |

## Blocker

None.

## Major

None.

## Minor (new)

- **N1. The fallback counter has no defined source and would over-count** (data-model :424-425: "from the carry's `written:refill` or mark-only path").
  - `written:refill` is an outcome of the spike only. The built `sagat_step_log_carry` returns `stored`, `mark_only` or `contended`, with a cause (data-model :534-553).
  - "Mark-only path" also takes in carries the application planned as `MarkOnly` and that never refilled: secret, content fallback, `invalid`, and the ensure's `limit` or `held`. Those would inflate the very signal Major H asked for.
  - No T041-36 test covers this counter. The affinity counter has one.
  - Fix: count a fallback only when the carry took the block, meaning a `mark_only` with a non-null `cause`, or a `stored` with `room IS NULL` (data-model :747-748). Better, return an explicit `path` column (`fast`, `fallback`, `none`). Add one T041-36 assertion per path.
- **N2. The planned charge is computed in two places with no stated equality.**
  - The application passes `planned` to the port (design :1222-1224), and data-model :589 calls it "the same planned charge the store passed to the ensure".
  - data-model :294-296 has the function compute it in SQL "before CC" from `sagat_quota_measure` sizes.
  - If the Rust figure is higher than the SQL one, a false `limit` becomes `MarkOnly`, and the records are dropped while the tenant had room. If it is lower, an ensure answering `room` is followed by a failed reservation and a fallback under the run lock.
  - Fix: pass the store's `planned` into the function as its reservation bytes, after checking that it is at least the function's own measure. Extend T041-36's planned-charge property test to compare the two.
- **N3. Two "every call returns" claims have no mechanism, and one counter undercounts.**
  - ADR-0097:133 and design :1236 say a batch's T8 and the script marker return the pid, the room and the last charge. Their charge runs in a statement trigger, which returns nothing to the caller.
  - `step_log_quota_ensure_affinity_misses_total` counts a miss only when an ensure ran. Behind a transaction pooler in steady state, most reports skip the ensure, so the miss never shows.
  - Fix: either drop batch and marker from the list (unknown room then means an ensure runs), or give them the reserve's return. Also count a miss when the carry's `writer` differs from the pid the connection remembers.
- **N4. The clock used for aging is not stated.**
  - The store ages its kept room from a database `charged_at` (data-model :275, ADR-0097:133-135). It is not said whether that is compared with the replica's clock.
  - A skew of more than 1 minute uses up the 9-minute-versus-10-minute margin, and the store would then trust room that a reclaim may already have taken.
  - Fix: return the lease's age (`now() − charged_at`), or the seconds left until stale, and have the store age it with a monotonic clock.
- **N5. "At most twice, once under the run lock" is not qualified in either ADR.**
  - ADR-0097:132 and :303-304, and ADR-0095:105, state it as a general rule.
  - A report that stores output also takes the usage row at its commit, under the run lock, for the content class (G041-024). The design does qualify the claim ("before its commit", :913).
  - Fix: add "for the step-log class, before its commit" in both ADRs.
- **N6. The setup's fitness test runs in no workflow.**
  - The lease tests compile only with `perf-spike` (`step_log_spike.rs:739-741`).
  - `perf-spike.yml:59-64` builds and then goes straight to the rounds, and `carry_spike_rounds.sh` does not run the tests.
  - D041-121 (1)'s "not ignored" therefore still means "run by hand".
  - Fix: add one step before the "rounds" step, `cargo test -p sagat-tests --test perf_scalability_ha --release --features perf-spike --locked leases_tests`. That stops a setup defect in minutes instead of after two hours.
- **N7. The "Enforced by" lists do not follow the change.**
  - ADR-0095:13-21 says each check has "its row in `docs/engineering/architecture-principles.md`". There is no ADR-0095 row: the only step-log row is :31, which belongs to ADR-0097. ADR-0095's list also lacks the ensure source-text check and a `CarryRoom` mapping test.
  - ADR-0097:34-43 still lacks the `pg_locks` test, the 200 ms migration test, the ensure source-text check and the ceiling contract test. This is old Minor 9, partly open.
  - Fix: add the ADR-0095 row, and add the four tests to ADR-0097's list.
- **N8. Gate (2b) case (i) is circular** (design :2351-2357).
  - Case (i) names "the built ensure" as the lever and then says T041-5 "waits for the design change it names". The ensure is already in the design.
  - In case (ii) it is not said whether "the excess" is measured on the full p99 or on `complete_p99_without_slow_paths_ms`.
  - Fix: for (i), say that T041-5 may start and that T041-9's cold-start and pooler rows judge the ensure, unless option 1e is chosen. For (ii), name which p99.
- **N9. The reopening threshold is ambiguous and is echoed inconsistently.**
  - In O041-041 (research.md:215), "busy answers exceed 0.1% of carries" could mean the ensure's `busy` (not a loss) or the carry's `tenant_quota_busy`.
  - ADR-0097:235-236 reopens option 1c on "fallbacks above the reopening threshold of O041-041", which is a different quantity.
  - T041-9's criteria (design :3025-3038) do not carry the threshold. The near-limit and pooler rows just say "recorded".
  - Fix: write "carries answered `mark_only:busy`" in all three places, and put the threshold into each T041-9 criterion.
- **N10. CI does not run the ceiling-reason fitness test when only the domain changes.**
  - `scripts/select-test-targets.py:378-386` sends a `crates/domain` change to unit tests only, and the test lives in the suite `contract_platform_foundation`.
  - A change to openapi.yaml does select the suite (literal match, :278).
  - So a pull request that adds a domain variant and leaves the contract alone passes until the release.
  - Fix: add a `HARNESSES` entry (`crates/domain/src/step_logs\.rs` → `contract_platform_foundation`), or move the test into the domain crate's tests with a YAML parser as a dev-dependency.
- **N11. "Must equal" is declared but not checked** (design :2332-2333).
  - Nothing asserts that `complete_slow_paths_tagged` equals the `written:refill` plus `mark_only` carries (main.rs:2268-2270 only prints both).
  - Tagging is by step id (`step_log_spike.rs:1590-1594`), so a retried step or a duplicated report would skew it without any sign.
  - Fix: have the judge treat a mismatch as a void round. Declare that before the first round.

## Minor (carried over, still open or partly open)

- **Minor 2 (open).** The `invalid` ceiling is still an attempt's maximum, about 29.6 MB (data-model :266-267), not a carry's bound (ADR-0097:121).
- **Minors 3 and 10 (text closed, test still missing).** No T041-36 case covers a stale kept room or a reused pid.
- **Minor 5 (open).** A sequence "one per refusal cause" (data-model :326) sits beside store-counted refusals.
- **Minor 6 (open).** The comment at data-model :224 still says the reclaim scans "one tenant's rows by the key's prefix". The cost of `idle_tenants` is not stated.
- **Minor 7 (partly open).** Stale text remains in three places:
  - `docs/adr/README.md:39`: "bounded at 200 ms", and no idle reclaim or new supersede entries.
  - The budget table (design :1129): "older than 9 minutes".
  - Complexity Tracking (:1923): "six metrics".
- **Minor 8 (open).** The lock-order test's writers (design :901-910) still leave out the ensure, the idle and ended reclaims, the limit-change trigger and `sagat_quota_lock_log_usage`'s holders.
- **Minor 11 (partly open).** Two things are still missing:
  - No task changes `infra/helm/templates/_helpers.tpl:86`.
  - T041-9's Files (design :3044) name no PgBouncer component with a pinned image, loopback-only ports and a healthcheck (AGENTS rule 16).
- **Minor 12 (open).** The `pg_subtrans` criterion "1,000 a second" (:3034-3035) is still underived. The cold-start row (:3018-3024) still has no consequence for a breach.
- **Minor 14 (partly open).** openapi.yaml:7636 still lacks the `ceiling` description. :7639 still says "batch" only. There is no drift check against contracts.md §2, and no rule says whether readers must accept unknown values.

**Closed since the last read:** Minors 1, 4, 9 (the T041-5 lists and mutations 16 to 21), 13 and 15.

## Nit

- ADR-0097:25 says the ensure has "its own budget"; decision 4 says "the same budget".
- ADR-0097 decision 8 has `step_log_purge_failures_total` without `{cause}`; the design (:1355) has `{cause="underflow"}`.
- ADR-0097's "Decided by" (:4-6) stops at D041-119.
- data-model :438-442 lists T041-6 and T041-13 twice each among the metric owners.
- The "only the ensure … lock `tenant_retained_usage`" wording is unchanged (ADR-0097:38). The ensure takes the row through the refill.
- The setup's 11 retries of `contended` have no pause between them (`step_log_spike.rs:1814-1827`). That is harmless during setup.
- The per-replica `application_name` behind a transaction pooler is still the pooler's (carried over).
- The earlier Nits on the p99 index, the share index and `lease_rows` are closed. I did not re-check `IMMUTABLE` in `without_the_refill_bound`.

## Consistency across documents

ADR-0097, ADR-0095, the design, data-model §3b and §7, research.md (D041-120's note, D041-121, G041-021, G041-025, O041-041 and O041-042), contracts.md and the code now agree on four points:
- the bound of the ensure;
- "at most twice";
- the `error` answer;
- staleness counted from the lease's last charge.

They also agree on the telemetry list, the acceptance path (ADR-0095 only after a pass) and G041-025's closing condition. The remaining disagreements are the ones listed under N5, N9 and Minor 7.

## Areas with no finding

- **Layers.** `CarryRoom` is a closed answer type of the store port. The mapping to `CarryPlan` is in the application, and the domain only gains a variant. No I/O, framework or `serde_json` enters the domain. The contract test reads YAML into `serde_json::Value` in the test crate only, which is allowed.
- **Transactional boundaries.** The ensure is still a transaction of its own, writing only quota state, before the report. The downgrade re-runs carry `MarkOnly` or `None`, so they run no ensure.
- **Compatibility.** `tenant_quota_busy` is added to a planned, unreleased enum.
- **Arithmetic.**
  - 29,520,000 / (160 × 1,048,576) = 0.176.
  - 37 / 90 = 41%.
  - *i* = 20*r* + *j* matches `API_POOL` = 20.
  - The setup test's shares for *i* = 3 and 4 match `setup_share`.

## Files

- `$SNAP/docs/adr/ADR-0097-the-step-log-quota-is-charged-to-exact-per-tenant-leases-never-to-the-usage-row-at-commit.md`
- `$SNAP/docs/adr/ADR-0095-the-last-step-log-records-of-an-attempt-ride-on-its-report.md`
- `$SNAP/docs/adr/README.md`
- `$SNAP/docs/designs/041-step-logs.md`
- `$SNAP/docs/engineering/architecture-principles.md`
- `$SNAP/specs/041-step-logs/data-model.md`
- `$SNAP/specs/041-step-logs/research.md`
- `$SNAP/specs/041-step-logs/contracts.md`
- `$SNAP/specs/041-step-logs/validation-report.md`
- `$SNAP/specs/001-platform-foundation/contracts/openapi.yaml`
- `$SNAP/crates/domain/src/step_logs.rs`
- `$SNAP/tests/contract/platform-foundation/step_log_ceiling.rs`
- `$SNAP/tests/performance/scalability-ha/step_log_spike.rs`
- `$SNAP/tests/performance/scalability-ha/step_log_leases_tests.rs`
- `$SNAP/tests/performance/scalability-ha/main.rs`
- `$SNAP/scripts/select-test-targets.py`
- `$SNAP/.github/workflows/perf-spike.yml`
