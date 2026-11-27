# egm-review, Stage 9 (codebase details lens) of design 013 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 077e1473
Reviewer: egm-review (Stage 9, egm-codebase-details)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author to one paragraph per finding, every severity, location and fix kept. The reviewer confirmed some thirty claims, among them the Problem's description of `AuthConfig`, `TokenVerifier` and `authenticate`, the membership columns, the Studio's sign-in, the dev-idp, the leeway and cache settings, the five `Core` implementations, the twelve caller families, the registry's capability and heartbeat, `KeyRing::parse`, `EgressClient`, `Who` and `StreamLimits`, the transfer bounds, `BROWSER_SUITES`, the `x-planned` and `not_built` lists, and the toolchain.
Counts: Blocker 2, Major 8, Nit 12

1. **Blocker. Technical Plan 13, Requirement 013-016, Task 013-5, the architecture rule.** A third plain read of `memberships` exists (`reveal_grants.rs:89-96`, `step_log_usage_if_revealed`); the rule would fail on it, or it would decide on a suspended member's row. Fix: name it, route it through `sagat_active_membership`, test its caller family.
2. **Blocker. Technical Plan 4 "Test sign-in and link", CUJ 1, Technical Plan 13.** The Studio sends `Cross-Origin-Opener-Policy: same-origin` on every response (`studio.rs:232-249`), so after the trip through the provider the small window cannot `postMessage` its opener; no confirmation, no `TESTED`, no connection ever activated. Fix: a `BroadcastChannel` or a bearer poll of `GET /v1/auth/links/{attempt}`, recorded against `studio.rs`.
3. **Major. Technical Plan 11 "Alerts", Tasks 013-4, 013-13, Requirement 013-021.** `AlertKind` alerts are namespace-scoped (a non-null `namespace_id`), detected and resolved by the monitor for every kind, enumerated in three contracts and anchored in the runbook test; a tenant alert has no namespace, the monitor would resolve it at once, and break-glass is an event, not a condition. Fix: a namespace-less scope with all its parts, or audit events and metric rules.
4. **Major. Technical Plan 4 "Verify", Task 013-10.** `CountingStore` (`tests/support/api.rs:94-118`) counts the two calls `resolve_caller` replaces, which throttling tests read; `FakeStore` implements the trait. Fix: name and update both.
5. **Major. Requirement 013-007, Technical Plan 2, Task 013-8.** `JwksCache::usable_key` (`jwks.rs:148-158`) keeps RS256 and ES256 only and pins an RSA key without `alg` to RS256, so PS256 providers are refused. Fix: keep the declared `alg`; an explicit rule for RSA keys without one; a unit test.
6. **Major. Technical Plans 3 and 6, Task 013-5.** The classification manifest (`storage-classification.json`) and its test require ALWAYS quota triggers on `content` tables; `runtime_privileges` and the manifest test are in other targets; "the tenant quota for members" names nothing. Fix: name the manifest and triggers, the two targets, `TenantContentBytes` and the answer when exceeded.
7. **Major. Task 013-5.** The migration anchor is stale: migrations up to `20270210000100_step_logs_validate` exist. Fix: after the latest when the task starts.
8. **Major. Requirement 013-033, Technical Plan 9, Task 013-3.** "The existing image check" does not exist; only the Dockerfile's separate target keeps the dev-idp out. Fix: the test is new; say what it inspects and where it runs.
9. **Major. Task 013-6.** `jwks.rs` has no test module; the cache is covered by `fault_platform_foundation` and `contract_platform_foundation`. Fix: name them.
10. **Major. Task 013-15.** `sagat doctor` sends no token to the engine (`doctor.rs:266-294`). Fix: only `worker dev` refuses a key-store token.
11. **Nit.** `config.json` and the policy are built once at start-up (`studio.rs:120-149`), and the entries drop `scope`. Fix: per request; add `scope`.
12. **Nit.** `VerifiedCaller` lacks `iat`, needed by the break-glass key. Fix: add it; both verifiers fill `jti` and `iat`.
13. **Nit.** Task 013-6 mixes `HttpJwksFetcher` and `JwksCache` users. Fix: list both and the re-exports.
14. **Nit.** Verification filters miss tests (`identity_service`, `egress`, the battery's target name). Fix: name them.
15. **Nit.** No `Slot` exists between `Receipt` and `Authority`. Fix: a new variant.
16. **Nit.** The key-store account needs a slug ordinary commands lack. Fix: where it comes from.
17. **Nit.** The router's 64 KiB body limit refuses SAML and SCIM bodies first. Fix: per-route limits.
18. **Nit.** Architecture decision 0101 is not in this tree. Fix: cite pull request #267.
19. **Nit.** Reuse unnamed: the reveal-grant trigger, `tests/support/canary.rs`, `tests/support/oidc.rs`, the collapsing audit helpers, the `Guard` trait.
20. **Nit.** `ON DELETE SET NULL` on a composite tenant key would null `tenant_id`. Fix: the column-list form.
21. **Nit.** The `NOT VALID` pattern has a separate validate migration. Fix: say so.
22. **Nit.** Feature 029's group-source contract still names the silence alert and an unstated `lock_timeout`. Fix: amend it; state the timeout.
