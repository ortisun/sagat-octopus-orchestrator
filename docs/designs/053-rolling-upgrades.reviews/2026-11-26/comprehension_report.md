# egm-comprehension on design 053, full round of 2026-11-26

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 7771688e6ab65562395c6c2faa8c548e66a50bdd
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing. Run on the text revised after `zoom_out_report.md` and `critic_report.md` (at 212e4262).

## Major

1. Requirement 053-001 forbids a new `CHECK` on an existing table, while Requirement 053-006 adds each capability by
   rewriting the capability lists' checks; the lint would refuse every new capability. Fix: exempt a check that only
   widens those lists, with its own rule and fixture.
2. No value exists for the online rules' start when the lint merges. Fix: a placeholder until 0.3.0 is cut, and the step
   that sets it.
3. Whether the partitioned-index refusal applies from the cut-off or from the online rules' start is unclear; design 049
   creates indexes on partitioned `run_events` in 0.3.0.
4. Design 049 writes `-- lint-allow: stopped-upgrade` and calls the unique-index rule "design 053's contract rule";
   design 053 defines neither. Fix: any waiver before the online rules' start is accepted whatever its rule name;
   correct 049's wording.
5. The schema check has two names: 049 grants `sagat_schema_handshake()`, 053 defines `sagat_schema_missing(bigint[])`.
6. Design 048's Requirement 048-014 says design 053 adds two gate reasons; it adds one.
7. Requirement 053-004 reads as one confirmation, design 048 as a wait before each migration (up to 60 s each), outside
   the 171 s bound.
8. Two retry budgets meet on a migration without a transaction (the 10 attempts and the 3 invalid-index retries), and the
   171 s bound does not say whether it covers a concurrent build.
9. Nothing maps each capability to the migration that adds it to the closed list, which registering "only the
   capabilities whose migration is applied" needs.
10. At activation the danger is a process meeting state it cannot process, not only a late write. Fix: reword.
11. Phase 2 "is refused" in the requirement and the criterion but "waits" in CUJ 3 and a test name. Fix: name the error
    and who retries.
12. "N-1" is the release before N in the terms but the newest patch of the previous minor in Requirement 053-008; a patch
    release is then never gated against its predecessor.
13. The rollback step and the per-capability scenario cannot both be built when N has a capability that cannot be
    removed, or whose drain check passes only when no N state remains.
14. Before 0.3.0 is cut, the CI run cannot prove the online migration rules, which do not apply yet.

## Nit

1. A lock or statement timeout error names no table; take it from the lint's parse of the last statement.
2. "Count against the second" reads as one second; say `statement_timeout`.
3. Name sqlx's `-- no-transaction` header.
4. "Succeeds `instance_heartbeat_reporting_change`" reads as "returns success"; say "replaces".
5. Terms used without definition: requirement, capability, a capability's row, closed list, kind, deferred epoch check,
   the `verify` step, a full registry table, baseline run, transition latency.
6. Decisions D006-098, D006-100, D043-072, feature 018, designs 051 and 052, owner decisions and "the coordinating
   session's split" have no link or gloss.
7. `input-schema-inventory` uses exit code 2 for a failed wait (design 048); the `secret-cleanup` job is the command
   `cleanup-secrets`.
8. Success criterion 053-001 and Requirement 053-008 disagree (the 10 s floor; "within" against "above").
9. The engine answers gRPC `UNAVAILABLE`, not 503.
10. The drain functions' arguments are unnamed; activating a `draining` row is unspecified.
11. "(the requirement set it keeps)" describes nothing in Task 053-3.
12. A 0.3.0 binary on a 0.2.x schema finds no `sagat_schema_missing` (`42883`); say what it does.
13. ADR-0131's title promises contracts, its Context names two runbooks, and Task 053-1 says "revise" an ADR already
    revised.

Counts: Blocker 0, Major 14, Nit 13
