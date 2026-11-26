# Correctness review of design 040, round 1 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: e5bda0415ca8eddee6135047ce6d4d4366aae3dc
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 16, Nit 9

1. Major — The design does not say where the cursor stops after a partial batch, and the tenant-sink query lets withheld records through
   - Where: section 4 (SQL, lines 301-307); section 5 steps 2-3 (325, 333); Success criterion 040-003.
   - What is wrong: step 2 reads 500 rows and encodes "up to 1 MiB". Step 3 "advances the cursor" without saying to which record. If a builder advances it to the last row read instead of the last row placed in the body, every row cut by the byte bound is skipped for good.
     - The tenant-sink SQL has no `NOT (action = ANY(WITHHELD_FROM_TENANT))`. `GET /audit` has that filter (`crates/infra-postgres/src/operations.rs:44`). A cross-tenant denial is written in the prober's tenant (`crates/application/src/api_admission.rs:464-472`). The design never says whether the cursor moves past rows filtered in memory.
     - It does not say whether 1 MiB counts gzip bytes or the raw body.
     - It does not cover one event larger than 1 MiB: `resource_id` is unbounded.
   - Why the tests miss it: the byte bound only engages when the average event exceeds 1,048,576 / 500 ≈ 2,097 bytes. The 10,000 fixture records of Success criterion 040-003 are about 1 KB each, so the bound is never reached. In production a `pii` ceiling with a 2,048-character reason reaches it.
   - Correction:
     - State "the cursor moves to the `(tx_id, id)` of the last record placed in the body, or of the last record read when every record read was withheld".
     - Add the withheld filter to the tenant-sink SQL.
     - Say the bound counts uncompressed bytes.
     - Cap each string attribute.
     - Add a test with events of about 3 KB that asserts no record id is missing at the batch edges.
   - Confidence: confirmed in code (the withheld write and filter). The security review's finding 1 overlaps.

2. Major — Success criterion 040-003 cannot fail for the defect it targets
   - Where: Success criterion 040-003; Task 040-13.
   - What is wrong: "16 concurrent transactions that commit out of order" leaves the interleaving to chance. The xmin rule matters in one case only: a transaction with a low xid is still open while the pass runs, and a later xid has already committed. Free-running writers that commit in milliseconds seldom produce that while a pass runs. A wrong bound such as `pg_snapshot_xmax`, `pg_current_xact_id()`, or `created_at` ordering would then pass most runs. Rule 18 forbids sleeps for races, and the kill point "between a delivery and the advance" has no deterministic hook.
   - Correction:
     - Hold transaction T1 open behind a barrier after its audit insert.
     - Commit T2, which has a higher xid.
     - Run a full pass and assert the cursor is still below T1's xid and T2 was not delivered.
     - Release T1 and assert both records arrive in `(tx_id, id)` order.
     - Put a fault hook after the 2xx and before the commit.
     - Hand-mutate `pg_snapshot_xmin` to `pg_snapshot_xmax` and show the test failing.
   - Confidence: confirmed in the text.

3. Major — A logical restore or a logical-replication upgrade silently stops every stream
   - Where: section 4 ("`xid8` ... does not wrap around"); Requirement 040-009; Risks.
   - What is wrong: `xid8` is monotonic only within one cluster. `pg_dump`/`pg_restore` into a fresh cluster, or a blue/green move through logical replication, copies `tx_id` and `cursor_tx` as data, but new transactions start from the new cluster's much lower counter. Every new record then sorts below every cursor and is never selected. No request fails, so the sink never becomes `STALLED`, and the lag histogram gets no samples.
   - Correction:
     - At each pass, if `pg_current_xact_id() < cursor_tx`, set a `CURSOR_INVALID` state, audit it and alert.
     - Repair from the last delivered record's `created_at` with a stated overlap.
     - Add a step to the restore runbook.
     - Add a test that restores into a fresh container.
   - Confidence: plausible (PostgreSQL semantics). The security review's finding 10 overlaps.

4. Major — "The lag metric and alert show it" is false for the risk it is meant to cover
   - Where: Risks ("A long transaction stalls every sink"); section 6 metrics; Non-functional requirement 040-003.
   - What is wrong:
     - `audit_export_lag_seconds` is a histogram of delivered events. While `xmin` is held back, nothing is delivered and nothing is observed. The only alert is on `STALLED`, which counts HTTP failures, and a held `xmin` causes none.
     - The snapshot's `xmin` covers every xid-holding backend in the cluster, including other databases and sessions outside the platform's pools. Only those pools carry the 60-second `idle_in_transaction_session_timeout` (`crates/infra-postgres/src/pool.rs:63`).
     - "Commit to acknowledgement" cannot be measured: there is no commit timestamp, and `created_at` comes from the application clock before the commit (`api_admission.rs:470`).
   - Correction:
     - Add a gauge of the age of the oldest undelivered record above each cursor (the maximum over sinks, labelled by kind) and alert on it.
     - Add a gauge of `now() - xact_start` of the oldest backend holding `backend_xid`.
     - Define lag as acknowledgement minus `created_at`, and state the clock skew.
   - Confidence: confirmed in code.

5. Major — `audit.sink_delivered` "with a count" cannot be collapsed, and the self-audit feeds itself
   - Where: Requirement 040-010; section 5 steps 2-3; section 6.
   - What is wrong:
     - The table is append-only. Collapse is insert-if-absent on (tenant, actor, action, resource, `reason IS NOT DISTINCT FROM`) (`crates/infra-postgres/src/identity.rs:265-277`), so a count can never be updated.
     - If the count is in the reason, every batch writes a new row. That row is in the tenant trail, so it is the next batch. An idle sink would never go quiet.
     - If the count is not in the reason, the one row per minute carries the first batch's count only, and the counter disagrees with the deliveries.
     - Even with a correct one-row-per-minute collapse, the rows stream to every sink. An idle tenant with 4 sinks emits 4 rows a minute, 5,760 rows a day, in a trail kept forever (Decision 040-006). Each row goes to 4 sinks.
     - `secret.resolved` collapses forever (`crates/domain/src/audit.rs:271`; `repeat_window` returns `None`). "Audited as `secret.resolved`" per batch therefore records either only the first read ever, or one row per batch, depending on an unstated reason.
   - Correction:
     - Put no count in the record. Key it by a minute bucket with a 60-second window, and carry counts in metrics and spans.
     - Name a dedicated, collapsed action for the engine's read of the two secrets.
     - State the steady heartbeat and accept it, or exclude a sink's own `audit.sink_*` rows from that sink.
   - Confidence: confirmed in code.

6. Major — The pass's commit is fenced on the shard but not on the sink, so it can overwrite API changes and move the cursor back
   - Where: section 5 steps 1 and 3; section 6 (pause, resume, PATCH, DELETE).
   - What is wrong:
     - ADR-0020 decision 3 says the shard fence "is not what makes the result correct". The design uses it as the only guard.
     - No sink state machine is given. Case 1: an admin `:pause` commits while a batch gets a 503. The failure branch writes `failures + 1` and, at the fifth failure, `STALLED`, so the pause is lost. A later 2xx clears `STALLED`, so a paused or `PAUSED_GRANT_REVOKED` sink comes back to life.
     - Case 2: two tasks of one owner handle one sink. ADR-0020 decision 6 runs candidates eight at a time, and `:resume` or `:test` may also trigger a pass. A slow commit then sets the cursor back and counts failures wrongly.
     - Case 3: an owner that lost and regained the shard passes a fence that is re-read at commit.
     - A PATCH of the URL during a batch moves the new destination's cursor over records it never received. That may be intended, but the design does not say.
     - On DELETE, the commit updates nothing, yet it still writes `audit.sink_delivered`.
   - Correction:
     - Write the state table: ACTIVE, STALLED, PAUSED, PAUSED_GRANT_REVOKED, deleted, with who may move each.
     - Claim one sink per pass with `FOR UPDATE SKIP LOCKED` or a claim column.
     - Commit with `UPDATE ... WHERE id = $id AND revision = $r AND (cursor_tx, cursor_id) = ($c0) AND state IN ('ACTIVE', 'STALLED')`, using the fence generation read at step 1.
     - When no row is updated, discard the batch and write no audit.
     - Add tests for pause, revocation and delete against an in-flight batch, in both orders, held by barriers.
   - Confidence: confirmed in the text and ADR-0020.

7. Major — A poison batch blocks a sink for ever, and Principle VII's dead-letter path is refused without justification
   - Where: Requirement 040-009 ("never dead-lettered"); Decision 040-006; Constitution Check VII.
   - What is wrong:
     - Principle VII requires "a dead-letter path" for every integration. Complexity Tracking justifies only replacing the outbox, not dropping the dead-letter path.
     - A collector that answers 400 to one event, or 413 because its limit is below 1 MiB, rejects the same batch on every retry. The egress policy or the secret can also stop working for good. All of these are treated as retryable.
     - The only way out is to delete and re-create the sink, which starts at now and skips the whole backlog. That breaks the "never skips" promise more widely than a dead letter would.
   - Correction:
     - Classify 4xx other than 408 and 429 as permanent.
     - Halve the batch on 413 or 400 until one event is isolated.
     - Give that event an audited, admin-visible record (`audit.sink_record_refused` with the record id, cursor moved past it), or add an explicit `:skip` operation.
     - Record the deviation from Principle VII in Complexity Tracking if any part of this is refused.
   - Confidence: confirmed in the text.

8. Major — Stored actions are text and nothing maps text back to the enum, so a mixed fleet can stall the stream
   - Where: section 2 ("turns an `AuditRecord`"); section 7 ("a mixed fleet streams correctly"); Success criterion 040-001 ("every action added later").
   - What is wrong:
     - The read returns `AuditEntry` with `action: String` (`operations.rs:835-844`). `AuditAction` has no `FromStr` and no list of all variants. The exhaustive `match` covers the enum, not the parse from storage.
     - Scenario: a newer binary writes a new action, and a rollback or slow rollout leaves an older engine on the shard. The older engine cannot map the row. It either errors on every pass (a silent stall that is never `STALLED`) or skips the row (a skip). `GET /audit/ocsf` returns 500 for the whole page.
     - Without an `ALL` list, the "every action" validation test is a hand-kept list that a new mapped action can bypass.
   - Correction:
     - Add `AuditAction::parse` and an `ALL` list generated by one macro with the enum, with a round-trip test.
     - Define the projection of an unknown string: API Activity 99, severity Unknown, `unmapped.sagat.action` raw.
     - Test both paths.
   - Confidence: confirmed in code.

9. Major — Detection Finding is filled with base-event semantics
   - Where: appendix rows for `script.violation`, `egress.denied` and `access.denied.cross_tenant` ("Medium, Failure"); section 2 table; Requirement 040-003.
   - What is wrong:
     - Finding classes override `status_id` with the finding lifecycle (1 New, 2 In Progress, 3 Suppressed, 4 Resolved, 99 Other), set by the consumer. "Failure" is 2, which on a finding reads as "In Progress". Schema validation still accepts it.
     - The control's outcome belongs in `security_control`'s `action_id` (2 Denied) and `disposition_id` (2 Blocked), which the design omits.
     - `finding_info.analytic` requires `type_id`; the design gives only `name`.
     - `actor` is not a top-level attribute of Detection Finding unless the `host` profile is declared, and the design rejects that profile. It belongs in `evidences[].actor`.
   - Correction:
     - Set `status_id` 1 New, or leave it out.
     - Add `action_id` 2 and `disposition_id` 2.
     - Set `analytic.type_id` (1 Rule).
     - Put the actor in `evidences[0].actor`.
     - Read and confirm these against the vendored 1.10.0 files, for example `crates/telemetry/ocsf/1.10.0/events/findings/detection_finding.json`, once Task 040-2 exists.
   - Confidence: plausible, from OCSF 1.x. Version 1.10.0 is not in the repository to check.

10. Major — The validation tools named cannot validate events
    - Where: Success criterion 040-001 and 040-002; Tasks 040-2, 040-5 and 040-6.
    - What is wrong:
      - `ocsf-validator` (the OCSF project's Python package) checks the integrity of a schema repository's definitions, not event instances. Success criterion 040-002 cannot be met with it.
      - The vendored files listed (classes, objects, dictionary, profiles) are source definitions with `extends`, profiles and `$include`. Validating an event against them means re-implementing OCSF's compiler, and the design does not plan that.
    - Correction:
      - Vendor the JSON Schema that the pinned OCSF server exports, per class with the declared profiles, with SHA-256 sums, and validate with a JSON Schema crate.
      - Check the `at_least_one` constraints in code.
      - Or run the OCSF server image pinned by digest in compose and call its validate endpoint from the contract job.
      - Check first: `pip download ocsf-validator` and read its README.
    - Confidence: plausible.

11. Major — Non-functional requirement 040-001 is false and its budget test cannot fail
    - Where: Non-functional requirement 040-001; Task 040-14; Complexity Tracking.
    - What is wrong:
      - `audit_records` today maintains three index entries per row: the primary key, `audit_records_tenant_created_idx` and `audit_records_denial_dedupe_idx`. Every new row satisfies `tx_id IS NOT NULL`, so the two new partial indexes make it five per row, with more WAL, on the most-written table. This holds whether or not a sink exists.
      - A test that counts statements is unchanged by a column default by construction, so it can never be seen failing.
    - Correction:
      - Restate the requirement as "no statement, row, lock or trigger; two index entries per row".
      - Budget WAL bytes and insert latency before and after (difference of `pg_current_wal_lsn()` over N inserts).
      - Or build the operator index only when an operator sink is configured.
    - Confidence: confirmed in code.

12. Major — The runbook's pre-built index is impossible, and the migration blocks the trail
    - Where: section 4 (lines 294-298); Task 040-9.
    - What is wrong:
      - The indexes are on `tx_id`, a column that does not exist before the migration. "Build both `CONCURRENTLY` before the upgrade" cannot be done, unlike the purge-index precedent, which indexed existing columns.
      - In one migration transaction, `ALTER TABLE` holds `ACCESS EXCLUSIVE`, not `SHARE`, through two full-table scans. That blocks every audited transaction and `GET /audit`.
      - `IF NOT EXISTS` also accepts an INVALID index left by a failed concurrent build. The export would then scan the whole table each pass.
    - Correction:
      - Migration A adds the column and the default (instant).
      - Migration B runs outside a transaction (sqlx `-- no-transaction`) with `CREATE INDEX CONCURRENTLY` and checks `indisvalid`.
      - Fix the runbook to match.
    - Confidence: confirmed in the text and migrations.

13. Major — Replay "by recreating the sink with a start time" has no API and no defined mapping
    - Where: section 6 (runbook bullet); section 4 (start cursor).
    - What is wrong:
      - Create takes no start time.
      - `created_at` order is not `tx_id` order, so a time does not map to a `(tx_id, id)` without a stated rule.
      - Re-creating starts at now, so the documented recovery skips the backlog it is meant to replay.
      - Replaying history to a new endpoint is also a disclosure act with no permission named.
    - Correction:
      - Add `startFrom: {createdAt}` on create, mapped to `(min tx_id of records with created_at >= t) - 1`. This sends some records earlier than t, which at-least-once allows.
      - Name the permission it needs and audit it.
      - Or remove the runbook claim.
    - Confidence: confirmed in the text.

14. Major — The operator sink's lifecycle is undefined
    - Where: section 5 ("a row with `tenant_id` null created from configuration at start"); Requirement 040-011.
    - What is wrong:
      - Several engines start at once and nothing makes the row unique. Two operator rows mean two cursors and a doubled stream.
      - Nothing says whether a restart keeps the cursor or resets it to the current `xmin`. A reset drops what was written while the engine was down.
      - Nothing says what a change of URL or a removed configuration does.
      - Its `audit.sink_*` and secret records cannot be written: `audit_records.tenant_id` is `NOT NULL REFERENCES tenants` (`20261008000000_identity.up.sql:57`).
      - Its ceiling is undecided.
    - Correction:
      - Use a one-row table keyed by a constant, holding the cursor only, with insert-if-absent that never resets.
      - Define behaviour on configuration change.
      - Send its evidence to logs and metrics.
      - Decide its ceiling.
    - Confidence: confirmed in code. The security review's findings 6 and 7 overlap.

15. Major — The batch id is undefined, and a deterministic one loses records at receivers
    - Where: section 5 step 2 ("the batch id as the event id"); Decision 040-008.
    - What is wrong: suppose the batch id is derived from the start cursor so that retries are "idempotent". Attempt 1 from C0 under `xmin1` holds n < 500 records and crashes before the commit. Attempt 2 from C0 under a higher `xmin2` holds a superset, under the same `x-octopus-event-id`. Webhook-style receivers dedupe on that header, so they drop attempt 2 and lose the new records.
    - Correction:
      - Make the batch and delivery ids unique per attempt (UUIDv7).
      - State in the contract that receivers dedupe by `metadata.uid` only.
      - Test that a retry carries a new event id.
    - Confidence: plausible.

16. Major — The rollout gate is not the mechanism ADR-0082 provides
    - Where: section 7; Task 040-16.
    - What is wrong:
      - ADR-0082 gates by an operator activation that records a requirement, after which an engine whose configuration does not run the pass is refused and closes its whole gate (decision 7).
      - "Gates `POST /audit/sinks` until every engine lists it" is a request-time check that does not exist.
      - Without an activation, a sink whose shard is held by an engine that has the pass turned off by configuration is never served. It shows no failure and no `STALLED`.
    - Correction:
      - Decide one of two options. Either `audit-export-v1` is activated through ADR-0082 (and every engine must then run the pass), or the shard keeper gives sink work only to engines that list the capability.
      - Add a test for a sink on a shard held by an engine without the capability.
    - Confidence: confirmed in ADR-0082.

17. Nit — The `trace` profile is declared on every event, but the trace id is optional
    - Where: Requirement 040-003; Decision 040-003 ("declaring one makes its attributes required").
    - What is wrong: OCSF keeps each attribute's own requirement level when a profile is declared, so the stated reasoning is wrong. If it were right, records without a trace id (for example `AuditRecord::provisioning`) would fail validation.
    - Correction:
      - Declare `trace` only when a trace id is present.
      - Make the per-action fixtures of Success criterion 040-001 cover: no trace, no reason, both ceilings, and a `system:` actor.

18. Nit — Gaps in the appendix legend
    - Where: appendix.
    - What is wrong:
      - API "5 List" is listed but unused, and is not an API Activity activity in OCSF 1.x as far as I know (plausible).
      - EM 99 is used for `worker.denied` but missing from the legend.
      - The seven new `audit.sink_*` actions have no activity ids (pause and resume: 12/13 or 9/8?), so the total becomes 134.
      - `PAUSED_GRANT_REVOKED` names no action.
    - Counting checked: the 34 rows hold exactly the 127 values of `AuditAction::as_str` (`crates/domain/src/audit.rs:328-454`), each once, none extra.

19. Nit — The status rule is a constant per action
    - Where: Requirement 040-001 ("a status rule") versus section 1 (`OcsfStatusRule { Success, Failure }`).
    - What is wrong: `run.finished`, `approval.decided`, `run.updated` ("applied or rejected receipt") and `script.built` carry outcomes that can be failures, yet always map to Success.
    - Correction: state that `status_id` reports the act of recording, not the outcome, or derive it from a structured field.

20. Nit — Complexity Tracking overstates the cost of an outbox
    - Where: The Problem; Complexity Tracking ("a change at every call site").
    - What is wrong: there are 6 `INSERT INTO audit_records` statements in 4 modules (`identity.rs` ×3, `blobs.rs`, `leaf_certification.rs`, `scoped_leaf_finish.rs`), so an outbox would change 6 sites, not 300.
    - Correction: keep the argument about the second write and drop the call-site argument.

21. Nit — The compile-fail test and the hand mutation cannot fail
    - Where: Task 040-3; slice 1 verification.
    - What is wrong:
      - A compile-fail test of a toy enum proves that the compiler enforces exhaustiveness, not that this module has no wildcard. The lint already holds that.
      - Re-mapping an action to another class makes the encoder fill that class's required objects, so the mutation produces a valid event and survives.
    - Correction: mutate the encoder so it omits one required object, for example `finding_info.title`, and show the validation test failing.

22. Nit — "At most one batch resent per crash" undercounts
    - Where: Risks.
    - What is wrong: a lapsed lease sends the same batch twice. A cursor commit that fails after a 2xx (database unavailable) resends on every retry. The count is per sink.
    - Correction: say "at least one batch per sink in flight, repeated for each failed commit".

23. Nit — The start cursor delivers more than "a few" earlier records
    - Where: section 4 (lines 311-313).
    - What is wrong: with `xmin` held by a 60-second transaction in the platform's pools, or for longer by a session outside them, a new sink receives everything committed since the oldest running xid.
    - Correction: state the bound.

24. Nit — The pass cadence and concurrency are unstated
    - Where: section 5; Non-functional requirement 040-003.
    - What is wrong:
      - "Due" means only `next_attempt_at`, and there is no wake-up signal.
      - If sinks are handled one after another, endpoints that time out at 10 s hold back healthy sinks on the same shard.
      - The idle cost is one query per sink per tick.
    - Correction: state the tick, the concurrency per shard and a per-sink claim.

25. Nit — An asynchronous region failover is not covered
    - Where: Delivery Constraints of the constitution; not addressed in the design.
    - What is wrong: a SIEM may hold events whose records the promoted standby lost. The design neither reports this nor sends tombstones.
    - Correction: list it as a gap, or include the lost window in the failover report.

Files read: `docs/designs/040-ocsf-event-standard.md`, `docs/adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md`, `docs/adr/ADR-0020-the-scheduler-is-sharded-by-run-and-owned-by-lease-with-a-fence.md`, `docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md`, `specs/040-ocsf-event-standard/research.md`, `crates/domain/src/audit.rs`, `crates/infra-postgres/src/identity.rs`, `crates/infra-postgres/src/operations.rs`, `crates/application/src/signing.rs`, `crates/infra-postgres/migrations/20261008000000_identity.up.sql`.
