# egm-review for the approval of design 046 (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cd82af9376ce4a8d40196f11541a3ca1ac754207
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The complete runs of the five stages of `egm-review` are rounds 1 and 2 (2026-11-24, at
`cfb22df5` and `c00d7999`; their stage reports beside this folder under `2026-11-24/`), each finding answered in that
round's `resolution.md`. Round 3 (at `4c35bed4`) was the repository reviewers'. On the approval rule of Owner decision
2026-11-25-2 (pull request 284: Blockers and security findings always fixed, Majors fixed or settled, a short re-read
of the fixes, the rest settled in writing), the fixes after round 3 were re-read by the security, architecture and
correctness reviewers in this folder, and by the EGM implementation-readiness lens at the reviewed commit, whose output
follows, condensed by the author with every finding and severity kept.

## The implementation-readiness re-read (egm-implementation-readiness)

The reviewer read the design at the reviewed commit, `specs/046-script-dependencies/` and the round-2 readiness report;
round-2 items 3, 4, 5, 7, 8 and 10 are fixed; items 2 and 6 are partly fixed and return as findings 4 and 1.

1. Major. The test-run path: `script_test_runs` gains only `manifest`, so no column holds a test run's lock, digest or
   report; blocks are keyed by artifact and a test run has none, so the test-run runnable check has no definition; no
   builder-B task covers the runner side of a test run with packages.
2. Major. An npm document over the cap answers `502` and fails `RESOLUTION_FAILED` (Technical Plan 3), but any forwarded
   `502` fails the job `PACKAGE_GATEWAY_UNAVAILABLE`, retryable (Technical Plan 4, `errors.md`); the runner cannot tell
   them apart. Fix: a distinct status and code.
3. Major. The block pass, the advisory refresh pass and the re-evaluation pass have no interval, lease or owning task;
   "no registered target has a layer record for 10 minutes" defines neither the check's owner, its start nor a registered
   target; the image-digest list at registration has no route.
4. Major. Operator and read surfaces are missing from the contracts or the tasks: the forced import, the start of the
   backfill pass, the removal of an offline-bundle pin, the two `control-plane` commands, and the packages and SBOM
   reads of a version.
5. Major. Task 046-5 is stale against the data model's lock protocol (a writer "held open with its `FOR SHARE`", "30 s");
   the data model cites follow-up Task 046-28, which the design does not list.
6. Nit. Decisions 046-009, 046-017, 046-026, 046-028, 046-038, 046-042, 046-045, 046-065 and 046-074 conflict with the
   design and carry no superseding note.
7. Nit. Contract leftovers: the materialization row's `DONE` and expiry, `compileFailures`, the 300 s resolve grant in
   `package-gateway.md`, "a row the block pass reads", the author lock's fetch-budget charge.
8. Nit. Task bookkeeping: schedule rows for Tasks 046-1 and 046-26, owners of `scripts/` and `docs/`, `infra-blobstore`
   in builder B's crates, tests named twice or in no task, Task 046-14 before `fail_ready_step`, the fetch budget's phase.

Counts: Blocker 0, Major 5, Nit 3

## The author's answers

Each Major is settled in writing by a delegate row and fixed, documentation first, by a follow-up task of the design
before the build of the task it names: 1, Owner decision 2026-11-24-315 and Task 046-29; 2, Owner decision 2026-11-24-316
and Task 046-30; 3, Owner decision 2026-11-24-317 and Task 046-31; 4, Owner decision 2026-11-24-318 and Task 046-32; 5,
Owner decision 2026-11-24-319 and Task 046-28 (now listed, with Task 046-27). Nit 6 is fixed by the superseding notes
of Decision 046-079; Nits 7 and 8 by Task 046-33. Why settled and not fixed in the text before approval: each is a gap of
the plan's completeness, not of its safety; every security finding of the re-reads was fixed in the text, and a further
round of text changes would restart the re-reads for findings a builder meets as a written task before its code.
