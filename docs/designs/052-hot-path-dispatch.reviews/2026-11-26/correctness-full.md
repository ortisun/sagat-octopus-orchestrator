# Correctness review of design 052, full round of 2026-11-26

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d734e0266bc4333e3e2cf8944a37034d296ab278
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, and did not repeat the reports beside this one.

## Major

1. A poll ended by the bound reports no work while ready work remains (CUJ 1: late polls meet their first 8 candidates
   leased or locked and end with 56 untried), a unary poll then waits 400 to 600 ms, and a stream parks on a counter
   nothing moves for a static backlog. Fix: a bound-ended poll with untried candidates does not report empty: fetch
   again once from the spread start, or answer the minimum retry and claim again without parking; a test of a static
   backlog behind 8 locked runs leasing within 100 ms.
2. The scenario of the gate and of Success criterion 052-004 cannot measure the bound: its workers re-poll at once and
   retire after three empty polls, so with the bound the figure counts surviving workers. Fix: end on completions; sleep
   `receive_delay(retry_after_ms)` as the SDK does; name the configuration.
3. Success criterion 052-002 (under 50 ms at the 99th percentile) is unreachable: a stream window ending by its 5 s
   deadline tells the SDK to wait `idle_retry`, so no stream is registered about 9.5% of the time. Fix: such a terminal
   frame asks for the minimum reopen delay (`retry_after_ms` 1, clamped to 50 ms by the SDK); measure over many windows.
4. "Park only if no counter moved" lets every stream that just ended an empty claim claim again, uncapped and
   uncoalesced: 31 of 32 streams spinning at about 10,000 empty claims per second. Fix: per-key permits (a commit adds
   one per step made ready; a stream takes one or parks; one whose permitted claim leased nothing takes none until
   `idle_retry`); a test.
5. The session's offset added to design 038's turn makes polls of one process start at the same point again (one in
   four for two sessions with `wanted` 16), breaking FR-038-004 inside a process. Fix: hash the process's identity, not
   the session's; a second round of K pollers after a contended first asserting K probes.
6. The candidate scan's eligible compensation branch and forward pool-refused branch lack the path filter for protocol
   minors below 5, so a minor-4 session receives scoped compensations `claim_run` drops without a transaction; with the
   bound, nine of them starve its forward work for ever; `probed` counts them. Fix: the filter in both branches; do not
   count a candidate declined without a transaction; a test.
7. Success criterion 052-001 can pass while round trips per lease do not fall (probes become polls, uncounted). Fix:
   statements per lease, and leases per second not lower.

## Nit

1. Which ending counts as "ended by the bound" (the window's last candidate, `consumer_gone`, the byte budget, a full
   session learned late).
2. `a_poll_ends_after_eight_probes_that_leased_nothing` passes vacuously with `wanted` up to 2.
3. The spread's stride uses each poll's shrinking `wanted`; use a fixed stride.
4. The A/A pilot does not reproduce the gate's null (two builds): run A/A′ with a no-op commit.

Counts: Blocker 0, Major 7, Nit 4
