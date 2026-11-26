# Design 047, EGM stage at `4bc5767b` (2026-11-26)

The reviewer's output, condensed by the author; every finding and severity kept. Summarised and answered in `egm-review.md`.

## Codebase details (egm-codebase-details)

Checked and correct: Architecture decisions 0055 clause 11(e), 0074, 0082, 0122 and 0123; the three capability
constraints; `storage-classification.json`; the Rust string-literal scan; default-deny `egress.rs`; `infra-egress` not
following redirects; `worker.proto` and its baseline; the feature 021 and 028 cross-references; `sagat_app`.

1. Blocker. "Built like the Studio's OIDC callback from the configured Studio origin": the Studio uses
   `globalThis.location.origin` (`frontend/apps/studio/src/auth/oidc.ts:90,99`) and no Studio-origin setting exists.
2. Blocker. "The shims in `runners/`": the TypeScript and Python shims are `crates/infra-sandbox/shims/runtime.{mjs,py}`
   (`docker.rs:948-951`), and the sandbox also runs C#, Go and Java scripts (`docker.rs:339-354`).
3. Major. "After `20270119000000`": migrations up to `20270210000100` exist.
4. Major. "Feature 015's OpenBao service": OpenBao is in feature 017's `infra/docker/compose.database.yaml`, policies in
   `infra/openbao/policies/*.hcl` applied by the bootstrap scripts, tokens from `infra/helm/templates/openbao-agent.yaml`,
   the runbook `docs/runbooks/openbao.md`; `docs/runbooks/oauth-and-hub.md` is marked Modify but does not exist.
5. Major. Only feature 028's `crates/application/src/templates.rs` is listed, while 028 also has domain, store and route
   files and plans `sagat template` and Studio `features/templates`; a separate `sagat hub` and `features/hub` duplicate
   them, and Requirement 028-041 allows only workflow and script drafts.
6. Major. The ordinary-delivery checks exist in `crates/application/src/worker/secrets.rs` (`:53`, `:113`, `:146`) and the
   protocol-minor gating in `execution.rs:845` and `worker.rs:108`; the design names neither.
7. Major. An intermediate-representation field is added without `workflow.schema.json` (compiled in by
   `infra-ir/src/schema.rs:11`) or a fragment contract.
8. Nit. The five permissions are missing from `Permission::ALL` (`crates/domain/src/identity.rs:401`).
9. Nit. The limit class `resource_delivery` does not exist (`limits.rs:499-517`).
10. Nit. `crates/application` already depends on `tokio` (`rt`): "free of a runtime" is inexact.
11. Nit. Decision 047-019 says `oauth-broker`; the design says `oauth-broker-v1`.
12. Nit. `tests/support/oidc.rs`, `apps/dev-idp` and `OpenBao::retire_inner` (`openbao.rs:297-304`) are not named for reuse.

Counts: Blocker 2, Major 5, Nit 5
