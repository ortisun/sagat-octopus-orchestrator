# Security final review of design 041 (owner answers of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 35070fe9 (the owner's answers built in), with the diffs b1b2c249..69e0f828 and 69e0f828..35070fe9
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: one fresh `security-reviewer` agent, given the current files, the two diffs and its `security-reaudit.md`, not `resolution.md`; no shell. Recorded here condensed. The author's answers are in `resolution.md` section 7.

## Re-audit findings

M1, M2 and N3-N8 all answered (checked against `execution.rs:4797-4818`, `canonical.rs:53-58`, both insert sites).

## Blocker

None.

## Major

A. The script marker's step comes from fields the runner sends on an endpoint with no lease check (`dto_scripts.rs:953-982`, `scripts.rs:1798-1807`): a `ScriptExecute` holder (a runner token, `identity.rs:689`, or admin) chooses which iteration's stream gets the marker and its generation. *Fix:* write it only if a `step.leased` event with that path, attempt and generation exists; a test.
B. The runtime role cannot run the new accounting as written, and an implementation that follows the text can bypass the quota: `sagat_app` holds only `SELECT` on `tenant_retained_usage` and nothing on `quota_accounting_control` (`durable_quotas.up.sql:564-566`); `sagat_quota_row_delta` routes every non-content class to `log_delta` (`:231-238`) and the finalizer loop skips rows whose only delta is a new one (`:305-308`). *Fix:* name `SECURITY DEFINER` accessors of the existing pattern, the `add_delta` argument, the routing branch and the loop filter; a runtime-role test that a transaction writing only step logs over the limit is refused.

## Nit

C. The classified trigger is not `ENABLE ALWAYS` and has no pinned `search_path` or qualified names (a temp table could capture the record); the migration must run trigger and backfill in one transaction.
D. `tenant_step_log_bytes` usage under `quota:read` (viewers) shows step logs alone; ADR-0085 decision 6 speaks of total retained log bytes.
E. A reader can make the server spend a transaction id per read by stripping `end` from the readable cursor; take `end` from `pg_snapshot_xmax(pg_current_snapshot())` instead.

Counts: Blocker 0, Major 2, Nit 3
