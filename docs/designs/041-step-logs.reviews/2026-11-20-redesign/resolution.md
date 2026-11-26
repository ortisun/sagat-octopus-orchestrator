# Resolution of the reviews of the 2026-11-19 revision of design 041

Reviewed commit 4c027adf. Each finding was checked against the code or the text before it was answered. "Fixed" means
the text changed in the commit that adds this file; "Stated" means the limit is now written where a reader meets it,
with its gap, and is not otherwise changed. The decisions are D041-095 and D041-096 of `specs/041-step-logs/research.md`.

## The substantive changes

1. **The policy read** (A-B1, C-M1, CD-1): CA takes only the fences; CB reads the values in a statement of its own,
   whose snapshot follows the locks. Fixed.
2. **The carry's size** (K-1, S-M2, A-M8, SY-5): 64 records and 16 KiB, bounded by the time under the run lock; T041-35
   adds workload (h), the largest carry, recorded. Fixed; the 64-record figure is measured, not gated.
3. **The waits under the run lock** (C-M3, K-2, SY-19): CA sets a 200 ms lock timeout and the last statement restores it;
   a longer wait fails the carry into the re-run; every holder of L2 is listed. Fixed. Cost: five statements, not four;
   the six equalities restated (55/1/23, 54/1/13, 53/1/11; 52/1/21, 51/1/11, 50/1/9).
4. **The roles** (S-M1, K-9): C0 ignores a carry from a runner or database-runner token or on a managed step. Fixed.
5. **The report paths** (A-M4, C-M4, CD-2, IR-5, S-N1): a table of every transaction that records an attempt, with a
   `CarryPlan` argument; refused outputs and content fallbacks write a mark-only carry. Fixed.
6. **The re-run** (A-M5, ST-2, SY-9, K-12, C-N9): defined as the report's method run again with the carry downgraded,
   never re-run; any raise of the step-log branch mapped; the lease-expiry residual stated (G041-021). Fixed and stated.
7. **The order guard** (C-M2, C-N1): `last_stored_xid` only raised, by a statement that inserted a row; a trigger refuses
   a lower value; `in_order` is an input of `plan_batch`. Fixed.
8. **FINAL_RECORDED** (C-M6, K-7, ST-4): a later batch gets `LEASE`, `FINAL_RECORDED` only when it read the lease before
   the report committed; the T041-8 case moved to T041-36 with a hold after T2. Fixed.
9. **Short steps** (ST-3, SY-1, SY-2, C-N7): defined in Terms; the carry holds the newest records that fit, the rest go
   as batches within 2 seconds; the SDK stops resending its batch in flight at the deadline. Fixed.
10. **ADR-0095** (A-M1, A-M2, A-M9, ST-1, SY-17, SY-18): supersedes ADR-0084 decisions 2, 6, 11 and decision 5's session
    clause for a carry, and ADR-0027 Amendment item 1's clause; six equalities in "Enforced by"; one port; acceptance
    asked as O041-038, which T041-3 and T041-36 wait for. Fixed.
11. **Owner gates** (A-M9, SY9-1, SY9-2, A-N8): OD-2026-11-19-1 on T041-4, -5, -6, -8, -9, -14 to -20, -32; T041-6 carries
    OD-2026-11-16-11; T041-34 withdrawn. Fixed.
12. **CI selection** (A-M6): T041-32 gains `integration_step_logs` for the worker and execution paths. Fixed.
13. **The spike** (A-M7, IR-1 to IR-3, CD-6, C-M5, S-N6, IR-12, IR-13, SY9-3 to SY9-8, C-N8): the report hook named on
    `PgRunStore`, latched from `effects.reported`, behind an opt-in `sagat-tests` feature; the round's order; a `{}`
    baseline; a baseline-drift bound; the offered rate per replica and a ticket-refusal bound; fewer than three valid
    rounds is not decided; the defaults' formula; the unmeasured costs listed. Fixed.
14. **T041-36** (IR-4 to IR-8, CD-3 to CD-5, A-M3, SY9-9 to SY9-11, SY9-16): carrying entry points beside the present
    methods; `WorkerSettings.step_logs_enabled`; the port as a supertrait with a crate accessor; `StoreError::StepLogCarry`;
    root and scoped, completion and failure tests; barrier mechanisms named; metrics; files. Fixed.
15. **Scoped stop rule** (CD-8): stated as D041-095. Stated.
16. **Capacity** (K-6, ST-14, SY-16): both sizes' S, the failed ticket gate, the defaults as defaults. Fixed.
17. **Lost carries and rollout** (K-8, ST-11, A-N7, K-13, SY-30): G041-014 restated; C2 keeps a valid final mark; the
    rollout keeps the switch off until every replica serves carries; G041-020. Fixed and stated.
18. **Structure and style** (ST-5 to ST-10, ST-12 to ST-18, SY-3, SY-4, SY-6 to SY-8, SY-10 to SY-16, SY9-12 to SY9-18
    and the Nits): rewritten where they named a real inconsistency.

## Stated, not changed

- K-3 (round trips on a remote database): five statements are kept so that the budget counts rows; G041-022 states it.
- K-4 (fan-out not in the spike) and K-5 (purge beside the workloads): the report hook reaches the root path only and
  the purge is not built; both are listed as not measured by T041-35 and measured by T041-9 (G041-019).
- A-M3 (cost on FailStep, scoped and fallback paths): the six equalities are on root completions as the budget test is;
  T041-36's tests cover the other paths for behaviour, not for the statement count.

## Answers to the re-reads of 78139ffb (`rereads-78139ffb.md`)

Decision D041-097. Fixed unless said otherwise.

1. **Leaked-secret paths** (correctness New 1, A-M4, codebase N1): every report whose output or failure code holds a
   declared secret, root or scoped (`reject_secret_output`, the root `fail` with `leaked`, `apply_path_report` with
   `leaked`, `refuse_scoped_output`), writes a mark-only carry; the table of section 2a and T041-36's tests name each.
2. **A mark-only carry that fails** (correctness New 3, design critic N-1, readiness N-1): a carry never waits for a
   step-log lock. CA tries the fences and a new per-attempt stream lock (L1b, also taken by the ingest's T3); CC inserts
   a new stream, which with the stream lock held never waits; CC′ locks an existing one with `SKIP LOCKED`. A lock held
   makes the carry `contended` and the report commits in its first transaction. The 200 ms lock timeout and its restore
   statement are gone. A failed statement in a mark-only carry runs the transaction once more with none: `Full` →
   `MarkOnly` → `None`, at most four transactions with the content fallback.
3. **Telling the carry's failure apart** (codebase N2): `StoreError::StepLogRefused` and `StoreError::StepLogCarry`
   become internal `AppError` variants that only the re-run wrapper consumes; an ordinary `Unavailable` is never re-run
   (tested).
4. **The re-run's refusals** (A-M5): listed in FR-041-005, G041-021 (restated) and ADR-0095 decision 4's exception.
5. **Cost per call site** (A-M3): T041-36 measures the cost with and without a carry at every other call site.
6. **Barriers and outcomes** (correctness New 2, IR-6): a test wrapper of `StepLogCarryTx` with pause points before CA
   and before CC; no lock timeout remains in the carry; each test has one expected outcome.
7. **T041-35's dependency** (A-NM1, SY9 N-2): the report hook plans its two cases inline, as T041-31's driver did.
8. **Expiry and contracts §4** (ST-7, structural N-1): "last recorded batch or carry" in data-model.md §2 and the
   contract's `expiresAt`; contracts §4 separates a lost carry (a refused report) from a mark-only one.
9. **One pool** (SY9 N-1): `SAGAT_DB_MAX_CONNECTIONS` (the benchmark's `API_POOL`), with the two keys T041-7 reads.
10. **Wording** (SY-2, SY-11, SY-12, style N-1, N-2, ST-10, SY9-14, SY9-17, SY9-18, the Nits): fixed.

Stated, with the reason they may wait for T041-9 (Open Questions): K-3 (a remote database's round trips), K-4 (the
scoped path in the gate) and K-5 (the purge beside the workloads). The cost table now gives five statements for a short
step and six for a long one; the six equalities are 55/1/23, 54/1/13, 52/1/11 and 52/1/21, 51/1/11, 49/1/9.

## Answers to the re-reads of fe99a99f

- Architecture: no Blocker or Major; Nits fixed in 3b9cee4f (G041-014 lists a `contended` carry; T041-35 voids a round
  where more than 0.1% of carries were not written).
- Design critic: no Blocker or Major; K-3, K-4 and K-5 accepted as stated gaps before the spike; Nits fixed in d710de3b
  (the fan-out fallback, the round-trip share in G041-022, at most four transactions).
- Correctness: one new Major, CC's speculative insertion waiting for an updater of an existing stream row (the purge's
  mark and delete, the delete route). Fixed by D041-098 (`INSERT … SELECT … WHERE NOT EXISTS … ON CONFLICT DO NOTHING`),
  with T041-36 tests that hold each of those updaters after its update.
- Correctness re-read of 7e7dcc3b: the Major is closed (the `NOT EXISTS` cannot miss a row: worker streams are created
  only by ingests holding the stream lock, and an ingest that committed before CA is in CC's snapshot); the six
  equalities unchanged; at most four transactions confirmed. Counts: Blocker 0, Major 0, Nit 0.

No Blocker and no Major is open after the targeted re-reads; the design stays `Draft` until a complete approval run
(all five `egm-review` stages and the three repository reviewers) on the revised text, after T041-35.

## Answers to the targeted re-reads after OD-2026-11-20-1 (ceb0ad42, 7d5c2737, d8d1400a)

- Correctness on ceb0ad42: two Majors (the function's plan not equal to `plan_batch`; the budget blind inside the
  function), fixed in 7d5c2737 (exact T5/T6, the mark-only reason, drops by reason; a nested-statement measure);
  re-read of 7d5c2737: closed, no new Blocker or Major; its three Nits fixed in 0da45229.
- Architecture on ceb0ad42: A-M10 to A-M12, fixed in 7d5c2737; re-read of d8d1400a: closed. Its new A-M13 (gate (2b)
  resting on the coordinator's reading) is closed by the owner's confirmation before any round, "Confirmo"
  (OD-2026-11-20-5). Its Nits fixed: the test asserts the fixture's `pg_stat_statements.track=all` instead of changing a
  server setting; section 3's grants name the two new functions; research's O041-040 lists the README exception.
- Design critic on ceb0ad42: K-15 (gate (2b) across sizes), fixed per size (D041-101); re-read of d8d1400a: closed, no
  new Blocker or Major; its Nits: the owner's confirmation (OD-2026-11-20-5) and the tracking setting (fixed as above).

No Blocker and no Major is open.
