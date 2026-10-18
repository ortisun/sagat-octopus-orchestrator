# Report rollback, End replay branches and exact Run-byte boundary

Production is unchanged from the preceding composition checkpoint. Test base `21b2b793`,
plus the frozen native inputs here, later lands as test increment `6ed26a12`. Each revision
has its own 1,121-input before/after manifest. Revisions and repeated selections are not
combined into a single passing suite or presented as one frozen source revision.

Use the owned PostgreSQL 16.15 server, coordinated target and environment from
[the preceding evidence](../collection-finalization-integration-20261018/reproduce.md).
Exact commands, UTC starts, wall times and harness output are retained in JSON/TXT pairs.

## Actual results

| Root selection, first revision | Default | Runtime flag |
| --- | --- | --- |
| Existing End tests plus two branch replays | 4/4, 70.87s including 50.65s build | 4/4, 13.48s |
| Child consuming prior and current collection outputs | 1/1, 4.23s | 1/1, 3.32s |
| Finish replay with contradictory End bytes | 1/1, 2.41s | 1/1, 2.65s |
| Late refused-report receipt fault/retry | 1/1, 9.49s | 1/1, 9.44s |
| Legal End bytes over Run budget, initial oracle | 1/1, 5.07s | 1/1, 4.93s |

The Flow selections always use sagat_app, even without the flag. The quota test uses Platform's
actual default owner pool and the explicit runtime role in the second pass. There are no skips.
Strict integration Clippy passed in 51.38s, formatting in 5.08s, architecture in 0.51s.

The initial quota oracle needed stronger observations despite passing. C2 counts every terminal
outbox before checking its sole routing/status and compares the expected audit actions, including
absence of storage-quota audit. Its own focused default/runtime tests, strict Clippy and format
passed; records and source pairs are under `quota-oracle-c2/`. C3 additionally proves the exact
refused 17 payload is absent from all tenant payload rows before and after, independently of
projection references. C3 passed 1/1 default (75.34s including 68s build) and 1/1 runtime (4.47s),
plus formatting; see `quota-oracle-c3/`. Clippy was not repeated on that final two-query assertion
delta and remains part of the later merged delivery gate. No failed test is relabeled as passing.

## Accepted observations

- The actual scheduler's failed End EXPRESSION_ERROR and pre-evaluation INSTANCE_LIMIT branches
  replay through a fresh store/cache with exact original digest/source/outbox/revision identity.
  Matching bodyless replay is unapplied; contradictory evaluated answer or unexpected answer
  for the no-evaluation branch is DuplicateMismatch, with complete payload/accounting state stable.
- The child End expression consumes both owners using an eagerly evaluated list before selecting
  scalar17. The evaluator evaluates every list member (`infra-expr/src/eval.rs`, list); this
  requires the prior owner context. Parent propagation and immutable source/certificate still pass.
- The successful-End replay also rejects bytes18 where the original retained answer was17;
  no payload, accounting, projection or outbox changes follow either conflicting replay.
- The real refused report blocks at its actual receipt insert after root stop, scoped success
  and audit writes. A second connection sees the prior complete state. Releasing the gate gives
  the fixed labelled SQL failure rather than timeout; the whole transaction rolls back. The
  original public key/lease/request then succeeds, exact replay is stable, and normal discovery
  plus 17 genuine compensation tasks finishes correctly. These are engine/worker task semantics,
  not a durable external provider effect or exactly-once claim.
- Sixteen real source outputs retain 4MiB-3. Aggregate[] charges2 and remains successful; valid
  End17 needs2 when only1 remains. The Run fails RUN_DATA_LIMIT, retaining only prefix+owner,
  no EndReached/output/result, no orphan17, one correct collection/run notification and no
  storage-quota audit. Journal replay and telemetry head match the terminal state.

## Review and remaining limits

End-branch C1 found an invalid SQLx PgRow import; root fixed it before compiling, without
claiming a failed compile as RED. Report-fault C1 found None. Quota C1 found filtered-outbox
and missing-audit oracles; C2 found missing orphan-payload coverage; both were actionable and
corrected. Fresh C3 found None. The owner previously declined external review; none ran.
Raw logs and sealed unified patches retain original whitespace; source checks passed.

The copied missing-output worker decision is a design input, not executed acceptance. Its
worker-defined input contract is clarified in the quota decision; no IR extension is implied.
Prior-stop/current-secret races, output-dependent compensation, actual old in-flight report
orders/Applied/restart, uncertain COMMIT, complete rollout, nesting/resource/release gates and
the master corrections/canonical open feature014 tasks remain open.
