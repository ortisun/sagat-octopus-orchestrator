# Targeted re-read 5 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 9d192beea73a950359ca26cd3f95ba8eb467208a
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A fifth targeted re-read
of the fixes of re-read 4, not a full review. The reviewer read files only and ran nothing.

Re-read 4's four Nits and its suspicion are resolved.

No Blocker, no Major.

## Nit

1. Other refusals before authentication still reveal the cause by their `Retry-After` or reason (a failed storage ping
   at 5 s, a stale registry at 10 s, gRPC's `REGISTRY_SILENT`). Fix: every `UNAVAILABLE` before authentication is one
   constant answer; `REGISTRY_SILENT` after authentication; one test for every cause.
2. The change to ADR-0082's decision 3 is not recorded, and the runbook, contract and tests that state the old public
   reason are not listed.
3. The probe's direct connection contradicts `pg_hba`'s "through HAProxy only"; put `db-probe` and its one line in a
   drill-only file.

A suspicion: a blob transfer has no deadline, so its bound is not "its own timeout".

Counts: Blocker 0, Major 0, Nit 3

All three and the suspicion were fixed at `32d579ff`.
