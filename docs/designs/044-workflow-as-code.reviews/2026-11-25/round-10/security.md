# Security review of design 044, round 10 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 4a943e7c15d9ca73e9c1afa6274ad5da8cfb5cbb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 9's Majors 1, 2
and 4 are closed and 3 partly closed; Nits 5 to 10 addressed. No finding on the tenant boundary, secrets,
authorization, disclosure, injection, the runtime role and definer functions, the races or the other bounds.

## Major

1. A cancellation without grace cannot be built as written: the "later request" rule swallows it, the source check
   forbids it, the idempotency key ignores `graceMs`, and the stops that close `~workflow` omit it: a fourth arm, the
   grace in the key, a `422` for other values, tests.
2. Validating blob-carried values lets one completion make a shared process read and parse unbounded bytes (one large
   blob named 256 times): validate each distinct blob once, cap the bytes per completion with a fair semaphore, or keep
   them opaque as design 043 does; a test.

## Nit

3. The eager workflow task must go through the claim's arm, which writes the binder.
4. The activator of an earlier code version is not an initiator.
5. Worker-chosen identifiers in reviews against Non-functional requirement 044-007.
6. Where risk metadata lives: `detail` against `server_detail`.
7. The pause lock's key.
8. Worker-query routes for a graph run or an undeclared name.

Counts: Blocker 0, Major 2, Nit 6
