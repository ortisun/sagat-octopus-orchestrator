Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: egm-implementation-readiness (egm-review stage 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 13, Nit 3

1. **Major.** Task T047-1 is left to design the endpoints, the gRPC messages, the host call, the item schemas, the columns and the CLI. Put them in the plan.
2. **Major.** No task creates the hub tables or extends feature 028's; slice 3 omits `infra-postgres`.
3. **Major.** The PKCE verifier's sealing key, the account label's source, the cleanup of expired requests and the per-person limit are undefined.
4. **Major.** The waiter's mechanism and the four thresholds are unresolved.
5. **Major.** The proactive pass's lease, cadence, batch and `last_used` writes are unspecified.
6. **Major.** Revocation retry policy and the driver of `DISCONNECTING` are undefined.
7. **Major.** If feature 021 ships first with client secrets in runner-visible slots, nothing migrates them.
8. **Major.** The re-ask budgets have no store across control planes.
9. **Major.** Task T047-15 names no crate for the IR field and the checks.
10. **Major.** Vague dependencies: 021's tasks unnamed; "whichever is built first"; required scopes read across slices.
11. **Major.** Missing acceptance tests (budgets, capability, upgrade and block, cross-site request, offers, `OAUTH_SCOPE_INSUFFICIENT`, bounds, test runs, rollback); tests not linked to tasks.
12. **Major.** The local OAuth test server and the Studio and CLI test tools are unnamed.
13. **Major.** The ten integrations' presets, scopes and hosts are unspecified.
14. **Nit.** Slice 3 omits `runner.rs`; the release signing step names no file.
15. **Nit.** Who retires revisions written before a failed activation.
16. **Nit.** Wrong `Refs:` on Tasks T047-20 and T047-21.
