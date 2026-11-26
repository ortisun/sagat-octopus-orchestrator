# Correctness review of the 2026-11-19 revision of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 4c027adffaa889109a4b776c148d0f11263f9130
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Counts: Blocker 0, Major 6, Nit 9
Note: a fresh `correctness-reviewer` given the diff and the files, no conclusions; recorded condensed by the author. Confirmed: the scoped report path locks the run row, so the end rule holds for scoped carries; the budget sums matched the SQL shapes as written.

## Blocker

None.

## Major

- C-M1. CA's policy read uses a snapshot taken before the policy lock was granted (as A-B1).
- C-M2. A batch could overwrite `last_stored_xid` with a lower id, defeating the order guard (a stalled duplicate-only batch after a retry).
- C-M3. A carrying report waits for the stream row and the fences while holding the run row with no lock timeout; the 200 ms claim of the Risks row was wrong, and the purge and the delete route also hold the stream row.
- C-M4. Not every path that records a report, nor every re-run, is listed; scoped and failure re-runs map only content quota; a carry in `finish` would ride on fallbacks the design excluded.
- C-M5. T041-35's acceptance rule is not decidable: no `{}` baseline, no rule for fewer than three rounds, a redundant clause.
- C-M6. "Every later batch is answered FINAL_RECORDED" is left over; a batch begun after the report gets LEASE at T2.

## Nit

- C-N1. `in_order` false must exclude records at or below `last_sequence`. C-N2. Writing statements must not begin with `WITH`. C-N3. No hold after T4. C-N4. A held exclusive change and an ingest holding L1 cannot coexist. C-N5. Final-batch wording in ADR-0084 decision 7, D041-055 and ADR-0095 decision 1. C-N6. "One store function" against four methods. C-N7. The SDK's end window is ambiguous. C-N8. The seam must fire only for reports; (g) takes CB's update path. C-N9. The content fallback could keep a mark-only carry; the re-run's lease expiry should be stated.
