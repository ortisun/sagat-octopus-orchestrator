# Correctness review of design 041's revision (approval run, first round)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: ff8033cd933d0c6adf972706eb1bb38407bdb7c6
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a fresh `correctness-reviewer`, read-only, given the diff of the design and of its companion documents since the approved commit f9d4a65f, on the owner's approval path "Full review, capped" (OD-2026-11-20-17). Saved verbatim.

I found no Blocker. Six findings are Major and nine are Nit. I checked the quota arithmetic and found it sound. I0 to I5 hold and S + G stays at or under L at every commit while the fence is held. The F₀·C(2k,k)/4^k bound is correct (I checked k = 1 and 2 and the asymptotics). The 51/48 budget equalities match `work_budget.rs:223-225` (50 and 47, plus 1). The 26-statement breakdown matches the prototype's own comment. The fence keys and modes the carry takes in CA are the ones the content and log finalizer re-takes (`20261024000000_durable_quotas.up.sql:313-330`), so the finalizer never waits on them.

### Major

**C-1 Major — SC-041-007 is "settled" but still reads as a gate that T041-9 will fail by construction.**
Where: `docs/designs/041-step-logs.md` 374-386 (SC-041-007), 401-408 (Goals), 3071-3072 (T041-9).
- **What is wrong:** SC-041-007 still states "at least 90%" and "+10% / max(10%, 2 ms)". It then records the accepted 1 × 16 cost (82.0%, +8.6 ms over the allowance, 87.1%). T041-9 says "a failure of SC-041-007 stops the release". At 1 × 16 the known, accepted figures are below the criterion as written. So T041-9 either stops the release on a cost the owner accepted, or the implementer judges against a tolerance the text does not state. The Goals still promise 90% and 10% without qualification.
- **Recommendation:** restate SC-041-007 per size as the criterion T041-9 checks.
  - At 8 × 128: the original gates.
  - At 1 × 16: a stated regression bound against run 44834024933 (for example, rate at least 82.0% minus *x* points, p99 at most the allowance + 8.6 ms + *y*), or "recorded, not gated".
  - Align the Goals and T041-9's done rule with it.
- Recommended disposition: fix — a release gate that cannot be met, or met only by an unstated tolerance, is not derivable from the text.

**C-2 Major — SC-041-001 asks for a re-run that the design now forbids.**
Where: `docs/designs/041-step-logs.md` 348-350.
- **What is wrong:** the bullet says a carry "refused at the tenant's step-log limit … [is] recorded by a downgrade re-run". The design says the opposite in four places:
  - ADR-0095 decision 5 (revised).
  - Section 2a, 1100-1112.
  - Section 2b, "Where a refusal lands" ("no report is re-run for a quota refusal").
  - T041-36's own test ("answered in the same call … no re-run").
- **Effect:** an implementer reading the success criterion builds or accepts the wrong behaviour, and the criterion as written has no test.
- **Recommendation:** rewrite the bullet. A limit or held refusal is answered in the same call with a mark-only final mark. Only an uncaught guard (fast path), or a failed call, leads to a re-run.
- Recommended disposition: fix — the success criterion contradicts the tested behaviour.

**C-3 Major — the ensure's `limit`/`held` answer pre-empts `plan_batch`, so stored counts and reasons can differ from the rule.**
Where: section 2b 1240-1247; ADR-0097 decision 3b; `specs/041-step-logs/data-model.md` 531-532 and §8a item 5.
- **What is wrong:** the ensure compares an upper bound with L − U. The bound is every sent record's size + 128 a row + 128 for a marker. On `limit` (or the transient `held`) the application sends `p_mark_reason = tenant_quota`. The function then keeps nothing, plans no marker and sets `ceiling_reason = tenant_quota`.
- **Concrete case:**
  - Setup: L − U = 2,000 B; the stream holds 9,999 of 10,000 records; the carry holds 10 × 200 B.
  - The ensure plans 10 × 328 + 128 = 3,408 > 2,000 and answers `limit`. All 10 records are counted `tenant_quota`, and the stream says `tenant_quota`.
  - `plan_batch` (T5/T6) would keep 1 record plus the marker (328 + 128 = 456 ≤ 2,000), with `ceiling_reason = attempt_records` and a marker row.
- **Why the tests miss it:** T041-36's property test drives the function with its own inputs and never passes through the ensure's mapping. It passes while the stored outcome diverges.
- **Recommendation:**
  - Either pass the ensure's answer to the function as an input that only forbids the fallback refill. The function then plans normally (its own allowance read) and records mark-only only when its reservation fails.
  - Or state the divergence as part of the rule and add "ensure answered limit/held" to the property test's generated inputs and expected outputs.
- Recommended disposition: fix — the design claims the function and `plan_batch` "plan alike", and this path breaks that, unseen by the test.

**C-4 Major — several quota and drop counters cannot be derived from the text and will disagree with the events they count.**
Where: `specs/041-step-logs/data-model.md` 327, 416, 419-427, 431, 531-541, 604; design 2a 1121-1124.
- **(a) Fallbacks are inferred from `room IS NULL`.** §7 counts a fallback as "a `stored` carry that returned no room". But `room` is "from the reserve", and a carry with a planned charge of 0 makes no reservation (line 604). The empty carry the SDK sends after every `FINAL_ONLY` (contracts §4) is such a carry. It would count as a fallback and make the store forget its kept room, so the next report pays an extra ensure.
  - The T041-9 criterion that reopens option 1c ("carry fallbacks above half of the carries' refills") reads this counter.
  - A carry that took the block and then found its existing stream held (`contended` with a cause) is excluded, so it is undercounted.
  - The reserve amount (sizes + 128 a row + 128 for a marker) is 128, not 0, for an empty carry, so "planned charge of 0" has two readings.
- **(b) The drop-reason sets do not match.** `step_log_lines_dropped_total{reason}` lists worker_gap, attempt_records, attempt_bytes, tenant_quota, tenant_quota_busy and out_of_order. The function's `dropped_by_server` returns `invalid`, `secret` and `content_fallback` too. Section 2a counts a StepLogRefused re-run's records as `failed`, which is in neither the `p_mark_reason` list nor the metric. The closed-enum test of §7 either fails, or these drops go uncounted.
- **(c) Refusals have two producers.** `step_log_quota_refusals_total{cause}` is counted by the store from each call, and SQL also "advances sequences … one per refusal cause", published by the shard-0 replica. The text does not say which one feeds the metric. Sequences are non-transactional, so the SQL side also counts refills and refusals whose transactions rolled back.
- **`mark_only_fallback`** is listed as an outcome but never defined.
- **Recommendation:**
  - Have the function return an explicit `path` (`fast`, `refill`, `none`) and a room flag of "not measured" (keep the previous entry) rather than overloading null.
  - Define the drop-reason set once (a domain enum), and use it for `p_mark_reason`, the `dropped_by_server` keys and the metric label. Add `failed`.
  - Name one producer per metric, and state that SQL-side sequences count attempts, rollbacks included.
- Recommended disposition: fix — the counters feed a stated decision criterion and an enum test.

**C-5 Major — no test covers the kept-room rules.**
Where: section 2b 1255-1263; ADR-0097 decision 3b; T041-36 2765-2779 and 2826-2840.
- **Rules with no test:**
  - The entry goes stale 9 minutes after the lease's last charge or grant, aged on the store's monotonic clock from the database's `idle_for`.
  - The store forgets the room for that backend after a batch or a marker.
  - An answer `room`, or a carry that charged nothing, does not refresh the entry.
- **What the tests cover:** only a fresh connection, a room taken by a reclaim, and an affinity miss.
- **Failing scenario:** the store keeps an entry past the 10-minute idle reclaim, or after a batch on the same backend drained the lease. Every such carry then falls back under the run lock, and past *b* it drops records as `tenant_quota_busy`. The report still lands with the same outcome, so no listed test fails.
- **Recommendation:**
  - Store unit tests with an injected clock: an entry with `idle_for` 8 min 59 s is used, and at 9 min 0 s an ensure runs.
  - A test that a batch, and a marker, on the cached backend causes the next carrying report's ensure.
  - A test that a `room` answer does not extend the entry.
- Recommended disposition: fix — these rules decide how often a report waits under the run lock, and nothing fails when they break.

**C-6 Major — the ensure can make a report late; the accepted ADR text still says it never fails a report, and its delay has no stated bound.**
Where: `docs/adr/ADR-0095-…md` line 84 (decision 4, "with one exception"; "each a bounded delay that never fails the report"); design 2a 1134-1137 ("Those are the only cases where capture changes a report's answer"); against FR-041-005 177-178 and G041-021.
- **What is wrong:** the ensure runs before the report's lease check. It is bounded per lock acquisition by *b*, but in total only by the statement timeout (5 s). The text gives no bound on the number of acquisitions; the fallback text itself says a wait spanning several holders can exceed *b*.
- **Failing scenario:** a report sent 1 s before its lease expires waits behind a chain of holders of the usage row in the ensure. It arrives after expiry and is refused in its first transaction.
- **Why it slips through:** FR-041-005 and G041-021 acknowledge the case. ADR-0095 decision 4 (a MUST) and section 2a deny it, and ADR-0097's supersession of decision 4 adds the delay but not the exception. T041-36 asserts "the report's outcome … are those of the case without an ensure" without a lease near expiry, so it cannot see this.
- **Recommendation:**
  - Add the ensure's lateness to decision 4's exceptions and to 2a 1134-1137.
  - Bound the ensure's transaction with its own `statement_timeout` (for example 2*b* + a margin), so the delay before the report is a stated number.
  - Add a test with a lease that expires during a held ensure.
- Recommended disposition: fix — a normative MUST in an accepted ADR is contradicted, and the delay bound is not derivable.

### Nit

**C-7 Nit — section 2's lock-order text contradicts the limit change.** Section 2, 908-909, says a change of `tenant_step_log_bytes` "takes its L3 fence exclusively and nothing else". ADR-0097 decision 6 and section 2b have it take the leases `NOWAIT` and then L4. The lock-order test's slots should model the limit change as L3 (exclusive) → L3b (NOWAIT, exempt) → L4. Correct the text.

**C-8 Nit — some cost figures still describe the old multi-statement carry.**
- Goals line 402 says "six for a long step".
- Open Questions line 448 says "round trips (five for a short step)".
- G041-022 says "a short step's carry adds 5, about 10%", while section 2a 1181-1182 says one round trip, about 2%.

Restate all three for the one-call function.

**C-9 Nit — T041-35's tick and its own rule disagree.** T041-35 is `[ ]`, but its evidence (2438-2441) says "this task is ticked on that row". Its own rule ("passes when all four pass", "T041-3 and T041-4 stay unticked until it passes") is never satisfied. Either tick it with that evidence and state that OD-2026-11-20-13 stands in for the pass, or leave it unticked and say so.

**C-10 Nit — the third measure for the built function is blessed from the implementation.** Its nested-statement count is "recorded by T041-36", so the equality takes whatever the implementation does when first written. Require the derivation in the test's comment, seen failing once, as the prototype's comment at `step_log_leases_tests.rs:776-783` shows.

**C-11 Nit — the ensure's `invalid` bound is far above "any valid carry's".** The bound used (`data-model.md` 266, prototype `step_log_spike.rs:386-388`) is 16 MiB + 128 × 100,001 ≈ 29.6 MB. A valid carry is at most 64 × (8,192 + 8,192 + 2,048 + 128) + 128 = 1,187,968 bytes. Since g = max(n, share), a wrong caller can park about 29.6 MB in one lease. Tighten the bound to the derivable carry maximum.

**C-12 Nit — the source-text check cannot pass on a correct build.** The check of T041-5 (`data-model.md` 367; design 2564-2565) reads "nothing but `sagat_quota_step_log_usage` reads `step_log_bytes` or `step_log_leased_bytes`". The charge, reserve, refill, ensure, release, reclaims and the down migration all read them, so the check fails on a correct build. Scope it to Rust sources and to SQL outside the §3b functions.

**C-13 Nit — the cost table omits writes made near the limit.** Section 2a's cost table (1145, 1147) leaves out the reclaimed leases' writes and the lease insert of a refill or ensure there. `data-model.md` 336 includes "plus one per reclaimed lease". Align the two.

**C-14 Nit — no task owns the validation of one CHECK.** `tenant_retained_usage_step_log_leased_nonnegative` is added `NOT VALID`, "validated by the next migration" (`data-model.md` 209). No task or file list owns that migration, unlike T041-4's `_validate`.

**C-15 Nit — the `lock_timeout` rule should say that 0 means none.** "A caller's lower `lock_timeout` kept" should state that 0 (disabled) counts as no timeout. A report transaction runs with 0. The prototype handles it (`step_log_spike.rs:325`); the specification text does not. Also, ADR-0097 decision 3 describes the reserve as returning a boolean, while `data-model.md` §3b gives `RETURNS TABLE (reserved, room, idle_for)`. Pick one.

Counts: Blocker 0, Major 6, Nit 9
