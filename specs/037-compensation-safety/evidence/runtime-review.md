# Delivery-1 independent runtime review

Status: corrective verification in progress, not merge approval. Fresh-context R1 reviewed
production diff against feature 037; a different fresh-context reviewer reviewed the correction
in R2 and a bounded R3 follow-up. These are code reviews, not EGM design-review execution.
The owner selected continuing this review and tests instead of another-model CLI review.

| Finding | Classification and evidence | Resolution | Remaining validation |
|---|---|---|---|
| A stopped original can create a lookup that stays Pending | Actionable, high; root and scoped domain RED tests | Release the declared recovery lookup during stop, retaining ordinary forward fences | Composed PostgreSQL suite |
| Scoped tenant-content fallback replaces known success/uncertainty with storage failure | Actionable, high; real quota RED returned WAITING for successful report | Reacquire authority, original receipt/fence and scanned secret revisions; prepare first stop and retain source outcome in one transaction without payload | Quota matrix, secret/authority regressions |
| Worker can submit accepted reserved uncertainty codes with retryable=true and get RETRY_WAIT | Actionable, medium; gRPC root and compensation matrices RED on RECONCILIATION_NOT_FINAL | Use the same leaves_outcome_unknown taxonomy as operator/continuation policy; keep the exact source code | Focused GREEN 2; root composed gate pending |
| Forged compensation plan can include a succeeded derived lookup ID | Actionable narrow guard, medium; root/scoped RED | Reject derived IDs in candidates and irreversible lists before mutation, retaining Succeeded requirement | Focused GREEN 2; full domain rerun pending |
| Definition-less reducer cannot prove a declared ID belongs to the saga region | Existing limitation; reducer has no definition and normal planner is definition-aware | No speculative state/framework added; not claimed solved by derived-ID guard | Future invariant redesign would need separate evidence/scope |

## R2 correction

The first content-quota correction incorrectly invoked root stop settlement while a collection
was still open. Independent review identified UnsupportedStructuredNode and the diagnostic
run confirmed that exact error. The correction now uses prepare_scoped_content_refusal to
validate live scoped ancestry/fence and record only the first stop. The structured scheduler
retains drain ownership; addressed completion/failure and receipt remain in the same transaction.

R3 static review found the identified defect resolved and no new actionable correctness or
security issue in the bounded correction. It did not run builds/tests and does not establish
runtime acceptance. Source-preserving PostgreSQL and final composed gates remain decisive.

## Disagreements and test corrections

The quota test initially expected an empty routing event for declared forward success under
stop. Reading plan_addressed established that only derived completions retain that empty
route; a stopped declared forward node stays unrouted. The corrected assertion requires
known success, no retained output, no forward/error route and an unchanged first stop.
An ordinary failure report adds no retained payload, so the unknown-quota arm explicitly
injects the exact PostgreSQL constraint once; it is not presented as real content growth.

No review certifies a provider, financial settlement, performance SLO, US04 or the overall reset.

## Resumed verification checkpoint

The current tree passed 964 domain/application/telemetry tests, 81 saga integrations,
9 gRPC contracts, 22 continuation tests, 20 scoped-worker tests, 21 scoped HTTP tests
and 10 root quota tests. The named quota fallback also passed again after its test
evidence row was simplified for Clippy. These verify the four bounded runtime fixes
above; they do not certify the full security/CI matrix. Exact commands and outputs are
linked from validation-report.md under “Resumed defect closure”.
