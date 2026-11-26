**Re-read of design 041 at 4d6fe4cc: zoom-out findings Z-1 to Z-6 and I-1 to I-8**

The reviewed tree's HEAD (branch `feature/041-step-logs-server`) is commit `4d6fe4cc`. I had no shell, so I could not check whether the tree had uncommitted changes. I read every file and section the brief listed, plus the zoom-out standard. Line numbers below refer to `docs/designs/041-step-logs.md` unless another file is named.

One Major is still open: the T041-36 text contradicts itself on what a guard's raise does (Z-3). No new Blocker or Major came from the fixes.

## Stage 5, the first zoom-out report (`zoom_out_report.md`)

**Z-1 (the ensure before the report): resolved.** One Nit is left.
- What is fixed:
  - The ensure is gone, and with it the kept-room cache, `CarryRoom`, the dependency on session affinity, the affinity counter and the second wait. Section 2 (lines 1005-1009) says "No transaction runs before a report".
  - The top-up after the commit (lines 1379-1413) needs no state in the store. It is the owner's choice (OD-2026-11-20-18).
- Nit left: the line 2070 rejection of "a bounded fallback with no top-up" still gives the reason Z-1 said the measurements contradict: "every steady refill would wait under the run lock". The design itself, at lines 1403-1407, says that regime "passed every gate at 8 × 128".
  - Fix: state the trade as the owner's choice, made against a measured, acceptable baseline.
  - T041-37 records top-ups but no criterion says whether the top-up pays for itself. A cheap one: fallbacks per carry with the top-up against the fourth run's 0.31%.

**Z-2 (the exact-lease machinery): resolved.** I no longer hold it. Line 2070 now covers everything asked:
- it names ADR-0097 option 4 with its proved overshoot;
- it states the trade (a tolerance in quota views against the machinery and its three gaps);
- it says the leases leave the content class's ceiling unchanged (G041-024).

The owner accepted the leases (OD-2026-11-20-15). It does not count as open.

**Z-3 (the `MarkOnly` re-run tier): partly resolved.**
- Resolved: the stale text in SC-041-001 (lines 371-374), the table row at line 1101, lines 1141-1142, 1211 and 1219-1221, the risk row at line 2011 and the Verification Plan row at line 3257. The `tenant_quota` against `failed` contradiction is gone.
- Still open, and the same harm Z-3 named (it misleads whoever implements T041-36). T041-36 contradicts section 2a, ADR-0095 decision 5 (ADR lines 106-111), D041-128 and its own Builds list (lines 2763-2766):
  - **Line 2734:** the function refuses any `p_mark_reason` other than `invalid`, `secret` and `content_fallback`, which leaves out `failed`.
  - **Line 2867:** it repeats this as "outside the three". Section 2a (lines 1122-1123) and D041-128 include `failed`. As written, the re-run after a guard's raise would be refused by the function itself.
  - **Lines 2855-2858:** the test has a guard's raise re-run the report "with no carry … no final mark", as "the third and last transaction". Section 2a and ADR-0095 instead re-run it with a mark-only carry (`failed`) that keeps the final mark, with at most four transactions.
- Keeping the tier itself: the answer is adequate. ADR-0095 decision 5 explicitly requires the guard tier, and the delegation does not cover changing an accepted ADR. The tier still exists only for defects, so the YAGNI point stands. But removing it is the owner's call, and leaving it costs little.
- Disposition:
  - **Fix** lines 2734, 2856-2858 and 2867 so they match section 2a.
  - **Accept** keeping the tier, with one owner row citing OD-2026-11-20-17. "Reported to the coordinating session" is not an acceptance.
- I count Z-3 as open Major until the T041-36 text is fixed.

**Z-4 (the write rules in two languages): resolved.** Line 2074 records the duplication, its guard (D041-100) and the rejected single write path, with a principle II reason. Accept.

**Z-5 (SC-041-007, the Goals, the Open Questions and T041-9): resolved.**
- SC-041-007 is per size (lines 397-431).
- The Goals (lines 446-452) and the risk row at line 2006 follow it.
- The Open Question (lines 482-493) is now scoped to what is left to T041-9.
- T041-9's stop rule cites the per-size floors (lines 3182-3185).
- "six for a long step" and "five for a short step" are gone.

**Z-6 (spike protocols and changelogs): partly resolved.**
- Resolved:
  - T041-31 is withdrawn (lines 2203-2206).
  - T041-35 is cut to its outcome (lines 2275-2307).
  - The box stays unticked only because of rule 9, which lines 2305-2307 explain, and the owner row (OD-2026-11-20-13) says the same.
- The revision notes kept in the Overview (lines 32-84): the answer is adequate, given that D-12 was accepted and the amendment rule relies on those notes. Accept.
- Left, all Nit-level weeds: Disposition: accept, or move them as an optional fix. Not counted as Major.
  - The bound proof is still in section 2b (lines 1430-1444).
  - The prototype's statement-by-statement 26 is still in section 2a (lines 1273-1278), though the text says the built figure is derived again.
  - The answered owner questions (lines 503-614) repeat `docs/owner-decisions.md`, and the resolution does not address them.

## Stage 8, the intermediate zoom-out report (`intermediate_zoom_out_report.md`)

**I-1 (the Verification Plan and ADR-0097): resolved.** There are rows for exactness, the held-room bound, the deadlocks, the reclaims, the top-up, the wait slots and the busy loss (lines 3259-3264 and 3284). The gap list runs from G041-001 to G041-027 with each status (lines 3287-3306). Principle VIII at line 2059 now matches. Row 3267 says "stream lock at T3".

**I-2 (T041-9 lands last): resolved for the quota mechanism.** One Nit is left.
- T041-37 (lines 2900-2972) lands right after T041-36 and T041-8, before Slice C. Every regime has a criterion and a consequence fixed in advance (lines 2956-2965).
- Nit left: the SC-041-007 judgment of the built carry is still in T041-9, after the SDKs and Studio (line 3163).
  - The 1 × 16 floor is the prototype's worst valid round, and the built function does more work (it builds its own keys and checks the mark reason). A regression there could stop the release late.
  - Fix: run workloads (a) and (f) once in T041-37's rounds too.

**I-3 (the ensure on nearly every carrying report): resolved.** OD-2026-11-20-18 removes the kept room, the "forgets" sentence and the marker clause. T041-37 records top-ups.

**I-4 (the order guard): resolved.** The recommendation's minimum is met: line 2075 names the SDK rule as the rejected alternative, with a reason. Accept.

**I-5 (the busy loss in the Requirements): resolved.**
- FR-041-006 permits both drops (lines 203-206).
- FR-041-011 adds `notStoredReason` (line 259).
- FR-041-017 adds the busy state (line 305).
- The busy loss is no longer reported through `ceiling` (lines 1143-1145).

**I-6 (the scope of the 0.1% rule): partly resolved.**
- Resolved: lines 2952-2955 exclude the induced regimes by name, and every regime records the busy rate.
- Left: the owner's row OD-2026-11-20-14 in `docs/owner-decisions.md` still says "any regime that task T041-9 measures" and uses the old name `mark_only:busy`. The narrower reading is only "reported to the owner" (line 598, D041-131).
- Nit: record the owner's confirmation of that reading before T041-37's first round.

**I-7 (the pooler remedy and the scaling runbook): resolved.** Session pooling is not required (D041-130; lines 1403-1410, 2015, 3196-3199). `docs/runbooks/scaling.md` stands, and option 1c has a declared threshold (lines 2940-2942).

**I-8 (the owner-gate lines): resolved.**

| Task or place | Line | Gains |
| --- | --- | --- |
| Header | 6 | OD-2026-11-20-1 |
| T041-3 | 2342 | OD-2026-11-20-15 and -19 |
| T041-36 | 2884 | OD-2026-11-20-1, -14, -15, -16 and -18 |
| T041-37 | 2969 | a gate line (the task is new) |
| T041-9 | 3187 | OD-2026-11-20-13 and -14 |
| T041-25 | 3207 | OD-2026-11-20-13 |

## New findings from the fixes

No new Blocker or Major. The two mechanisms the fixes added are lighter than what they replace or address:
- the top-up, which is the owner's choice and simpler than the ensure;
- the wait slots (D041-125), which give a compact bound on one tenant's share of a replica's pool, with no Rust state.

New Nits:
1. **The wait slots name no simpler alternative.** They appear only inside the leases row at line 2070, so that row names no rejected alternative for them. Fix: add one row, for example "a per-tenant Rust semaphore held across every carry: it cannot bound a wait that starts inside the function and would take permits from fast-path carries".
2. **Three counts disagree in the new text.** Fix them together:
   - The privilege audit in T041-5 (lines 2496-2499) and the row at line 3283 say "six" runtime-executable quota-lease functions. Line 1553, ADR-0097 decision 1 (ADR lines 97-98) and `specs/041-step-logs/data-model.md` (lines 395-396) also grant `sagat_quota_step_log_top_up_mark` and `sagat_quota_step_log_wait_slots`.
   - T041-5's migration list (lines 2430-2438) does not create those two constants.
   - T041-36's plan property test (lines 2755-2757) leaves out "each answer of the block", which line 1149 includes.

Counts: Blocker 0, Major 1, Nit 7