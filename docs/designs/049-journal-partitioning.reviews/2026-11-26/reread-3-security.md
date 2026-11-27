# Targeted re-read 3 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 8a18ee1bf521acf4ff550c92caa5a48da365480d
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 78155363, not a full review. The reviewer read files only and ran nothing.

Re-read 2's Nits A to E are still resolved (E by reporting an unknown start, with no grant of a predefined role). Verified
with no finding: the verified-archive rule, no runtime grant on `journal_retirements`, the `runs` trigger's lock and its
clock, `ensure_journal_buckets` never recreating a retired month, the stopped-upgrade waiver never covering the security
rules, design 053's drain wording, ADR-0132's decision 6.

No Blocker, no Major.

## Nit

1. (Suspicion, wording.) The abandon writes its authorization rows "through the same definer function as feature 020's
   purge", which `sagat_app` may execute; if the abandon's deletes are a mode of that function, the runtime role could
   delete an active retirement's `run_payloads` after step 3 counted them, and the quota would be released twice. Fix: an
   owner-only writer called only by `journal_retire_abandon`, which checks `ABANDONING`.

Counts: Blocker 0, Major 0, Nit 1

Fixed at `577fc97d`: `authorize_abandon_deletes`, owner-only, called only by `journal_retire_abandon` in the state
`ABANDONING`; the purge function `sagat_app` executes cannot write those rows.
