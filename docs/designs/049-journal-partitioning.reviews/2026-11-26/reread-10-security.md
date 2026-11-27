# Targeted re-read 10 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: d459361f034ae45d81a759ac2613a1eed2298777
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 1f07592a, not a full review. The reviewer read files only and ran nothing.

Checked with no finding: the `journal.rs` read rule, the new refusal names (no tenant information, only at the edge of
the window), step 3's clones enabled always (closing a bypass through `session_replication_role`), the test and wiring
tasks.

No Blocker, no Major.

## Nit

1. A function whose `PUBLIC` grant was never revoked has a null `proacl`, which a raw comparison reads as "owner only";
   the owner is not checked. Fix: compare `aclexplode(coalesce(proacl, acldefault('f', proowner)))` and require the owner
   role as `proowner`.
2. libfaketime in the test image is not pinned. Fix: a fixed source commit with a checksum.
3. (Suspicion.) The reference triggers are on the static check's allow-list, so nothing asserts that each generated
   lookup compares the tenant and the bucket and binds its values. Fix: the generator's test asserts both and `USING`.

Counts: Blocker 0, Major 0, Nit 3

All three were fixed at `339ca37e`.
