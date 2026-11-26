# Targeted re-read 10 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 287c7bdbe65eb1fed638000b6f658029b107c7f2
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 9, not a full review. The reviewer read files only and ran nothing.

Re-read 9's five Nits are resolved (the fourth as a stated residual of feature 032).

No Blocker, no Major.

## Nit

1. Read literally, the rule refuses `InertHealth`'s own definition and the test constructor, which `tests/` cannot see
   behind `cfg(test)`; an allow-list by file would let it return. Fix: a `test-support` feature enabled only by `tests/`
   and dev-dependencies; the rule refuses the names outside items gated by it and the feature in `apps/`.

Counts: Blocker 0, Major 0, Nit 1

Fixed at the next commit.
