# Targeted architecture re-read of design 050 for approval, round 5 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 5a5ba25902f89c881088bd37e78642d50ba34563
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 4 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. No instance of the four round-4 findings remains.
Counts: Blocker 0, Major 0, Nit 2

1. **Nit. Technical Plan 6, "Cursor sealing"; Task 050-4.** Both designs' tasks create `tests/fixtures/sealed_envelope_malformed.json`, which will conflict; no 050 test is named as reading it; and the two length limits differ (512 bytes, 1 KiB). Fix: `a_cursor_with_a_wrong_field_is_malformed` reads the table; every entry at most 512 bytes and breaking the shared form; whichever task merges first creates the file.
2. **Nit. Technical Plan 6, "Cursor sealing" and "Metrics".** The key-read counter and gauge are missing from the Metrics paragraph the contract checks, and nothing checks the gauge. Fix: list both; the failed-read test also checks that the gauge grows.
