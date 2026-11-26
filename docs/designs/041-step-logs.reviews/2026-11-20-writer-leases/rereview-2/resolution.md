# Resolution of the second re-reads of the writer leases (9361a112)

Four read-only re-reads of decision D041-120 (the bounded ensure, the ended-backend reclaim, the fourth run's two setup
steps), on a snapshot of `9361a112`, given the diff since `fc4adff7` and the design. They are saved verbatim beside this
file:

| Reviewer | Blocker | Major |
|---|---|---|
| `correctness-reviewer.md` | 1 | 1 |
| `security-reviewer.md` | 0 | 0 |
| `architecture-reviewer.md` | 0 | 8 |
| `egm-design.md` | 1 | 5 |

The decision that answers them is D041-121 (`specs/041-step-logs/research.md`). The documents were changed first
(commit `0ea180ba`); each code fix was then seen failing before it was made (`validation-report.md`, decision D041-121).

## Blockers

| Finding | Answer |
|---|---|
| Correctness Blocker 1, design Blocker 1, architecture Major G: the setup's fold of ended leases asks a pool whose every connection the fill holds for one more, so every row of the fourth run stops | The fold runs on the first connection the fill holds. A setup lease answered `busy` or `contended` is retried at most 10 times, declared in task T041-35's fourth run. The test `the_setup_lease_fills_a_full_pool_and_folds_ended_leases`, which is not ignored, runs the setup on a pool of two after an ended backend's lease: it failed on the old code ("pool timed out while waiting for an open connection") and passes now |

## Majors

| Finding | Answer |
|---|---|
| Correctness Major 1, design Major 5: the (2b) reading rule cannot name the refill (at most 37 of the 90 carries above p99 at the expected share) | Replaced before any round: each completion is tagged with its carry's path, and the harness prints `complete_p99_without_slow_paths_ms`, `complete_slow_paths_tagged` and `carry_fast_p99_ms`. At 8 × 128: (i) the refill when (2b) passes without the refill and mark-only completions; (ii) else the lines' p99 when the excess is at most the fast-path carry p99; (iii) else the environment question, with the sampled commit waits behind carrying reports. At 1 × 16, (2b) is read as (1) and (3) there. The p99 threshold of the slow-carry count is now the printed p99 |
| Architecture Major A, design Major 3: stale bounds and telemetry in ADR-0097 decisions 4 and 8 and elsewhere | ADR-0097 decision 4 says the ensure waits under the same budget; decision 8 is the data model's list and the design's alert rule (0.1% of carries for ten minutes). The overview, FR-041-005, the lock order, section 2a, the Risks row and ADR-0095's waits now say "each lock acquisition at most 200 ms, the statement timeout the hard bound" and qualify "rare" |
| Architecture Major B, design Major 1 (and correctness minor 1, security minor 2): "a report waits on the usage row at most once" is false after a `busy` ensure | Restated everywhere as at most twice, once under no run lock for the ensure and once under the run lock for the fallback; a `limit` or `held` answer skips the second. A T041-36 test asserts the one wait under the run lock after a `busy` ensure. Passing `busy` to the carry too would remove the second wait but accepts more `tenant_quota_busy` losses, so it goes to the owner with question O041-041 |
| Architecture Major C: no port for the ensure; the downgrade rule sits in the adapter | The ensure is a step of the store's method that begins a carrying report, returning a closed `CarryRoom` (`Room`, `Made`, `Limit`, `Held`, `Busy`, `Contended`, `Invalid`, `Error`); the application maps `Limit` and `Held` to `CarryPlan::MarkOnly`. ADR-0095 decision 3, FR-041-008 and T041-36's build item name it; a source-text check fails when anything else calls the ensure |
| Architecture Major D: the domain enum lacks `tenant_quota_busy` | `CeilingReason::TenantQuotaBusy` (no marker) and `CeilingReason::ALL`. The fitness test `the_contracts_ceiling_reasons_are_the_domains` (`contract_platform_foundation`) compares the planned contract's enum with the domain's set: it failed before the variant and passes after. T041-4 lists it; T041-5 extends the test to the migration's CHECK |
| Architecture Major E: the owner would choose the busy policy on the regime where it is near zero | O041-041 and O041-042 now state the regime the fourth run's rate covers, and a declared threshold puts O041-041 again with task T041-9's figures if busy answers exceed 0.1% of carries in any T041-9 regime. O041-042 names G041-027 next to G041-026 |
| Architecture Major F, design Major 4: "whatever the result" put ADR-0095 to the owner against decision D041-115; two consequences left task T041-5 open | ADR-0097, O041-040 and O041-041 go to the owner whatever the result; ADR-0095 (O041-038) once task T041-35 passes. Every failing gate now says whether task T041-5 waits |
| Architecture Major H: no production signal for fallbacks and lost session affinity | `step_log_quota_fallbacks_total{caller}` and `step_log_quota_ensure_affinity_misses_total`, counted by the store; a runbook entry for gap G041-027 (task T041-25) and a Risks row |
| Design Major 2: option 1c is rejected on a deadlock argument that does not hold | Restated in ADR-0097: a shared slot would fail fast (`NOWAIT`), not deadlock. What 1c gains (affinity behind a transaction pooler, surviving pool recycles) and costs (a key that is the client's word, a token on every charging call, no ended-backend reclaim, the deploy herd stays) are weighed. Backend keys are kept; 1c is the named alternative if task T041-9's pooler scenario passes the reopening threshold. ADR-0097 states that decision 3b has no effect behind a transaction pooler |

## Minors and Nits taken

- **The ensure's place:** after C1 to C3 (C0 runs in the transaction); a carry C0 refuses may have cost an ensure.
- **An ensure that raises anyway:** the answer `error`, rolled back, counted, the report proceeding as if none ran; a T041-36 test with a statement timeout.
- **The kept room's age:** from the lease's last charge or grant, which the ensure, the reserve and the carry return (`charged_at`).
- **The room a `SECURITY INVOKER` carry returns:** from the reserve, which now returns the room and the last charge; null after a fallback, so the next report runs the ensure.
- **The ensure's `limit` with enforcement off:** never answered.
- **Nothing to charge:** a planned charge of 0, no record and no marker; a marker-only carry takes the reservation and the block, with a T041-36 test.
- **Refusal causes through the ensure:** counted from the ensure's answer.
- **Grants and function lists:** `sagat_quota_step_log_idle_tenants` in data-model §4; T041-5's creation and test lists completed.
- **Metric owners:** named in data-model §7.
- **Cross-tenant ensure:** a T041-36 test.
- **The ensure's delay and the lease's expiry, and the statement-timeout re-run:** gap G041-021 restated.
- **The reader text of `tenant_quota_busy`:** "these lines were not checked against your quota".
- **The fourth run's text:** (h) runs at 1 × 16 only; no share is declared for (f), (g) and (d); G041-025 closes on (a) and (h) and no `mark_only:quota` or `mark_only:held`; the share index runs across the row's replicas; "about 0.18 of its lease".
- **The harness:** the lease rows counted for the scenario's tenant only.
- **The ensure test's `again` assertion:** checks the answer `room`, so that hand mutation 14 fails it again.
- **Stale comments:** in `step_log_spike.rs` and `step_log_leases_tests.rs`.
- **Complexity Tracking and Constitution Check:** the store's kept room and the ensure's alternatives; gaps to G041-027.

## Not taken now

- **The `invalid` bound of the ensure** (correctness minor 8, security minor 3, architecture minor 2, design minor 7): the carry's largest planned charge needs C3's sizes with normalisation and escaping; task T041-36's planned-charge property test derives it.
- **A `held` test of the ensure in the prototype** (correctness minor 6): a hand mutation that drops the ensure's `program_limit_exceeded` handler survives the prototype's tests today. The built ensure's test list (task T041-36) now has the case; the prototype is not the build.
- **The ensure's subtransaction** (correctness minor 7): documented in ADR-0097's consequences, counted by task T041-9's criterion.
- **T041-9's criteria** (correctness minor 10, architecture minor 12, design minor 10): their consequences and the `pg_stat_slru` restatement go with task T041-9, which is measured after acceptance.
- **The limit-change herd** (design minor 9) and **option 1g's weighing** (design minor 5): task T041-9's near-limit scenario.
- **The reclaim's budget and the `idle_tenants` order and cost** (security nit 3 and its suspicion, architecture minor 6): task T041-5 builds the purge pass.
- **The OpenAPI description of `ceiling`** (architecture minor 14): task T041-3 copies contracts.md §2.
- **The diagnosis harness's p99 index** (`spike-diag-json`, not a gate).
