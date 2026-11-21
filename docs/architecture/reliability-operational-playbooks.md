# Catalog of operational scenarios and human actions

## Retry / reprocessing

- Step retry: requires a legal state (FAILED, RETRY_WAIT or controlled intervention), uses attempt+1 and an idempotency key stable per external effect; does not replace the old journal.
- Saga compensation retry: creates a new attempt of the same compensation, preserves trail and reason.
- Rerun as new Run: new IDs and opt-in for side effect replay; default requires explicit confirmation and an idempotency policy.
- Skip compensation: `SKIP_COMPENSATION` requires `saga:override`, a reason and a legal compensation state. The current command does not implement a general `skippable` node attribute or a financial-operation deny flag. Those additional restrictions remain a policy proposal, not a safeguard operators can assume. See [saga operations](../runbooks/sagas.md) and [operation authorization](../../crates/api-http/src/routes/execution.rs).

## Incident response order

1. Identify the affected tenant/namespace, Run and version.
2. Inspect persisted events and possible ambiguous side effects.
3. Apply quarantine/drain of the worker pool or version if the incident is systemic.
4. If the external provider has an uncertain return, prioritize reconcile over executing again.
5. Perform authorized intervention, write journal, verify the new attempt and monitor compensations.
6. Publish postmortem with timeline, blast radius, fix, regression test and ADR if behavior changed.

## Manual action restrictions

Interventions require the operation-specific permission, a legal state, a reason and an audited
decision; historical events are not edited. The inspected operational endpoints do not implement
step-up MFA or collect two approvals for payment interventions. An identity provider may enforce
MFA externally, but the deployment must establish that policy; workflow approval nodes do not
automatically protect these endpoints. Configurable dual approval for operational payments is a
proposal requiring a product/security decision and validation, not a delivered guarantee.

## Compensation versus rollback

Version rollback: changes routing of **new** Runs; does not undo completed payments. Compensation: attempts a new business action. Retry: repeats the physical attempt with safeguards. Reconciliation: queries external state to decide. The Studio must distinguish these actions with unambiguous labels.
