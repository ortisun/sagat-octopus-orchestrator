# egm-review for the approval of design 052 (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d770bb0c159f12d11e62f45cc4f327cb95a920df
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The approval follows Owner decision 2026-11-25-2: one full round by every required reviewer,
then the fixes re-read, then the remaining findings settled in writing. The full round of the EGM lenses ran on the
second draft at 2348509a (`zoom_out_report.md`, `critic_report.md`), whose findings narrowed the design (the queue and
the next-step lease deferred behind measured thresholds), and on the narrowed text at d734e026
(`comprehension_report.md`, `implementation_readiness_report.md`) with the repository's architecture, security and
correctness reviewers (`*-full.md`). Every Blocker of every lens was fixed. Every lens re-read the fixes
(`reread-1-*.md`); egm-implementation-readiness confirmed at this commit with no Blocker or Major
(`reread-2-implementation-readiness.md`), and egm-comprehension confirmed at this commit with its re-read's Major
resolved and one new Major, below. The Nits below are the minors left open at this commit.

## Major (approval confirmation, settled)

1. egm-comprehension, approval confirmation at this commit, Major 1: the wake payload's length rule says two things
   (Requirement 052-054: not exactly 16 bytes is malformed; the Technical Plan's Compatibility paragraph: bytes after the
   16th ignored). Settled: Requirement 052-054's rule governs, as the security re-reads confirmed it; follow-up Task
   052-13 (a) corrects the Compatibility paragraph and adds a long payload's test.

## Nit (settled)

2. egm-zoom-out, `reread-1-zoom-out.md`, Nit 3: the gate's ten-pair fallback written out in the requirement; kept, its
   arithmetic checked by the comprehension and correctness re-reads.
3. egm-implementation-readiness, approval confirmation, Nits 1 to 3 (the test broker, the development profile and the
   chart's wake value; the barrier test's moment; the chart checker named as "the chart's contract test", the new
   target created twice).
4. egm-comprehension, approval confirmation, Nits 1 to 4 (CUJ 1's wording; "less than" against "at most" `len / n`;
   whether an error resets the run of 8; a test listed under the wrong task in ADR-0134).
5. The Nits of the repository reviewers' approval confirmations at this commit (`approval-correctness.md` Nits 1 to 4,
   `approval-architecture.md` Nits 1 to 3).

Counts: Blocker 0, Major 1, Nit 4

Findings 1 to 5, and `approval-correctness.md`'s Major, are recorded as design 052's follow-up Task 052-13, added after
the approval, each built test first. None is a security finding: every finding of every security report of this
approval run is fixed. Owner decisions 2026-11-24-527 to 2026-11-24-529 record the settlements.
