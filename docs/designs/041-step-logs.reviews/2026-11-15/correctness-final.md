# Correctness final review of design 041 (owner answers of 2026-11-15)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 35070fe9 (the owner's answers built in), with the diffs b1b2c249..69e0f828 and 69e0f828..35070fe9
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: one fresh `correctness-reviewer` agent, given the current files, the two diffs and its `correctness-reaudit.md`, not `resolution.md`; no shell. Recorded here condensed. The author's answers are in `resolution.md` section 7.

## Re-audit findings

M1, M2, M3, M5 and N6, N8, N9, N11, N12 answered; M4 partly (new 4); N7 partly (ADR-0084 still names "a held transaction whose id equals the end horizon"); N10 answered but broken (new 2); N13 partly (new 5). No cycle found in the marker's lock order against the ingest, the purge, the delete route and a policy edit.

## Blocker

None.

## Major

1. Iterations of a collection collide on one script record (`scripts.rs:1784-1796`, `20261014000000_scripts.up.sql:277-278`): the second and third iterations of attempt 1 return as repeats with no record and no marker; research.md's claim and the section 7 test are wrong. Keep G041-007 open; add a two-iteration test.
2. The runtime role cannot read `step_log_enforcement_enabled` (`quota_accounting_control` is revoked from `sagat_app`) nor lock the usage row for the marker's exact check.
3. The 128-byte charge makes the default record ceiling unreachable if the attempt and run byte ceilings count it: 1 MiB / 128 = 8,192 rows; CUJ 3's 10,000 lines cannot happen. Measure the ceilings on text only, or raise them.
4. The first barrier test cannot show its claim: the usage row is taken by the deferred finalizer during COMMIT, so ingests "held just before their commit" hold no lock on it. Hold them after `SET CONSTRAINTS ALL IMMEDIATE`.

## Nit

5. ADR-0084's 2026-11-15 Revision contradicts itself ("proposed", "Revision accepted", "Not in force") and the design (decision 8's `log`, decision 11's fences).
6. The finalizer branch's fences are ambiguous (a positive log delta also takes the content fence today, `durable_quotas.up.sql:309-315`).
7. A skipped marker is recorded on a stream no listing shows.
8. A marker can outlive its execution (different retentions) and open a 404.
9. The delete route does not see uncommitted stream rows; say it removes what was committed before it.
10. The count rule breaks at saturation (2^53 against 2^62-1); state the bound.
11. The marker's exact read under the row lock and its stream's `last_at`/`expires_at` are unstated.

Counts: Blocker 0, Major 4, Nit 8
