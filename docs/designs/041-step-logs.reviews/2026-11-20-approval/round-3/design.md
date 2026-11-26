**Final short re-read of design 041 at c52b5d37: stages 5 to 8 (design, zoom-out, comprehension, structure and style)**

Six of the eight open items are resolved. Z-3's re-run tier and D-12 are kept, and I recommend "accept" for both. The second-round fixes add three new Majors and no Blocker.

I read `docs/designs/041-step-logs.md` from line 1 to "## Detailed Implementation Plan" (line 2229), plus T041-9, T041-37 and the parts of T041-36 and T041-5 they cite. I also read the four first-round reports, `resolution.md` "Second round", D041-134 and D041-135 in `specs/041-step-logs/research.md`, the per-round table in `specs/041-step-logs/validation-report.md`, and ADR-0095 decision 5. Every file could be read. The task named no separate standard, so the first-round reports were the review context.

## The items that were still open

- **Z-3: the contradicting text is fixed. For the tier kept, I recommend "accept".**
  - Line 1114, lines 1231–1235 and T041-36 (lines 2941–2943, "a guard's raise … re-run once with a mark-only carry whose reason is `failed` … the stream keeps its final mark") now match ADR-0095 decision 5 (ADR lines 106–110).
  - Line 1136 and T041-36 line 2962 ("outside the four") give the same four mark-only reasons. SC-041-001 (lines 373–376) and the Verification Plan row (line 3382) say quota and busy refusals are answered in the same call.
  - The `StepLogRefused` tier now serves only invariant-violating guard raises. It is a small cost, and removing it would change an accepted ADR.
- **K-7: resolved.** Lines 1167–1169 say the carry's own quota lease being held or short "never makes it `contended`". Lines 1220–1221 limit `contended` to CA and CC′. Lines 1022–1026 agree.
- **D-4 (remainder): resolved.** The section 5 "Contract" paragraph (lines 1740–1744) now states D041-133. Section 8, the risk rows (lines 2063 and 2086) and principle V (line 2097) agree.
- **D-12: still held. I recommend "accept".** The four revision notes are still at lines 32–84, as the amendment rule requires. The stale history sentences it named are gone.
- **N-1: resolved.** SC-041-007 (lines 430–433) and T041-9 (lines 3309–3310) state that the (2b) floor admits about +94% of p99. T041-9 reports to the owner any p99 above +36.9%. The eight rounds' gate values are in `specs/041-step-logs/validation-report.md` (lines 762–780). One residual is Nit 3 below.
- **N-2: resolved.** Gate (4) is judged at the cap and tickets of 10 that the fourth run measured (line 410, T041-9 lines 3289–3291). T041-37's "Ticket defaults" regime (lines 3056–3057) measures the shipped 7 at both sizes. Its consequence is set in advance (line 3077).
- **N-3: resolved in substance.** The replica slot *Q* bounds all tenants together (lines 1360–1375). T041-36 tests it, and T041-37 adds a regime with three or more tenants (lines 3052–3055). Two new problems with how this was written are Majors 2 and 3 below.

## New Majors

**Major 1: at 8 × 128 the new judging rule fails the build the owner accepted, about half the time.**
- *Where:* `docs/designs/041-step-logs.md` lines 413–416 and 437; T041-9 lines 3285–3286 and 3305–3307; D041-135.
- *Problem:* D041-135 is titled "How SC-041-007 is judged at 1 × 16". But SC-041-007 and T041-9 apply "the median of all its valid rounds" to every gate at both sizes, and 8 × 128 still uses "(2b) at most 0".
- *Evidence:* the second round's own table (`specs/041-step-logs/validation-report.md` line 775) gives the eight valid 8 × 128 (2b) rounds as −312.64, −25.40, −17.03, −2.19, +6.30, +11.50, +24.68 and +112.86. Their median is **+2.06 ms, a fail**. The fourth run passed only on its first three rounds (−17.0 ms, line 437).
- *Consequence:* the 5.49% and 2.38% false-stop figures cover only the 1 × 16 floors. An unchanged build would stop the release at 8 × 128 by a coin flip.
- *Recommendation:* fix the 8 × 128 statistic before any round. Either keep the fourth run's median of the first three valid rounds and state its false-stop rate, or derive a tolerance for 8 × 128 as was done for 1 × 16 and add an owner row, since the allowance is the owner's (OD-2026-11-20-1).

**Major 2: the lock order and the Overview still describe the wait slots and the top-up as they were before D041-134.**
- *Where:* L4, lines 987–991; Overview, lines 27–28.
- *Problem:* L4 says the top-up after a report is "first tried `NOWAIT`, then waited for at most *b* … only while holding one of the tenant's wait slots". Section 2b (lines 1374–1375 and 1419–1425), the Terms (lines 726–728) and D041-134 say two things instead:
  - the top-up never waits and takes no slot;
  - a refill waits only while holding both a tenant slot and a replica slot.
- The Overview also bounds the fallback "by the tenant's wait slots" only.
- *Why it matters:* the lock-order test is modelled on this list (line 1010). The bound of "Logs cap plus *Q*" holds only if the top-up holds no slot. The same kind of contradiction was a Major as K-7.
- *Recommendation:* restate L4 and the Overview sentence to match D041-134.

**Major 3: how *Q* reaches SQL is not stated, and it is unaddressed behind a transaction pooler, a deployment the design supports.**
- *Where:* section 2b, lines 1366–1368 and 1377–1378; T041-5, lines 2495–2498; T041-37's `pgbouncer.ini` (lines 3009–3017).
- *Problem:* *Q* comes from the session setting `sagat.step_log_wait_slots`, which "the pool sets … when it connects". But D041-130 and G041-027 say a transaction pooler is supported, and T041-37 judges a PgBouncer regime. The two obvious ways to set the value both fail there:
  - **As a startup `options` parameter:** PgBouncer refuses it unless `ignore_startup_parameters` lists it. If it is listed, PgBouncer discards it and *Q* reads 1.
  - **As a `SET` after connect:** the value stays on whichever server backend ran it. Backends opened later read *Q* = 1 when unset, so the slot count follows the backend, not the replica.
- *Effect:* the bound only gets tighter, never looser. But busy answers behind the pooler rise in a way no criterion explains, or the pooler regime cannot connect at all.
- *Recommendation:* name the mechanism in T041-5 and state how it behaves behind PgBouncer in section 2b. Then add a check to T041-37's pooler row that `sagat_quota_step_log_wait_slots()` returns *Q* in every transaction. An alternative is a server-side value keyed by `application_name`, which PgBouncer does pass through.

## Nits

1. **The false-stop rate is per gate.** SC-041-007 (line 429) gives 5.49% per gate. T041-9 (line 3308) says an unchanged build "fails one gate" with that probability, which reads as the release-level rate. Three floors are judged, so the chance of failing any one is higher (up to about 16% and 7%). Also, the Σ form reduces to C(*m*, *r*) / C(8 + *m*, *r*), and for even *m* it is an upper bound.
2. **No rule for too few valid rounds.** "At least six" valid rounds (line 415, line 3286) has no stated consequence when fewer are valid.
3. **The owner report has no stated consequence.** T041-9 does not say whether the release waits for the owner's answer when p99 exceeds +36.9%. It also does not say whether the 1 × 16 figures in the runbook and release notes are then restated.
4. **The revision note omits the second round.** The Overview's newest revision note (lines 32–41) lists D041-124 to D041-133 only, while the amendment rule asks re-reviewers to scope by these notes. D041-134, D041-135 and the second round are not mentioned.
5. **The connection claim is wider than the bound.** "At least N − ⌊N/2⌋ − *Q* are never held by step-log work" (lines 1371–1373, line 2055) also covers top-ups and refills that take the row with `NOWAIT`, neither of which takes a slot. The bound is on waits, so "never held by step-log waits" is what it proves.
6. **Workload (b) is sized to the wrong ticket count.** In T041-9 (line 3296) it is sized from "the tenant's seven tickets", while the same task sets the semaphore to 10.

Counts: Blocker 0, Major 5, Nit 6