# Security review of the digest-key force (T022-108, ADR-0093 decision 5; 2026-11-20)

Reviewer: `security-reviewer` subagent, read-only, given the working-tree diff (migration `20270202000000`, tests, runbook,
ADR and register updates) with the constitution, the spec, the earlier flow-control migrations and the owner's row
OD-2026-11-19-2 (not the author's conclusions).

## Findings

1. minor: the 1,000-character bound of `reason` is checked on `btrim(reason)`, so a reason padded with spaces is
   unbounded (an append-only row nothing can clean up); the runbook and the storage classification say otherwise.
2. minor: the stated over-admission bound does not hold after more than one force (a key could have rows under three
   versions), and is wider than "the holders at the force" when the old row grants its waiters later; `ALREADY_RETIRED`
   hides the rows a forced version still has.
3. minor: `actor` is not validated like `reason` (control characters, a forged second line; a NULL or long actor fails
   with 23502/23514 instead of 22023).
4. minor: the force has no audited API, CLI, span or metric (NFR-022-005, constitution VI).
5. minor (suspicion, depends on roles): the history is append-only only for roles that cannot drop its trigger; an
   operator on the schema owner's login can retire a version by a direct `UPDATE` with no record.
6. nit: `[[:cntrl:]]` depends on the collation and misses Unicode format characters.
- Nits: lock queueing behind an open operator transaction; never put key material in the reason; FR-022-056 not named
  in G1195; untested TRUNCATE, padded reason, invalid actor; the listing is unbounded (operator only).
- Checked with no finding: the runtime role cannot execute the force or the listing nor read the history (42501 in the
  tests); `SECURITY DEFINER` with a pinned `search_path` and qualified names; tenant isolation of the listing's joins;
  races (the force's `EXCLUSIVE` lock against the deciders' `FOR SHARE`); no key material or digest in the history,
  listing, errors or test output; the down migration only drops an empty table.

## Resolution (author, 2026-11-20)

| Finding | Resolution | Evidence |
|---|---|---|
| 1 | Fixed: `char_length(reason) BETWEEN 1 AND 1000 AND btrim(reason) <> ''` in the function and the table's CHECK | `migrations::the_force_is_the_operators_audited_and_retires_only_a_retiring_version` (a reason of 1,001 characters with padding), red before the fix |
| 2 | Fixed: a second force is refused `FORCED_VERSION_DRAINING` while an earlier forced version has rows (a key has at most two rows); `ALREADY_RETIRED` answers the rows the version still has; the bound is restated as "what the old row holds at each moment, never more than its limit, until its holders and waiters drain" in ADR-0093, D022-046 and the runbook, and O1174 asks the owner to confirm that reading | same test (`FORCED_VERSION_DRAINING`, then `FORCED` once the rows drain), red before the fix; hand mutation (the guard off) caught |
| 3 | Fixed: the actor is checked first (non-null, 1..255 as typed, not blank, no control character: 22023), and the table's CHECK refuses control characters | same test (NULL, empty, a line break), red before the fix |
| 4 | Recorded: G022-006 (no audited API, CLI, span or metric for the digest-key commands); T022-061, T022-082 | review only |
| 5 | Recorded: G022-006; the runbook tells operators to use their own login and never to update `flow_control_key_versions` by hand | review only (deployment roles) |
| 6 | Accepted: the same class the registry's history uses; a viewer sees the raw text in a SQL client | review only |
| Nits | The runbook advises `SET lock_timeout`, a script rather than an open session, and no key material in the reason; G1195 names FR-022-056 and SC-022-028; TRUNCATE is tested; the listing stays unbounded (operator only) | runbook, research.md |
