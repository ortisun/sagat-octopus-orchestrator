Doc: docs/designs/041-step-logs.md
Reviewed-commit: 7dd4ea3c
Reviewer: correctness
Counts: Blocker 0, Major 0, Nit 2

Both Majors and all four Nits are resolved, and the fix adds no new Blocker or Major. Two small Nits remain, listed after the per-finding results. I read the diff only; I did not run the tests.

**Major 1 (empty batch accepted): resolved.**
- `validate_batch` now returns `BatchInvalid::Empty` before checking anything else.
- An empty carry is still valid, as C2 requires.
- The test asserts `validate_batch(&[], 0)` fails, and the property-test generator now makes batches of 1 to 20 records.

**Major 2 (mark-only gap after a ceiling): resolved.**
- data-model.md §8a item 5 now says the unsent range is split as T5 splits it, worker's below `ceiling_sequence` and the ceiling's reason above it (D041-140 (2)).
- The new test `a_mark_only_carry_after_a_ceiling_splits_its_gap_as_t5_does` uses ceiling 4 and highest 10. It expects `attempt_records` 6, `worker_gap` 0 and `secret` 0. A change made only to the mark-only branch that sent the gap to the worker would now fail it.

**Nit 1 (property-test assertions that could not fail; byte reason not pinned): resolved.**
- The byte-ceiling test now asserts `AttemptBytes == 1` and `by_server == 1`.
- The new check of each plan's reasons accepts every legitimate case:
  - the stream's ceiling before or after the plan (`after.ceiling` equals this plan's `first_met` whenever the stream had none);
  - `tenant_quota` for a batch or a full carry;
  - `out_of_order` only for an out-of-order full carry;
  - the mark-only carry's own reason.
- I found no case where it fails on correct code. It catches a reason hard-coded to `AttemptRecords` wherever the byte ceiling is met.
- The old "by reason adds up" assertions are still true by construction, but the new check now does that work.

**Nit 2 (mark-only records never above `highest_sequence`): resolved.** A mark-only carry's highest is now `last_sequence + extra`, independent of its records (1 to 399), so records above it, and an empty range when extra is 0, now occur. The saturating `gap` hides nothing here: if the `<= highest` filter were removed, `sent` would exceed `mark − last` and the counting-rule assertion would fail.

**Nit 3 (`run:logs:read` not decisive in any test): resolved.**
- **Token:** a developer member presenting a Viewer token is withheld. Removing the `RunLogsRead` check now fails the test.
- **Membership:** a Viewer member with grant rows in both namespaces, presenting a Developer token, is withheld.
- **Tenant:** a member of two tenants who holds every grant only in the other tenant is shown there and withheld here. A query without `n.tenant_id = $1` would fail this.

**Nit 4 (a grant-read error fails the whole view): resolved as a decision.** D041-140 (6) accepts it as fail-closed, and the method's doc now says so.

**Remaining Nits:**
- **N1 (new, plausible):** `quotas.rs:666` adds a `run:read` check, citing ADR-0085 rule 6. The OpenAPI `usageWithheld` and `used` descriptions (`specs/001-platform-foundation/contracts/openapi.yaml:6731-6732`) and the T041-4 task text (design line 2522) still name only `run:logs:read` and the grant. No role holds `run:logs:read` without `run:read`, so nothing observable changes and no test can make the new check decisive. Fix: add `run:read` to the two contract descriptions and the task text, regenerate `schema.d.ts`, and state in D041-140 that the check is defence in depth and untestable through the roles.
- **N2 (carried over, cosmetic):** the property test's doc comment still says "the drops by reason add up to the two drop counts". That holds by construction; the meaningful check is now the reason-set loop, and the comment could say so.
