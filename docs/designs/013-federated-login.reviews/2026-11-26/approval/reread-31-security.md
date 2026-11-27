# Targeted security re-read of design 013 for approval, round 31 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: a2d90aa882afd6350d806390a1cfada602d28448
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-30 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found no gap between check and use, the development gate sound and every ring and identity-mount operation covered by the per-request check.
Counts: Blocker 0, Major 3, Nit 0

1. **Major. Technical Plan 12, *Transport*.** "Exactly `identity` and `default`" does not match the bootstraps, which install policies as `sagat-<name>` and issue tokens with `token_no_default_policy`; following them ships a token the check refuses, and the fix that passes the test, adding `default`, widens the most sensitive token. Fix: require exactly `["sagat-identity"]` with no `default`, and state that the identity role uses `token_no_default_policy`.
2. **Major. Technical Plan 12, *Transport*.** The check reads lookup-self's `policies`, but entity and group policies appear in `identity_policies` and are enforced too, so an entity carrying `sagat-management` passes and one token file serves both clients. Fix: `identity_policies` and `external_namespace_policies` absent or empty; a test case with a token whose entity carries the secrets policy.
3. **Major. Technical Plan 12, *Transport*.** The secrets-mount client keeps a rule that refuses only `root`, so an engine or management token that also carries the identity policy is accepted and reads the rings directly. Fix: the secrets client refuses a token whose `policies` or `identity_policies` include the identity policy; a test case.
