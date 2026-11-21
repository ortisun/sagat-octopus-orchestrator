# EGM stage 8 of the second approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fefcfafbb40d153ebd3e462c7ba2d94a7c0d9286
Reviewer: egm-review (stage 8)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: a fresh `egm:egm-zoom-out` agent, dispatched after stages 5 to 7 and given their report `egm-stages-5-7-approval-2.md`, the full document, the standard and the reading list. Recorded condensed; no severity was lowered.

## Merge

- Major: K2-1, ingest step 2 refuses batches of compensation, reconciliation and cancel acknowledgements in a stopping run (`crates/application/src/worker.rs:2828-2831`); drop the `stop_reason` condition or copy the report fence; ADR-0084 decision 5 then needs the owner.
- Major: K2-2, a per-tenant step-log usage row separate from `tenant_retained_usage` is not evaluated; it would remove the tenant-row queue; it changes the Amendment the owner accepted as written, so raise it as an owner question.
- Major: K2-3, no capacity estimate; T041-31 records the ingest's service time and derives the ticket default; let a final batch wait for a ticket within its 2 s instead of `RETRY_LATER`.
- Major: C2-2, research.md's prose and D041-006 and D041-034 contradict the design; one supersession sentence each.
- Major: S2-1, Complexity Tracking omits the largest moving parts.
- Major: T2-2, FR-041-023's "commits after it is stored" contradicts `REMOVED`.
- Major: T2-4, two first failures for the conformance scenarios.
- Major (raised from a critic Nit): the run ceiling never resets (`run_log_usage` is never reduced), so a long-lived loop stops logging for good; decrement on purge, or a gap and an owner question.

## Merge partly

- Major: Z2-1 (scoped): move the statement-level steps and pins to data-model.md or the tasks; keep the control flow, the lock order and the bound once.
- Major: Z2-3: T041-9 cannot run the 8-replica row; reuse T041-31's driver switched to the application's ingest in `scalability-ha`, or restate SC-041-007.
- Major: K2-4: give the list route the tail's `progress` rule after a bounded scan; add a risk row.
- Nit (downgraded): Z2-2, the unlocked run ceiling is not simpler at 10,000 streams and overshoots 19% at R = 8; O041-020 is likely moot.
- Nit (downgraded): C2-1, T041-1 needs only a cross-reference to the approval rule.
- Nits: T2-1, T2-5, T2-6, T2-8.

## Merged Nits

- "Owner decisions needed" repeats the record; T041-32 order and T041-2 tick; chunk variant only on failure; runner tokens `PERMISSION_DENIED` against `AUTHORITY`; a full secret set stops capture; the down migration on `classified_namespaces`; stale ADR text; `/logs/stream` against `/logs/streams`.

## Reject

- T2-3 as a Major (five namespace resources plus one tenant resource is consistent); a Nit remains on FR-041-020's wording.

## Missed

- Major: the classified-namespace record (table, trigger, backfill, tests) reinvents what the insert-only `workflow_versions` and a partial index on `position('"x-sagat-classification"'::bytea in document) > 0` give.
- Major: the spike's fan-out criterion cannot tell ticket shedding from the run-row wait, and O041-020 would not change it; count the refusals apart and judge the wait on `run_log_usage`.
- Nit: `SAGAT_STEP_LOGS_READ_ENABLED` is not needed (YAGNI).

Counts: Blocker 0, Major 13, Nit 17
