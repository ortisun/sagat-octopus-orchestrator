# Security re-audit of design 041 (revision of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: b1b2c249 plus the uncommitted revision of 2026-11-15 (committed with this report)
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: one fresh `security-reviewer` agent was given the revised design, `specs/041-step-logs/{contracts,data-model,research}.md`, ADR-0084 and ADR-0085 with their Revision (2026-11-15) sections, the constitution, and its own report `security.md`; it was not given `resolution.md`. It read `execution.rs` (671-713, 1017-1044, 4756-4817), `worker.rs:2564-2586, 2715-2740`, `bindings.rs`, `reveal_grants.rs`, `identity.rs`, `audit.rs`, `limits.rs`, both `build.rs`, `primitives.rs`, `dto_scripts.rs`, the instance-registry migration and the `steps` and `workflow_versions` DDL. Its report is recorded here, condensed. The author's answer to each open finding is in `resolution.md` section 5.

## Previous findings

r1-M1, r1-M4, M3, M4, M5, r1-N4, N1-N4, N6-N9 answered; M1 and M2 answered and owner-gated (O041-017, O041-016); N5 answered by redefinition; r1-N5 answered for the server, open for the Rust SDK (Nit 4); M6 partly (Nit 6).

## Blocker

None.

## Major

1. A valid lease can write lines into another run: step 2 finds the `steps` row by tenant and `step_id` only (`steps_step_id_uniq`) and never requires the request's `run_id` and instance to equal the row's, while every existing fence matches run, node and path (`execution.rs:4783-4817`). A worker bound in namespace A with a lease there can send another run's id (namespace B), pass the authority checks against A, and fill B's run with forged lines up to its ceiling. *Fix:* match run, node and path as `lease_unexpired_instance` does; take run, namespace, node and path only from the row; a T041-8 case.
2. The classified-namespace record can fail open after the backfill completes: `platform_requirement_remove` exists (`20270113000000_instance_registry.up.sql:357-378`) and the registry cannot see binaries that do not register (`:21-22`). *Fix (smallest):* an `AFTER INSERT` trigger on `workflow_versions` that records the namespace when the document holds the text `"x-sagat-classification"` (it over-matches, which errs closed), and the same search once over existing rows in the migration.

## Nit

3. Authority last leaks verdicts to a revoked caller (`CAPTURE_OFF`, `REMOVED`, `FINAL_RECORDED`, `FINAL_ONLY`); the second transaction of step 10 must pass step 7 again.
4. `sdks/rust/build.rs:4-8` has no `skip_debug`, so the Rust SDK derives `Debug` for the request; the other languages need a check that the worker's own log holds no canary.
5. `store_error` logs the database error text, which can quote bound values (`identity.rs:753-757`); use `store_error_without_text` (`:727-751`) for every statement that binds line text or `q`, and force one error in the canary run.
6. The cursor's `id` is a database-wide identity, which shows every tenant's line volume; `(stored_xid, sequence)` is unique within a run. Put the xid part (G041-008) to the owner.
7. ADR-0085 rule 6 ("how large") is wider than the platform keeps: viewers hold `quota:read` and see `tenant_log_bytes` usage.
8. The cross-tenant audit would be rolled back with the ingest transaction; `missing_task_error` writes it through `record_denial` outside it.

Counts: Blocker 0, Major 2, Nit 6
