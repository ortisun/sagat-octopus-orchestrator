# Catalog of operational scenarios and human actions

## Retry / reprocessing

- Step retry: requires a legal state (FAILED, RETRY_WAIT or controlled intervention), uses attempt+1 and an idempotency key stable per external effect; does not replace the old journal.
- Saga compensation retry: creates a new attempt of the same compensation, preserves trail and reason.
- Rerun as new Run: new IDs and opt-in for side effect replay; default requires explicit confirmation and an idempotency policy.
- Skip: allowed only on nodes classified `skippable` and with justification/authorization; for financial actions it is blocked by default.

## Incident response order

1. Identify the affected tenant/namespace, Run and version.
2. Inspect persisted events and possible ambiguous side effects.
3. Apply quarantine/drain of the worker pool or version if the incident is systemic.
4. If the external provider has an uncertain return, prioritize reconcile over executing again.
5. Perform authorized intervention, write journal, verify the new attempt and monitor compensations.
6. Publish postmortem with timeline, blast radius, fix, regression test and ADR if behavior changed.

## Manual action restrictions

Only operators with specific RBAC; configurable MFA/dual approval for payments; mandatory reason; tamper-proof audit log at the logical level; never mutate/delete historical events.

## Compensation versus rollback

Version rollback: changes routing of **new** Runs; does not undo completed payments. Compensation: attempts a new business action. Retry: repeats the physical attempt with safeguards. Reconciliation: queries external state to decide. The Studio must distinguish these actions with unambiguous labels.
