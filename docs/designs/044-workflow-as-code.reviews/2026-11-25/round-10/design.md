# egm-review Stage 7 design lens, targeted re-read, round 10 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 4a943e7c15d9ca73e9c1afa6274ad5da8cfb5cbb
Reviewer: egm-design, feature-spec standard, FeatureRange from the start to the Detailed Implementation Plan
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 9's findings,
3 to 14 are closed and 1 and 2 partly closed. Claims were checked against the blob migrations.

## Major

A. A continue-as-new input the SDK sends as a blob is refused (inline object required), so success depends on message
   size: accept a blob input in the handoff, or reserve its inline size and throw a typed error at the call.
B. Blobs written under a workflow lease have no storage model (`blobs_owner_valid` admits only step outputs and run
   inputs) and the deferral's hold cannot be built: a new purpose owned by the run and lease, commitment by the
   accepting completion, a hold for the latest deferral.
C. Commands the SDK issues itself (update decisions, the consumption, the set-state, scope aborts, the re-emitted
   deferral) can cross the batch limits with no call to throw at: reserve headroom or bound them separately.
D. A cancellation without grace after a graceful request is swallowed by the "later request" rule: a forced arm first.

## Nit

1. Requirement 044-008: a review ends on any cancellation.
2. Requirement 044-023 omits the `HISTORY_LIMIT` review case.
3. The cache's incremental read can exceed the 768 KiB bound.
4. The window's single counter in section 6.3's wording.
5. The continue-as-new suggestion for runs that cannot continue as new.
6. No statement budget for the workflow-task claim.

Counts: Blocker 0, Major 4, Nit 6
