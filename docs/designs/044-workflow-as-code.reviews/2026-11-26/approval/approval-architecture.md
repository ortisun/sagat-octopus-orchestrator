# Targeted re-read of design 044 for approval: architecture, confirmation (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 14513c8533b0a0a475d77fe21f3886c4e07c52cc
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Scope: the text changed
since eeee9d0e (the first architecture pass, `reread-1-architecture.md`). All 18 findings of that pass are closed. The
Majors are settled in writing under OD-2026-11-24-4 (rows in `docs/owner-decisions.md`), each as a follow-up task.

## Major

1. The lifetime of an earlier lease's command-value blobs is decided three ways (ownership, reuse by descriptor, the
   24-hour sweep): pick one and state it.
2. `tenant_share` is counted although, like `command_value_ceiling`, other runs of the tenant cause it; the streak rule.
3. The SDK's carried-state throw depends on an `active` successor it is never told; the server should check it.
4. The command-value ceiling departs from accepted Architecture decision 0023 decisions 5 and 7 without citing it.

## Nit

5. Architecture decision 0105's Enforced by and Consequences for its new clauses.
6. Architecture decision 0107's ceiling tests; the 1,024 is not configurable.
7. `COMMAND_VALUE_RELEASED` missing from section 13.3.
8. `CancelOrigin` on `run.stop_requested` needs a decoding rule for older rows.
9. The new setting should end in `_SECS`.
10. A misplaced clause in Architecture decision 0108 decision 2.
11. The register misses the new ceilings and rules.
12. "The unnameable mark" is undefined in Task 044-9.
13. Counted failure causes are not visible in a metric.

Counts: Blocker 0, Major 4, Nit 9
