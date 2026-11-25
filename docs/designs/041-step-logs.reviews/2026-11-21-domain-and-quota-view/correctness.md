Doc: docs/designs/041-step-logs.md
Reviewed-commit: c1fc1206
Reviewer: correctness
Counts: Blocker 0, Major 2, Nit 4

I found no Blocker. `plan_batch` follows the stated rule on every path I traced:

- **Batch:** duplicates are not counted. The gap below a ceiling goes to `worker_gap`; the gap above it goes to the ceiling's reason. Records are kept until the first that does not fit, and the rest go to the ceiling's reason. When the tenant charge does not fit, the tenant drop is all or nothing and every record goes to `tenant_quota`.
- **Full carry:** same as a batch, with `highest_sequence` as the high mark.
- **Out-of-order carry:** every fresh record goes to `out_of_order`, nothing is kept and no marker is planned.
- **Mark-only carry:** each distinct sequence in (`last_sequence`, `highest_sequence`] is counted once with its reason. `tenant_quota` sets the ceiling only when none is met. `service_busy` sets `not_stored_reason` and leaves the ceiling alone.

The counting rule holds on every path, the subtraction cannot underflow, and saturation is applied to both sides. `validate_carry` matches C2 exactly (64 records, 16 KiB, `highest_sequence` up to 2^62 − 1 and at least the last record's sequence; Full / MarkOnly / Ignored). The quota view change does not alter any other resource: `usage_withheld` is false for all of them, and a namespace view never reaches the store call.

---

### Major 1 — `validate_batch` accepts a batch with no record (confirmed in code)

- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/domain/src/step_logs.rs:417-425`.
- **What the spec says:** P4 (design line 928), FR text (design line 196) and `contracts.md:32` all require 1 to 500 records. The contract lists "no record" as an `INVALID_ARGUMENT`.
- **What the code does:** before this commit an empty batch was legitimate, because it was the empty final batch. This commit removes the final mark from batches (D041-090) but adds no lower bound. `validate_batch(&[], 0)` returns `Ok(())`.
- **Failing scenario:** once T041-6 wires it, a worker sends `AppendStepLogs` with `records = []`.
  - The batch passes P4 and runs the full 14-round-trip transaction.
  - T4 creates or locks the stream row, and T9 moves `last_at` and `expires_at`.
  - The worker gets `CONTINUE` instead of `INVALID_ARGUMENT`.
  - A short step's later carry then finds an existing row. It goes through CC′/CE instead of CC, so the six carry equalities of `work_budget.rs` no longer hold for that attempt.
  - A worker can also keep a stream's retention alive with empty calls.
- **Why the tests miss it:** `a_batch_is_refused_outside_its_bounds` (line 966) has no empty case. The property test's batch generator, `btree_set(1u64..400, 0..20)`, produces empty batches itself and treats them as valid.
- **Smallest fix:** add `BatchInvalid::Empty` and check `records.is_empty()` first in `validate_batch` (not in `check_records`, since an empty carry is valid). Add an assertion for `validate_batch(&[], 0)` and change the generator's batch range to `1..20`.

### Major 2 — the mark-only gap after a met ceiling: data-model §8a item 5 and `plan_batch` disagree, and no test pins either reading (plausible spec drift; the code's reading is defensible)

- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/domain/src/step_logs.rs:610-633` (gap split, shared by every path) and `:641-650` (mark-only branch). On the spec side, `/Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/data-model.md:676-679`.
- **What the spec says:** for a mark-only carry, item 5 says "the rest of the range is the worker's gap".
- **What the code does:** it splits that gap at `ceiling_sequence`, as T5 and design 2a do for a full carry.
- **Concrete case:**
  - Ceilings are `records = 2`. A batch `[1,2,3,4]` keeps 1 and 2 and meets `attempt_records` at `ceiling_sequence = 4`.
  - The SDK stops sending. The report holds a secret, so it carries a mark-only `secret` carry with no record and `highest_sequence = 10`.
  - `plan_batch` gives `attempt_records: 6` (server side) and `worker_gap: 0`.
  - The literal text gives `worker_gap: 6`.
- **Why it matters:** T041-36's property test makes `plan_batch` the oracle for the SQL. An implementer who follows item 5 literally gets a mismatch, or worse, someone "fixes" `plan_batch` to match the text.
- **Why the tests miss it:** no unit test runs a mark-only carry on a stream with a met ceiling. The property test's allowed set (lines ~1202-1206) accepts both `WorkerGap` and the ceiling's reason, so either reading passes. The evidence's hand mutation "the gap above a ceiling counted as the worker's" changed the shared code; a change confined to the mark-only branch would survive.
- **Smallest fix:**
  - Change item 5's parenthetical to "the rest of the range is the gap of T5 (the worker's below `ceiling_sequence`, the ceiling's reason above it)". This matches the comment "one carry can have two" at data-model line 632.
  - Add the unit test above, with `secret` and `attempt_records = 6`.

### Nit 1 — two of the property test's assertions cannot fail, and the byte-ceiling reason is not pinned by any test (confirmed in code)

- **The assertions that cannot fail:** at `step_logs.rs:1222-1223` the property test checks that `by_reason[WorkerGap] == dropped_by_worker` and that `sum − worker == dropped_by_server`. Both counters are computed from `drops.by_worker()` and `drops.by_server()`, and `by_server` is itself `sum − by_worker` (line 355). Saturation, the only thing that could break them, is never reached with sequences below 400. So "the drops by reason add up to the two drop counts" is true by construction.
- **Why the counting rule does not help:** stored + dropped always equals `mark − last`, so it cannot see which side or reason a drop went to.
- **A mutation no test catches:** hard-code `DropReason::AttemptRecords` as the reason for unkept records at line 698. `the_byte_ceiling_is_met_by_the_record_that_does_not_fit` (line 1232) asserts only `keep` and the marker. The property test does not check the reasons of batches or full carries.
- **Fix:**
  - Assert `drops.get(DropReason::AttemptBytes) == 1` in the byte test.
  - In the property test, require for every plan that each server reason is the stream's ceiling before or after the plan, or `tenant_quota`, or `out_of_order` for an out-of-order carry, or the mark-only reason.

### Nit 2 — the mark-only generator never produces a record above `highest_sequence` (confirmed in code)

- **Where:** `step_logs.rs:1473` says the records "may ... lie outside its range". But `highest = max(sequences).max(last) + highest_extra` (line 1512) is never below any generated sequence.
- **Consequence:** the `record.sequence <= highest` filter at line 589 is exercised only by the single unit case (sequence 9 with highest 6). Removing it would make `gap` underflow, but only that one unit test would catch it.
- **Fix:** draw `highest` independently, for example `last + (0..30)`, so that some records fall above it.

### Nit 3 — the `run:logs:read` half of D041-133 is not tested (confirmed in code)

- **Where:** `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/application/src/quotas.rs:665` and the test at `/Users/marcelo/Documents/Projects/sagat-wt-041/tests/integration/security-multitenancy/quotas.rs:615`.
- **Why the check is never decisive in the tests:** every role whose membership may use a reveal grant (Admin, Developer, Operator) also holds `run:logs:read`.
- **A mutation that passes every test:** delete `principal.authorize(Permission::RunLogsRead).is_ok() &&`. The evidence's mutation ("the permission or the grant", && to ||) is caught; this one is not.
- **The missing case:** a member whose membership role is Developer and who holds the grant in both namespaces, but presents a `Principal` whose token role is Viewer. Viewer still holds `quota:read`. That case must return `(None, true)`.
- **Fix:** add that case to the test.

### Nit 4 — a store error in the grant read fails the whole quota list (plausible, design choice)

- **Where:** `quotas.rs:409` and `:552`.
- **What happens:** an error from `reveals_in_every_namespace` makes `list`, `set` and `clear` return `Unavailable` for every resource. The read is also a separate transaction that takes `sagat_lock_memberships` on a GET, so a membership change holding the row delays the quota view up to the lock timeout. This is not wrong (it fails closed), but it is a new way the quota view can fail.
- **Option:** either state this in the method's doc, or map the error to `usage_withheld = true`, which is still fail-closed under D041-133, so the rest of the list is still returned.

---

To confirm Major 1 and Nit 1 after a fix, run `cargo test -p sagat-domain --locked step_logs`, with the batch-generator change from Major 1, the new byte-ceiling assertion and the reason-set check from Nit 1 in place. Each should fail against the current code and pass after the fix.

Files read: `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/domain/src/step_logs.rs`, `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/application/src/quotas.rs`, `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/reveal_grants.rs`, `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/infra-postgres/src/quotas.rs`, `/Users/marcelo/Documents/Projects/sagat-wt-041/crates/api-http/src/dto_gov.rs`, `/Users/marcelo/Documents/Projects/sagat-wt-041/tests/integration/security-multitenancy/quotas.rs`, `/Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/data-model.md`, `/Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/contracts.md`, `/Users/marcelo/Documents/Projects/sagat-wt-041/specs/041-step-logs/research.md`.
