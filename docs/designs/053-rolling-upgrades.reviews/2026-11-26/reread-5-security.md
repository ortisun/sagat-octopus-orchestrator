# Targeted re-read 5 of design 053: security (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 24fa9a3c6d3341518c0f56da7ab8915e405cfb22
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the fix
of `reread-4-security.md` (resolved), not a full review. The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. The "whole privilege set" check misses column-level grants (`has_table_privilege` sees table-level ones only), role
   attributes and schema or database `CREATE`. Fix: `has_any_column_privilege` over every relation, sequences
   included; the role attributes false; no `CREATE` on any schema or the database.

Counts: Blocker 0, Major 0, Nit 1

Fixed at the next commit.
