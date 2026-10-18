# Dormant leaf schema implementation review

Claim: the dormant immutable schema and lifetime/identity guards implement the bounded contract;
real constraints/privileges/refusals/DDL ordering are verified, without claiming source closure.
Fresh Astra/xhigh cycle1: valid/actionable. Duplicate identity also reused audit_id and could fail
only on audit uniqueness; add fresh audit ID. Namespace probe was cross-tenant only; add a second
same-tenant namespace. Source retention needed populated direct DELETE checks, not just names/
TRUNCATE CASCADE that could fail at a descendant. Added populated relational source fixtures,
exact rejection-table messages and retained-row counts for8sources; catalog independently binds
both exact guards to kind/function/ALWAYS. Owner rollback-only parent-guard bypass reaches an
actual ordinary-mode FK cascade, with retained checkpoint count afterward. No source proof claim.
Cycle2: valid/actionable. Existing chain Boundary::obstruct deleted its ghost row before reseed;
two repeated-blocker tests break under retention. Direct built-binary execution reproduced23001,
then root replaced DELETE/reinsert with ON CONFLICT update preserving the step identity. Both
existing tests pass afterward. An operations migration fixture already disables ALL step triggers
for its explicit privileged drain, so it remains compatible without changes.
Cycle3: no concrete issues in bounded SQL/privileges/guards/down and fixture implementation.
No test run by reviewers. Three-cycle stop at no findings. Root sole writer/test executor;
user retains current review, no external CLI. No adapter/source/producer/nesting acceptance.
