# egm-zoom-out on design 049, full round of 2026-11-26

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 66634fb997c95e6725e2e8773f120c112759b347
Note: the reviewer's output, kept whole. The reviewer read files only and ran nothing.

## Major

1. The real cost driver (gap 1099, the reference check on shared payloads) is removed by run-owned `run_payloads` alone.
   Partitioning only pays off through retirement, and retirement (T049-14) may not ship in 0.3.0. (Overview items 2–5,
   Goals, Tasks 049-8/10/14.) Fix: either state why 0.3.0 is the right window (for example, the stopped upgrade) and add a
   measured per-run delete/bloat cost, or ship unpartitioned run-owned payloads with foreign keys now and move
   partitioning to land with retirement.
2. The 0.2.x→0.3.0 upgrade is a stopped upgrade (ADR-0131 decision 8; the design admits Requirement 049-004 then
   "protects nothing"), and the conversion runs only once. So the online machinery is never used online: lock_timeout
   retries, the step-by-step no-transaction files, both design 053 lint exceptions, and SC 049-003 under load. (Req
   049-004, SC 049-003, "`run_events`, online".) Fix: write it as a stopped-upgrade migration that still attaches without
   a copy, and drop SC 049-003 and the online requirement.
3. Retirement is overbuilt for what the ADR calls an "opportunistic" path. A single namespace with no policy (the
   default) holds every month of a shared database. Mark-only still does per-run work, so the drop saves only the
   `run_events`/`run_payloads` deletes. It adds `CONTENT_PENDING_DROP`, resume from the catalog, reattach-and-restart,
   `journal_retirements` and a chart job holding an owner Secret. (Req 049-009, Req 049-014 mark-only form, T049-14.)
   Fix: defer it past 0.3.0. At minimum, drop the chart scheduled job and the pending-drop state.
4. The late-legacy refusal (30 s window, its own SQLSTATE, `JOURNAL_SWITCHED`, refusal metric) duplicates protection
   that already exists. The creating transaction already reads `journal_bucket_switch` FOR SHARE (Req 049-010), and the
   registry refuses older processes. Besides, a legacy run created late is valid and is purged per run. (Req 049-001 last
   clause, Req 049-008, "The switch".) Fix: remove the late-legacy trigger, the window, the error reason and the metric.
5. Adding `journal_bucket` to 12 unpartitioned run-owned tables stores the same value twice. The reference trigger must
   read `runs` anyway: it needs `root_run_id` for the `run_payloads` key and compares the row's bucket with the run's.
   (Req 049-006, Layout row "steps and the other run-owned tables", step 1.) Fix: take the bucket from the `runs` lookup
   the trigger already does, and add the column only to the partitioned tables.
6. SC 049-002 (12 × 1,000,000 chain runs) and SC 049-003 (100 GiB `run_events`) cannot run on the free runners
   OD-2026-11-25-1 requires, and no runner is named. (Success criteria 049-002/003.) Fix: scale them to a declared free
   runner or the local machine, and state which runner the report used.

## Nit

7. `sagat_quota_release_bytes` partly duplicates `sagat_quota_add_delta(tenant, content, reserved, logs)` with a negative
   content delta, plus the existing deferred flush (migration 20261024000000). (Req 049-009, step 6.) Fix: reuse it; add
   only the underflow check if it is missing.
8. T049-16 measures `steps` partitioning, which is a Non-Goal of 0.3.0 and depends on design 052. Fix: move it to
   "Follow-up tasks".
9. The bucket rule is enforced four times: the static Python check, a `pg_proc` migration test, a suite assertion and
   design 038's lock budget. (Req 049-011.) Fix: keep the static check and the lock budget; drop the `pg_proc` test unless
   a migration function actually reads a partitioned table.
10. The requirements are at implementation altitude: `FOR KEY SHARE`, `relfilenode` assertions, and 23514 told apart by
    "no constraint name and partitioned table". (Req 049-002/006/008.) Fix: state the outcomes in the requirements and
    move the mechanics to the Technical Plan. Map every missing partition to one retryable `UNAVAILABLE`.

Counts: Blocker 0, Major 6, Nit 4
