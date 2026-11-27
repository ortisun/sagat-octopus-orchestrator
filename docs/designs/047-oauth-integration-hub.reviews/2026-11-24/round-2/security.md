Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: e1d438b0ff4e17994feb29517a4d0cd2582b1349
Reviewer: security (repository agent `.claude/agents/security-reviewer.md`, round 2)
Agent-blob: security-reviewer
Counts: Blocker 1, Major 7, Nit 8

Condensed by the author; every finding and severity kept. Round 1: answered 1, 3 to 7, 11, 13, 14; not answered 2 (finding 5) and 8 (open until the wording merges; finding 3); in part 9, 10, 12.

1. **Blocker.** Nothing stops a client-credentials credential on an operator's deployment registration: a tenant that opted in gets an app-only token of the shared client, which can read what the app holds for every tenant. Fix: refuse client credentials on a deployment registration with a check constraint and a test.
2. **Major.** Delivery rechecks only the publisher's `resource:use`; missing the initiator's, the slot access rules and `resource:effect` (feature 021 checks all three); a raw token in a script can do any effect. Fix: check exactly what feature 021's resolution checks; declaring a resource requires `resource:effect`.
3. **Major.** The proposed Principle V wording omits the caller's membership, binding and pool-delivery checks, does not say whether delivery is bearer or bound to the executor's session, calls slot values "declared secrets", and authorises only use. Fix: name the checks and the binding model per executor; "the secrets its slots bind at the pinned revision"; the effect permission.
4. **Major.** Nothing ties a credential to destinations: an `http_api` resource naming another person's Google credential with a `baseUrl` of the attacker's host sends the bearer token there (RFC 9700 section 4.10). Fix: providers declare their API hosts; an `oauth_credential` slot binds only to resources within them; binding needs `oauth:credential:manage`, audited.
5. **Major.** A disconnect during a refresh: the holder drops a rotated token, or its pending row lands after the pass revoked the old token; the new token stays live and the outcome says `REVOKED`. Fix: a holder that loses the fence revokes the token it got; the pass sweeps pending rows of disconnecting credentials; a barrier test.
6. **Major.** Request limits are per registration only; one tenant on a shared client starves the others; per replica or in the database is unstated. Fix: a per-tenant share within the registration's limit, counted in PostgreSQL.
7. **Major.** The revocation feed has no version and no expiry: a replayed older feed drops a revocation; a blocked fetch voids the 24 hours. Fix: feature 028's rules for the feed; entries only add blocks; a staleness alarm.
8. **Major.** `oauth_provider_versions` has no `tenant_id` in its key; a registration's provider version cannot be held to its tenant. Fix: tenant and scope in the key, composite foreign keys, a cross-tenant test per route.
9. **Nit.** "A path inside the Studio" undefined for `return_path`.
10. **Nit.** The callback statement lacks a tenant predicate; the order of the `iss` and state checks; offer state not checked at callback and acquire.
11. **Nit.** `registration` must equal the credential's; reconnects with subject `none`; the `id_token`'s `iss` and `aud`.
12. **Nit.** A second OpenBao token file and its policy for the OAuth mount; a test that the ordinary token is refused there.
13. **Nit.** The role that writes hub blocks; grants on `oauth_passes`.
14. **Nit.** Superseded decisions and ADR text not marked (Decisions 047-005, 047-014, 047-016, 047-019; Gap 047-005; ADR-0122's Context and Consequences).
15. **Nit.** Parse rules for the bound directory identifier and `instance_url`.
16. **Nit.** A token without `expires_in` delivered until revoked; use a default lifetime or a ceiling.
