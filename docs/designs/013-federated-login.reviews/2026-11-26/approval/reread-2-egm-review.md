# egm-review re-read of design 013 for approval, round 2 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: e1537935
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since round 1 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its round-1 findings closed and the lock orders, the 029 contract and decision 11 consistent.
Counts: Blocker 0, Major 2, Nit 4

1. **Major. Requirement 013-026, Technical Plan 12, Task 013-8.** The ring's rotation protocol needs per-key states and times (pending, retained, added, activated) that a `KeyRing` cannot hold; who drops retained keys is unsaid; no test finishes an interrupted rotation. Fix: define the stored document, the loader, the expiry, a test.
2. **Major. Requirement 013-026, Technical Plan 4.** The whole refresh protocol sits in one parenthetical of one sentence, and Technical Plan 4 defers back to it. Fix: a Technical Plan 4 subsection "Refresh tokens" with one paragraph per concern, the ring's lifecycle in Technical Plan 12, the requirement keeping its MUSTs and a pointer.
3. **Nit. Technical Plan 11.** `REFRESH_RATE_LIMITED` is missing from the list of reasons, and the body of the token endpoint's `429` and `503` is unstated. Fix: add it; the RFC 6749 form with `temporarily_unavailable` and `error_reason`.
4. **Nit. Technical Plan 6.** The superseding deletion of older per-member rows also deletes guarded rows. Fix: exempt them.
5. **Nit. Technical Plan 3.** The guarded marker and the administrator count are not columns; `identity_connection_health` has no table row. Fix: add them.
6. **Nit. Requirement 013-026, Task 013-8.** The shared `derive_purpose_key` signature conflicts with design 050's. Fix: one signature.
