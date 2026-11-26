# Targeted re-read of design 047 for approval: security (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 4bc5767bfdf90d50d4890dba8b7b572f15e0e42e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; nothing of substance left out. The fourth and last targeted
security re-read after review round 2, on the approval rule of Owner decision 2026-11-25-2. Its history in this folder:
`reread-1-security.md` (Major 4, Nit 3), `reread-2-security.md` (Major 1, Nit 1), `reread-3-security.md` (Nit 1), every
finding fixed before the next pass. The reviewer read the design and the diff of the last fix, and ran nothing.

## The previous finding

Answered. The holder and the sweeper or adopter race on one atomic check-and-set write to the write-once reference, and
only one can win: a holder resuming after its row was closed or abandoned gets a refused write, and a holder that wins
makes the claim fail, so the row is adopted or revoked. A late write can no longer leave an untracked token.

## New findings

1. Nit (a failure, not an attack). The tombstone is "the same kind of write as the holder's own", so a later probe cannot
   tell it from a token: an adopter that writes the tombstone and crashes before marking the row abandoned lets the next
   acquire activate the tombstone and retire the live revision; the next refresh sends the tombstone, gets
   `invalid_grant` and the grant is lost (nothing leaks). Fix: a marker the probe recognises (custom metadata or a fixed
   payload), the probe treating a tombstoned reference as claimed; a test crashing between the tombstone and the mark.
   A constraint, not a finding: destroying must keep the key's KV metadata, or the reference becomes writable again.

Counts: Blocker 0, Major 0, Nit 1

## The author's answer

Fixed where the build reads it, without changing the design's text (Decision 047-064): the data model's new section on
vault references gives the tombstone a fixed payload and custom metadata that a probe reads as claimed, has the next
probe finish an interrupted claim, keeps the key's metadata on destruction, and names two tests.
