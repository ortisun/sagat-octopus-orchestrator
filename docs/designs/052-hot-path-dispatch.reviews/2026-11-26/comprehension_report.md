# egm-comprehension on design 052, full round of 2026-11-26

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d734e0266bc4333e3e2cf8944a37034d296ab278
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing. Run on the text narrowed after `zoom_out_report.md` and `critic_report.md` (at 2348509a).

## Major

1. The spread gives few distinct starts: the window is `min(4 × wanted, 64)`, so `(h × wanted) mod len` has at most 4
   offsets for `wanted` up to 16, sessions with `h` 0 or 4 never spread, none at `wanted` 64. Fix: offset `⌊h × len / 8⌋`.
2. "A poll alone keeps the scan's order" contradicts a trigger on the previous poll's contention.
3. The pool-refused sample is inside the same scan result, ordered and rotated with the window, unmarked; only the
   ineligible sample is appended behind.
4. Which bytes of the session are hashed, and where the flag lives (a unary poll may reach another instance), unsaid.
5. "Park" is undefined and used before its definition; a received wake must increment the local counter, or a message
   between the empty claim and the park is lost.
6. Requirement 052-012's cap cannot be computed from a message that carries no count.
7. "Discovery admission" undefined; today a refusal ends the stream with `rate_limited(1 s)`; whether the change applies
   to woken streams only is unsaid.
8. The `task.ready` hint is written in `execution.rs`, `execution/path.rs`, `chain_progress.rs`, `loop_progress.rs` and
   `primitives.rs`; the publish sites are incomplete.
9. The gate's scenario configuration (of nine), its number of chains and its time budget are unsaid.
10. The statistics: the MDE formula is a paired t-test's, but the rule is 6 of 6 signs plus the median, so the 80% power
    does not carry; power cannot come from an A/A pilot; "15% of the completion's time" is not 15% of throughput; no
    fallback if 1.425 × sd exceeds the mutation's effect. The arithmetic holds.
11. The open-loop runs depend on design 051's unbuilt harness; "maximum sustained rate" is defined only in Requirement
    051-006; Success criterion 052-001 omits `wanted`.
12. CUJ 2 and Success criterion 052-002 misdescribe the cross-instance path: only the run's start crosses.
13. Design 038's references: FR-038-004, its guards and the selector are in `docs/designs/038-claim-transition-efficiency.md`
    and `scripts/select-test-targets.py`, neither linked.

## Nit

1. Undefined project terms (activation condition (d), pool-refused, breaker, binding keys, the reducer, features 002,
   009, 010, 019, "both products", "Delegate.").
2. With `wanted` up to 2 the window is at most 8, so the bound never fires.
3. Design 038's budget "would have caught" the fourfold regression; its own Complexity Tracking says it missed it.
4. "Each remedy is switchable" does not hold for the gate; the default during measurement is unsaid.
5. Open Question 4 is the owner's decision, not a delegate's; OD-2026-11-24-515 cites a "T052-12" that no longer exists.
6. The findings section points at a directory without answers.
7. ADR drift (modulo the window, "one row per run" against "per run and activity", ADR-0029's credit).
8. "Run" for a workflow run and a benchmark run.
9. "Did not itself lease" filters only the publish, not the local counter.
10. Broker wording (existing credentials against a permission change; "the platform's subjects"; no encoder named).
11. How the twelve runs pair, and whether "base" is the merge base.
12. How many repetitions define "the range" of Success criterion 052-004.
13. The counters' labels; the wake's counters unnamed.
14. A poll that leased some tasks before the bound; whether errors count towards the 8.

Counts: Blocker 0, Major 13, Nit 14
