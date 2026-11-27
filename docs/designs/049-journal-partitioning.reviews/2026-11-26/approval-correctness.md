# Approval confirmation of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: c621f25b4f50038411fd38bc746a4d92867dc9d5
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`correctness-full.md`, at 66634fb9, 2
Blockers and 9 Majors), then the fixes re-read in `reread-1-correctness.md` to `reread-10-correctness.md`, each Major
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 10's four Nits are resolved (the third as written, with Nit 1 below).

No Blocker, no Major.

## Nit

1. The `RUN_PURGED` mapping depends on `journal_retirements`, not on `runs.purged_at`, so deferring its test to T020-045
   leaves it with no owner; Task 049-15's amendment of T020-045 does not carry it or the `journal.rs` check. Fix: test
   the mapping in Task 049-8 with a row marked dropped; add the read-path check and its test to the amendment.
2. With a frozen clock file and libfaketime faking the monotonic clock too, PostgreSQL's timeouts never fire and the lock
   timeout tests would hang. Fix: a running offset in the clock file and `DONT_FAKE_MONOTONIC=1`.
3. The generator test's assertion cannot hold for the legacy lookup in `payloads` or the owning-run lookup in `runs`, and
   omits the digest. Fix: scope it per target.
4. "Default: executable by the owner only" conflicts with "a name with no entry fails". Fix: a template, or no default.
5. An entry for a function another feature creates (feature 020's purge) would fail the audit until it exists. Fix: such
   an entry is audited once its function exists.

Counts: Blocker 0, Major 0, Nit 5

Settled: the five are recorded as design 049's follow-up Task 049-19, added after the approval as the approval rule
allows, to be fixed in the build, test first (the egm-review summary of this approval lists them).
