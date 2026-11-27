Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: e1d438b0ff4e17994feb29517a4d0cd2582b1349
Reviewer: architecture (repository agent `.claude/agents/architecture-reviewer.md`, round 2)
Agent-blob: architecture-reviewer
Counts: Blocker 1, Major 11, Nit 2

Condensed by the author; every finding and severity kept. Round 1: answered 2 to 4, 7, 9 to 13; in part 5, 6, 8; not answered 1 (finding 1 below) and 14.

1. **Blocker.** The Principle V wording is not ready: it says "lease-owning executor" without choosing bearer or session binding (the two existing sets differ); it omits membership, binding and pool-delivery checks; the replacement for "Both paths" is not drafted; feature 021 records no dependency on it. Fix: the full text including every path; a binding model per executor; the dependency in feature 021's research.
2. **Major.** ADR-0055 clause 11(e) admits only RFC 6749 grants and says scripts and activities use declared-secret delivery; the design adds RFC 7009 revocation, a userinfo call and a new gRPC method without amending it.
3. **Major.** Operator routes under `/v1/admin` on the tenant API is the option ADR-0074 rejects; blocks and lifts are not gated on ADR-0074; index and feed blocks have no writer. Fix: ADR-0074's listener; a definer function for the runtime role; until then a schema-owner command.
4. **Major.** The rollback story contradicts ADR-0082 (a binary below a recorded requirement serves nothing); an old control plane answers `UNIMPLEMENTED`; an old scheduler does not gate by protocol minor. Fix: rollback requires removing the requirement first; SDKs and shims map `UNIMPLEMENTED` to a retryable `CAPABILITY_UNAVAILABLE`.
5. **Major.** Keys cannot work: a nullable `tenant_id` in a primary key; `oauth_provider_versions` without tenant or scope. Fix: separate deployment tables or a non-null scope key carried into children.
6. **Major.** Feature 028's `UNIQUE (tenant_id, namespace_id, actor, request_key)` with a nullable namespace lets two tenant-wide imports with one key both insert. Fix: `NULLS NOT DISTINCT`.
7. **Major.** The hub routes duplicate feature 028's and skip its permissions for workflow and script drafts. Fix: extend feature 028's routes; check each part's permission.
8. **Major.** Only `RESOURCE_REAUTHORIZATION_REQUIRED` is added to feature 021's list, not `OAUTH_SCOPE_INSUFFICIENT` and `RESOURCE_ASK_LIMIT`; feature 021's runner contract maps credential failures only to `RESOURCE_UNAVAILABLE`.
9. **Major.** Provider rate limits counted per process and per registration; one tenant can take every slot of a shared registration. Fix: fleet-wide counts; a per-tenant share; a starvation test.
10. **Major.** Waiters, deadlines and passes in `crates/application` need `tokio`, which the architecture check refuses there. Fix: a sleeper port, or loops in `apps/control-plane`.
11. **Major.** The checks do not fail when broken: `grant_type=` misses form tuples; the `OAuthVault` rule fails on its own implementation; nothing checks that no transaction is open during a provider call. Fix: the string-literal scan, an allow-list, a `pg_stat_activity` test; list them in ADR-0122.
12. **Major.** ADR-0122's Context, option 2 and Consequences describe the withdrawn scheme and the dropped pass; Gap 047-005 stale; ADR-0123 refuses wildcard hosts while its decision 2 allows a pattern.
13. **Nit.** No task provisions the broker's OpenBao policy and token in compose, chart and configuration.
14. **Nit.** Who removes `oauth_delivery_asks` rows; provider imports while the capability is off; `sdks/SPEC.md` for the limit class.
