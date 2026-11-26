# egm-review for the approval of design 044 (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 14513c8533b0a0a475d77fe21f3886c4e07c52cc
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The last complete run of `egm-review` lenses is round 11 (2026-11-25, at `9bdb707d`; its
reports in `../../2026-11-25/round-11/`). On the approval path of OD-2026-11-25-2, the fixes of round 11 were re-read
once by every reviewer whose findings they answer, one at a time, each Major of that re-read fixed (`reread-1-*.md`
beside this file); the security reviewer re-read until a pass found no Major. The EGM design and
implementation-readiness lenses then confirmed the text changed since their re-read at the final commit
(`approval-egm-design.md`, `approval-egm-readiness.md`); this summary lists their open findings in one numbered list.
The Majors are settled in writing under OD-2026-11-24-4 (`docs/owner-decisions.md`, rows OD-2026-11-26-441 to
OD-2026-11-26-459, one per finding), each answered by a follow-up task of the design (Task 044-34 to Task 044-38).

## Major

1. (design lens 1) Reusing an earlier lease's command-value blob contradicts the ownership rule; a cold worker cannot
   name it; a swept blob has no cause.
2. (design lens 2) The tenant and namespace command-value ceiling can stall a namespace for up to 24 hours, silently.
3. (design lens 3) The run's 32 reviews a run for a condition only the 24-hour sweep clears.
4. (design lens 4) `tenant_share` refusals count although sibling runs cause them.
5. (design lens 5) The SDK cannot apply the carried-state bound for an `active` successor; the server's refusal is unstated.
6. (design lens 6) A verifier session no longer live has no defined item outcome.
7. (readiness 1) A run at its 32-blob ceiling has no specified way out.
8. (readiness 2) The item's outcome after a verifier session is no longer live is not stated.

## Nit

9. (design lens 7) The `InlineValuesExceeded` catalog row is out of date.
10. (design lens 8) `run.stop_requested` has no `HistoryEvent` body for `CancelOrigin`.
11. (design lens 9; readiness 7) The new setting should end in `_SECS` and be listed in `alerts.md`.
12. (design lens 10) The register is out of date for the ceilings and the session rule.
13. (design lens 11) "Consecutive" uncounted expiries; the order of failed update results.
14. (readiness 3) New tests sit in Task 044-24 while their behaviour is built in earlier tasks.
15. (readiness 4) Task 044-9's file lists for the blob changes.
16. (readiness 5) The `retry-after-ms` of the two ceiling refusals.
17. (readiness 6) Precedence when two refusal reasons hold at once.
18. (readiness 8) A split table row in section 13.1.
19. (readiness 9) The union classification of a child's output has no implementing task.

Counts: Blocker 0, Major 8, Nit 11
