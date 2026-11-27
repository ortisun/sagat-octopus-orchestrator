# Targeted re-read 4 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 44d6a0b3cd3afe687d5144167c8d18ef73870e19
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 8a18ee1b, not a full review. The reviewer read files only and ran nothing.

Re-read 3's Nit is resolved (`authorize_abandon_deletes`). Checked with no finding: the handshake granted to
`sagat_retirer` (a pinned-path definer function returning versions only), the permanent capability's guard and check,
the gate's requirement set (a stale set yields a valid legacy root), the test-only capability (a disposable database
only).

No Blocker, no Major.

## Nit

1. The `EXECUTE` list gives the handshake to `sagat_retirer` and ends "no one else any of them", which read literally bars
   `sagat_app`, to which design 053 grants it; the `proacl` audit would then reject a correct grant. Fix: say the
   handshake's grants are design 053's and outside the list.

Counts: Blocker 0, Major 0, Nit 1

Fixed at `2245250b`: the handshake's grants are named as design 053's and outside the list.
