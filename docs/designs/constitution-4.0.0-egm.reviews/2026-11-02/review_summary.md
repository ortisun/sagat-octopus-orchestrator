# Review summary: constitution-4.0.0-egm

**Status of the design doc after this review (2026-11-02): `Draft`. Not approved.**

The owner concluded the review after stage 9 ("Conclude review and finalize document"). Concluding does not approve the
doc. Under the approval rule of the doc itself (Technical Plan section 3), a doc is `Approved` only when `egm-review`
and the three repository reviewers each end with no Blocker or Major finding open. That condition is not met.

## What ran

| Stage | Text audited | Reviewers | Blocker | Major | Nit | Report |
|---|---|---|---:|---:|---:|---|
| 5 Zoom Out | commit `0bc7c03b` (880 lines) | `egm-zoom-out` | 0 | 6 | 6 | `zoom_out_report.md` |
| 6 Comprehension | commit `0bc7c03b` | `egm-comprehension` | 0 | 16 | 7 | `comprehension_report.md` |
| 7 Critic, round 1 | commit `0bc7c03b`, lines 1-590 | `egm-design`, `egm-structural`, `egm-style` | 1 | 24 | 27 | `critic_report.md` (52 entries from 69 raw findings) |
| Resolution of stages 5-7 | commit `0bc7c03b` to `87dbb02e` | the owner chose "auto accept" | | | | `goldfish_review_report.md` |
| 8 Intermediate Zoom Out | commit `87dbb02e` (1,216 lines) | `egm-zoom-out` | 0 | 6 | 11 | `intermediate_zoom_out_report.md` |
| 9 Implementation Readiness, round 1 | commit `87dbb02e`, lines 908-1216 | `egm-implementation-readiness`, `egm-codebase-details`, `egm-style` | 8 | 31 | 19 | `implementation_readiness_report.md` |

The three repository reviewers (`architecture-reviewer`, `security-reviewer`, `correctness-reviewer`) were **not run**.
The approval rule requires them, and they cannot run until their output contract is amended.

## Why the doc is not approved

*   **Open at the current text (commit `87dbb02e`):** 8 Blocker and 31 Major from stage 9, and 6 Major from stage 8.
    These are mostly defects in the rules that the resolution round added: the `Reviewed-blob` fallback, the findings
    footer, the `Owner-gate` pointer, the evidence rule, the duplicate-number rule, the backticked-path check, the
    scoped search of PR 23, and the transitional clause (all ten `speckit-*` skills read the constitution path that
    PR 2 moves).
*   **Not re-audited:** stages 5, 6 and 7 audited the text before the resolution round. Their reports describe text
    that was replaced. The resolution round applied 35 fixes in one rewrite and no stage 7 re-audit followed.
*   **Contested findings awaiting the owner's written acceptance:** 4 (`goldfish_review_report.md`, Issues 31, 32, 34, 35).
*   **Escalated to the owner, recorded as Open Questions Q2 to Q5 of the doc:** branch protection, a document-level
    scope gate, when the Spec Kit tooling is removed, and the version number. Q1 (the unadopted constitution
    candidate) is also open.

## Decisions for the owner

*   **Patch.** Classify the 75 open findings of stages 8 and 9 with `egm-resolve`, rewrite again, and re-run stages 7
    to 9. Every added rule has so far produced new findings when coded against the real files.
*   **Reduce the scope first.** Remove the rules that were not owner decisions and cause most of the Blockers (the
    blob fallback, the footer parsing and count equality, the append-only `research.md` check, the amendment formatting
    exception, the strict forward-only evidence rule), or split the work: this design keeps the constitution
    pull request, the minimal tooling and the removal, and the conversion of the 20 features becomes a follow-up design
    after the first conversion is measured. The conversion wave is an owner decision, so this one is the owner's to take.
*   **Stop here.** Nothing is implemented.

## Not done

Nothing is implemented, nothing is deleted, and nothing is pushed. The Spec Kit tooling is untouched. The owner's
decisions of 2026-11-02 are in `tasks/reviews/2026-11-02-egm-constitution/concepts_brief.md` and are not yet in
`docs/owner-decisions.md` (plan item 25 puts them there in PR 2).
