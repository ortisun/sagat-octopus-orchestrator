# Targeted correctness re-read of design 050 for approval, round 1 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 5d451eb9
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a re-read of the text changed since the reviewer's full review of 2026-11-25 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer confirmed that deadlines are not null, that iteration events name their owner node, and the policy resolution and audit-read collapse as stated.
Counts: Blocker 0, Major 2, Nit 3

1. **Major. Technical Plan 6, "Paging and access filters" and "Items"; Architecture decision 0144 decision 5.** A page scans up to 2,000 rows but returns at most `limit` (200) items with the cursor at the last row scanned, so decidable rows 201 to 300 of the first 2,000 are skipped on every refresh. Fix: when the page fills, the cursor is its last item's key; give the publication list its `limit`; a test with a publication twin.
2. **Major. Technical Plan 6 "Per-run list", Requirement 050-009, Architecture decision 0144 decision 2.** `may_decide` has no status clause and the per-run list returns every status, so settled items and pending items of an ended run show `youMayDecide: true`. Fix: `status == PENDING && !run_ended && may_decide`; two tests.
3. **Nit. Task 050-4.** `an_ended_run_is_left_out_of_the_inbox` passes without the predicate unless the inconsistent row is seeded. Fix: seed it.
4. **Nit. Non-functional requirement 050-003, Architecture decision 0144 decision 2.** Moving the guard changes one answer (an ineligible caller on an ended run: `403` with a committed denial audit instead of `503`). Fix: name it as the one deliberate change.
5. **Nit. Technical Plan 5, "The source".** The failed iteration's row stays `ADMITTED` then `DRAINED`, so the lifecycle rules applied first would hide it. Fix: the `drain_iteration` rule takes precedence; assert the counts mid-drain.
