# Approval confirmation of design 052: security (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d770bb0c159f12d11e62f45cc4f327cb95a920df
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`, at d734e026, 2
Majors and 5 Nits), then the fixes re-read in `reread-1-security.md` to `reread-6-security.md`, every finding of each
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 6's Nit is resolved: `scripts/check-chart-wake.py` is a step of the `documentation` job, whose only condition is
a pull request, so a chart-only pull request runs both chart checks once the step lands; the checker fails rather than
skips without `helm`; until constitution 5.1.0 admits the step (Task 052-10), it runs in the `architecture` job and at
every full run and release, so no release goes out unchecked. Remark, not graded: Task 052-6 is not ticked while the
step is still in its interim place. The earlier conclusions stand across the poll's bound and spread, the wake's
registry, permits and publish after commit, the broker's credentials and permissions, the chart's refusal and a
session revoked while parked. Gap 052-G1 is outside this design's scope and handed to feature 004 (Task 052-12).

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
