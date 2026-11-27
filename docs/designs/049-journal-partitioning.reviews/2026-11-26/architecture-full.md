# Architecture review of design 049, full round of 2026-11-26

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 66634fb997c95e6725e2e8773f120c112759b347
Reviewer: architecture (full round; the approval's architecture report is the re-read that follows the fixes)
Note: the reviewer's output, kept whole. The reviewer read files only and ran nothing. Line numbers are of the reviewed
commit.

## Blocker

1. A child run's bucket breaks running work. Requirement 049-001 (:93-101, step 1 at :374) gives a child its root's
   bucket, but the `runs` check refuses any bucket other than the month of the child's own `created_at`. The late-legacy
   trigger (:98-101, :416-419) also refuses every legacy child created after the switch, so a long run that spawns a child
   in a later month fails, and so does any legacy tree after the switch. This breaks runs in progress (Principle IV). Fix:
   replace the CHECK with a trigger (a root gets its creation month, a child must equal its root) and exempt the children
   of legacy roots.
2. `run_payloads` is keyed by the tree's root (Requirement 049-005), but the purge key, the bucket trigger and the
   reference trigger use the run (Requirement 049-014 at :206, 049-002 at :103, 049-006 at :134 "same tenant, run and
   bucket"). Feature 020 purges one run per job (`archival_jobs` primary key is per run). Purging the root deletes the
   whole tree's payloads before its children are archived, which is irreversible loss and breaks ADR-0054 decision 3.
   Fix: purge a tree's `run_payloads` by `(tenant_id, root_run_id, journal_bucket)` only after every run of the tree is
   verified and purged, and make the triggers check by root.

## Major

1. The switch race is not closed. Requirement 049-010 (:184) takes the `FOR SHARE` read in the run insert, but the input
   payload is written first (:96-97, :75), and its table and bucket depend on the switch. A switch landing between the
   two writes makes a dated run whose input sits in `payloads`, which the reference trigger refuses. Fix: fold the `FOR
   SHARE` read into the first write of the creating transaction.
2. The late-legacy fence adds nothing and rests on an application-set clock. Once creation reads the switch row, 0.3.0
   code cannot write a late legacy root, and 0.2.x cannot run (stopped upgrade). The 30 s window compares against
   `created_at`, which the application sets (:76). Fix: the trigger refuses a legacy root once the switch row is set. Drop
   the window and `JOURNAL_SWITCHED`, and update `docs/designs/053-rolling-upgrades.md:129`.
3. Rows that are not owned by a run can still point at a dated run's content, and retirement drops it. Examples: feature
   022's `step_cache_entries` (kept to their own TTL) and `test_run_pins`
   (`specs/020-visibility-analytics-archival/data-model.md:230-232`). Requirement 049-015 (:214-219) only points them to
   049-005, which covers runs. The triggers check on write only, so the drop leaves these references dangling. Fix: such
   holders copy into `payloads` as shared columns, and the registry test refuses a non-run column that references
   `run_payloads`.
4. Lint waivers are undercounted. ADR-0132 (:5) and the design (:378-381, :400) name two exceptions. Steps 1 and 2 are
   no-transaction files with many non-concurrent statements or several concurrent ones, step 3 replaces the primary key,
   and step 5 drops triggers. These break Requirement 053-002. Fix: split them into migrations of one statement each, or
   list every waiver as `-- lint-allow` in the design and the ADR.
5. The "online" claims contradict the stopped upgrade. Requirement 049-004, Success criterion 049-003, CUJ 3 ("while it
   serves") and :401-404 ("from 0.3.x on they run while the platform serves", "while the previous release still runs")
   are not true: these migrations run once, inside the stopped 0.2.x to 0.3.0 upgrade (ADR-0131 decision 8). Fix: tie
   them to 053's develop-to-develop mixed-writer run (Requirement 053-009), or drop them, and rewrite CUJ 3.
6. Ownership with feature 020 is split and the reference list is duplicated. :462 says 049 "defines the registry file";
   :131 says feature 020 owns it, and the migration keeps a second list. T049-11 and T049-14 build parts of 020's purge.
   T020-046 (`specs/020-visibility-analytics-archival/tasks.md:87`) still finds references from catalog foreign keys,
   which T049-10 drops. Fix: one owner per purge path and one registry, and land T049-15's amendment of T020-046 before
   T049-10.
7. The ADR misstates who authorises deletion. ADR-0132 decision 6 (:37) says the amendment of Principle I is "that
   feature 020 carries". The design (:306-311) says constitution 5.1.0 under Owner decision 2026-11-25-3, while T020-061
   (020 `tasks.md:18`) still opens its own amendment. Fix: cite 5.1.0 and Owner decision 2026-11-25-3 in the ADR, and
   retire T020-061 or point it to 5.1.0.
8. The build order conflicts with design 053. Design 053 (:242) puts its lint (Task 053-2) before any migration of design
   049; design 049 (:468-470) lets Task 049-8 proceed without it, "retried by hand". Fix: make Task 053-2 a hard
   prerequisite of Task 049-8.
9. "Enforced by" has gaps. ADR-0132:6 omits the `pg_constraint` check that no foreign key touches a partition tree
   (Requirement 049-006), the single-adapter read rule (Requirement 049-005) and "child takes its root's bucket". The rule
   "retirement is never a platform process" (:429-431) has no check. Smallest fix: a runtime-role privilege test that
   DETACH and DROP fail, a fixture in `scripts/check-architecture.py`, and the three checks listed in the ADR.

## Nit

1. Partitions ahead disagree: "current and next three" (:150), "current and next two" (:397), "the current one, two
   ahead" (:274).
2. The legacy partition is "detached and dropped once no legacy run remains" (:448-449), but Requirement 049-009 (:165)
   says it is never retired. Stub rows stay forever, so the condition never holds. Delete the sentence.
3. ADR-0132 Context (:11-12) says "one column without a key" and then "three more"; the design says three.
4. `crates/infra-postgres/src/telemetry.rs:361-364` builds SQL that names `run_events` with `format!`, which the new
   static check will refuse. Add it to T049-8.
5. Open Question 8 (:292-297) leaves out a per-function `SET plan_cache_mode` on the trigger functions. Only that option
   reaches the cached plans of the PL/pgSQL triggers without a role-wide planning cost.
6. The monthly width is fixed in the `runs` CHECK. Record it in the ADR as hard to reverse.
7. The deactivation of `journal-buckets-v1` (Requirement 053-008 drain check) is not stated. Say it is never deactivated,
   or define the drain.
8. The labels of `journal_bucket_held_runs` are unspecified. State `reason` only, with no bucket label.
9. T049-5 binds `journal_bucket` before T049-8 step 1 adds the column. Order the tasks.
10. The switch is off by default, so 0.3.0 gains nothing until an operator runs `journal switch`. Say so in T049-17 and
    the release notes, or activate it at first start.
11. ADR-0027 and ADR-0131 still lack the "Refined in part by ADR-0132" note (T049-1 is pending).
12. Duplication with other branches and worktrees was not checked: this review had no shell.

Counts: Blocker 2, Major 9, Nit 12
