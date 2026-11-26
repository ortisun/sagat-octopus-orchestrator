# Security review of design 044, round 7 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: e0a23b09f7ddbe225e80110c2cf25f3da960b730
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 6's Majors 2 and 3
are closed and 1 partly closed. No finding on the tenant boundary in SQL, secrets on the workflow task, export and
blob disclosure, authorization, definer functions, injection, metric labels or the re-checked races.

## Major

1. Copied initiators keep the source's sequences, and runs that mix graph and code carry no initiators: copy at
   `first_seq = 0`; a child start writes the parent's starter, chain initiator and publisher; any approval of a run with
   rows uses them; three tests.
2. Literal flow-control keys go into the node spec and the journal unclassified, against feature 022's privacy rule:
   store them as business literals and keep only feature 022's keyed digest in the spec; a test.
3. The new messages exceed the worker service's 320 KiB decode limit, and a poll answer with first pages has no
   ceiling: per-RPC ceilings, a bounded first page and poll answer, streamed or chunked replay-check digests; tests.
4. The server's overlay cache has no total or per-tenant bound: a byte-bounded cache with a tenant share, or none.

## Nit

5. The pause flag needs a column grant and a `FOR SHARE` read.
6. The Python hash seed can be searched from the digest; rely on the key cap.
7. The deterministic random values are predictable to holders of the seed.
8. Marker identifiers have no pattern.
9. Worker-query rows lack the requesting principal; their wait schedule is unstated.
10. `ReleaseSticky` has no count limit.
11. `history:export` is missing from the custom-roles catalog.
12. `platform_requirement_hold` should be compared, not trusted, when it already exists.

Counts: Blocker 0, Major 4, Nit 8
