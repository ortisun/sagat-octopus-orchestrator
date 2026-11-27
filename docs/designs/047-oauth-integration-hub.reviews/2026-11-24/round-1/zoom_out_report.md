Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: egm-zoom-out (egm-review stage 5)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 6, Nit 4

1. **Major.** Two features that do not need each other (a broker and a content hub), and ten provider integrations as a release requirement. Fix: split the hub, or cut Requirement 047-022 to two or three samples.
2. **Major.** Feature 021 is described as before its 2026-11-24 revision: 021 now has an `oauth` scheme, an `oauth_credential` slot and `OAUTH_BROKER_UNAVAILABLE`. Use 021's names and code.
3. **Major.** Four ways to keep a token fresh; the fenced refresh at delivery suffices; the proactive access-token pass is a premature optimisation. Keep only the keep-alive refresh for providers that expire idle refresh tokens.
4. **Major.** Operator-offered registrations are a second, cross-tenant ownership model no core journey needs. Defer; keep the `scope` column.
5. **Major.** Feature 021's type-version catalogue already has a provenance field (`platform` or `hub`); a separate tenant table risks a second catalogue. Extend 021's.
6. **Major.** The repository's constitution is 4.7.0 (a connector marketplace out of scope); the hub's legitimacy rests on 5.0.0; the Principle V condition is left to someone else. Quote 5.0.0; settle the condition.
7. **Nit.** `ResolveResources` is one letter from 021's `ResolveResource`; rename and state why it is separate.
8. **Nit.** The `INTEGRATION` kind's atomic import is unjustified; a manifest of parts would do.
9. **Nit.** SQL, file maps and constants in the design hide the invariants.
10. **Nit.** "Whichever is built first" leaves the index code without an owner; `template_*` tables reused for non-templates.
