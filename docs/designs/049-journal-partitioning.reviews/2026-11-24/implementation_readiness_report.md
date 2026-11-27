# Implementation readiness review of design 049 (2026-11-24, stage 9 of egm-review)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 3d09d92388c49d565826a1bb21e7901840638f6d
Reviewer: egm-implementation-readiness
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read the
working tree and confirmed the design's factual claims (the triggers, the absence of partitioning, the 22 foreign-key
columns in 14 tables, `collection_scope_items.item_digest` without a key, `run_events_pkey`, `child_run_id`, the input
written before the run row, the report figures, the existence of the cited files and test targets).

Verdict of the reviewer: not ready.

## Blockers

1. No task names its files, functions, migrations or tests (3 run-insert sites, 14 `run_events` insert sites, about 15
   payload insert sites in 10 files, the command, housekeeping and capability files).
2. No task rewrites the 43 existing references to `run_events` in 18 files to name their bucket, and the plan does not
   decide how each reader obtains the bucket without an added statement.
3. Tasks 049-10 and 049-11 depend on the registry `crates/infra-postgres/run-content-references.json`, which does not
   exist and is created by feature 020's Task T020-046, itself gated on the owner's amendment.

## Majors

4. Tasks 049-5 and 049-8 overlap (both add `runs.journal_bucket`); how the creating transaction learns that
   `journal-buckets` is active without a statement, and the check expression, are not given.
5. The partitioning DDL is under-specified: the `run_events_pkey` name collision after the rename, the index
   `run_events_instance_idx` and three check constraints not carried to the parent, cloned triggers clashing with the
   legacy table's, the grants on the renamed table not revoked.
6. Design 048's lint would refuse more than the rename (a `DROP` in an expand migration twice, a primary key that cannot
   be `NOT VALID`, a check added and validated together); the lint, the retrying `migrate`, the `-- no-transaction`
   convention and design 051's driver do not exist and are not declared as prerequisites.
7. Feature 020 has no "mark-only form"; its registry covers only copies with run provenance; no task amends feature
   020's data model or tasks.
8. The registered columns are incomplete: `collection_scope_items` makes 11 tables; `leaf_collection_success_certifications`
   has two digest columns without a key and `run_resets.prefix_digest` is run-linked; which column is "the run" for
   `operation_update_receipts` and `chain_segments` is not said.
9. Cross-run payload copies (the chain handoff's `transfer_input_digest`, resets, reruns, children) are not designed.
10. `run_payloads`' classification conflicts with the quota test until the contract adds its triggers.
11. Task 049-7 rests partly on a wrong premise: only the classification test enumerates by kind; the startup privilege
    check and the runtime-privilege audit look tables up by name and would ignore partitions.
12. The typed errors, the metric's name, the alert's file, the retry of `ensure_journal_buckets()` and its execute
    grant are undecided.
13. Tasks 049-2 and 049-3 name no harness, report path or decision threshold.
14. Task 049-4 does not say how relations locked and round trips are counted.
15. Task 049-6's static check has no specification (language, extraction from Rust literals, CI wiring, exclusions for
    older migrations).
16. The new artifacts have no schemas (the move's cursor, settings, census call, `journal_retirements`, the fault
    injection inside a detach, the chart job, the quota function).
17. No task declares its prerequisites or its release.
18. The Verification Plan omits `integration_security_multitenancy`, the downs per migration, test names, and the owners
    of the barrier and reconciliation tests.

## Nits

19. Gap 1099 states no count; the 22 is right.
20. `runtime_privileges` is a module of `integration_platform_foundation`.
21. "The three checkers" is ambiguous (CI runs at least five).
22. `journal-buckets` breaks the `-v1` naming of capabilities; the readers' capability has no name.

Counts: Blocker 3, Major 15, Nit 4
