# Targeted re-read 3 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: bd3c8fe8399b4d2fbe82c5e8c53e07c15a14bbbd
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A third targeted re-read
of the fixes of re-read 2, not a full review. The reviewer read files only and ran nothing.

Re-read 2: Nits 1, 2, 3, 5, 6, 7 resolved; Majors 1 and 2 partly (Majors A and B); Nit 4 partly (Nit c).

## Major

A. The superseded set still locks out the confirmation: a stale null report with a later ticket supersedes the fresh
   identity, the registry's connection is discarded and refused on reconnect, and the confirmation that would remove it
   can never run. Fix: a second ticket when a confirmation is applied; discard a null report older than it; a barrier
   test that forces the reconnect.
B. Success criterion 048-003's 15 s mixes an outage with a heartbeat gap (the gap a lease must survive is the time to
   live less 5 s), and on Compose no repetition would qualify, while "the drill says so" fails nothing. Fix: time to
   live less 5 s, the drill's time to live stated, a missing qualifying repetition fails the drill.

## Nit

a. The budget omits the worker request's 3 s (17 s, leaving 43 s).
b. No test of a reason set by an unknown outcome after a confirmation's ticket.
c. "Set before its ticket" compares a time with a counter; setting a reason must record the counter.

Counts: Blocker 0, Major 2, Nit 3

All five were fixed at `3ec72276`, and the Compose reference's lease time to live of 90 s at the next commit.
