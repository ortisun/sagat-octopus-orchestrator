# Re-reviews of the design of the rest of feature 022 (commit `6d02632e`), and the second rulings (2026-11-25)

The three read-only reviewers re-read the revision (decisions D022-222 to D022-241, ADR-0135, ADR-0136, ADR-0137,
ADR-0146, the amendments). Totals: architecture 1 Blocker (N0), 11 Majors (N1 to N11), 7 minors, 3 nits; correctness
2 Blockers (B1, B2), 10 Majors (M1 to M10), 9 minors, 5 nits; security 0 Blockers, 4 Majors (A to D), 9 minors, 4 nits.
Closed by the revision, as all three agree: Security Blocker 1, Correctness Blocker 1 in its reducer part, Security
Majors 1 (in part), 2 and 4, Correctness Majors 3, 4, 5 (for one hit), 7 and 9. Every ruling below is delegate
(OD-2026-11-24-4), recorded as decisions D022-242 upward, written into the documents before any code, and every
document that still carries an old row is corrected (Architecture N1, Correctness M8, Security D: `plan.md`'s
governing table "Transactions, as built" at lines 903-924 is the one that wins; the superseded table at lines 98-117 is
marked superseded, its rows moved into the governing one).

## Lineage, dispatch and recovery (Architecture N0, N11; Correctness B1, M5; Security A, B, C)

The reviewers show that "a machine identity" is not a principal kind the platform has (`identity.rs:152`; the only
machine roles cannot start runs, `identity.rs:688-690`), so decision D022-241 cannot be built; that the lineage unique
index makes the terminal commit fail with a unique violation (23505) and retry forever; that a handler starting a fresh
run escapes the bound; and that the prescribed recovery credential, an Operator or Developer subject, can approve
publications and decide approvals, defeating four-eyes review (`identity.rs:650-675`, `versioning.rs:465`).

Rulings:
1. Decision D022-241 is reversed (option (a) of O1176): every reset and every rerun extends its source's lineage,
   whoever starts it. A dispatch suppressed by its lineage is not silent: it is written as a dispatch row in state
   `SUPPRESSED_LINEAGE` beside the lineage's first row (no unique conflict: the unique index covers live rows only,
   `WHERE state <> 'SUPPRESSED_LINEAGE'`, or an `ON CONFLICT DO NOTHING` that counts `skipped_lineage` and journals
   the outcome on the run), counted, shown in the dispatch listing and redrivable by an operator through the existing
   redrive route. Reason: an operator sees and controls every suppressed handler, nothing loops, and nothing depends on
   a principal kind the platform lacks.
2. The terminal commit's insert is `ON CONFLICT (tenant_id, namespace_id, lineage_run_id) DO NOTHING` over the live
   rows, placed last before the usage rows; ADR-0050's recorded deviation is extended to the lineage key: the insert is
   followed only by rows of the run's own commit and usage rows, so a wait on another run's uncommitted entry is a
   leaf wait. A fault test: two failing runs of one lineage commit concurrently, both reach their terminal status,
   exactly one live dispatch row.
3. Fresh starts by a handler: a namespace dispatch budget (a ceiling of dispatches a namespace may admit per window,
   with a per-workflow share) defers dispatches beyond it (never drops them) and raises an alert; it bounds a handler
   that starts fresh runs, triggers or Signal-with-Start. Runs a dispatched handler starts through its own nodes
   (`child`) are handler-tree runs.
4. Recovery credentials: a new built-in role, `RunRecovery` (`run:read`, `run:start`, `run:operate`; no publication,
   approval, version or secret permission), is what ADR-0136 requires for a credential a handler uses; a subject whose
   only role is `RunRecovery` is refused as a publication approver and as an approval decider. Task T022-011's catalog
   counts it. No run is ever granted its publisher's rights.
5. `lineage_run_id` is nullable, read as `coalesce(lineage_run_id, id)`, filled by a `BEFORE INSERT` trigger so older
   binaries and the chain insert keep working, and immutable (`ENABLE ALWAYS` guard); the same guards as `test_tree`
   apply to `handler_tree` and `handler_kind` (Security minor 5).

## The handler phase (Correctness B2, M1, M2, M3, minors 2, 5, 6, 7, 8; Architecture N3)

Rulings:
1. One journaled event `failure_handler.draining {cause}` with causes `CANCELLED`, `RUN_DEADLINE`, `TIMEOUT`,
   `REGION_FAILED`, `STORAGE_QUOTA`, `RUN_DATA_LIMIT`, `EXPRESSION_ERROR`; during the phase every stop source
   (`run.stop_requested`, the storage-quota stop, a region step failing without `onError`, an expression error in a
   region condition or switch) maps to it and never to the run's stop; the first cause wins and later ones are recorded
   in its detail; the projection column follows the journal (a domain case rebuilds the state from the journal after
   each commit).
2. Draining overrides pause as a stop does; the recovery sweep also selects draining runs.
3. The run deadline does not apply during the phase: only the handler's own timer bounds it (D1174), `timeoutMs` has
   a ceiling, and the overdue pass excludes `failure_handler_state IS NOT NULL` (Architecture N3: otherwise a
   `TIMED_OUT` handler is cancelled by the next pass). A deadline that fires before the phase opens is the run's stop
   and the phase then opens normally.
4. Qualification for the namespace handler: no workflow handler phase ended `SUCCEEDED` and no operator asked to
   cancel the run; a draining cause other than `CANCELLED` qualifies.
5. Region children (`child` nodes in a region) are handler-tree runs; the derived `onFailure~h` node cancels and awaits
   its child; T7 creates the handler timer and T8 cancels it and holds the region step's gates in its sorted set.
6. Domain and integration cases: a parallel region with one branch failing while the other is leased; a storage-quota
   stop in the phase; paused then timeout; `on: [TIMED_OUT]`; cancellation during `TIMEOUT` draining and timeout during
   `CANCELLED` draining, both orders.

## Finish paths and continue-as-new (Architecture N4, N6, N7; Correctness M4, minor 3)

Rulings:
1. Continue-as-new is refused, at publication and by `refuse_gated_successor`, for a definition that declares `cache`,
   `onFailure` or `earlyReturn`, and for a successor of a `test_tree` or `handler_tree` run (`FLOW_CONTROL_COMBINATION`
   and the chain successor guard extended).
2. The chain finalization's terminal write goes through `persist`'s terminal path for the namespace dispatch, so a
   failing chain head is dispatched like any run; ADR-0146's allow-list reason is rewritten (the namespace handler is
   not a declaration).
3. The fitness rule of ADR-0146 is defined by enclosing function (`enclosing_function`, as `run_finished_errors`
   does), allow-lists the persist family (`persist`, `persist_addressed`, `persist_scoped_addressed`,
   `outbox_task_ready*`, `outbox_run_finished`), body iteration announcements (with their reason) and the chain
   hand-off and finalization, restricts the `finished_at` clause to `UPDATE runs`, and has negative fixtures.
4. ADR-0146 adds a mixed-version test (an older binary replays a journal finished by the new path) and a held-lock test
   against concurrent feature 014 iteration transactions.

## The cache store (Correctness M6, M7, M9, M10, minor 1, minor 4, nits; Architecture N10; Security minors 1, 2, 3, 4, 7, 8, 9)

Rulings:
1. The entry is stamped with the database instant of the producing attempt's lease (`step.leased`), and expires from
   it; a value computed by an attempt leased before an invalidation is never stored as live.
2. Invalidation by key writes a per-key tombstone (`invalidated_at` by scope and key digest) that T3 and the hit check
   read; barrier test with T2 held open across T5.
3. `step.cache_missed` has the reset rule `REMATERIALIZE` and the reset copies `steps.cache_lookup` from the source step.
4. Hits staged in one round count cumulatively, in round order, against the retained-output limit; test with two hits.
5. One usage row holds both counts (entries and obligations), or a fixed order, so T3 and T18 cannot deadlock on them;
   a barrier held after T3's delete.
6. Invariant 18 is restated about hits (no hit on a dead entry), so a test of it can fail.
7. Entries record secret ids, not names; the binding check stores the producing worker's subject, takes the binding
   advisory lock shared at a stated slot, checks the worker's membership, and states the rule for scripts.
8. For a caller who may not disclose every location the key reads: `entry_id`, the expiry and `entriesDeleted` are
   null, listing instants are cut to the minute (as decision D022-060), and the span and metric attribute sets are
   stated (no key, no label, no source run).
9. Store obligations keep one column `UPDATE` grant guarded by an `ALWAYS` trigger, so `FOR UPDATE` works as
   `sagat_app`.
10. Namespace-scope cache names are declared resources of the namespace (an admin setting listing the workflows that
    may write the name); any workflow of the namespace may read; this closes the poisoning through fields a key does
    not read (Security minor 7). Version and workflow scopes need no declaration.
11. Invalidation inserts a scope row only for a scope a published version can derive; any other target is 404.
12. The lookup's secret-access, membership and binding locks are taken at slot 5 for an admission (the definition is
    known there) and listed in ADR-0056 decision 2(a) with the leaf argument for a later round (Architecture N5);
    a `slot_order.rs` case: a cacheable admission against a revocation.
13. `UPDATE (expires_at)` is listed as an exclusive taker in ADR-0135 decision 7; two writers replacing one dead entry
    use `ON CONFLICT DO NOTHING`.

## Orders and statements as built (Architecture N2, N5, N9)

Rulings:
1. The order is restated everywhere as "entity locks, then the rounds (lookup, gates), then `persist` (the counter row
   when the commit ends the run, the give-backs, the rest), then usage"; ADR-0056 decision 2's clauses on the policy
   row `FOR SHARE` and "`runs` update, then semaphore" are struck with a dated line; the no-cycle argument for the
   counter row after cache rows is written (the cache side is shared-only; no exclusive taker of a cache row takes the
   counter row); ADR-0094 is cited as the authority.
2. The handler configuration is read in the run-row statement (a left join in the statement that locks the run) with
   its statuses, revision and enabled flag; the budget line is corrected; the definition cache carries nothing mutable.

## Capability serving (Architecture N8, minor 1)

Ruling: a per-capability manifest of required artefacts (event types the codec decodes, `Evaluation` variants,
migrations, routes, the named acceptance test present and not ignored); the test fails when the serving constant is
true while any artefact is missing. Rollback order is stated: `platform_requirement_remove`, then the switch; a switch
gates new use only, never phases or obligations in flight.

## Draft executions (Security minor 6)

Ruling: `draft_executions_enabled` is read inside the admission transaction under the namespace policy's existing
lock; runs already admitted continue when it is turned off (documented); creating the policy row to set the flag keeps
the environment's approval default; `prod` refuses standalone activity and script steps as it refuses test runs, unless
the setting is enabled; a rerun of a standalone run is checked again.

## The rest

Accepted as the reviewers propose: the remaining architecture minors (the interim `COLLECTION_FINISH_ROUTING` refusal
kept as defence and said so; unrelated automation reruns extend the lineage, now stated as a consequence with the
suppressed-and-redrivable row of ruling 1; a budget line for the child `test_tree` check; a Complexity Tracking row for
the recorded deviation and a re-dated Constitution Check); the nits (the `definitions.rs` path is
`crates/infra-postgres/src/definitions.rs`; `plan.md:859` order; the old text of T022-215 marked superseded; D022-238's
citation `plan.rs:316`; `FAILURE_HANDLER_SELF_GATE` direct only, cycles bounded by timeouts, stated); the security nits
(`plan.md:958-960` wording; the `unauthorized` outcome is journaled only as `bypassed` to run readers who may not see
the producer; the key's classification in namespace scope is the producer's, stated; `SECURITY DEFINER` triggers
schema-qualify `public.` tables; T17's fence is `sagat_lock_namespace`, never the `namespaces` row).
