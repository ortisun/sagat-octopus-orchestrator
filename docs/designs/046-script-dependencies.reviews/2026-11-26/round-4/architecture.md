# Targeted re-read of design 046 for approval: architecture (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cd82af9376ce4a8d40196f11541a3ca1ac754207
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The second and last targeted
architecture re-read after review round 3 (approval rule of Owner decision 2026-11-25-2); the first is
`reread-1-architecture.md` (Major 5, Nit 6). The reviewer read the diff since `4b6a3e6d`, ADR-0120 and ADR-0121, the crate
manifests and the existing advisory-lock usage, and ran nothing.

## The previous findings

1. Answered: `crates/infra-egress` links only `sagat-domain` and `sagat-telemetry` links no workspace crate, so the
   allow-list holds. 2. Answered (Task 046-26). 3. Answered (`BACKFILL` claimable on `READY`, the phase, the completion
   shape, the tests). 4. Answered: Decision 046-077 replaced the retry; staleness never refuses the switch and the rows it
   was not given go to re-evaluation (finding 1 follows from it). 5. Answered (`fail_ready_step`: chain, run, step;
   recheck; `system:engine`; the racing-lease test). 6. Deferred to the approval commit: Owner decisions 2026-11-24-312 and
   2026-11-25-2 must be in `docs/owner-decisions.md`. 7. Answered. 8. Answered (finding 4 is a side effect). 9. Answered
   (finding 2). 10. Answered. 11. Answered.

## New findings

1. Major. Rows above the mark that the application did not evaluate are now blocked by the re-evaluation pass, whose
   pages write and may remove only `REEVALUATION` blocks; a block records the policy and reservation revisions, not the
   snapshot, and the deletion trigger guards only `ADVISORY_IMPORT` rows. A malware block on this path could be removed
   automatically, with no `script:block:release` and no audit, against Requirement 046-010 and ADR-0121 decision 6. Fix:
   key the trigger and the page's "never remove" rule on the malware reason, whatever the source; extend
   `reevaluation_never_removes_a_malware_block` to the late-writer path.
2. Nit. ADR-0121's "Enforced by" omits the six new tests, and the rejected row-lock option of Decision 046-078 is only in
   research.md.
3. Nit. The data model's quota row counts `RUNNING` jobs only while their version is `BUILDING`, against the design;
   "Advisory barrier" appears twice in the lock order.
4. Nit. The data model claims the snapshot lock's key space is the definer function's alone; `pg_advisory_xact_lock` is
   executable by `PUBLIC`. Fix: name the key as `hashtextextended('sagat:package_advisory_snapshot:' || ecosystem, 0)`,
   the repository's convention, and drop the isolation claim.

Counts: Blocker 0, Major 1, Nit 3

## The author's answers

1. Fixed where the rule is built, and settled for the design's wording by Owner decision 2026-11-24-313 (delegate): the
   design already states the invariant ("A malware block is released only through `DELETE /v1/admin/script-blocks/{id}`
   ... which a database trigger allows only when the advisory is withdrawn", Technical Plan 7); the data model's trigger
   and the page's rule are now keyed on the malware reason whatever the source, ADR-0121 says so, and
   `reevaluation_never_removes_a_malware_block` covers a block written by a re-evaluation page. The sentence of Technical
   Plan 6 that lists what a page may remove is restated by follow-up Task 046-27, before the build of Task 046-11.
2. Fixed in ADR-0121 (the tests, and considered option 7). 3. Fixed in the data model. 4. Fixed in the data model (the
   key named, the claim dropped). Owner decisions 2026-11-24-312 and 2026-11-24-313 are registered with the approval;
   Owner decision 2026-11-25-2 is cited with its pull request.
