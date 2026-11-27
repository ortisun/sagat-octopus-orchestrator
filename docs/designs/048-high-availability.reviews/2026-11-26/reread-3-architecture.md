# Targeted re-read 3 of design 048: architecture (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: e21f8403a1e92226765b440f5ad63c889e64e057
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A third targeted re-read
of the fixes of re-read 2, not a full review. The reviewer read files only and ran nothing.

Re-read 2: Majors 1 to 3 and Nits 2, 3 resolved; Nits 1 and 4 partly.

No Blocker, no Major.

## Nit

1. "Four" profiles, three named; 048-020 says "a fourth".
2. Three statements still refuse the observer read literally (the identity admission, "only the registry's connection
   connects", ADR-0130 decision 5).
3. The lint refuses `current_setting`, which the identity function needs; limit it to the forms that set a value.
4. The dropped-`WriteTx` test must write a row and check another backend and the row's absence.
5. On a fresh database, say `command_await_durability` still checks `region-v1` first.
6. The ADR's header and Task 048-1 name only `instance_confirm_durability()` for the moved guard.
7. Task 048-9's list of causes sits after the wrong test.

Counts: Blocker 0, Major 0, Nit 7

All seven were fixed at `853ac19a`.
