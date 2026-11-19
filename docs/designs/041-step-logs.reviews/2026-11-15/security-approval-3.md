# Security review of design 041, the third approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 6dceb5482d1a5a27b22be4018ed2df83e106137b
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a fresh `security-reviewer`, given the committed design, its companions, the ADRs, the constitution, the owner decisions and its own `security-approval-2.md`; no shell. Recorded condensed.

Previous findings: 1 to 9 closed; 10 partly open (Nit 3).

## Blocker

None.

## Major

1. The classified-namespace rule is the reverse of feature 016's default: an unannotated field is `pii` (`crates/domain/src/classification.rs:41-42`; `crates/infra-schema/src/classification/publication.rs:484-490`), so in a namespace that never used an annotation a developer or operator without the PII grant gets no payload values today, yet reads them in step logs; adding one annotation would close the logs. Register a gap and ask the owner, with the 016 fact stated: (a) keep and record the condition; (b) require the grant where the version has `classification_rules = 1` or no annotation; (c) require it everywhere. Correct the "safe side" wording.

## Nit

1. Functions the migration replaces (`sagat_quota_add_delta`, `sagat_quota_row_delta`, `sagat_quota_flush_deltas`, `sagat_quota_measure`) are not covered by the `prosecdef`/`proconfig` test.
2. The `AUTHORITY` audit cites a path that fires only on an error status; call `spawn_audit_access_denied` explicitly.
3. Line text can reach PostgreSQL's own log (`DETAIL: Failing row contains`; `log_parameter_max_length`).
4. The life-of-process secret set contradicts `sdks/SPEC.md` §6.7 and has no handling rules.
5. A tail may not notice that its namespace became classified.
6. `worker_time_epoch_ms` is not bounded before SQL.
7. Two flooding tenants can starve a third's ingest.
8. ADR-0085 is stale where it matters in an incident (the read switch in Rollback).

Counts: Blocker 0, Major 1, Nit 8
