# Correctness re-read of the feature 024 fixes (2026-11-27)

Doc: specs/024-data-engines-pipelines/
Reviewed-commit: 985dd80b1f46c119dcc8f718c9657ae8a8426eaa
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative. A short re-read of the
fixes made after `2026-11-27-correctness.md`, under the approval rule of Owner decision 2026-11-25-2.

Counts: Blocker 0, Major 5, Nit 6

## Previous review

The earlier review's header says 25 findings (Nit 12), but its body lists 23: Nits 14 to 23 are 10. Research Decision 024-016 and checklist line 156 repeat the wrong "12 Nits". Status of each of the 23:

1. Blocker, `dataQuery` over a step source has no level: fixed. spec.md:211-217, 459-474, 626-635, 809; data-model.md:173-181; contracts/data-runtime.md:263-271; contracts/workflow-data.schema.json:47-50, 304; tasks.md:103; ADR-0058:152-161.
2. Major, field 13 collides with feature 041: fixed. contracts/worker-data.md:10-16, 143-148; research.md:31, 384; tasks.md:32. Field 14 is free in specs/001-platform-foundation/contracts/worker.proto:154-163.
3. Major, a full quota rolls back API and run publications: fixed (the tombstone claim it added is finding 4). spec.md:589-611, 810; data-model.md:26, 147-156, 211-217, 266; plan.md:228-234, 432; tasks.md:89, 111, 116.
4. Major, uncommitted parts held for 24 hours: **partly**. spec.md:351-356, 694-703; data-model.md:25, 264, 295; plan.md:307-310, 413-414; contracts/worker-data.md:134-139, 180-185; tasks.md:98. See finding 1.
5. Major, the digest check cannot fail: fixed. spec.md:560-571; contracts/data-runtime.md:188-201; plan.md:193-198, 438; contracts/worker-data.md:156-159; tasks.md:91; ADR-0058:103-110.
6. Major, no Parquet mapping: fixed (one table row is finding 8). contracts/data-runtime.md:224-255; spec.md:293-297, 550-557; tasks.md:88, 128; quickstart.md:139.
7. Major, `ABANDONED` hits a live run's snapshot: fixed for both scenarios. spec.md:348-350, 503-504, 584-588, 678-685; data-model.md:51, 58-63; plan.md:424; tasks.md:117. A path through a reset remains (finding 3).
8. Major, reset rematerialization: fixed. spec.md:494-501; data-model.md:183-185, 202-209, 235-236, 269; plan.md:423, 433; tasks.md:22, 150.
9. Major, external observation write: fixed. spec.md:654-663; data-model.md:268; plan.md:321-329, 434; tasks.md:97, 133.
10. Major, two outputs on one asset: fixed. spec.md:360-361, 480-482; contracts/workflow-data.schema.json:5, 237; contracts/README.md:43; tasks.md:84.
11. Major, purge evidence impossible in slice 0: fixed. tasks.md:34, 152-153, 296-298; spec.md:815.
12. Major, freshness measured from publication time: fixed. spec.md:242-245, 642-647; data-model.md:219-222; plan.md:240-243; tasks.md:109.
13. Major, audit class and infrastructure failures: fixed. spec.md:560-577; contracts/data-runtime.md:119, 203-209, 292; plan.md:428-429; tasks.md:91.
14. Nit, two errors for a rollback to a retired snapshot: fixed. spec.md:277-278; contracts/README.md:47; contracts/data-assets.openapi.yaml:150-153; tasks.md:116.
15. Nit, quarantine alert has no resolve condition: fixed. spec.md:648-651; quickstart.md:151-155; plan.md:242-243; tasks.md:109.
16. Nit, reuse "already tested": fixed. research.md:377; contracts/worker-data.md:126-133; tasks.md:85.
17. Nit, stale snapshot kept twice its window: fixed. spec.md:584-588, 682-684; data-model.md:50, 54-55, 60-61, 292-293; tasks.md:89, 94.
18. Nit, object-store outage during footer reads: fixed. spec.md:344-347, 558-559; contracts/data-runtime.md:219-222, 294; plan.md:218-221, 426; tasks.md:88.
19. Nit, tombstone and the current snapshot: fixed. data-model.md:40-43, 65, 267; spec.md:357-359, 684-685; tasks.md:94.
20. Nit, duplicate-delivery test cannot produce a duplicate: fixed. spec.md:236-238; plan.md:446; tasks.md:111, 143.
21. Nit, orphaned container and retry budget: fixed. spec.md:516-518; contracts/data-runtime.md:73-76, 291; plan.md:393, 444-445; tasks.md:81, 143. The new counting rule has no state (finding 2).
22. Nit, snapshot-resolution rules: fixed. spec.md:478-480, 494-495; data-model.md:182-183; plan.md:223-224; tasks.md:85, 115.
23. Nit, open event payload and timeout split: **partly**. contracts/events.asyncapi.yaml:43-45; spec.md:512-516; contracts/workflow-data.schema.json:248-253; plan.md:390-391. `dataQuery` is not fixed (finding 7).

## Findings

1. Major — A late upload can escape the claim of the transaction that ends its attempt, and the list of ending transactions is incomplete (Major 4 only partly fixed)
   Where: data-model.md:25, 264; plan.md:307-310; contracts/worker-data.md:137-139, 183-185; tasks.md:98; spec.md:698-703. Code: crates/infra-postgres/src/blobs.rs:114-121 and 165-179; crates/api-grpc/src/service.rs:1310-1316; crates/domain/src/execution/command.rs:1809-1829.
   What: Every text claims only `UPLOADED → DELETING`. A part still `UPLOADING` is not matched. The lease check of `WriteBlob` runs outside the reserve transaction, and `mark_uploaded` locks only the blob row.
   - Scenario: part 3 is `UPLOADING` when the lease lapses. The expiry transaction claims parts 0 to 2. Then `mark_uploaded` sets part 3 to `UPLOADED`. Its "fence still live" recheck can run before the expiry commits, and no lock orders the two. Part 3 is then held for 24 hours.
   - `reserve` checks only that `attempt` and `generation` equal the step's. A step that failed terminally or was cancelled still matches, so it accepts new reservations for an ended fence.
   - The list of ending transactions names only "lease expiry that grants a new generation". It leaves out an expiry with no attempts left and `AttemptTimeout`, which also end the fence through `end_attempt`, and run stops (deadline, failed-run stop).
   Confidence: plausible (the race is read from the code).
   Why: Task 024-078 runs failures one after another, with no barrier between finalization and the ending transaction.
   Correction:
   - Claim every non-committed row of the fence (`state IN ('UPLOADING','UPLOADED')`). `mark_uploaded` already turns a `DELETING` in-flight row into a measured `DELETING` row.
   - Do the claim wherever a data fence ends (`end_attempt`, cancellation, run stop).
   - Make the `reserve` INSERT require the step to be `LEASED` to that fence.
   - Add a barrier test between a finalization and an expiry to Task 024-078.

2. Major — The ten uncounted `DATA_RUNTIME_UNAVAILABLE` attempts have no durable state, journal event or reset rule
   Where: spec.md:516-518; contracts/data-runtime.md:291; plan.md:200-201, 393, 444; tasks.md:81, 147; research.md:36. Code: crates/domain/src/execution/command.rs:1831-1876.
   What: `end_attempt` spends an attempt. The only mechanism that gives an attempt back is `withdraw_lease`:
   - it works only from `LEASED`, which a data attempt has left once it heartbeats;
   - it is bounded at 4 per attempt;
   - it is counted from the journal's `step.lease_withdrawn`.
   The set never says which command ends an uncounted attempt, whether the attempt number is reused, where the count of 10 lives, or which reset rule applies. A counter kept in a mutable column would differ from a replay of the journal, and a reset run would inherit it or not, undefined.
   Confidence: confirmed in the text.
   Why: Task 024-072 tests only the runner's ending table. Nothing tests a replay or a reset of the count.
   Correction: Define a domain command, a withdrawal-like event with a data cause that is allowed after the heartbeat. It gives the attempt number back, advances the generation and is counted from the step's journal events (at most 10). Declare its reset rule. Add a replay-equality test and a reset test.

3. Major — After an 018 reset, `from` and `dataPublish` can meet an `ABANDONED` snapshot that the spec calls unreachable
   Where: spec.md:503-504, 584-585; contracts/data-runtime.md:296; data-model.md:62-63, 236; contracts/workflow-data.schema.json:45; tasks.md:150.
   What: Scenario:
   - Run R commits S from transform T (`publish: manual`) and ends at a rejected approval.
   - Another run publishes. R has ended, so S has no live pin and is abandoned as passed over.
   - An operator resets R to after T. The rematerialization of `data.snapshot_committed` refuses only `RETIRED`, so the reset is accepted.
   - In the reset run, `dataPublish` fails `DATA_PUBLISH_STALE`. A `from` reader fails `DATA_CONFIGURATION_INVALID`, which is not retryable and is not a configuration error.
   Confidence: confirmed in the text.
   Why: Task 024-068's case "a reset between a manual transform and its `dataPublish` publishes the source run's snapshot" passes only if no publication passed S over.
   Correction: At reset, refuse a restored snapshot that is not `AUDITED` or `PUBLISHED` (a `CONFLICT` reason, as for `SNAPSHOT_RETIRED`). Remove "unreachable". Add the passed-over case to Task 024-068.

4. Major — "A tombstone still works at a full quota" is claimed, but the tombstone writes a content-class revision row
   Where: research.md:392 (Decision 024-017); data-model.md:103-107 ("as in 017"), 151-154, 267. Precedent: crates/infra-postgres/storage-classification.json:871-874 (017 `config` is `content`). ADR-0027:59.
   What: The tombstone transaction writes a revision. Its `declaration jsonb` (up to 128 KiB) is class `content`. If it copies the declaration as 017's delete copies `config`, the net change is positive and a full quota refuses the tombstone. That is the opposite of what Decision 024-017 claims and what ADR-0027 requires of deletions.
   Confidence: plausible.
   Why: No task tests a tombstone at full quota.
   Correction: State that the tombstone revision stores no declaration (null, measured zero). Add "tombstone at full content quota commits" to Task 024-029 or 024-038.

5. Major — `data_protected_outputs` is an accounting exemption that the runtime role can write
   Where: data-model.md:26, 211-217, 278 (INSERT granted to `sagat_app`). ADR-0027:51-53. crates/infra-postgres/migrations/20261009000100_execution.up.sql:10-18 (`payloads` keyed by tenant and digest).
   What:
   - Any digest that `sagat_app` inserts makes a payload of up to 4 KiB with that digest measure zero. A buggy or injected runtime path can store any number of 4 KiB payloads uncharged. ADR-0027 forbids any runtime ability to bypass accounting and requires a test of the runtime role against each bypass.
   - Because `payloads` is content-addressed, a row charged before its digest is listed is "never refunded" (data-model.md:26). The usage counter then disagrees with a recomputation from the manifest.
   Confidence: confirmed in the text.
   Why: The runtime-role sweep checks privileges, not this bypass.
   Correction: Classify by the content, not by a list. Store these documents where a CHECK enforces the closed schema and the size, as `inbox_events` does for `sagat.data`. Alternatively, list a digest only through a `SECURITY DEFINER` helper that validates the document. Add the bypass test to Task 024-010.

6. Nit — "A full content quota can never roll back an auto-publishing completion" claims too much
   Where: spec.md:606-607; ADR-0058:167-168; plan.md:432.
   What: If the completion also creates charged rows for the next node, for example a child workflow's input, ADR-0027:61-63 rolls it back and commits a failed-run stop. The publication is lost with it. Confidence: plausible.
   Correction: Limit the claim to the records the data completion writes itself. Test it with a child-workflow next node, or state the exception.

7. Nit — `dataQuery` keeps the timeout problem of Nit 23, and the staging allowance has no value (Nit 23 only partly fixed)
   Where: contracts/workflow-data.schema.json:341-346 (default 120,000 ms); plan.md:389 (wall time 2 min), 391; spec.md:514-516.
   What:
   - The `dataQuery` lease timeout equals its wall time, yet staging up to 1 GiB happens inside the lease.
   - The "fixed staging allowance" is never given a value, so the publication check of Task 024-030 ("at least 60 s left for the transform") cannot be written.
   Correction: Give the allowance a value, apply the same split to `dataQuery`, and raise its default `timeoutMs`.

8. Nit — The encoding table probably refuses DuckDB's wide decimals
   Where: contracts/data-runtime.md:237.
   What: The table requires `FIXED_LEN_BYTE_ARRAY` "of the minimal length for p". DuckDB writes `DECIMAL` with p > 18 as a 16-byte fixed array, so `decimal(20,2)` would be refused on the happy path. Confidence: plausible; Task 024-033 records the observed encodings.
   Correction: Accept any length from the minimal one up to 16 bytes.

9. Nit — "Every publication MUST write an inbox event" contradicts the `TOMBSTONE` record
   Where: spec.md:597-601 against data-model.md:154-155 and contracts/events.asyncapi.yaml:56.
   Correction: Say "every pointer move to a snapshot" or name the `TOMBSTONE` exception.

10. Nit — The start-up sweep can end another data runner's live attempts
    Where: contracts/data-runtime.md:73-76; plan.md:204-206.
    What: A restarting data runner ends every attempt directory it finds in the work volume. If two data runner processes share one volume, it kills the other's attempts. Confidence: plausible (deployment shape open, Gap 024-001).
    Correction: Give each instance its own subdirectory and sweep only that, or state and check one data runner per volume.

11. Nit — Stale references and counts after Decision 024-017
    Where:
    - spec.md:883 says "D024-001–D024-016" (omits 017).
    - ADR-0058:281 says "024-001 to 024-015".
    - contracts/events.asyncapi.yaml:22-24 says "a full quota never rolls back the publication that wrote it", which is false for API publications.
    - research.md:391 and checklists/requirements.md:156 say "12 Nits"; the review lists 10.
    Correction: Update the four texts.
