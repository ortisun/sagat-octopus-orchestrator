Fresh Astra/xhigh read-only first-test review: three actionable findings.
1. Single-iteration refusal_state snapshot fails with SQLSTATE21000 on the real two-item source.
2. CAS checks only on ACTIVE rows fail to discriminate an incorrectly early inactive check.
3. Member finished_at existence does not prove exact shared result/audit/receipt time.
Root replaced only the new module snapshot with ordered multi-row owner-source aggregation
and an independent Run/journal hash, added allthree typed stale-CAS refusals on the same inactive
fixture, and moved exact member state/time comparison into common positive linked-effect oracle.
No production implementation defect or runtime test result was claimed by this review.
