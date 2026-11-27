# Targeted re-read 1 of design 052: correctness (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 9b3ec8989fb54f8e033fb3991a81844c0d9eecf9
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`correctness-full.md`, not a full review. The reviewer read files only and ran nothing.

Full round: Majors 2 to 7 and Nits 1, 2 and 4 resolved; Major 1 and Nit 3 partly (below).

## Major

1. (Plausible) The second fetch can start where the first did (same turn, same `r`), probing the same locked runs again.
   Fix: start past what the first walk tried and exclude the runs probed; a test with offset 0 and the flag already set.
2. Surplus permits (a process's own echoed message, a fan-out leased by one stream, a permit added while a claim leases
   that step) make the next claim empty and bar the stream for `idle_retry`, so a real step waits up to 500 ms. Fix:
   drop a process's own messages; debit leased tasks from the permits; cap them at the registered streams; a test.
3. (Plausible) The spread's flag dies with the in-flight entry, so non-overlapping polls collide again across replicas;
   `r mod 8` collides one time in eight for two instances. Fix: a flag per namespace with a short decay; `hash mod len`;
   fixed identifiers in the test; a one-poller case; the offsets in the report.
4. (Plausible) "Once" is unenforceable for unary polls, which loop at 20 per second under contention. Fix: remember the
   last 50 ms answer per session and answer `idle_retry` within it; a test.
5. (Plausible) A wake is lost while registering if registration happens at the park. Fix: registered from before the
   claim until the park ends; the barrier after the scan; rename the registry test.

## Nit

1. One message adds one permit whatever the remote fan-out; state it.
2. The ten-pair fallback does not restate its threshold or run order; the factors are a mean's applied to a median.
3. The 10 ms coalescing has no meaning under permits.

Counts: Blocker 0, Major 5, Nit 3

All eight were fixed at the next commit.
