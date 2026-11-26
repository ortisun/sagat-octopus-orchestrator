# Targeted re-read of design 044 for approval: correctness, first pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: f4f55b14cfdf0214644dc13ebd81002b12f94d72
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since round 11 was reviewed. Round 11's Majors 1 to 4 are closed (1 with two caveats, 2 with leftovers). Every finding
below was fixed in the next commit.

## Major

1. A held deferral blob counts against the run's 16, so a changed branch after a deferral loops into review.
2. The semaphore's 4 MiB guarantee cannot hold once four tenants hold their quarter.
3. "Not counted" after `UNAVAILABLE` has no durable record for an expiry in another process.
4. Gated inputs are inline but the state's size is known only at the end of the activation.
5. The gated set from the registration answer misses a republished envelope with the same digest.
6. A moved child's new `secret` annotation is checked only at the parent's publication.
7. `NO_TARGET_WORKER` counted from `created_at` "whatever the claims" contradicts the other sentence.
8. A 300 s replay-check lease with no extension cannot replay the longest runs.
9. An upload charged to the run's bytes reopens the 4 MiB edge through `WriteBlob`.

## Nit

10. `steps.code_step` is not in the migration list; a `verify_run` check.
11. The claim budget omits the release of the previous lease's uploads.
12. Section 10.4 still refuses update results and terminal outputs over the bound.
13. The `InlineValuesExceeded` row omits the 16-blob throw and gated inputs.
14. `run.cancel_requested` should carry its `CancelOrigin`.
15. The per-run count of 16 needs a lock.
16. The binding re-read must take `FOR SHARE`.
17. A refused per-run upload has no named cause.
18. Several values crossing the bound together.
19. Signals refused at the same 3 MiB where continue-as-new is suggested.

Counts: Blocker 0, Major 9, Nit 10
