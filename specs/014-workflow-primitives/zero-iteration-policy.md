# Explicit zero iteration policy — bounded O701 delivery

Refs: O701, D703–D705, T014-014–017/040, FR-014-005/015.

A tenant may explicitly set iteration_concurrency=0 through the existing authorized quota
operation. This records a tenant policy (distinct from an absent/default policy), preserves
normal ceiling and tenant-scope checks and audits the change. Zero blocks fresh admission;
accepted members, their reserved work, durable waits and drain continue to completion. Raising
the policy permits remaining captured members to be admitted normally. Local maxConcurrency
still requires1..256; this change concerns only the tenant quota.

The current API validator and SQL policy CHECK require at least1. Change them together in an
additive migration, retaining the existing policy/run/usage lock order. No counter, column,
permission or group runtime is introduced. The existing native admission comparison already
handles zero; do not add a separate execution throttle or pause accepted work.

Downgrade must lock policy writers, refuse while any explicit zero policy exists, and then
restore the old positive constraint. Do not rewrite an applied migration or silently delete/
raise a zero policy. Zero configuration requires compatible readers during rollout; proving
actual released-profile upgrade/rollback remains a separate complete release gate.

Required evidence: a domain/API behavioral RED; accepted scoped timer progress while limit0
prevents the next member, exact quota source/usage/audit and later resumption; schema negative/
positive boundaries and retained-zero downgrade refusal followed by positive down/up. Existing
role, namespace, ceiling and positive-policy regressions must stay green. No nested capacity
activation or canonical task closure follows this bounded delivery.

Upgrade schema first, then enable zero-capable writers. The combined CHECK replacement takes
ACCESS EXCLUSIVE and validates the existing policy table; run it in the migration window because
policy reads/admissions can briefly wait across tenants. For rollback first fence/drain the
zero-capable quota writers, deploy the old validator, explicitly replace retained zero policies
with an owner-selected positive value or clear them, then migrate down. Leaving a new writer
after schema down makes its PUT0 incompatible with the restored constraint; table locking alone
is not mixed-writer compatibility. Actual released-profile execution remains a release gate.
