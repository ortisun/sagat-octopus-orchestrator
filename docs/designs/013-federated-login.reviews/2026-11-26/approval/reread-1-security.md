# Targeted security re-read of design 013 for approval, round 1 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: db1bea08
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the changed text since 5f90d452 (Owner decision 2026-11-25-2, step 4), not a full review; condensed by the author, every severity and fix kept. The reviewer found the attempt sealing, the PS256 rule, the opener policy reasoning, the transfer bounds and the other changes sound, and disagreed with the Stage 8 resolution's "no security property is weakened" for findings 2 to 5.
Counts: Blocker 0, Major 5, Nit 4

1. **Major. Technical Plan 4 step 3, the route table.** The new poll `GET /v1/auth/links/{attempt}` has no authorization rule; the attempt id travels in the start address, the cookie name and the provider's `state`, so another bearer could read the verified provider identity. Fix: only the attempt's tenant and subject, `404` otherwise, `no-store`, battery entry, a test.
2. **Major. Requirements 013-010, 013-019, lock order, maintenance.** Raising the context only refuses until the loop marks the sessions; lowering it before then revives every cut-off session. The same holds for disable then re-enable. Fix: record the raised context and its time as a floor that the per-request check applies; a test that a lowering revives nothing.
3. **Major. Technical Plan 6, Constitution Check VII, the Overview.** Convergence only at the provider's next change means a retried reactivation without `If-Match`, overtaken by a deactivation, reactivates a leaver indefinitely. Fix: refuse, within 10 minutes of a suspension, a request without `If-Match` that would reactivate; a test.
4. **Major. Requirement 013-021, Break-glass, Alerts.** The tenant no longer learns of a break-glass use except from the audit trail; the operator's counter has no tenant. Fix: `breakGlassLastUsedAt` and a 30-day count on the identity settings; a test.
5. **Major. Requirement 013-007, Tasks 013-3, 013-4.** End-to-end proof of the ID-token checks was cut to three; wiring defects on the callback path (an `iss` compared with the discovery document, a pending revision's client identifier as `aud`) escape unit tests. Fix: one table-driven integration test through the callback with a claims override.
6. **Nit. The route table.** A same-origin `GET` carries no `Origin`, so the poll as tabled cannot be called. Fix: bearer and `X-Sagat-Renew` for the `GET`.
7. **Nit. Technical Plan 4, Requirement 013-001 (suspicion).** With `identity_revision` removed, the immutable columns are enforced only in the domain while the runtime role can update the table. Fix: a trigger or column grants, checked by `runtime_privileges`.
8. **Nit. Technical Plan 7 (suspicion).** "Read on each request" may mean a query per anonymous hit of `config.json`. Fix: the in-memory registry view.
9. **Nit. Architecture decision 0140 decision 9.** It still lists the capability only with federation configured, against the engine listing it always. Fix: update it.
