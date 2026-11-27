# Targeted correctness re-read of design 050 for approval, round 2 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 051e5f47
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a re-read of the text changed since round 1 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its five round-1 findings still closed and checked the moved ended-run guard against the adapter.
Counts: Blocker 0, Major 0, Nit 4

1. **Nit. Task 050-5.** The publication list has only the sealing twins, not the paging twins, though it filters differently. Fix: three twins.
2. **Nit. Technical Plan 6, Requirement 050-010, Task 050-10.** The client marks every `UNAVAILABLE` retryable and refetches on focus, so the inbox could keep polling a server without a ring; no test covers the stop. Fix: `retryable: false`, the reason read first, a test with a focus event, a test for a bad cursor.
3. **Nit. Task 050-4.** Its closed label enumeration includes `CliRefresh`, which design 013 confines to `IdentityKeyRing`'s module. Fix: only the two inbox labels.
4. **Nit. Technical Plan 6, Task 050-4.** The bound test does not say what it observes, and a metric would not see a bound applied after the predicate. Fix: 2,000 seeded ended-run rows ahead of a live row.
