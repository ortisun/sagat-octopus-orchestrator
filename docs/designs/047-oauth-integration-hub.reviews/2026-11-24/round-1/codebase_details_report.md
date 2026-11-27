Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: egm-codebase-details (egm-review stage 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 3, Major 10, Nit 3

1. **Blocker.** The OpenBao paths are not feature 017's: the real path is `<mount>/<endpoint>/sagat/credentials/<tenant>/<namespace>/<operation uuid>` (`crates/infra-secrets/src/openbao.rs`), `ProviderReference` is tenant, namespace, operation (`crates/application/src/secret_provider.rs`); tenant and deployment registrations have no namespace.
2. **Blocker.** "The deployment's egress policy" does not exist: egress policy is per namespace and denies by default (`crates/application/src/egress.rs`, `crates/domain/src/egress.rs`).
3. **Blocker.** "The runner REST surface for scripts" is not a delivery path: the sandbox runner gets a script step's secrets through the worker gRPC `ResolveSecrets` (`apps/sandbox-runner/src/lib.rs`); a script node is an activity `script.<digest>`. Use the gRPC method for scripts too.
4. **Major.** The runner fixes the redaction set before the run (`secrets_to_hide`) and the `secret` host call answers from a local map; a token fetched mid-run would not be redacted.
5. **Major.** Drift from 021's revision: `oauth` scheme and `oauth_credential` slot; 021's types are code, not a table; 021's error mapping; the manage permission 021 expects.
6. **Major.** 028 places its index code in `crates/domain/src/templates.rs` and `crates/application/src/templates.rs`; entries are replaced per refresh and there is no revocation list; the release catalogue comes from `templates/catalog/`.
7. **Major.** Feature 017's provisioning needs a principal with `SecretManage`, a `SecretName` and the secrets quota; the unattended refresh has none.
8. **Major.** The instance registry knows only control planes and engines; a capability needs a variant and a CHECK migration.
9. **Major.** "Activity definitions" do not exist; secrets are declared on `activity` and `script` IR nodes (`crates/infra-ir/src/convert.rs`); `infra-ir` missing from placement.
10. **Major.** The Studio is served under `/studio/` with its OIDC callback at `/studio/auth/callback` from `location.origin`; `SAGAT_STUDIO_PUBLIC_URL` does not exist.
11. **Major.** `scripts/check-architecture.py` cannot express "only infra-oauth builds token requests"; name a mechanism and its negative test.
12. **Major.** `sdks/csharp` does not exist; it is `sdks/dotnet`.
13. **Major.** Paths not marked create or modify; missing sites (`crates/api-http/src/lib.rs`, `crates/api-grpc/src/limits.rs`, the SDK proto copies, `specs/001-platform-foundation/contracts/worker.proto` and its baseline, `storage-classification.json`); tasks name no files or tests.
14. **Nit.** `oauth-broker-v1` naming.
15. **Nit.** Protocol minor unnamed; relation to 021's `ResolveResource` unstated.
16. **Nit.** No migration timestamp rule.
