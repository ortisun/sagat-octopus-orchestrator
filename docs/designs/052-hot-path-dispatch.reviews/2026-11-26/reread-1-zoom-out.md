# Targeted re-read 1 of design 052: egm-zoom-out (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 472b27825d37703aa9f212bc013aae549a2cfef8
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of
`zoom_out_report.md` (at 2348509a), not a full review.

Full round: Majors 2, 5 and 7 resolved; Majors 1, 3, 4 and 6 settled by the design's written reasoning (the queue and
the next-step lease deferred, Open Questions 2 and 3); Nits 1 to 3, 5 and 6 resolved, Nit 4 settled.

## Major

1. The Redis wake transport is the largest single addition and nothing measures it (NATS is the default; the success
   criteria measure one transport). Fix: Redis deployments wake within the process only in 0.3.0, stated beside the
   no-transport case; the Redis port a follow-up if measured. (One of the two options architecture's Major 2 offered.)

## Nit

1. The 10 ms coalescing is redundant with permits.
2. Nine wake counters could be one with a closed outcome label.
3. The gate's ten-pair fallback is written out in the requirement; the pilot could record the count, the rule and the
   threshold.

Counts: Blocker 0, Major 1, Nit 3

The Major and Nits 1 and 2 were fixed at the next commit (Redis wakes within the process; no coalescing; two counters
with closed outcomes). Nit 3 is settled by keeping the written formulas, which the comprehension and correctness
re-reads checked.
