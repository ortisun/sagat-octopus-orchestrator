# Design 047, EGM stage at `4bc5767b` (2026-11-26)

The reviewer's output, condensed by the author; every finding and severity kept. Summarised and answered in `egm-review.md`.

## Structure (egm-structural)

1. Major. Requirement identifiers are prose ("Requirement 047-001", "Non-functional requirement 047-001", "User story
   047-01") where the design template asks for `FR-NNN-xxx`, `NFR-NNN-xxx`, `SC-NNN-xxx` and `US0n`, and approved design 043
   uses `FR-043-001`; `Refs:` lines cannot change after approval.
2. Major. The text says no task delivering a declared resource is built until constitution 5.1.0 merges, but none of
   Tasks 047-9 and 047-14 to 047-17 has an `Owner-gate:` line; the condition sits under Open Questions, which says
   "None blocks"; the approval rule has no conditional approval.
3. Major. Owner decisions 2026-11-25-2, 2026-11-25-3 and 2026-11-24-320 are not in `docs/owner-decisions.md` at the
   reviewed commit, and "Owner decision 2026-11-24-4 (Owner decision 2026-11-24-320)" reads as garbled.
4. Major. No task refs Success criterion 047-002, and Success criterion 047-001 is ref'd only by the benchmark task.
5. Major. Decision 047-064 (after the reviewed commit) gives "present when probed" a meaning the design's text and its
   stated range ("047-001 to 047-063") do not carry.
6. Nit. `a_withdrawn_offer_cannot_be_opted_into` belongs to Requirement 047-002, and it and
   `the_next_attempt_after_reconnecting_succeeds` are in no task.
7. Nit. Feature 028's range reads "028-039 to 028-042" in Terms and "028-039 and 028-040" in Requirement 047-020.
8. Nit. "Gap 047-00n" and "Decision 047-0nn" against the register's `G047-` and `D047-`.
9. Nit. `contracts/integrations.md` cites Requirement 047-023 for the curated set, which is 047-024.
10. Nit. Requirement 047-013 carries sweeper and adoption mechanics that belong in Technical Plan 4.
11. Nit. Some lines past the wrap width; journeys headed "Journey n" where the template and 043 use "CUJ n".

Counts: Blocker 0, Major 5, Nit 6
