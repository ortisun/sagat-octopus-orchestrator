# Targeted re-read of design 044 for approval: egm design lens, confirmation (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 14513c8533b0a0a475d77fe21f3886c4e07c52cc
Reviewer: egm-design, feature-spec standard, the text changed since c6d90019
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of the first pass's
findings (`reread-1-design.md`), 1, 2 and 5 to 10 are closed and 3 and 4 partly closed (carried as findings below).
This lens report is summarized in `approval-egm-review.md`.

## Major

1. Reusing an earlier lease's blob contradicts the ownership rule and a cold worker cannot name it; a swept blob has no cause.
2. The tenant and namespace command-value ceiling can stall a namespace for up to 24 hours, silently.
3. The run's 32 reviews a run for a condition only the 24-hour sweep clears.
4. `tenant_share` refusals count although sibling runs cause them.
5. The SDK cannot apply the carried-state bound for an `active` successor; the server's refusal is unstated.
6. A verifier session no longer live (every restart rotates its token) has no defined item outcome.

## Nit

7. The `InlineValuesExceeded` catalog row is out of date.
8. `run.stop_requested` has no `HistoryEvent` body for `CancelOrigin`.
9. The new setting should end in `_SECS`.
10. The register is out of date for the ceilings and the session rule.
11. "Consecutive" uncounted expiries; the order of failed update results.

Counts: Blocker 0, Major 6, Nit 5
