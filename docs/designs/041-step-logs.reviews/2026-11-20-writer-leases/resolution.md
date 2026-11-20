# Resolution of the reviews of the writer leases (ba47aa81, 5458820b)

Four read-only reviews of D041-117 and D041-118 (the step-log lease owned by its writer, a refill that waits with a bound
from `max_connections`), saved verbatim beside this file:

| Reviewer | Blocker | Major |
|---|---|---|
| `correctness-reviewer.md` | 0 | 2 |
| `security-reviewer.md` | 0 | 2 |
| `architecture-reviewer.md` | 1 | 9 |
| `egm-design.md` | 1 | 7 |

The decision that answers them is D041-119 (`specs/041-step-logs/research.md`). The coordinating session approved it
before it was built. Its parts are:

- the store makes the lease's room before the report;
- a fallback refill waits at most 200 ms;
- past that, the carry gets a mark-only answer that the reader can see;
- the setup lease of the fourth run, with its two conditions.

## Blockers

| Finding | Answer |
|---|---|
| Architecture Blocker 1: a report failed past the bound and resent by the worker can change a step's outcome (ADR-0084 decision 7, ADR-0095 decision 4) | The report never fails for a step-log reason. Past the 200 ms budget, the carry's block answers in the same call with a mark-only final mark, `tenant_quota_busy`, and the report commits. The busy answer is counted, alerted and shown to the reader. It is rare, because the store makes the lease's room before the report (ADR-0097 decisions 3b and 4; ADR-0095 decision 5; test `a_refill_past_its_budget_answers_busy_and_the_report_lands`) |
| Design Blocker 1: the first refill of every new backend queues on the usage row at each row's start, and the bound does not cover it | In the built design, the ensure makes that first charge's room before the report, under no run lock (decision 3b). In the spike, the declared setup step leases each filled connection. It stands in for the ensure for each connection's first charge only, so later refills stay in the transaction and the spike is conservative. The herd of a deploy is T041-9's cold-start scenario, with its criterion fixed in advance (design 041, T041-35's fourth run and T041-9) |

## Majors

| Finding | Answer |
|---|---|
| Correctness Major 1, security Major 1, architecture Majors 1 and 2, design Major 2: *b* is not a derived worst case (wrong hold, one refill assumed, per-acquisition `lock_timeout`, 1,103 wrong, startup check missing) | *b* is now a latency budget of 200 ms, the ingest's budget on the same row. It is no longer derived from `max_connections`, and there is no startup check. Past it, the result is a visible mark-only answer, not a failed report. The design states that a wait spanning holders can exceed *b*, and that the statement timeout stays the outer guard |
| Correctness Major 2, architecture Major 6, design Major 5: the held-room bound is misstated | It is now stated from F₀ as F₀ · C(2*k*, *k*) / 4^*k* (> F₀ / √(π(*k* + ½))) for consecutive fresh grants, with the general case and the three-line derivation, in ADR-0097 decision 4 and design 2b. Test `room_held_by_open_writers_is_bounded_from_the_free_room` covers *k* = 1, 2, 8 and 32 from a quarter-used limit. Mutation 10 grants F / (*A* + 1) |
| Security Major 2, correctness nit 12, an architecture minor: the session setting can switch the bound off | No session setting is read. The budget comes from `sagat_quota_step_log_refill_wait()`, which a test database may replace. The test checks that `sagat.step_log_refill_wait = '1h'` and `lock_timeout = 0` do not lengthen the wait |
| Architecture Major 3, design Major 4: the error mapping contradicts itself | The busy answer is a carry outcome caught in SQL, not an error, so no mapping is needed. Data-model §8a now has one rule. A batch's `55P03` stays `RETRY_LATER` |
| Architecture Major 4, correctness minor 6, security minor 7, design Major 6: the timeout counter mixes in ingests | `step_log_quota_refill_budget_exceeded_total{caller="carry"\|"ingest"}` is counted by the store. The alert fires on carries only. The sequence was removed |
| Architecture Major 5, correctness minor 7, design minor 1: the grant differs between the ADR and the code | The ADR, design 2b and the data model now describe what was built: fold whole, max(*n*, …) over the leases holding a grant, and the order fence, usage row, own lease `NOWAIT` |
| Architecture Major 7, design minor 2: the lock-order fitness function | The refill takes its own lease `NOWAIT` after the usage row, so nothing waits after L4. The design's lock-order test text and the fitness row in architecture-principles.md are rewritten |
| Architecture Major 8, correctness minor 8, design Major 1: waiting under the run lock against the ADR's force | The force is restated (one bounded, counted wait of the rare fallback). A Risks row names who queues behind the run row. T041-9 measures a fan-out run during fallback waits |
| Architecture Major 9, design Major 6: the fourth run cannot fail on its failure mode, and no reading rule is declared | A busy answer counts as not written, so it voids a round. The declaration states how a result is read: (i) G041-025 is closed by the validity counts alone; (ii) a (2b) fail is attributed by the share of `written:refill` carries; (iii) a 1 × 16 fail is the carry's known cost; (iv) a full pass puts the ADRs to the owner. The build measured and the setup addition are named |
| Design Major 3: the reasons for rejecting 1c and 1e do not hold, and the top-up option is missing | 1c is restated: the backend key is unique by construction, which the deadlock argument needs, and its costs are named. 1e is re-argued. The ensure before the report (the top-up on the same backend) is adopted as decision 3b |
| Design Major 7: "one in 300" holds only for high-volume tenants, and there are two subtransactions | The condition is stated (F ≥ 2(*A* + 1) MiB, at least 1 MiB per backend per 10 minutes). The refill has no block, so only the fallback carry's block remains. T041-9 adds the low-volume and small-quota scenarios |

## Minors and Nits taken

- **Idle reclaim and the limit change:** the idle reclaim tries the fence shared first and its interval is fixed (test `a_limit_change_waits_for_an_open_idle_reclaim_instead_of_failing`).
- **Test filtering:** `a_backend_waits_on_a_fence` now filters by database.
- **Lease-table index:** no index besides the key, so charges are HOT updates.
- **The count *A*:** only leases holding a grant are counted.
- **Reservation size:** §8a and §3b now state it the same way, ceiling marker included.
- **Usage readers:** `quotas.rs` readers move to the accessor (T041-5), with a source-text check for direct reads of S and G. The `SELECT` grant of `sagat_app` on the usage table is stated.
- **Privilege lists:** the new functions are in the grants and the privilege audit.
- **Replica attribution:** a per-replica `application_name` (T041-5).
- **Stale text:** corrected in the README, ADR-0027, ADR-0084, ADR-0095 and the design.
- **Acceptance condition:** now points at the fourth run.
- **Down migration:** stated as one-way once lines exist.
- **Telemetry:** D041-117's per-tenant wording is corrected (no tenant label).

## Not taken

- **A server-side re-run of the full carry after an ensure:** the ensure before the report makes the fallback rare in the first place, and the busy answer is visible.
- **A cap on concurrent refill waiters:** the 200 ms budget bounds each wait, and the ensures run under no run lock.
- **A startup check of `max_prepared_transactions`:** the refill's own lease is taken `NOWAIT`, so a prepared transaction can only send that refill to the busy answer.
