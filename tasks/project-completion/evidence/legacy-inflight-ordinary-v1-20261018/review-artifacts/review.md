# Bounded review and reconciliation

Plan/draft implementation used GPT-6 Astra / GPT-5.6 Sol; consequential fresh reviews used GPT-6 Astra. Root alone applied changes and executed commands. Optional external review was declined by the user earlier; no further external reviewer was invoked.

C1 required fixes: protobuf/output equality must not dump capabilities or bodies; ScopePathSegment uses scope_node_id; receipt checks require full independently reconstructed scope/fingerprint/resource/accepted-command values. These were actionable and corrected in C2. The C2 fresh delta review returned None.

Final C3 names the two large query tuple types and uses a fixed-message boolean for native journal verification, whose error could otherwise print RunState. Third fresh delta review returned None. Root then formatted and removed an unused import and constant assertion after the retained preflight strict Clippy failure. The actual effective-total SQL oracle remains.

The first real coordinator ran the immutable-old producer successfully, then current consumer failed before migration/report at an incorrect fixture assertion requiring tenant_alias=acme on TaskLease. Both frozen-old and current crates/api-grpc/src/service.rs deliberately set tenant_alias to empty: authenticated worker identity supplies the tenant. Root corrected only that assertion to require empty; receipt tenant and original authenticated identity checks remain. This is a test-contract failure, not an engine compatibility regression. Separate source pins and logs retain both attempts.

The wire-contract correction passed original authority, migration, exact success/refusal/replay and normal stopped-limit drain checks, then failed at its later pre-compensation status assertion: actual COMPENSATING, fixture expected CANCELLING. The domain LiveSummary::status prioritizes active compensation over stopping; state.rs scope-saga reconstruction uses SagaPhase::Compensating. Root corrected the Run status assertion and strengthened saga phase from not-DONE to exact COMPENSATING. No engine change. The failed attempt and distinct pins remain.
