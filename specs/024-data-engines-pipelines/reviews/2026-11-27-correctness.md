# Correctness review of the revised feature 024 set (2026-11-27)

Doc: specs/024-data-engines-pipelines/
Reviewed-commit: b51b3c2ce5a460cdac9801f81470aee1f1bc3ecb
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 1, Major 12, Nit 12

1. Blocker — A `dataQuery` over a step source has no classification, so a database result can be disclosed below its protection
   Where: specs/024-data-engines-pipelines/spec.md:545-551 (FR-024-029), spec.md:433-440 (FR-024-016); specs/024-data-engines-pipelines/contracts/workflow-data.schema.json:30-83 and 310-317; specs/024-data-engines-pipelines/contracts/data-runtime.md:192-197; specs/024-data-engines-pipelines/plan.md:298
   What: `dataQuery.inputs` accepts the `step` form, which reads a feature 017 database result or a Parquet step-output blob. The output's level is "the highest level among the pinned input columns", and a step source has no declared columns. Concrete case: a developer with `run:start` and `data:asset:use` runs a workflow with `extract` (a MySQL `query` step) and then `dataQuery` with `inputs: [{alias: "x", step: "extract"}]` and `SELECT * FROM x`. The highest of an empty set is undefined, and the natural implementation reads it as `public`. The feature 017 result, which only `database:result:read` (admin) may read, then becomes a feature 016 carrier at the lowest level. Confidence: confirmed in the text.
   Why: This breaks ADR-0035 decision 10 and ADR-0036 decision 6, which the spec says it leaves untouched. SC-024-009 tests only `pii` asset columns, so no planned test exercises a step source.
   Correction: Either refuse `step` inputs in `dataQuery` at publication (`DATA_NODE_SCOPE`), or rule that a step-source input counts as `pii` (undeclared means `pii`, as FR-024-016 already says for columns). Add the case to SC-024-009 and Task 024-041.

2. Major — `CompleteStepRequest.data_result = 13` collides with feature 041's field 13
   Where: specs/024-data-engines-pipelines/contracts/worker-data.md:129-132; specs/024-data-engines-pipelines/research.md:27 and 369 (Decision 024-014); specs/001-platform-foundation/contracts/worker.proto:160-163 (and 171-172 for `FailStepRequest`)
   What: The evidence row says `CompleteStepRequest` uses fields 1-12 at `e5bda041`. The canonical file already has `StepLogCarry step_logs = 13` (protocol 1.7, feature 041). Decision 024-014 assigns 13 to `data_result`. Confidence: confirmed in code.
   Why: Merged as written, an older 1.7 peer decodes `data_result` bytes as a `StepLogCarry`. That is a wire break, and the "confirmed at merge" caveat rests on an evidence row that is already false.
   Correction: Fix the evidence row, take `data_result = 14` (or the next free number), and add a check against the canonical proto to Task 024-008.

3. Major — "A full quota never rolls back a publication" is false for API publications and for the step output
   Where: specs/024-data-engines-pipelines/spec.md:527-528 (FR-024-027) and 704 (SC-024-010); specs/024-data-engines-pipelines/data-model.md:137; specs/024-data-engines-pipelines/plan.md:377; specs/024-data-engines-pipelines/tasks.md:86 and 107; docs/adr/ADR-0027-durable-content-rate-and-retention-quotas.md:43 and 61-63
   What: Only the inbox document is exempted as `protected_system`. Two other rows are charged:
   - `data_asset_publications.justification` is class `content` and required for API origins. With the content quota full, `publishDataSnapshot` fails with `QUOTA_EXCEEDED`.
   - The `dataTransform` and `dataPublish` step outputs (structure-only metadata, spec.md:635-636) are stored as run output payloads. Under ADR-0027 lines 61-63, a refused charge there turns the completion into `STORAGE_QUOTA_EXCEEDED` and a failed-run stop.
   Confidence: confirmed in the text.
   Why: SC-024-010 and Task 024-046 claim a publication commits at full content quota. The API case cannot pass, and the run case only passes if the output schema happens to be uncharged.
   Correction: Declare the transform and publish step outputs as closed, bounded, protected records, and either make the justification protected (bounded at 1,024 bytes) or state that API publication obeys content quota. Extend the full-quota test to an API publish and to a `dataPublish`.

4. Major — Uncommitted parts keep `tenant_data_bytes` and the pending-upload slots for 24 hours after a failed or stale attempt
   Where: crates/application/src/blobs.rs:36-39 (`DEFAULT_UPLOAD_TTL` 24 h); docs/adr/ADR-0027-durable-content-rate-and-retention-quotas.md:71 ("Finalized garbage releases only after confirmed deletion"); specs/024-data-engines-pipelines/spec.md:332-334 and 339-340; specs/024-data-engines-pipelines/plan.md:337-347 and 363
   What: Every failed attempt, every attempt whose lease lapsed and keeps uploading, and every cancelled run leaves its parts `UPLOADED`. They stay charged until the 24-hour sweep. With the plan's hard ceilings, one transform of 2 GiB with 5 attempts reserves 10 GiB, the whole default `tenant_data_bytes`, for a day. At 32 parts × 5 attempts = 160 parts, it also exceeds the default pending ceiling of 128. Every other data step of the tenant then fails `STORAGE_QUOTA_EXCEEDED`. Confidence: confirmed in code for the time to live and the release rule; plausible for the operational effect.
   Why: The edge cases say "earlier parts are swept" but never say when. The ceilings suite (Task 024-065) exhausts quotas directly and never exercises retries, so it cannot catch this drift.
   Correction: In the transaction that fails, retries (new generation), cancels or stales a data attempt, claim that attempt's uncommitted `DATA_SNAPSHOT_PART` uploads for deletion. Alternatively give the purpose a short time to live. Add a test: a retried failing transform, then the tenant's usage back to baseline without waiting a day.

5. Major — The audit-to-bytes binding check cannot fail for the scenario its test names
   Where: specs/024-data-engines-pipelines/contracts/data-runtime.md:176-179; specs/024-data-engines-pipelines/plan.md:191-193 and 382; specs/024-data-engines-pipelines/spec.md:504-505; docs/adr/ADR-0058-sandboxed-data-transforms-and-pointer-publication.md:103-106; specs/024-data-engines-pipelines/research.md:336
   What: The audit container returns only check outcomes; the data runner adds the SHA-256 "of every part it staged". The engine compares those digests with the committed parts, which are "by construction" the same bytes. The plan's failure row "Audit container reports digests of other files" cannot be written, because the container reports no digest. Confidence: confirmed in the text.
   Why: The check is a self-consistency check of trusted platform code. It proves nothing about which bytes the untrusted audit actually read, yet ADR-0058 decision 4 and SC-024-007 present it as binding the audit to the uploaded bytes.
   Correction: Restate the guarantee: the audit is bound to the staged bytes by the data runner's read-back verification, and audit outcomes rest on the sandbox (Gap 1249). Replace the test with a data-runner fault injection (stage a blob other than the committed part, expect `DATA_CONTRACT_VIOLATION`).

6. Major — The engine's "schema equal to the contract" check has no normative Parquet mapping and conflicts with the not-null audit and with Polars parity
   Where: specs/024-data-engines-pipelines/contracts/data-runtime.md:171-173 and 184-187; specs/024-data-engines-pipelines/spec.md:279-281 (User story 024-07 independent test) and 497-501 (FR-024-023); specs/024-data-engines-pipelines/tasks.md:85 (Task 024-033) and 124 (Task 024-053); specs/024-data-engines-pipelines/quickstart.md:110-114
   What: The engine requires "physical/logical types, repetition" to equal the contract, but no table maps `decimal(p,s)`, `int32`, `timestamp`, `json` or nullability to Parquet physical types and repetition.
   - Repetition: DuckDB's `COPY (SELECT …)` writes query columns as `OPTIONAL` (no NOT NULL is known for a query result). If a `nullable: false` column must be `REQUIRED`, the quickstart's `order_id` fails `DATA_CONTRACT_VIOLATION`. If `OPTIONAL` is accepted, "repetition equal" is false as written.
   - Physical type: DuckDB writes `decimal(18,2)` as `INT64` and PyArrow as `FIXED_LEN_BYTE_ARRAY`, so "the same snapshot" from Polars cannot hold byte for byte.
   Confidence: plausible (the DuckDB writer behaviour should be confirmed on the pinned image).
   Why: Task 024-033's "lying sandbox" tests cover wrong schemas, not the accepted encodings, so the happy path may fail or the check may be weakened ad hoc.
   Correction: Add a normative table to data-runtime.md: contract type → accepted physical and logical types (for example both decimal encodings), and nullability → `OPTIONAL` accepted, with nulls caught by the generated not-null audit. Define User story 024-07 parity as equal row sets and equal contract fingerprint, not identical bytes.

7. Major — `ABANDONED` can hit a live run's own manual snapshot, and `dataPublish` or `from` on an abandoned snapshot has no defined outcome
   Where: specs/024-data-engines-pipelines/data-model.md:44-47 and 247-249; specs/024-data-engines-pipelines/plan.md:207-212; specs/024-data-engines-pipelines/spec.md:456-457; specs/024-data-engines-pipelines/contracts/workflow-data.schema.json:362; specs/024-data-engines-pipelines/contracts/data-runtime.md:219
   What: Two paths move a snapshot to `ABANDONED` while its run still needs it:
   - Time to live: run R produces S with `publish: manual`, then waits more than `retainUnpublishedDays` (default 7) on a feature 014 approval. Retention moves S to `ABANDONED`. The "pinned by a live run" row protects only against retirement.
   - Passed over: any publication (an API rollback, for example) abandons every passed-over `AUDITED` snapshot, including S.
   R's `dataPublish` then meets an `ABANDONED` snapshot. The node schema defines only "quarantined or missing", and the failure map defines only `DATA_INPUT_QUARANTINED` for `from`. `onStale: skip` is never applied. Confidence: confirmed in the text.
   Why: No task tests an approval longer than the time to live or a rollback during an approval (Task 024-049 races only the deadline against the decision).
   Correction: Never move a snapshot pinned by a live run (own staged, `from`, output base) to `ABANDONED` by time to live. Define that `dataPublish` or a `from` on an `ABANDONED` snapshot is stale and follows `onStale` (`DATA_PUBLISH_STALE` or skip). Add both races to Task 024-049.

8. Major — Feature 018 reset rematerialization is undefined for step sources and same-run snapshots, and its race with retention is untested
   Where: specs/024-data-engines-pipelines/data-model.md:176-181, 197-199 and 214-229; specs/024-data-engines-pipelines/research.md:362 (Decision 024-007: "a step-output blob of the same run"); crates/infra-postgres/src/execution/database.rs:316-320 (pins only `run_id = this run`, `STEP_OUTPUT`, `COMMITTED`); crates/domain/src/execution/reset.rs:64 (`step.completed` is Copy); specs/024-data-engines-pipelines/plan.md:371; specs/024-data-engines-pipelines/tasks.md:146
   What: Three gaps, plus a race:
   - After a reset, a rematerialized `STEP` pin names the source run's output blob. The "same run" lease allow-list cannot pin it, so every lease of the data node in the reset run fails.
   - `from` and `dataPublish` resolve "the snapshot staged by an earlier node of the same run", but in the reset run that snapshot belongs to the source run. Only `run_data_snapshot_refs` links them, and no resolution rule uses it.
   - A restored `dataPublish` that publishes a source-run snapshot under the new run's `RUN_NODE` origin is not stated as allowed.
   - The race: the reset transaction's lock on the asset head before reading `RETIRED` is not in the transactions table. Plan line 371 covers only the first lease.
   Confidence: plausible.
   Why: Task 024-068 tests only "refused when retired" and "never re-published", not a reset after a step-source transform, a reset between a manual transform and `dataPublish`, or a barrier race against retention.
   Correction: Add a reset row to the transactions table: slot 4b heads `FOR SHARE`, then snapshot state read under that lock, then `run_data_snapshot_refs` and pins. Define step-source pins as cross-run references allowed through `run_data_snapshot_refs`-like rows, or refuse reset points after a step-source pin. Resolve `from` and `dataPublish` through those references. Add the three cases and a barrier race test to Task 024-068.

9. Major — The external-asset observation write has no transaction, lock order or budget shape
   Where: specs/024-data-engines-pipelines/spec.md:566-568 (FR-024-032) and 640-641 (FR-024-042); specs/024-data-engines-pipelines/data-model.md:81-83, 170-174 and 214-229; specs/024-data-engines-pipelines/tasks.md:94 and 129
   What: A successful database or `icebergWrite` completion with `assets.writes` must insert `data_asset_observations`, update `data_assets.last_observed_at`, write `data.observation_recorded` and recheck the producer binding and status. The transactions table has no row for it: no asset head lock, no position after the run and binding guards, no recheck under lock. Concrete race: a concurrent declaration revision that changes the producer commits between the publication-time check and the completion, and the observation is still recorded and resolves a stale alert. Confidence: confirmed in the text.
   Why: The statement budget test covers only "data first lease and data completion". The ordinary database completion grows with this write, against FR-024-042's "MUST NOT grow", and nothing catches it.
   Correction: Add a "declared external write completion" row: run → managed completion guards → external asset heads `FOR NO KEY UPDATE` ascending, status and producer recheck, observation, head update, journal. Add its budget shape to Task 024-074 and a race test to Task 024-054.

10. Major — Two outputs of one node on the same asset fail at first lease with an internal error instead of a validation refusal
    Where: specs/024-data-engines-pipelines/contracts/workflow-data.schema.json:233-240; specs/024-data-engines-pipelines/data-model.md:159-160 (primary key of `step_data_output_bases` is run, node, asset); specs/024-data-engines-pipelines/spec.md:441-444; specs/024-data-engines-pipelines/tasks.md:81
    What: Nothing forbids `outputs: [{asset: orders-daily, …}, {asset: orders-daily, …}]`. The producer binding is node-level, so it passes publication. The first-lease transaction then violates the primary key, rolls back, and the task is re-offered and fails forever. Even without the key, the second auto-publish CAS in one transaction would be stale against the first. Confidence: confirmed in the text.
    Why: Task 024-030's refusal fixtures do not list it, and a client mistake surfaces as a store error at lease.
    Correction: Require unique output assets per node at publication (`VALIDATION_FAILED`, reason `DATA_NODE_SCOPE` or `ASSET_PRODUCER_MISMATCH`) and add the fixture.

11. Major — Task 024-010's evidence needs a feature 020 run purge that does not exist
    Where: specs/024-data-engines-pipelines/tasks.md:31 (Task 024-010 evidence); docs/adr/ADR-0054-run-content-leaves-the-hot-store-only-through-a-verified-archive-under-a-namespace-retention-policy.md:3 ("its implementation waits"); no run purge exists under crates/ (the only `RUN_PURGED`-like names are 018 reset migrations)
    What: Slice 0 can close only with "a 020 run purge that leaves `DATA_SNAPSHOT_PART` blobs and all snapshot metadata … untouched". No such purge is implemented, so the test cannot be written in slice 0. Confidence: confirmed in code.
    Why: Either the task stays open and blocks both lanes, or it is ticked without the evidence it names.
    Correction: Move that evidence to a task gated on feature 020's purge (Owner-gate line). In slice 0, prove only what exists: the `blobs` purpose CHECK, and the guarded trigger refusing `COMMITTED → DELETING` for other purposes.

12. Major — Freshness counts the time of publication, so a rollback or a late manual publish of old data reads `FRESH`
    Where: specs/024-data-engines-pipelines/spec.md:230-231 and 557-560 (FR-024-031); specs/024-data-engines-pipelines/data-model.md:183-184; specs/024-data-engines-pipelines/plan.md:216-218
    What: Status is derived from `last_published_at`. A rollback to a 10-day-old snapshot, or an API publish of a 6-day-old `AUDITED` snapshot, under `maxLagSeconds: 3600` turns the asset `FRESH` and resolves `DATA_ASSET_STALE`. The data is as old as before. Confidence: plausible (it may be intended, but the spec does not say so).
    Why: The freshness fixture of Tasks 024-044 and 024-055 publishes only new snapshots, so the projection never meets a rollback.
    Correction: Measure lag from the current snapshot's `created_at` (its producer completion), or state explicitly that a rollback refreshes freshness. Add a rollback case to Task 024-044.

13. Major — The audit always runs in class `small`, and infrastructure failures of the audit become permanent quarantines
    Where: specs/024-data-engines-pipelines/contracts/data-runtime.md:111 and 226; specs/024-data-engines-pipelines/plan.md:340-341, 374 and 388; specs/024-data-engines-pipelines/spec.md:508-509; specs/024-data-engines-pipelines/research.md:224-225 (Decision 1259)
    What: A `large` transform can write 2 GiB. Its `unique` audit in class `small` (3 GiB of memory including tmpfs, "small" spill) can be killed deterministically. Every attempt then commits a `QUARANTINED` snapshot ("quarantine is final"), fires `DATA_SNAPSHOT_QUARANTINED` and keeps its bytes for `retainUnpublishedDays`. The case where the data mode refuses or is unavailable for the audit attempt (no slot) is undefined: retryable `DATA_RUNTIME_UNAVAILABLE` with nothing committed, or `ERROR` with a quarantine. Confidence: plausible.
    Why: Task 024-035 tests pass, fail and error on small fixtures only.
    Correction: Choose the audit class from the snapshot size (the transform's class). Rule that the audit's own infrastructure failures (refusal, unavailable, memory kill) fail the step retryably with no snapshot, and that only check outcomes `FAILED`, or `ERROR` from tenant-authored checks, quarantine.

14. Nit — A rollback to a retired snapshot has two different errors
    Where: specs/024-data-engines-pipelines/spec.md:263-264 (`NOT_FOUND`); specs/024-data-engines-pipelines/contracts/README.md:45 (`CONFLICT` reason `SNAPSHOT_NOT_PUBLISHABLE` for "abandoned or retired")
    Correction: Pick one (the metadata row survives, so `CONFLICT SNAPSHOT_NOT_PUBLISHABLE` fits better) and fix the other text and Task 024-048.

15. Nit — `DATA_SNAPSHOT_QUARANTINED` has no resolve condition
    Where: specs/024-data-engines-pipelines/spec.md:561-562; specs/024-data-engines-pipelines/quickstart.md:149-150; docs/runbooks/alerts.md:11
    Correction: State the condition (for example "an unretired quarantined snapshot newer than the current one exists"), so that the fire/resolve test of Task 024-044 can be written.

16. Nit — Decision 024-007 says the reuse is "already tested", but the existing pin writer and read gate are lake-only and same-run
    Where: specs/024-data-engines-pipelines/research.md:362; crates/infra-postgres/src/execution/database.rs:311-320; crates/application/src/worker.rs:2243-2263
    Correction: Say that a new pin writer (cross-run `DATA_SNAPSHOT_PART`, any state but `DELETING`) and a `ManagedKind::DataTransform`/`DataQuery` branch of `check_blob_access` are new code. Give them their own tests in Task 024-031.

17. Nit — A snapshot refused as stale stays `AUDITED`, and its bytes live for twice `retainUnpublishedDays`
    Where: specs/024-data-engines-pipelines/plan.md:268-271; specs/024-data-engines-pipelines/data-model.md:49 and 248-249
    Correction: Commit a stale or skipped auto-publish snapshot directly as `ABANDONED` (widen the creation states), and state whether the `ABANDONED` time to live counts from creation or from abandonment.

18. Nit — An object-store outage while the engine reads footers has no failure code
    Where: specs/024-data-engines-pipelines/plan.md:199-206; specs/024-data-engines-pipelines/contracts/data-runtime.md:225; specs/024-data-engines-pipelines/spec.md:328-329
    Correction: Map `BlobReadError::Unavailable` during verification to a transient gRPC status with no state change (the runner retries `CompleteStep` with the same key), never to `DATA_CONTRACT_VIOLATION`.

19. Nit — Tombstoning makes the current snapshot eligible for retirement while the pointer still names it
    Where: specs/024-data-engines-pipelines/data-model.md:245 and 255
    Correction: Either clear the pointer in the tombstone transaction (with a publication record) or keep the current snapshot. Enforce in the snapshot trigger that a snapshot named by a non-deleted asset's pointer cannot become `RETIRED`.

20. Nit — The test of "duplicate delivery of the publication event" cannot produce a duplicate
    Where: specs/024-data-engines-pipelines/spec.md:226 and 529-530; specs/024-data-engines-pipelines/plan.md:390; specs/024-data-engines-pipelines/tasks.md:107
    Correction: The platform writes the inbox row once, inside the publication transaction. Test the real duplicate instead: a crash of the trigger dispatcher after creating the run and before marking the event processed (and redelivery of the outbound webhook row).

21. Nit — An orphaned container keeps its slot until its label deadline, and refusals use up the retry budget
    Where: specs/024-data-engines-pipelines/plan.md:388-389; specs/024-data-engines-pipelines/contracts/data-runtime.md:217
    Correction: Do not count `DATA_RUNTIME_UNAVAILABLE` against `maxAttempts` (default 2), or let the restarted data runner end attempts it can identify. Otherwise a data runner crash plus a full node fails the step permanently.

22. Nit — Two snapshot-resolution rules are unstated: the producer check for a node publication, and which snapshot a node "staged" after a retry
    Where: specs/024-data-engines-pipelines/plan.md:207-209; specs/024-data-engines-pipelines/spec.md:441-444 and 449-457
    Correction: State that `RUN_NODE` checks the binding against the `from` transform node, not the `dataPublish` node. State that `from` and `dataPublish` resolve the snapshot of the node's successful completion, not a quarantined earlier attempt.

23. Nit — The event payload is called closed but its schema is open, and the step timeout is not split between transform and audit
    Where: specs/024-data-engines-pipelines/contracts/events.asyncapi.yaml:43 (`additionalProperties: true`) against specs/024-data-engines-pipelines/spec.md:527; specs/024-data-engines-pipelines/contracts/workflow-data.schema.json:247-252 against specs/024-data-engines-pipelines/plan.md:343
    Correction: Close the payload schema, or say that the bound is enforced by the database CHECK. State whether `timeoutMs` (default 10 minutes, equal to the default transform wall time) covers staging, upload, read-back and audit, and size the default to cover them.
