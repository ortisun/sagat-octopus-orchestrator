# Targeted re-read 4 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 874192ae51140e0de3b8f98ed345717773d24188
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A fourth targeted re-read
of the fixes of re-read 3, not a full review. The reviewer read files only and ran nothing.

Re-read 3: Majors A and B and Nits a, b resolved (a real crash after a confirmation is still caught, since a later
reconnect takes a newer ticket); Nit c partly (Nit 3).

No Blocker, no Major.

## Nit

1. Success criterion 048-003 does not define "qualify"; the gap is the outage plus up to one heartbeat interval; the gap
   is per lease, not per worker; an expired lease's open gap needs an end.
2. Two tests of Task 048-10 need a barrier between the confirmation's ticket and its apply.
3. Pin the comparison: a ticket is the counter after its increment, and "before" is strictly less.
4. Setting the confirmed identity should happen only when the pass clears.
5. The Compose reference's lease time to live is built by Task 048-12, which does not cite Requirement 048-002.

Counts: Blocker 0, Major 0, Nit 5

All five were fixed at the next commit.
