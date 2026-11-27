# Targeted re-read of design 054 for approval: architecture (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: b83781f7c0dfea6d880837aa2c439d4a384a0e5a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The second targeted re-read
(Owner decision 2026-11-25-2, step 4), not a full review. The reviewer read files only and ran nothing.

The history: the full review `../architecture.md` (at `0473c4dc`: Blocker 0, Major 7, Nit 4), each Major fixed; re-read 1
`../reread-1/architecture.md` (at `3798bec9`: Major 3, Nit 4), each fixed; this re-read, at `b83781f7`.

Re-read 1 Majors 1 to 3 and Nits 4, 6 and 7: resolved. Nit 5 reopened (finding 1). No Blocker or Major; charging the tenant
aggregate through the deferred quota flush writes no row of the kinds Architecture decision 0050 decision 3 lists.

Counts: Blocker 0, Major 0, Nit 4

## Nit

1. Open Questions says "Gaps 054-001 to 054-011", and Gap 054-012 exists.
2. Architecture decision 0111 decision 4 said "holds more than 2 MiB" where the design says "pending bytes and the payload
   together would pass 2 MiB", and its Enforced by omitted the refusal test and the two source rules. *Answer: fixed in the
   record* (records are not part of the approved body).
3. Slice 2 no longer depends on slice 1 when slice 2a is skipped.
4. Slice 4's folders omit `scripts/` and `tests/integration/buffered-signals/`, which Task 054-9 writes.

Findings 1, 3 and 4 are settled in writing and carried into the build by follow-up Task 054-19 (Owner decisions 2026-11-26-544 to
2026-11-26-546); finding 2 is fixed in Architecture decision 0111.
