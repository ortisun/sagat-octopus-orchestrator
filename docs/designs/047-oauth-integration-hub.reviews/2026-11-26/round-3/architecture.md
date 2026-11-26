# Targeted re-read of design 047 for approval: architecture (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 6a57e37db990c842ad1066f8bba1cf62f3e33d28
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A targeted re-read of the
changes since review round 2 (approval rule of Owner decision 2026-11-25-2), not a full review. The reviewer read the diff
since `e1d438b0`, the design, the companions, ADR-0122, ADR-0123, `docs/owner-decisions.md`, `docs/constitution.md` and
`scripts/check-architecture.py`, and ran nothing.

## Round-2 findings

1. Principle V wording: answered (Technical Plan 12's full wording, a binding per executor, the replacement for "Both
paths"; Decision 021-022); the approval condition is finding 2. 2. Clause 11(e): answered (ADR-0122 refines ADR-0055).
3. Operator routes: answered. 4. Rollback and `UNIMPLEMENTED`: answered. 5. Keys: answered (finding 3). 6. `NULLS NOT
DISTINCT`: answered. 7. Duplicate hub routes: answered. 8. Step codes: answered. 9. Fleet-wide and per-tenant limits:
answered. 10. Timing in the application: answered (finding 1 is related). 11. Checks that do not fail: answered (the
string-literal scan exists, `scripts/check-architecture.py:130`). 12. Stale ADRs: answered (finding 4). 13. OpenBao
provisioning: answered. 14. Nits: answered.

## New or remaining findings

1. Major (new). The `resource_delivery` class never waits: a call that finds a refresh running gets an immediate
   "`RESOURCE_ASK_LIMIT`-style refusal", a reason not in the closed list, and nothing says whether it uses up the two full
   calls; Requirement 047-010 still promises a wait of up to 10 s, which only the integration runner gets. After a long
   sleep (Journey 2) steps can spend their budget on these refusals. Fix: a closed reason (for example
   `CAPABILITY_UNAVAILABLE` with `REFRESH_IN_FLIGHT` and a retry delay) that counts against neither budget; reword
   Requirement 047-010 for the gRPC paths; a case in `resource_access`.
2. Major (remaining). Owner decision 2026-11-25-3 approved "the settings of a declared resource and its current access
   token"; Technical Plan 12's wording also discloses the secrets the slots bind, on which Requirement 047-015 and feature
   021's runner depend. If 5.1.0 is drafted from the decision as recorded, delivering slot secrets has no constitutional
   basis. Fix: make approval conditional on 5.1.0 including slot secrets as Technical Plan 12 words it, or have the owner
   confirm it; also replace "limited to two declared sets" (constitution line 156).
3. Nit. Window and broker-call tables use a `(registration_kind, registration_id)` pair with no foreign key and no
   `tenant_id` for tenant registrations; `oauth_deployment_offers` has a tenant column though deployment tables are said
   to have none; `oauth_deployment_registrations` points at the tenant provider versions.
4. Nit. Gap 047-005 is still "Open (measure)" for windows that no longer exist.
5. Nit. The lock order omits the window rows (credential, registration window, tenant window), for every broker call.
6. Nit. `OAUTH_PROVIDER_ERROR` sits under the retryable `UNAVAILABLE` but covers permanent answers; `SCRIPT_DENIED`'s
   origin is not named.
7. Nit. "Owner decision 2026-11-24-320" is not yet in `docs/owner-decisions.md`, and the same sentence accepts ADR-0122 on
   approval although ADR-0122 says it cannot be accepted before Principle V changes.

Counts: Blocker 0, Major 2, Nit 5

## The author's answers

1. Settled by Owner decision 2026-11-24-327 (delegate) and follow-up Task 047-24, before Task 047-15: a closed refusal
   `CAPABILITY_UNAVAILABLE`, reason `REFRESH_IN_FLIGHT`, with a retry delay and counted against neither budget, Requirement
   047-010 restated for the gRPC paths, and a `resource_access` case. Not fixed in the text now because the text is the
   one the security re-reads cleared; the task lands before any delivery code.
2. Settled by Owner decision 2026-11-24-328 (delegate), and reported to the coordinating session for the owner: the
   approval is conditional on constitution 5.1.0 merging with Technical Plan 12's wording, slot secrets included; if
   5.1.0 merges narrower (settings and the access token only), follow-up Task 047-25 narrows Requirement 047-015 and the
   delivery tasks to settings and access tokens before they are built. The delivery tasks carry
   `Owner-gate: OD-2026-11-25-3`. The constitution's line 156 is the 5.1.0 drafter's.
3. to 6. Fixed in the companions (Decision 047-068): window tables split by owner with foreign keys and their lock order;
   broker calls foreign-keyed; the deployment foreign key corrected; `oauth_deployment_offers` named as the one
   deployment table with a tenant column; Gap 047-005 closed; `OAUTH_PROVIDER_REFUSED` for permanent answers;
   `SCRIPT_DENIED` named as existing code. 7. Owner decision 2026-11-24-320 is registered with the approval; ADR-0122 is
   accepted conditionally on 5.1.0 merging, as its status line says.
