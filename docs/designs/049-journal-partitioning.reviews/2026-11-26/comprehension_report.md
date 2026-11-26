# egm-comprehension on design 049, full round of 2026-11-26

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 66634fb997c95e6725e2e8773f120c112759b347
Note: the reviewer's output, kept whole. The reviewer read files only and ran nothing.

## Blocker

1. A child run must take its root's bucket, yet the `runs` check refuses any bucket that is not the month of the run's
   own `created_at`. The late-legacy trigger also refuses a legacy child created after the switch. So a child spawned in a
   later month, or by a legacy root after the switch, is refused. Where: Req 049-001, 049-010. Fix: check against the
   root's bucket and exempt children of legacy roots.
2. The input payload is written with its bucket before the run row, but the switch row that decides dated vs legacy is
   only read inside the run insert ("so no statement is added"). The bucket is therefore unknowable when the payload is
   written. Where: Req 049-001, 049-010, 049-012. Fix: say where the switch is read before the payload write, and budget
   that statement.
3. `run_payloads` is keyed by `root_run_id`, but the bucket trigger, the reference check and the purge all key on
   `(tenant_id, run_id, journal_bucket)`. Purging a child by run_id is undefined or deletes its siblings' payloads, and
   `runs` lists no `root_run_id`. Where: Req 049-002, 049-006, 049-014, Layout. Fix: define the purge unit (run or tree)
   and where the root id comes from.

## Major

1. The number of partitions kept ahead contradicts itself. Req 049-008 says the current month plus three; step 5 says
   plus two; Open Question 2 says "the current one, two ahead" (+3). Fix: pick one number and recompute the bucket counts
   and the alert threshold.
2. Step 2 (Task 049-8, not gated) builds the partial legacy indexes, but Req 049-007 and T049-11 build them only after
   Task 049-3, gated by 5.1.0. OQ 4 says new writes "never touch" them; Req 049-007 says their cost is "not nil". Fix:
   remove them from step 2 and reconcile the cost claim.
3. The gating is inconsistent. The plan header says only T049-14 waits for the amendment, but Owner decisions and
   T049-11 gate T049-11 too, and T049-18 is in no release. Fix: list every gated task.
4. These migrations ship in 0.3.0, and the 0.2.x to 0.3.0 upgrade is a stopped upgrade, yet CUJ 3 and SC 049-003 say
   they run online. "From 0.3.x on" is unclear because the migrations will already have run. Fix: say which upgrade runs
   them under load and what SC 049-003 measures.
5. The append-only triggers are "enabled always" on every partition and refuse DELETE, yet Req 049-014's purge deletes
   `run_events`/`run_payloads` rows. The text never says how the purge gets past the triggers. Where: Req 049-003,
   049-014. Fix: name the mechanism and the role that uses it.
6. Who owns the registry is contradictory. Req 049-006 says feature 020 owns `run-content-references.json`;
   Interactions says "this design defines the registry file". The "exclusion list" is never defined. Fix: name one owner
   and define the exclusion list.
7. Neither the "two shared columns" nor the 20 foreign keys that step 1 drops are named. Step 1's 12 tables include two
   with no foreign key, and it is unclear whether `runs` references payloads. ADR-0132's Context counts the no-key columns
   differently (1 + 3 vs 3). Fix: list the columns and the constraints.
8. `CONTENT_PENDING_DROP` is recorded "per run" with no column or table given. A retirement "abandoned" after its purges
   is undefined, since every step is said to resume. Fix: define how a retirement is abandoned and where the state lives.
9. The late-legacy refusal has "its own SQLSTATE and constraint name", but neither is given, and the adapter must map it
   to `JOURNAL_SWITCHED`. Comparing the app-set `created_at` against a switch time from an unnamed clock is ambiguous.
   Fix: specify the SQLSTATE, the constraint name and the clock source.
10. Req 049-005 makes the static check refuse statements that read `payloads` "with a run's digest", but Req 049-011
    does not include this rule, and it is unclear how a static check could detect it. Fix: specify the rule in Req
    049-011 or move the enforcement elsewhere.
11. The legacy partition "is never retired" (Req 049-009), yet it is "detached and dropped once no legacy run remains".
    It is not said who drops it, and stubs remain forever. Fix: state the condition and the actor, or delete the sentence.
12. A reader without the bucket "reads it (one statement)", but Req 049-012 forbids changing the statement count of a
    lease or completion. "Holds the run row" is undefined. Fix: say that hot-path readers always have the bucket, or
    budget the read.

## Nit

1. `journal_bucket_switch` is "one row, empty", and it is unclear what an empty row is (a NULL time?). Reading it
   `FOR SHARE` "folded into the run insert" is not shown. Fix: give the column and the SQL shape.
2. Req 049-008 says 049-013's audit "fails on a duplicated trigger", but Req 049-013 doesn't say so.
3. The error mapping has no case for a missing bucket that is before the current month but not retired.
4. Step 4's "the two checks" are unnamed. "Step 1 of the next migration" is ambiguous. Step 1's reference triggers refer
   to `run_payloads` before it exists.
5. Terms used but undefined: "both completions" (SC 049-001), "chain shape", "nine shapes" (T049-2), "run-purpose
   blobs", "sink/hold", D298, "Erasure", "active region", "design 052's queue", "feature 020's job state". The held-run
   reasons are not mapped to the eligibility conditions.
6. Architecture decisions 0054, 0027, 0082, 0084, 0131 and 0067 are cited by number but not linked. "Owner decision
   2026-11-24-511" looks like a typo.
7. Req 049-001 cites Req 049-010 for the 30 s liveness window, but Req 049-010 doesn't state it.
8. The rationale "(one detach may be pending per table)" for the global lock is cryptic, and the advisory-lock keys are
   unspecified.
9. The last sentence of Req 049-014 (receipts owned by `resolved_run_id`, `transfer_input_digest`) and the "re-check
   under a savepoint" in Req 049-007 can't be followed without feature 020.

Counts: Blocker 3, Major 12, Nit 9

The reviewer's note: the working tree was read; the commit could not be confirmed from inside the reviewer's sandbox.
