# Targeted re-read 1 of design 052: security (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: f111acc7508b8388d62cf87f1fc81f0abe452409
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`security-full.md`, not a full review. The reviewer read files only and ran nothing.

Full round: Major 1 partly (below); Major 2 resolved but for one counter (Nit 1); Nits 1, 2, 3 and 5 resolved, Nit 4
partly (Nit 5 below).

## Major

1. The channel is closed to the platform's user but not to the broker's other users (Redis's `default` user, a NATS
   `no_auth_user` or open account), through which a tenant's connector could subscribe and read every tenant's
   identifiers, activity tokens and commit timing, or aim forged wakes; the chart cannot see whether a Redis URL
   authenticates. Fix: no other user or anonymous connection on the channels; a start-time check that keeps the wake
   local-only otherwise; a test.

## Nit

1. No counter of a woken claim that leased nothing.
2. The rate limiter's state must not grow with forged keys: look the entry up first, the bucket inside it.
3. "No JetStream API" contradicts the one NATS credential the outbox needs; the wake on its own credentials secret.
4. The control plane gains egress to the broker; open it only with the wake, and add the broker to gap 052-G1's list.
5. The gate's scenario comes from the head tree; drive both builds with the merge base's.
6. A woken claim must validate the session as today, so a session revoked while parked leases nothing.

Counts: Blocker 0, Major 1, Nit 6

All seven were fixed at the next commit.
