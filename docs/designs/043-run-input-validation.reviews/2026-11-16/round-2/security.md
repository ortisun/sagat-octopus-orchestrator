# Security review of design 043, round 2 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: b9b0f1b56d121ba3f977e2cea888305402f2eb47
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept, numbered across the
report. The reviewer read files only and ran nothing.

## Major

1. The pre-check's pointer bookkeeping costs quadratic memory and CPU, and `$defs`/`definitions` names have no length
   limit: a 64 KiB schema with a ~32 KiB `$defs` name over ~3,600 positions gives each position a ~32 KiB pointer
   (estimated ~115 MB twice and ~1.4 GB of comparisons), reached on draft save, publication, ad hoc execution and start.
   Fix: the name limit on `$defs`/`definitions`, positions as (parent, token), or a cap on total pointer bytes; an
   adversarial test.

## Nit

2. The pre-check pool waits without a limit and is not fair among tenants.
3. `refuse_cron_tick` updates a trigger without the tenant in the predicate.
4. A too-deep value under an unknown keyword reports a data key as the keyword.

Classes checked with no finding: tenant boundary, authorization and admission, secrets and payload values, least
privilege, injection and parsing, races.

Counts: Blocker 0, Major 1, Nit 3
