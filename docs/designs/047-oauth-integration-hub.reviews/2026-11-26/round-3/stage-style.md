# Design 047, EGM stage at `4bc5767b` (2026-11-26)

The reviewer's output, condensed by the author; every finding and severity kept. Summarised and answered in `egm-review.md`.

## Writing (egm-style)

1. Major. Open Questions and Technical Plan 12 make approval conditional on 5.1.0 merging, while the Constitution
   Check's Principle V row says the declared set must be in Principle V "before approval".
2. Major. Requirement 047-011 counts only `invalid_client` toward `BROKEN`; Technical Plan 4 also counts
   `unauthorized_client`.
3. Major. Requirement 047-013 carries five rules in one paragraph; "the lease plus the store deadlines",
   "downgrades `revocation_outcome`" and "as Technical Plan 4 says" cannot be acted on.
4. Major. "30 times per hour of the attempt's life" (Requirement 047-016, Technical Plan 5) reads as a sliding window or
   a growing budget; "one per-resource time" does not say what is stored.
5. Major. Requirement 047-009's acceptance names `a_withdrawn_offer_cannot_be_opted_into`, an offers test.
6. Major. "Revision" means three things (resource, token, registration secret) and access and refresh revisions are not
   defined; the passes (sweeper, revocation, keep-alive, hourly, daily) have no list or cadence.
7. Nit. Grouped: "fails retryable" as an adverb; "acquiring itself"; two fragments; an unclear "it" in Requirement
   047-023.
8. Nit. Old identifier forms in prose ("Tasks T028-062 to T028-066", "Tasks T021-005").
9. Nit. "IR" never written in full; slashes as shorthand in the proposed constitution text.
10. Nit. "10 divided by the number of opted-in tenants per second, at least 1" reads more than one way.

Counts: Blocker 0, Major 6, Nit 4
