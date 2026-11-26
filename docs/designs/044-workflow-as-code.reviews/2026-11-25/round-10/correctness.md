# Correctness review of design 044, round 10 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 4a943e7c15d9ca73e9c1afa6274ad5da8cfb5cbb
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 9's Major 2 is
closed (the chain lock order checked in code) and 1 partly closed.

## Major

1. Only blob-carried values are hashed as descriptors, so a deferred output that changes carrier between activations
   is a `CHAIN_MISMATCH`: hash every value field as its descriptor whatever its carrier; a test.
2. The SDK may blob a continue-as-new input that the server refuses unless inline: take it out of the blob list and
   throw a typed error at the call, or accept a blob input.
3. A cancellation without grace does nothing after a cooperative request (and the chain route answers `409` once
   latched): a no-grace arm that stops the run whatever was journaled; tests for the route and the chain path.
4. A reset copies rows without their `server_detail`, losing the seed, the body digests and the deferral digests:
   copy it with every re-emitted row; a test.

## Nit

5. A test still names the 56,320 ceiling.
6. Readers of `detail` said to strip what now lives in `server_detail`.
7. The cache entry's highest row against the command count; dropping against incremental reads.
8. The cancel route is `POST /runs/{runId}/operations` with `CANCEL`.
9. The SDK should drop its cache on a `REVIEW` answer.
10. A `version: active` refusal that depends on mutable state offers no retry.
11. The stops that close `~workflow` omit a cancellation without grace; Requirement 044-008's wording.
12. Whether a re-emitted deferral counts against the activation's limits.
13. A pause refusal judged at the lease's expiry.
14. The reconciler's once-per-wait check against the risk table's claim.

Counts: Blocker 0, Major 4, Nit 10
