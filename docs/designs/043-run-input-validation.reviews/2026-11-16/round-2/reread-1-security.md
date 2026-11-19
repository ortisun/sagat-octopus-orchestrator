# Targeted re-read 1 of design 043: security (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: a763d9d01d36acea3bb3cad1e6e861d7c4f5ce74
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read, not a
full review. The reviewer read files only and ran nothing.

Round-2 Major 1 (pointer bytes): closed. Round-2 Nit 3 (tenant predicate): closed, D043-175's wording overstated. Round-2
Nit 2 (pre-check pool): half closed (finding 1).

## Nit

1. Every tenant's pre-checks share one FIFO owner of the queue, ad hoc starts included: a tenant flooding dry runs can delay
   another tenant's publication to 503 (Major if a release compile takes 150 ms or more).
2. `version_still_started` ignores stored starters (a pinned schedule) and scans the runs of the workflow.
3. The sentinels' positive controls are weak (row 2's "422" matches a client log line; row 13 has none).
4. `MAX_POINTER_BYTES` changes what v1 admits but is not pinned.

No finding: the 503 body, starving starts, the `interrupted` label, the nil owner.

Counts: Blocker 0, Major 0, Nit 4
